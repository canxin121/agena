//! User actions and observations through the production HTTP client.
//!
//! Helpers do not supply model, prompt, permission or sampling overrides. Live
//! notifications wake readers; authoritative, revalidated HTTP snapshots decide
//! whether a condition holds. Every wait has a deadline and watches model-fixture
//! failures. A session handle belongs to one server incarnation: attach again
//! after restarting a case.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::Path,
    sync::Arc,
    time::Duration,
};

use agena_api::{
    commands::{
        Command, CommandResult, ReplyPermissionParams, ReplyUserInputParams,
        ResolveWorkspaceParams, SubmitRunParams,
    },
    part::PartResource,
    resource::{
        ExecutionAttemptResource, ExecutionAttemptSettlement, PendingInteractiveRequestKind,
        PendingInteractiveRequestResource, PermissionReply, PermissionReplyKind, PermissionScope,
        RunOptions, SessionExecutionReceiptResource, SessionExecutionResource, UserInputReply,
        UserInputReplyKind,
    },
};
use agena_client::{AgenaClient, NotificationSubscription};
use agena_domain::{
    AttachmentItem, AttachmentSource, ComposerDocument, ComposerNode,
    session_kernel::ExecutionPhase,
};
use anyhow::{Context, Result, bail, ensure};
use tokio::sync::Notify;

use crate::{case::TestCase, model::ModelHandle};

const DEFAULT_ATTACHMENT_BYTES: usize = 4 * 1024 * 1024;

/// Options for creating an actual server-owned session.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub title: String,
    pub parent_id: Option<i64>,
    /// Match the public create-session API's default. Opt in explicitly when
    /// testing title scheduling, and register its model expectations as well.
    pub auto_title: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            title: "System test".into(),
            parent_id: None,
            auto_title: false,
        }
    }
}

/// Creates or attaches user sessions in a case's current server incarnation.
#[derive(Clone)]
pub struct UserDriver {
    case: TestCase,
}

impl TestCase {
    pub fn user(&self) -> UserDriver {
        UserDriver { case: self.clone() }
    }
}

impl UserDriver {
    pub async fn new_session(&self) -> Result<UserSession> {
        self.new_session_with(SessionOptions::default()).await
    }

    pub async fn new_session_with(&self, options: SessionOptions) -> Result<UserSession> {
        self.new_session_in_with(self.case.workspace(), options)
            .await
    }

    /// Create a session in an explicit, absolute server-side directory. Agena
    /// resolves and registers the workspace through its public API; the helper
    /// does not create fixture files or change the server's default workspace.
    /// Prepare case-owned directories with `seed_file` or normal Rust file I/O.
    pub async fn new_session_in(&self, path: impl AsRef<Path>) -> Result<UserSession> {
        self.new_session_in_with(path, SessionOptions::default())
            .await
    }

    /// The same explicit workspace selection with normal session options. A
    /// relative path is rejected instead of depending on the server's cwd.
    pub async fn new_session_in_with(
        &self,
        path: impl AsRef<Path>,
        options: SessionOptions,
    ) -> Result<UserSession> {
        let path = path.as_ref();
        ensure!(
            path.is_absolute(),
            "session workspace must be an absolute server-side path"
        );
        let path = path
            .to_str()
            .context("session workspace path is not UTF-8")?;
        let client = self.case.client().await;
        let workspace = client
            .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
                path: path.to_owned(),
                create_if_missing: true,
            }))
            .await
            .with_context(|| format!("resolve session workspace {path:?} through Agena"))?;
        let CommandResult::Workspace(workspace) = workspace else {
            bail!("resolve workspace returned an unexpected command result");
        };
        let session = client
            .create_session_with_auto_title(
                workspace.id,
                options.title,
                options.parent_id,
                options.auto_title,
            )
            .await
            .context("create the user session through Agena")?;
        self.attach_session(session.id).await
    }

    /// Reattach an existing persistent session, including after a server restart.
    pub async fn attach_session(&self, session_id: i64) -> Result<UserSession> {
        let client = self.case.client().await;
        let connection = client
            .connect_session(session_id)
            .await
            .with_context(|| format!("subscribe before reading session {session_id}"))?;
        let changed = Arc::new(Notify::new());
        let case_cancel = self.case.cancellation_token();
        let server_cancel = self.case.server_cancellation_token().await;
        let wake = changed.clone();
        let observe_client = client.clone();
        self.case.spawn_task(async move {
            let mut subscription: NotificationSubscription = connection.subscription;
            loop {
                let event = tokio::select! {
                    () = case_cancel.cancelled() => break,
                    () = server_cancel.cancelled() => break,
                    event = subscription.recv() => event,
                };
                wake.notify_waiters();
                if matches!(event, Some(Ok(_))) {
                    // Lag notifications also cause a fresh snapshot. No live
                    // event is treated as a durable replay cursor.
                    continue;
                }
                subscription.close();
                loop {
                    let replacement = tokio::select! {
                        () = case_cancel.cancelled() => return Ok(()),
                        () = server_cancel.cancelled() => return Ok(()),
                        result = observe_client.stream_changes(agena_api::Scope::Session { session_id }) => result,
                    };
                    match replacement {
                        Ok(next) => { subscription = next; wake.notify_waiters(); break; }
                        Err(_) => tokio::select! {
                            () = case_cancel.cancelled() => return Ok(()),
                            () = server_cancel.cancelled() => return Ok(()),
                            () = tokio::time::sleep(Duration::from_millis(100)) => {},
                        },
                    }
                }
            }
            subscription.close();
            Ok(())
        });
        Ok(UserSession {
            client,
            session_id,
            model: self.case.model(),
            changed,
            timeout: self.case.timeout(),
        })
    }
}

/// A real user session with a live subscription installed before any message.
#[derive(Clone)]
pub struct UserSession {
    client: AgenaClient,
    session_id: i64,
    model: ModelHandle,
    changed: Arc<Notify>,
    timeout: Duration,
}

impl UserSession {
    pub fn id(&self) -> i64 {
        self.session_id
    }
    pub fn client(&self) -> &AgenaClient {
        &self.client
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Read a session-visible attachment through the production media endpoint,
    /// with a 4 MiB response-byte budget and this session handle's timeout.
    /// Opaque media references are forwarded unchanged; local paths are read by
    /// the server under its workspace/history authorization. This helper never
    /// reads the test process's filesystem or fetches an external URL.
    pub async fn download_attachment(
        &self,
        attachment: &AttachmentItem,
    ) -> Result<(String, Vec<u8>)> {
        self.download_attachment_with_limit(attachment, DEFAULT_ATTACHMENT_BYTES)
            .await
    }

    /// Choose an explicit media response budget, still capped by the production
    /// client's 100 MiB hard limit. The limit is enforced during transfer, even
    /// if attachment size metadata is absent or inaccurate. Added error context
    /// identifies the session and attachment without copying the opaque reference.
    pub async fn download_attachment_with_limit(
        &self,
        attachment: &AttachmentItem,
        max_bytes: usize,
    ) -> Result<(String, Vec<u8>)> {
        let name = attachment
            .filename
            .as_deref()
            .or(attachment.title.as_deref())
            .unwrap_or("<unnamed>");
        let description = format!(
            "download {:?} attachment {name:?} ({})",
            attachment.kind, attachment.mime
        );
        self.bounded(&description, async {
            let path = match &attachment.source {
                AttachmentSource::LocalPath { path } => path,
                AttachmentSource::Url { url } if url.starts_with("agena:media:") => url,
                AttachmentSource::Url { .. } => bail!(
                    "external attachment URLs are not fetched; expected a session media reference or server-local path"
                ),
                _ => bail!("this attachment source is not downloadable through the session media API"),
            };
            Ok(self.client.download_session_media_file_with_limit(self.session_id, path, max_bytes).await?)
        }).await
    }

    /// Ask the scripted AI to call one tool through the real provider protocol,
    /// then wait for the result to return to the model and for the run to settle.
    /// A failed tool is returned normally so the test can assert its failure.
    /// Permissions and run options are not changed by this convenience helper.
    pub async fn call_tool(
        &self,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Result<crate::tool::ToolResult> {
        self.start_tool(name, input).await?.wait_settled().await
    }

    /// Start a scripted tool round trip without waiting, allowing the caller to
    /// answer permission or user-input requests through the returned run handle.
    pub async fn start_tool(
        &self,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Result<crate::tool::ToolRun> {
        self.start_tool_with(name, input, RunOptions::default())
            .await
    }

    pub async fn start_tool_with(
        &self,
        name: impl Into<String>,
        input: serde_json::Value,
        options: RunOptions,
    ) -> Result<crate::tool::ToolRun> {
        crate::tool::start(
            self,
            self.model.clone(),
            self.timeout,
            name.into(),
            input,
            options,
        )
        .await
    }

    pub async fn send(&self, text: impl Into<String>) -> Result<RunHandle> {
        self.send_document(
            ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
            RunOptions::default(),
        )
        .await
    }

    /// All run options are explicit. An omitted option stays omitted so real
    /// configuration, session selection and default inheritance remain testable.
    pub async fn send_with(
        &self,
        text: impl Into<String>,
        options: RunOptions,
    ) -> Result<RunHandle> {
        self.send_document(
            ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
            options,
        )
        .await
    }

    /// Also supports real attachments and composer activities constructed by a test.
    pub async fn send_document(
        &self,
        document: ComposerDocument,
        options: RunOptions,
    ) -> Result<RunHandle> {
        let response = self
            .bounded("accept user message", async {
                Ok(self
                    .client
                    .submit_message(SubmitRunParams {
                        session_id: self.session_id,
                        options,
                        document,
                    })
                    .await?)
            })
            .await?;
        let receipt = response
            .receipt
            .context("accepted message has no execution receipt")?;
        Ok(self.attach_run(receipt))
    }

    pub async fn continue_run(&self, options: RunOptions) -> Result<RunHandle> {
        let response = self
            .bounded("accept continuation", async {
                Ok(self.client.continue_run(self.session_id, options).await?)
            })
            .await?;
        Ok(self.attach_run(
            response
                .receipt
                .context("continuation has no execution receipt")?,
        ))
    }

    pub fn attach_run(&self, receipt: SessionExecutionReceiptResource) -> RunHandle {
        RunHandle {
            user: self.clone(),
            receipt,
        }
    }

    pub async fn snapshot(&self) -> Result<SessionExecutionResource> {
        self.bounded("read session state", async {
            Ok(self
                .client
                .get_session_state_validated(self.session_id, true)
                .await?)
        })
        .await
    }

    pub async fn parts(&self) -> Result<Vec<PartResource>> {
        self.bounded("read complete paginated history", async {
            Ok(self.client.session_all_parts(self.session_id).await?)
        })
        .await
    }

    /// Wait for an application-level condition without assuming an idle session
    /// means all runs, hooks, agents or background work have finished.
    pub async fn wait_state(
        &self,
        description: &str,
        predicate: impl Fn(&SessionExecutionResource) -> bool,
    ) -> Result<SessionExecutionResource> {
        self.bounded(description, async {
            loop {
                let state = self
                    .client
                    .get_session_state_validated(self.session_id, true)
                    .await?;
                if predicate(&state) {
                    return Ok(state);
                }
                self.changed_or_poll().await;
            }
        })
        .await
    }

    pub async fn wait_permission(&self) -> Result<PendingInteraction> {
        self.wait_interaction(PendingInteractiveRequestKind::Permission, None)
            .await
    }

    pub async fn wait_user_input(&self) -> Result<PendingInteraction> {
        self.wait_interaction(PendingInteractiveRequestKind::UserInput, None)
            .await
    }

    async fn wait_interaction(
        &self,
        kind: PendingInteractiveRequestKind,
        receipt: Option<SessionExecutionReceiptResource>,
    ) -> Result<PendingInteraction> {
        self.bounded("wait for a pending user interaction", async {
            loop {
                let attempt = if let Some(receipt) = &receipt {
                    Some(self.ensure_current_attempt(receipt).await?)
                } else {
                    None
                };
                let state = self
                    .client
                    .get_session_state_validated(self.session_id, true)
                    .await?;
                let target_active = receipt.as_ref().is_none_or(|receipt| {
                    state
                        .session
                        .state
                        .active_execution()
                        .is_some_and(|active| active.execution_id == receipt.execution_id)
                });
                if target_active
                    && let Some(request) = state
                        .session
                        .state
                        .pending_interactive_requests()
                        .iter()
                        .find(|pending| pending.request.kind() == kind)
                        .cloned()
                {
                    // The attempt can advance while its session snapshot is
                    // being read. Revalidate before returning this specific
                    // request ID; never replace it with a newer pending ID.
                    if let Some(receipt) = &receipt {
                        self.ensure_current_attempt(receipt).await?;
                    }
                    return Ok(PendingInteraction {
                        user: self.clone(),
                        request,
                        receipt,
                    });
                }
                if let Some(attempt) = &attempt
                    && attempt.settlement == ExecutionAttemptSettlement::Terminal
                    && state.session.state.active_execution().is_none()
                    && let Some(pending) = state.session.state.pending_interactive_requests()
                        .iter().find(|pending| pending.request.kind() == kind)
                {
                    let helper = match kind {
                        PendingInteractiveRequestKind::Permission => "wait_permission",
                        PendingInteractiveRequestKind::UserInput => "wait_user_input",
                    };
                    bail!(
                        "attempt {} generation {} is terminal while its supervisor is still retiring; pending request {:?} has no active execution identity in the session projection. Use UserSession::{helper}() to handle the session-level request by its fixed request_id",
                        attempt.execution_id, attempt.attempt_generation, pending.request.request_id()
                    );
                }
                self.changed_or_poll().await;
            }
        })
        .await
    }

    async fn ensure_current_attempt(
        &self,
        receipt: &SessionExecutionReceiptResource,
    ) -> Result<ExecutionAttemptResource> {
        let status = self
            .client
            .execution_attempt(
                self.session_id,
                receipt.execution_id,
                receipt.attempt_generation,
            )
            .await?;
        ensure!(
            status.session_id == self.session_id
                && status.execution_id == receipt.execution_id
                && status.message_id == receipt.message_id
                && status.attempt_generation == receipt.attempt_generation,
            "execution attempt endpoint returned a different identity: {status:?}"
        );
        ensure!(
            status.current_attempt_generation == receipt.attempt_generation
                && !matches!(
                    status.settlement,
                    ExecutionAttemptSettlement::Superseded | ExecutionAttemptSettlement::Recovered
                ),
            "attempt {} generation {} no longer owns user interactions: {status:?}",
            receipt.execution_id,
            receipt.attempt_generation
        );
        ensure!(
            !(status.phase.is_terminal()
                && status.settlement == ExecutionAttemptSettlement::Settled),
            "attempt {} generation {} ended before the interaction could be handled: {:?}",
            receipt.execution_id,
            receipt.attempt_generation,
            status.phase
        );
        Ok(status)
    }

    pub async fn assistant_text(&self) -> Result<String> {
        self.bounded("read full assistant text", async {
            let parts = self.client.session_all_parts(self.session_id).await?;
            read_assistant_text(&self.client, self.session_id, &parts).await
        })
        .await
    }

    pub async fn wait_text_contains(&self, expected: &str) -> Result<String> {
        self.bounded("wait for visible assistant text", async {
            loop {
                let parts = self.client.session_all_parts(self.session_id).await?;
                let text = read_assistant_text(&self.client, self.session_id, &parts).await?;
                if text.contains(expected) {
                    return Ok(text);
                }
                self.changed_or_poll().await;
            }
        })
        .await
    }

    async fn changed_or_poll(&self) {
        // Polling is a bounded fallback for best-effort notifications, not an
        // assumption that a fixed delay makes the application ready.
        tokio::select! {
            () = self.changed.notified() => {},
            () = tokio::time::sleep(Duration::from_millis(50)) => {},
        }
    }

    async fn bounded<T>(
        &self,
        description: &str,
        operation: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let result = tokio::select! {
            error = self.model.failed() => Err(error).context("scripted model failed while observing Agena"),
            result = tokio::time::timeout(self.timeout, operation) => {
                result.with_context(|| format!("session {} timed out after {:?}: {description}", self.session_id, self.timeout))?
            }
        };
        result.with_context(|| format!("session {}: {description}", self.session_id))
    }
}

/// Identity of one accepted attempt, not the session's latest execution.
#[derive(Clone)]
pub struct RunHandle {
    user: UserSession,
    receipt: SessionExecutionReceiptResource,
}

impl RunHandle {
    pub fn receipt(&self) -> &SessionExecutionReceiptResource {
        &self.receipt
    }
    pub fn session_id(&self) -> i64 {
        self.user.id()
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.user.timeout = timeout;
        self
    }

    pub async fn status(&self) -> Result<ExecutionAttemptResource> {
        self.user
            .bounded("read exact execution attempt", async {
                Ok(self
                    .user
                    .client
                    .execution_attempt(
                        self.user.session_id,
                        self.receipt.execution_id,
                        self.receipt.attempt_generation,
                    )
                    .await?)
            })
            .await
    }

    pub async fn wait_terminal(&self) -> Result<RunResult> {
        self.wait(false).await
    }

    /// Wait for this generation's supervisor and run.post to finish. This does
    /// not mean unrelated scheduled work or automatic titles have stopped.
    /// A recovered or superseded attempt is deliberately not called settled.
    pub async fn wait_settled(&self) -> Result<RunResult> {
        self.wait(true).await
    }

    async fn wait(&self, settled: bool) -> Result<RunResult> {
        self.user
            .bounded(
                if settled {
                    "wait for exact attempt settlement"
                } else {
                    "wait for exact attempt terminal state"
                },
                async {
                    loop {
                        let status = self
                            .user
                            .client
                            .execution_attempt(
                                self.user.session_id,
                                self.receipt.execution_id,
                                self.receipt.attempt_generation,
                            )
                            .await?;
                        if settled
                            && matches!(
                                status.settlement,
                                ExecutionAttemptSettlement::Recovered
                                    | ExecutionAttemptSettlement::Superseded
                            )
                        {
                            bail!(
                                "attempt {} generation {} cannot normally settle: {:?} ({:?})",
                                self.receipt.execution_id,
                                self.receipt.attempt_generation,
                                status.settlement,
                                status.phase
                            );
                        }
                        if status.phase.is_terminal()
                            && (!settled
                                || status.settlement == ExecutionAttemptSettlement::Settled)
                        {
                            let all = self
                                .user
                                .client
                                .session_all_parts(self.user.session_id)
                                .await?;
                            let parts = attempt_parts(all, &self.receipt);
                            let assistant_text = read_assistant_text(
                                &self.user.client,
                                self.user.session_id,
                                &parts,
                            )
                            .await?;
                            return Ok(RunResult {
                                status,
                                parts,
                                assistant_text,
                            });
                        }
                        self.user.changed_or_poll().await;
                    }
                },
            )
            .await
    }

    pub async fn cancel(&self) -> Result<agena_domain::CancellationOutcome> {
        self.user
            .bounded("cancel the exact execution attempt", async {
                Ok(self
                    .user
                    .client
                    .cancel_execution_attempt(
                        self.user.session_id,
                        self.receipt.execution_id,
                        self.receipt.attempt_generation,
                    )
                    .await?)
            })
            .await
    }

    pub async fn wait_permission(&self) -> Result<PendingInteraction> {
        self.user
            .wait_interaction(
                PendingInteractiveRequestKind::Permission,
                Some(self.receipt),
            )
            .await
    }

    pub async fn wait_user_input(&self) -> Result<PendingInteraction> {
        self.user
            .wait_interaction(PendingInteractiveRequestKind::UserInput, Some(self.receipt))
            .await
    }

    pub async fn wait_text_contains(&self, expected: &str) -> Result<String> {
        self.user
            .bounded("wait for this attempt's visible text", async {
                loop {
                    let all = self
                        .user
                        .client
                        .session_all_parts(self.user.session_id)
                        .await?;
                    let parts = attempt_parts(all, &self.receipt);
                    let text = read_assistant_text(&self.user.client, self.user.session_id, &parts)
                        .await?;
                    if text.contains(expected) {
                        return Ok(text);
                    }
                    self.user.changed_or_poll().await;
                }
            })
            .await
    }
}

/// Durable attempt status and the full visible text read through content APIs.
#[derive(Debug)]
pub struct RunResult {
    pub status: ExecutionAttemptResource,
    pub parts: Vec<PartResource>,
    pub assistant_text: String,
}

impl RunResult {
    pub fn assert_succeeded(&self) -> Result<()> {
        ensure!(
            self.status.phase == ExecutionPhase::Succeeded,
            "expected success, got {:?}; parts: {:?}",
            self.status,
            self.parts
        );
        Ok(())
    }
    pub fn assert_cancelled(&self) -> Result<()> {
        ensure!(
            self.status.phase == ExecutionPhase::Cancelled,
            "expected cancellation, got {:?}",
            self.status
        );
        Ok(())
    }
    pub fn assert_failed(&self) -> Result<()> {
        ensure!(
            self.status.phase == ExecutionPhase::Failed,
            "expected failure, got {:?}",
            self.status
        );
        Ok(())
    }
    pub fn assert_assistant_text(&self, expected: &str) -> Result<()> {
        ensure!(
            self.assistant_text == expected,
            "assistant text differs\nexpected: {expected:?}\nactual: {:?}",
            self.assistant_text
        );
        Ok(())
    }
    pub fn assert_text_contains(&self, expected: &str) -> Result<()> {
        ensure!(
            self.assistant_text.contains(expected),
            "assistant text {:?} does not contain {expected:?}",
            self.assistant_text
        );
        Ok(())
    }
}

/// A server-issued interaction. Replies preserve its actual owning session,
/// including requests surfaced from a child agent.
#[derive(Clone)]
pub struct PendingInteraction {
    user: UserSession,
    receipt: Option<SessionExecutionReceiptResource>,
    pub request: PendingInteractiveRequestResource,
}

impl PendingInteraction {
    pub fn id(&self) -> &str {
        self.request.request.request_id()
    }
    pub fn session_id(&self) -> i64 {
        self.request.session_id
    }

    pub async fn mark_presented(&self) -> Result<SessionExecutionResource> {
        self.bounded("mark the exact interaction as presented", async {
            Ok(self
                .user
                .client
                .mark_interactive_request_presented(self.session_id(), self.id())
                .await?)
        })
        .await
    }

    pub async fn reply_permission(
        &self,
        kind: PermissionReplyKind,
        reason: Option<String>,
        scope: Option<PermissionScope>,
    ) -> Result<SessionExecutionResource> {
        ensure!(
            self.request.request.kind() == PendingInteractiveRequestKind::Permission,
            "interaction {} is not a permission request",
            self.id()
        );
        self.bounded("reply to the exact permission request", async {
            Ok(self
                .user
                .client
                .reply_permission(ReplyPermissionParams {
                    session_id: self.session_id(),
                    options: RunOptions::default(),
                    reply: PermissionReply {
                        request_id: self.id().to_owned(),
                        kind,
                        reason,
                        scope,
                    },
                })
                .await?)
        })
        .await
    }

    pub async fn allow_once(&self) -> Result<SessionExecutionResource> {
        self.reply_permission(PermissionReplyKind::AllowOnce, None, None)
            .await
    }
    pub async fn deny_once(&self) -> Result<SessionExecutionResource> {
        self.reply_permission(PermissionReplyKind::DenyOnce, None, None)
            .await
    }

    pub async fn reply_user_input(
        &self,
        kind: UserInputReplyKind,
        answers: BTreeMap<String, Vec<String>>,
        reason: Option<String>,
    ) -> Result<SessionExecutionResource> {
        ensure!(
            self.request.request.kind() == PendingInteractiveRequestKind::UserInput,
            "interaction {} is not a user-input request",
            self.id()
        );
        self.bounded("reply to the exact user-input request", async {
            Ok(self
                .user
                .client
                .reply_user_input(ReplyUserInputParams {
                    session_id: self.session_id(),
                    options: RunOptions::default(),
                    reply: UserInputReply {
                        request_id: self.id().to_owned(),
                        kind,
                        answers,
                        reason,
                    },
                })
                .await?)
        })
        .await
    }

    pub async fn answer(
        &self,
        answers: BTreeMap<String, Vec<String>>,
    ) -> Result<SessionExecutionResource> {
        self.reply_user_input(UserInputReplyKind::Submit, answers, None)
            .await
    }

    async fn bounded<T>(
        &self,
        description: &str,
        operation: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        self.user
            .bounded(description, async {
                if let Some(receipt) = &self.receipt {
                    self.user.ensure_current_attempt(receipt).await?;
                }
                operation.await
            })
            .await
    }
}

fn attempt_parts(
    parts: Vec<PartResource>,
    receipt: &SessionExecutionReceiptResource,
) -> Vec<PartResource> {
    // Each attempt's assistant boundary is a stable run_id for its tools and
    // content. The user entry itself belongs to generation one even after retry.
    let execution_id = receipt.execution_id.to_string();
    let run_ids: BTreeSet<i64> = parts
        .iter()
        .filter(|part| {
            part.role == "assistant"
                && part.message_id == Some(receipt.message_id)
                && part
                    .content
                    .get("execution_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| id == execution_id)
                && part
                    .content
                    .get("generation")
                    .and_then(serde_json::Value::as_i64)
                    == Some(receipt.attempt_generation)
        })
        .map(|part| part.run_id.unwrap_or(part.part_id))
        .collect();
    parts
        .into_iter()
        .filter(|part| {
            if part.role == "user" {
                return false;
            }
            if part.message_id != Some(receipt.message_id) {
                return false;
            }
            if let Some(generation) = part
                .content
                .get("generation")
                .and_then(serde_json::Value::as_i64)
            {
                return generation == receipt.attempt_generation
                    && part
                        .content
                        .get("execution_id")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|id| id == execution_id);
            }
            part.run_id.is_some_and(|run_id| run_ids.contains(&run_id))
        })
        .collect()
}

async fn read_assistant_text(
    client: &AgenaClient,
    session_id: i64,
    parts: &[PartResource],
) -> Result<String> {
    let mut bytes = Vec::new();
    for part in parts
        .iter()
        .filter(|part| part.role == "assistant" && part.kind == "text")
    {
        client
            .write_part_text(session_id, part, &mut bytes)
            .await
            .with_context(|| format!("read complete text for part {}", part.part_id))?;
    }
    String::from_utf8(bytes).context("server returned non-UTF-8 assistant text")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn attempt_parts_reject_other_executions_and_keep_correlated_tools() {
        let execution_id = uuid::Uuid::from_u128(1);
        let receipt = SessionExecutionReceiptResource {
            execution_id,
            message_id: 10,
            attempt_generation: 2,
            turn_id: uuid::Uuid::from_u128(2),
            reply_id: uuid::Uuid::from_u128(3),
        };
        let part = |part_id, run_id, message_id, kind: &str, content| PartResource {
            part_id,
            run_id: Some(run_id),
            message_id: Some(message_id),
            role: "assistant".into(),
            kind: kind.into(),
            content,
            ..PartResource::default()
        };
        let parts = vec![
            part(
                1,
                20,
                10,
                "text",
                json!({"execution_id": execution_id, "generation": 2}),
            ),
            // Same message and generation are insufficient without execution identity.
            part(
                2,
                21,
                10,
                "text",
                json!({"execution_id": uuid::Uuid::from_u128(99), "generation": 2}),
            ),
            part(
                3,
                22,
                10,
                "text",
                json!({"execution_id": execution_id, "generation": 1}),
            ),
            // Tool/error projections omit attempt metadata and inherit the actual run ID.
            part(4, 20, 10, "tool_call", json!({"name": "test.echo"})),
            part(5, 21, 10, "tool_call", json!({"name": "other.echo"})),
            part(
                6,
                20,
                11,
                "text",
                json!({"execution_id": execution_id, "generation": 2}),
            ),
        ];
        let selected = attempt_parts(parts, &receipt);
        assert_eq!(
            selected.iter().map(|part| part.part_id).collect::<Vec<_>>(),
            [1, 4]
        );
    }
}
