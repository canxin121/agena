use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Weak},
    time::Duration,
};

use agena_domain::{
    ExternalAgent, ExternalAgentAuthStatus, ExternalAgentOption, ExternalAgentOptionKind,
};
use agena_process::ManagedChild;
use agena_provider::{CompletionUsage, ProviderErrorKind};
use agena_storage::kernel::ExternalAgentNativeCursor;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::{
    io::AsyncWriteExt,
    process::{ChildStdin, ChildStdout, Command},
    sync::mpsc,
    task::{JoinError, JoinSet},
    time::Instant,
};
use tokio_util::{
    codec::{FramedRead, LinesCodec},
    sync::CancellationToken,
};

use super::{
    AppError, ExternalAgentState, ExternalTurn, cancellation, client_tools,
    elicitation::elicitation_id,
    input_requests, mcp,
    options::config_options,
    prompt::PromptCapabilities,
    steering,
    transcript::message::{MessageTarget, MessageUpdate, NativeMessages},
};

mod antigravity;
mod cursors;
mod inactivity;
mod index;
mod isolation;
mod lifecycle;
mod policy;
mod protocol;
mod providers;
mod startup_options;

use inactivity::PromptIdle;
pub(super) use index::MAX_BATCH_IDS;
pub(super) use lifecycle::{SessionContext, bounded_string, bounded_title};
use protocol::{ActivitySnapshot, PromptLifecycle, Version};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const CANCELLATION_GRACE: Duration = Duration::from_millis(500);
const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const MAX_OPENING_SESSIONS: usize = 8;
const MAX_OPENING_MESSAGE_UPDATES: usize = 256;
const MAX_REPLAYED_USERS: usize = 6_000;
const MAX_PEER_REQUESTS: usize = 128;
pub(super) const MAX_CURSOR_BYTES: usize = 4096;

type PeerReplies = JoinSet<Result<(Value, Reply), AppError>>;

async fn abandoned_steering(requests: &mut HashMap<u64, steering::Request>) -> u64 {
    std::future::poll_fn(|cx| {
        for (&id, request) in &mut *requests {
            if request.answer.poll_closed(cx).is_ready() {
                return std::task::Poll::Ready(id);
            }
        }
        std::task::Poll::Pending
    })
    .await
}

pub(super) struct AcpClient {
    agent: ExternalAgent,
    cwd: PathBuf,
    additional_directories: Vec<PathBuf>,
    previous_antigravity_history: bool,
    agent_arguments: Vec<String>,
    isolation: Option<isolation::Isolation>,
    child: ManagedChild,
    input: ChildStdin,
    output: FramedRead<ChildStdout, LinesCodec>,
    next_id: u64,
    active_request: Option<ActiveRequest>,
    incomplete_write: bool,
    terminal_auth: bool,
    live_auth: Option<super::auth::LiveStatus>,
    reported_auth: Option<ExternalAgentAuthStatus>,
    air_index: bool,
    plain_delete: bool,
    providers_prepared: bool,
    capabilities: Option<Capabilities>,
    initialization: Option<Value>,
    mcp: Option<mcp::AcpEndpoint>,
    mcp_activity: Option<mcp::ToolProgress>,
    native_work: Option<cancellation::Connection>,
    opening_turn: Weak<ExternalTurn>,
    opening_replies: PeerReplies,
    pending: HashMap<Value, PendingInteraction>,
    url_elicitations: HashMap<String, Instant>,
}

struct ActiveRequest {
    id: u64,
    awaiting_reply: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InteractionKind {
    Permission,
    Form,
    Mcp,
    ClientTool(client_tools::Operation),
}

impl InteractionKind {
    fn cancelled(self) -> Reply {
        Reply::Result(match self {
            Self::Permission => json!({"outcome": {"outcome": "cancelled"}}),
            Self::Form => json!({"action": "cancel"}),
            Self::Mcp => return Reply::Error(-32800, "MCP request cancelled"),
            Self::ClientTool(_) => return Reply::Error(-32800, "ACP client request cancelled"),
        })
    }

    async fn request(
        self,
        turn: &ExternalTurn,
        params: &Value,
        withdrawal: &CancellationToken,
    ) -> Result<Reply, AppError> {
        match self {
            Self::Permission => Ok(Reply::Result(
                turn.request_permission(params, withdrawal).await?,
            )),
            Self::Form => Ok(turn.request_elicitation(params, withdrawal).await?.map_or(
                Reply::Error(-32602, "Invalid or unsupported ACP elicitation request"),
                Reply::Result,
            )),
            Self::Mcp => unreachable!("MCP requests use the registered endpoint"),
            Self::ClientTool(operation) => {
                Ok(match operation.request(turn, params, withdrawal).await {
                    Ok(result) => Reply::Result(result),
                    Err(AppError::Cancelled) => self.cancelled(),
                    Err(error) => {
                        tracing::debug!(%error, "ACP client operation failed");
                        Reply::Error(
                            -32603,
                            "The requested file or terminal operation could not be completed",
                        )
                    }
                })
            }
        }
    }
}

struct PendingInteraction {
    kind: InteractionKind,
    withdrawal: CancellationToken,
    mcp_request: Option<String>,
    url_elicitation: Option<String>,
}

type OpeningStates = HashMap<String, OpeningState>;

#[derive(Default)]
struct OpeningState {
    state: ExternalAgentState,
    activity: ActivitySnapshot,
}

impl OpeningState {
    fn observe(&mut self, root: &str, source: &str, update: &Value) -> bool {
        let current_activity = root == source && self.activity.observe(update);
        let changed = super::state::apply_native_update(&mut self.state, root, source, update);
        current_activity || changed
    }
}

#[derive(Default)]
pub(super) struct OpeningMessages {
    frames: VecDeque<(Value, usize)>,
    bytes: usize,
    snapshots_only: bool,
    resume_rejection: Option<ResumeRejection>,
    replayed_users: Option<ReplayedUsers>,
}

#[derive(Default)]
pub(super) struct ReplayedUsers {
    ids: VecDeque<[u8; 32]>,
    complete: bool,
}

impl ReplayedUsers {
    fn record(&mut self, update: &Value) {
        match update["sessionUpdate"].as_str() {
            Some("replay_complete") => self.complete = true,
            Some("user_message" | "user_message_chunk") if !self.complete => {
                if let Some(id) = update["messageId"]
                    .as_str()
                    .filter(|id| id.len() <= MAX_CURSOR_BYTES)
                {
                    use sha2::{Digest, Sha256};

                    let digest = Sha256::digest(id.as_bytes()).into();
                    if self.ids.back() != Some(&digest) {
                        if self.ids.len() == MAX_REPLAYED_USERS {
                            self.ids.pop_front();
                        }
                        self.ids.push_back(digest);
                    }
                }
            }
            _ => {}
        }
    }

    fn contains(&self, id: &str) -> bool {
        use sha2::{Digest, Sha256};

        self.ids.contains(&Sha256::digest(id.as_bytes()).into())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OpenPurpose {
    Prompt,
    RetryPrompt,
    Compact,
}

#[derive(Clone, Copy)]
enum ResumeRejection {
    Missing,
    Unsupported,
}

impl OpeningMessages {
    fn record(&mut self, update: Value, bytes: usize) {
        if let Some(replay) = &mut self.replayed_users
            && !replay.complete
        {
            replay.record(&update);
            return;
        }
        let Some(message) = MessageUpdate::parse(&update) else {
            return;
        };
        if message.snapshot {
            self.retain(|old| old.id != message.id || old.reasoning != message.reasoning);
        }
        if self.frames.len() >= MAX_OPENING_MESSAGE_UPDATES || self.bytes + bytes > MAX_FRAME_BYTES
        {
            // Keep complete replacements if the catch-up buffer fills. A
            // suffix of chunks could be missing content from the same message.
            self.snapshots_only = true;
            self.retain(|message| message.snapshot);
        }
        if self.snapshots_only && !message.snapshot {
            return;
        }
        while self.frames.len() >= MAX_OPENING_MESSAGE_UPDATES
            || self.bytes + bytes > MAX_FRAME_BYTES
        {
            let Some((_, old)) = self.frames.pop_front() else {
                return;
            };
            self.bytes -= old;
        }
        self.frames.push_back((update, bytes));
        self.bytes += bytes;
    }

    fn retain(&mut self, mut keep: impl FnMut(MessageUpdate<'_>) -> bool) {
        let mut removed = 0;
        self.frames.retain(|(frame, bytes)| {
            let retained = MessageUpdate::parse(frame).is_some_and(&mut keep);
            if !retained {
                removed += bytes;
            }
            retained
        });
        self.bytes -= removed;
    }

    pub(super) fn into_updates(self) -> Vec<Value> {
        self.frames.into_iter().map(|(update, _)| update).collect()
    }

    pub(super) fn take_replayed_users(&mut self) -> Option<ReplayedUsers> {
        self.replayed_users.take()
    }
}

enum SessionUpdates<'a> {
    Ignore,
    RequestInput(&'a input_requests::Host),
    IndexChanges(&'a mut index::PendingChanges),
    Opening(
        &'a mut OpeningStates,
        Option<(&'a str, &'a mut OpeningMessages)>,
    ),
    Replay(&'a mut lifecycle::Replay, &'a str),
    Turn {
        turn: &'a Arc<ExternalTurn>,
        session: &'a str,
        steering: Option<(&'static str, mpsc::Receiver<steering::Request>)>,
        lifecycle: Option<&'a mut PromptLifecycle>,
    },
}

enum Reply {
    Result(Value),
    Error(i64, &'static str),
}

impl Reply {
    fn message(self, id: &Value) -> Value {
        match self {
            Self::Result(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Self::Error(code, message) => json!({
                "jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}
            }),
        }
    }
}

#[derive(Clone, Copy)]
struct Capabilities {
    version: Version,
    auth_status: bool,
    air_tasks: bool,
    air_rewind: bool,
    http_mcp: bool,
    acp_mcp: bool,
    sse_mcp: bool,
    stdio_mcp: bool,
    image: bool,
    audio: bool,
    embedded_context: bool,
    skill: bool,
    skill_inline: bool,
    load: bool,
    resume: bool,
    close: bool,
    delete: bool,
    options: bool,
    lifecycle: lifecycle::LifecycleCapabilities,
}

pub(super) struct OpenedSession {
    pub(super) id: String,
    pub(super) resumed: bool,
    pub(super) missing_previous: bool,
    pub(super) options: Vec<ExternalAgentOption>,
    pub(super) state: ExternalAgentState,
    pub(super) activity: ActivitySnapshot,
    pub(super) replacements: OpeningMessages,
    pub(super) close_supported: bool,
    delete_supported: bool,
}

pub(super) struct PromptResult {
    pub(super) stop_reason: Option<String>,
    pub(super) usage: Option<CompletionUsage>,
    pub(super) fork_cursor: Option<ExternalAgentNativeCursor>,
}

impl OpenedSession {
    pub(super) fn offers(&self, kind: &ExternalAgentOptionKind, value: &str) -> bool {
        self.options.iter().any(|option| {
            &option.kind == kind && option.choices.iter().any(|choice| choice.value == value)
        })
    }
}

fn agent_environment(command: &mut Command) {
    // Preserve the user's native configuration and network settings. Do
    // not give a coding agent the Agena server's environment or credentials.
    command
        .env_clear()
        .envs(std::env::vars_os().filter(|(key, _)| {
            let key = key.to_string_lossy();
            matches!(
                key.to_ascii_uppercase().as_str(),
                "PATH"
                    | "HOME"
                    | "USER"
                    | "LOGNAME"
                    | "SHELL"
                    | "LANG"
                    | "LANGUAGE"
                    | "TZ"
                    | "TMPDIR"
                    | "TMP"
                    | "TEMP"
                    | "SYSTEMROOT"
                    | "WINDIR"
                    | "COMSPEC"
                    | "PATHEXT"
                    | "USERPROFILE"
                    | "APPDATA"
                    | "LOCALAPPDATA"
                    | "HOMEDRIVE"
                    | "HOMEPATH"
                    | "SSL_CERT_FILE"
                    | "SSL_CERT_DIR"
                    | "NODE_EXTRA_CA_CERTS"
                    | "HTTP_PROXY"
                    | "HTTPS_PROXY"
                    | "ALL_PROXY"
                    | "NO_PROXY"
                    | "CLAUDE_CONFIG_DIR"
                    | "DBUS_SESSION_BUS_ADDRESS"
            ) || key.starts_with("XDG_")
                || key.starts_with("LC_")
        }));
    let no_proxy = std::env::var("no_proxy")
        .or_else(|_| std::env::var("NO_PROXY"))
        .unwrap_or_default();
    let no_proxy = format!("{no_proxy},127.0.0.1,localhost,::1");
    command
        .env("NO_PROXY", &no_proxy)
        .env("no_proxy", &no_proxy);
}

impl AcpClient {
    pub(super) fn spawn(agent: ExternalAgent, cwd: &Path) -> Result<Self, AppError> {
        Self::spawn_with_directories(agent, cwd, &[])
    }

    pub(super) fn spawn_with_directories(
        agent: ExternalAgent,
        cwd: &Path,
        directories: &[PathBuf],
    ) -> Result<Self, AppError> {
        Self::spawn_profile(agent, cwd, false, directories)
    }

    pub(super) fn spawn_profile(
        agent: ExternalAgent,
        cwd: &Path,
        previous_antigravity_history: bool,
        directories: &[PathBuf],
    ) -> Result<Self, AppError> {
        let (program, arguments) = agent.command();
        let agent_arguments = match agent {
            ExternalAgent::Antigravity => {
                policy::antigravity_arguments(previous_antigravity_history)?
            }
            ExternalAgent::ClaudeCode => Vec::new(),
        };
        let mut command = Command::new(program);
        command.args(arguments).args(&agent_arguments);
        agent_environment(&mut command);
        command.current_dir(cwd);
        let isolation = isolation::wrap(
            &mut command,
            agent,
            cwd,
            directories,
            previous_antigravity_history,
            None,
        )?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = agena_process::spawn(command).map_err(|error| {
            let setup = match agent {
                ExternalAgent::Antigravity => "Install the official Antigravity CLI with ACP support and sign in from the ACP session.",
                ExternalAgent::ClaudeCode => "Install Claude Code and the claude-agent-acp adapter on the server's PATH, then sign in using claude.",
            };
            AppError::Config(format!("Could not start {}: {error}. {setup}", agent.label()))
        })?;
        let input = child
            .stdin()
            .take()
            .ok_or_else(|| AppError::Internal("external agent did not provide stdin".into()))?;
        let output = child
            .stdout()
            .take()
            .ok_or_else(|| AppError::Internal("external agent did not provide stdout".into()))?;
        Ok(Self {
            agent,
            cwd: cwd.to_owned(),
            additional_directories: directories.to_vec(),
            previous_antigravity_history,
            agent_arguments,
            isolation,
            child,
            input,
            output: FramedRead::new(output, LinesCodec::new_with_max_length(MAX_FRAME_BYTES)),
            next_id: 0,
            active_request: None,
            incomplete_write: false,
            terminal_auth: false,
            live_auth: None,
            reported_auth: None,
            air_index: false,
            plain_delete: false,
            providers_prepared: false,
            capabilities: None,
            initialization: None,
            mcp: None,
            mcp_activity: None,
            native_work: None,
            opening_turn: Weak::new(),
            opening_replies: JoinSet::new(),
            pending: HashMap::new(),
            url_elicitations: HashMap::new(),
        })
    }

    pub(super) fn use_plain_delete(&mut self) {
        self.plain_delete = true;
    }

    async fn initialize(&mut self, cancel: &CancellationToken) -> Result<Capabilities, AppError> {
        if let Some(capabilities) = self.capabilities {
            return Ok(capabilities);
        }
        let mut requested = if self.air_index {
            // Claude exposes the AIR session index extension on v1 only.
            Version::V1
        } else {
            Version::V2
        };
        let (initialized, version) = loop {
            let mut params = requested.initialize_params(self.agent, self.terminal_auth);
            if requested == Version::V1
                && self.agent == ExternalAgent::ClaudeCode
                && !self.air_index
                && !self.plain_delete
            {
                protocol::add_air_capability(&mut params, cursors::REWIND_CAPABILITY);
                if self.live_auth.is_some() {
                    protocol::add_air_capability(&mut params, cancellation::ASYNC_TASKS_CAPABILITY);
                }
            }
            if self.air_index {
                protocol::add_air_capability(&mut params, "sessionIndex");
            }
            let response = self
                .request_for(
                    STARTUP_TIMEOUT,
                    "initialize",
                    params,
                    SessionUpdates::Ignore,
                    cancel,
                )
                .await
                .unwrap_or_else(|_| {
                    Err(AppError::Config(format!(
                        "{} did not answer ACP initialization.",
                        self.agent.label(),
                    )))
                });
            if requested == Version::V2 {
                match &response {
                    Ok(value) if requested.negotiated(value) == Some(Version::V2) => {}
                    Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                    _ => {
                        // An older agent may select v1 or reject the v2 request.
                        // Open a fresh connection with the v1 client capabilities.
                        if cancel.is_cancelled() {
                            return Err(AppError::Cancelled);
                        }
                        let mut replacement = Self::spawn_profile(
                            self.agent,
                            &self.cwd,
                            self.previous_antigravity_history,
                            &self.additional_directories,
                        )?;
                        replacement.terminal_auth = self.terminal_auth;
                        replacement.plain_delete = self.plain_delete;
                        replacement.live_auth = self.live_auth.take();
                        std::mem::replace(self, replacement).close().await;
                        requested = Version::V1;
                        continue;
                    }
                }
            }
            let value = response?;
            let version = requested.negotiated(&value).ok_or_else(|| {
                AppError::Config(format!(
                    "{} selected an unsupported ACP protocol version.",
                    self.agent.label()
                ))
            })?;
            break (value, version);
        };
        let v2 = version == Version::V2;
        let v1 = &initialized["agentCapabilities"];
        let v2_session = &initialized["capabilities"]["session"];
        if v2 && !v2_session.is_object() {
            return Err(AppError::Config(format!(
                "{} does not provide ACP sessions.",
                self.agent.label()
            )));
        }
        let (session, mcp, prompt) = if v2 {
            (v2_session, &v2_session["mcp"], &v2_session["prompt"])
        } else {
            (
                &v1["sessionCapabilities"],
                &v1["mcpCapabilities"],
                &v1["promptCapabilities"],
            )
        };
        let supports = |value: &Value| {
            if v2 {
                value.is_object()
            } else {
                value == &Value::Bool(true) || value.is_object()
            }
        };
        let antigravity_prompt_append = !v2
            && self.agent == ExternalAgent::Antigravity
            && v1["_meta"]["antigravity"]["systemPrompt"]["append"] == Value::Bool(true);
        let capabilities = Capabilities {
            version,
            auth_status: initialized
                .pointer(if v2 {
                    "/capabilities/auth/status"
                } else {
                    "/agentCapabilities/auth/status"
                })
                .is_some_and(supports),
            air_tasks: !v2
                && self.agent == ExternalAgent::ClaudeCode
                && self.live_auth.is_some()
                && protocol::advertises_air_capability(
                    &initialized,
                    cancellation::ASYNC_TASKS_CAPABILITY,
                ),
            air_rewind: !v2
                && self.agent == ExternalAgent::ClaudeCode
                && protocol::advertises_air_capability(&initialized, cursors::REWIND_CAPABILITY),
            http_mcp: supports(&mcp["http"]),
            acp_mcp: supports(&mcp["acp"]),
            sse_mcp: !v2 && supports(&mcp["sse"]),
            stdio_mcp: !v2 || supports(&mcp["stdio"]),
            image: supports(&prompt["image"]),
            audio: supports(&prompt["audio"]),
            embedded_context: supports(&prompt["embeddedContext"]),
            skill: v2 && supports(&prompt["skill"]),
            skill_inline: v2 && supports(&prompt["skill"]["inline"]),
            load: !v2 && v1["loadSession"] == Value::Bool(true),
            resume: v2 || session["resume"].is_object(),
            close: v2 || session["close"].is_object(),
            delete: session["delete"].is_object(),
            options: session["options"].is_object(),
            lifecycle: lifecycle::LifecycleCapabilities::read(
                session,
                v2,
                antigravity_prompt_append,
            ),
        };
        self.capabilities = Some(capabilities);
        self.initialization = Some(initialized);
        Ok(capabilities)
    }

    pub(super) async fn initialization(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<&Value, AppError> {
        self.initialize(cancel).await?;
        Ok(self
            .initialization
            .as_ref()
            .expect("successful ACP initialization"))
    }

    pub(super) fn uses_v2(&self) -> bool {
        self.capabilities
            .is_some_and(|caps| caps.version == Version::V2)
    }

    pub(super) fn agent(&self) -> ExternalAgent {
        self.agent
    }

    pub(super) fn agent_arguments(&self) -> &[String] {
        &self.agent_arguments
    }

    pub(super) fn enable_terminal_auth(&mut self) {
        self.terminal_auth = true;
    }

    pub(super) fn track_auth_status(&mut self, owner: super::auth::LiveStatus) {
        self.live_auth = Some(owner);
    }

    pub(super) fn supports_auth_status(&self) -> bool {
        self.capabilities.is_some_and(|caps| caps.auth_status)
    }

    pub(super) fn reported_auth_status(&self) -> Option<&ExternalAgentAuthStatus> {
        self.reported_auth.as_ref()
    }

    fn observe_auth_status(&mut self, params: &Value) {
        if self.supports_auth_status()
            && let Some(status) = super::auth::parse_status(params)
        {
            if let Some(owner) = &self.live_auth {
                owner.update(status.clone());
            }
            self.reported_auth = Some(status);
        }
    }

    pub(super) async fn prompt_capabilities(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<PromptCapabilities, AppError> {
        let caps = self.initialize(cancel).await?;
        Ok(PromptCapabilities {
            image: caps.image,
            audio: caps.audio,
            embedded_context: caps.embedded_context,
            skill: caps.skill,
            skill_inline: caps.skill_inline,
        })
    }

    pub(super) async fn call(
        &mut self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        self.request_for(
            STARTUP_TIMEOUT,
            method,
            params,
            SessionUpdates::Ignore,
            cancel,
        )
        .await
        .map_err(|_| AppError::Config(format!("The external agent did not answer {method}.")))?
    }

    pub(super) async fn call_with_input(
        &mut self,
        method: &str,
        params: Value,
        host: &input_requests::Host,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        self.request_for(
            input_requests::INPUT_TIMEOUT + STARTUP_TIMEOUT,
            method,
            params,
            SessionUpdates::RequestInput(host),
            cancel,
        )
        .await
        .map_err(|_| AppError::Config(format!("The external agent did not answer {method}.")))?
    }

    pub(super) async fn bridge(
        &mut self,
        turn: Arc<ExternalTurn>,
    ) -> Result<Option<mcp::Bridge>, AppError> {
        let caps = self.initialize(&turn.control.cancel).await?;
        self.opening_turn = Arc::downgrade(&turn);
        let v2 = caps.version == Version::V2;
        if v2 || caps.air_tasks {
            self.native_work = Some(cancellation::Connection::register(&turn, caps.air_tasks));
        }
        let supported = [
            (
                "http",
                caps.http_mcp && self.isolation.is_none(),
                mcp::Kind::Http,
            ),
            ("acp", caps.acp_mcp, mcp::Kind::Acp),
            (
                "sse",
                caps.sse_mcp && self.isolation.is_none(),
                mcp::Kind::Sse,
            ),
            ("stdio", caps.stdio_mcp, mcp::Kind::Stdio { typed: v2 }),
        ];
        let preference = std::env::var("AGENA_ACP_MCP_TRANSPORT").ok();
        let available = supported.into_iter().filter(|(_, supported, _)| *supported);
        let Some((_, _, kind)) = available
            .clone()
            .find(|(name, _, _)| preference.as_deref() == Some(*name))
            .or_else(|| available.into_iter().next())
        else {
            let notice = serde_json::to_string(&agena_runtime_contracts::NoticePart {
                kind: "external_agent_mcp_unavailable".into(),
                summary: format!(
                    "{} did not advertise an MCP transport compatible with Agena. Agena tools are unavailable for this turn.",
                    self.agent.label(),
                ),
                detail: None,
                title: Some("Agena tools unavailable".into()),
            })?;
            let request = super::RequestId::new();
            super::persist(|| turn.attempt.notice(request, notice.clone()))
                .await
                .map_err(super::kernel_error)?;
            return Ok(None);
        };
        let relay = self
            .isolation
            .as_ref()
            .map(isolation::Isolation::relay_socket);
        let bridge = mcp::Bridge::start(turn, kind, relay).await?;
        self.mcp = bridge.acp_endpoint();
        self.mcp_activity = Some(bridge.tool_progress());
        Ok(Some(bridge))
    }

    pub(super) async fn start_probe(
        &mut self,
        cwd: &Path,
        additional_directories: Vec<PathBuf>,
        cancel: &CancellationToken,
    ) -> Result<OpenedSession, AppError> {
        let context = SessionContext {
            additional_directories,
            ..Default::default()
        };
        self.start_with_context(
            cwd,
            None,
            None,
            false,
            OpenPurpose::Prompt,
            &context,
            cancel,
        )
        .await
    }

    pub(super) async fn start_with_context(
        &mut self,
        cwd: &Path,
        mcp: Option<Value>,
        previous: Option<(&str, &ExternalAgentState)>,
        has_images: bool,
        purpose: OpenPurpose,
        context: &SessionContext,
        cancel: &CancellationToken,
    ) -> Result<OpenedSession, AppError> {
        self.prepare_provider_routes(cancel).await?;
        if self.previous_antigravity_history && previous.is_none() {
            return Err(AppError::ExternalAgentResumeUnavailable);
        }
        let setup = async {
            let capabilities = self.initialize(cancel).await?;
            if has_images && !capabilities.image {
                return Err(AppError::Config(format!(
                    "{} does not support image input over ACP. Send the image as a project file reference.",
                    self.agent.label()
                )));
            }
            let opened = |id: String,
                          resumed,
                          response: &Value,
                          states: &mut OpeningStates,
                          replacements| {
                let OpeningState {
                    mut state,
                    activity,
                } = states.remove(&id).unwrap_or_default();
                super::state::apply_setup_response(
                    &mut state,
                    response,
                    capabilities.version == Version::V2,
                );
                OpenedSession {
                    options: state.options.clone(),
                    state,
                    activity,
                    id,
                    resumed,
                    missing_previous: false,
                    replacements,
                    close_supported: capabilities.close,
                    delete_supported: capabilities.delete,
                }
            };
            let mcp_servers = Vec::from_iter(mcp);
            let mut params = json!({"cwd": cwd, "mcpServers": mcp_servers});
            context.apply(&mut params, capabilities.lifecycle)?;
            policy::apply_to_session(self.agent, &mut params);
            let mut missing_previous = false;
            if let Some((previous, state)) = previous {
                let mut resume = params.clone();
                resume["sessionId"] = json!(previous);
                if purpose == OpenPurpose::RetryPrompt {
                    resume["replayFrom"] = json!({"type": "start"});
                }
                self.apply_initial_config_options(cwd, &mut resume, context, cancel)
                    .await?;
                let methods = [
                    ("session/resume", capabilities.resume),
                    ("session/load", capabilities.load),
                ];
                for (method, supported) in methods {
                    if !supported {
                        continue;
                    }
                    // A resume without replay can carry live updates before
                    // the response. Stage them until the resume succeeds.
                    let mut states = OpeningStates::from([(
                        previous.to_owned(),
                        OpeningState {
                            state: state.clone(),
                            ..OpeningState::default()
                        },
                    )]);
                    let mut replacements = OpeningMessages {
                        replayed_users: (purpose == OpenPurpose::RetryPrompt)
                            .then(ReplayedUsers::default),
                        ..Default::default()
                    };
                    match self
                        .request(
                            method,
                            resume.clone(),
                            SessionUpdates::Opening(
                                &mut states,
                                Some((previous, &mut replacements)),
                            ),
                            cancel,
                        )
                        .await
                    {
                        Ok(response) => {
                            if let Some(replay) = &mut replacements.replayed_users
                                && !replay.complete
                            {
                                tracing::debug!(
                                    "ACP v2 resume succeeded without a replay_complete update"
                                );
                                replay.complete = true;
                            }
                            return Ok(opened(
                                previous.to_owned(),
                                true,
                                &response,
                                &mut states,
                                replacements,
                            ));
                        }
                        Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                        Err(
                            error @ AppError::ProviderClassified {
                                kind: ProviderErrorKind::Authentication,
                                ..
                            },
                        ) => {
                            return Err(error);
                        }
                        Err(error) => {
                            tracing::debug!(%error, %method, "could not reopen external agent session");
                            match replacements.resume_rejection {
                                Some(ResumeRejection::Missing) => missing_previous = true,
                                Some(ResumeRejection::Unsupported) => {}
                                None => return Err(AppError::ExternalAgentResumeUnavailable),
                            }
                        }
                    }
                }
                if !missing_previous {
                    return Err(AppError::ExternalAgentResumeUnavailable);
                }
            }
            if self.previous_antigravity_history {
                return Err(AppError::ExternalAgentResumeUnavailable);
            }
            if purpose == OpenPurpose::RetryPrompt {
                return Err(AppError::ExternalAgentPromptUncertain);
            }
            if purpose == OpenPurpose::Compact {
                return Err(AppError::Config(
                    "Antigravity could not resume the existing conversation to compact.".into(),
                ));
            }
            let mut states = OpeningStates::new();
            if let Some(work) = &self.native_work {
                work.set_root(None);
            }
            self.apply_initial_config_options(cwd, &mut params, context, cancel)
                .await?;
            let created = self
                .request(
                    "session/new",
                    params,
                    SessionUpdates::Opening(&mut states, None),
                    cancel,
                )
                .await?;
            let id = created
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= 4096)
                .ok_or_else(|| {
                    AppError::Internal("external agent did not return a session ID".into())
                })?;
            if let Some(work) = &self.native_work {
                work.set_root(Some(&id));
            }
            let mut session = opened(
                id.to_owned(),
                false,
                &created,
                &mut states,
                OpeningMessages::default(),
            );
            session.missing_previous = missing_previous;
            Ok(session)
        };
        setup.await
    }

    pub(super) async fn set_option(
        &mut self,
        session: &mut OpenedSession,
        kind: &ExternalAgentOptionKind,
        value: &str,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        let option = session.options.iter().find(|option| {
            &option.kind == kind && option.choices.iter().any(|choice| choice.value == value)
        }).ok_or_else(|| AppError::Config(format!(
            "{} no longer offers the selected {}. Open its options and choose an available value.",
            self.agent.label(),
            kind.label(),
        )))?;
        if option.current_value == value {
            return Ok(());
        }
        let legacy = option.legacy_mode && !self.uses_v2();
        let (method, params) = if legacy {
            (
                "session/set_mode",
                json!({"sessionId": session.id, "modeId": value}),
            )
        } else {
            use agena_domain::ExternalAgentOptionType;
            let (value, config_type) = match option.value_type {
                ExternalAgentOptionType::Select => (Value::String(value.to_owned()), "id"),
                ExternalAgentOptionType::Boolean => (Value::Bool(value == "true"), "boolean"),
            };
            let mut params = json!({
                "sessionId": session.id,
                "configId": option.id,
                "value": value
            });
            if self.uses_v2() || config_type == "boolean" {
                params["type"] = Value::String(config_type.into());
            }
            ("session/set_config_option", params)
        };
        session.state.options.clone_from(&session.options);
        let mut states = OpeningStates::from([(
            session.id.clone(),
            OpeningState {
                state: std::mem::take(&mut session.state),
                activity: session.activity,
            },
        )]);
        let replacements = session
            .resumed
            .then_some((session.id.as_str(), &mut session.replacements));
        let response = self
            .request(
                method,
                params,
                SessionUpdates::Opening(&mut states, replacements),
                cancel,
            )
            .await?;
        let opening = states.remove(&session.id).unwrap_or_default();
        session.state = opening.state;
        session.activity = opening.activity;
        if legacy {
            if let Some(mode) = session
                .state
                .options
                .iter_mut()
                .find(|option| &option.kind == kind)
            {
                mode.current_value = value.to_owned();
            }
        } else if response["configOptions"].is_array() {
            session.state.options = config_options(&response);
        }
        session.options.clone_from(&session.state.options);
        if !session
            .options
            .iter()
            .any(|option| &option.kind == kind && option.current_value == value)
        {
            return Err(AppError::Config(format!(
                "{} did not confirm the selected {}.",
                self.agent.label(),
                kind.label(),
            )));
        }
        Ok(())
    }

    /// Release a disposable session opened only to read and validate options.
    pub(super) async fn close_probe(
        &mut self,
        session: &OpenedSession,
        cancel: &CancellationToken,
    ) {
        for (method, supported) in [
            ("session/close", session.close_supported),
            ("session/delete", session.delete_supported),
        ] {
            if supported {
                let _ = self
                    .request_for(
                        Duration::from_secs(2),
                        method,
                        json!({"sessionId": session.id}),
                        SessionUpdates::Ignore,
                        cancel,
                    )
                    .await;
            }
        }
    }

    pub(super) async fn prompt(
        &mut self,
        session: &str,
        prompt: Vec<Value>,
        activity: ActivitySnapshot,
        replayed_users: Option<ReplayedUsers>,
        turn: &Arc<ExternalTurn>,
    ) -> Result<PromptResult, AppError> {
        if let Some(work) = &self.native_work {
            work.set_root(Some(session));
        }
        let mut params = json!({"sessionId": session});
        params["prompt"] = Value::Array(prompt);
        if self.uses_v2() {
            params["idempotencyKey"] = json!(turn.control.execution.to_string());
        }
        let steering_method = if self.uses_v2() || self.agent == ExternalAgent::Antigravity {
            Some("session/prompt")
        } else if self.initialization.as_ref().is_some_and(|init| {
            init.pointer("/_meta/steering/supported") == Some(&Value::Bool(true))
        }) {
            Some("_session/steering")
        } else {
            None
        };
        let mut lease = None;
        let steering = steering_method.map(|method| {
            let (registration, receiver) = turn.offer_steering();
            lease = Some(registration);
            (method, receiver)
        });
        let v2 = self.uses_v2();
        let previous_tail = turn.transcript.lock().await.cursor_tail();
        let mut lifecycle = PromptLifecycle::new(
            activity,
            previous_tail,
            self.capabilities.is_some_and(|caps| caps.air_rewind),
        );
        lifecycle.replayed_users = replayed_users;
        let result = self
            .request(
                "session/prompt",
                params,
                SessionUpdates::Turn {
                    turn,
                    session,
                    steering,
                    lifecycle: Some(&mut lifecycle),
                },
                &turn.control.cancel,
            )
            .await;
        drop(lease);
        if !turn.control.cancel.is_cancelled() {
            if result.is_ok() {
                turn.transcript
                    .lock()
                    .await
                    .record_cursor_tail(&turn.round, lifecycle.cursor_tail())
                    .await?;
            }
            turn.finish_transcript().await?;
        }
        let result = result?;
        let stop_reason = result["stopReason"].as_str();
        match stop_reason {
            Some("cancelled") => Err(AppError::Cancelled),
            Some("error") => Err(self.peer_error("session/prompt", &result["error"])),
            None if !v2 => Err(AppError::Internal(
                "external agent did not return a turn outcome".into(),
            )),
            _ => Ok(PromptResult {
                stop_reason: stop_reason.map(str::to_owned),
                // The Claude v1 adapter resets prompt-response usage at turn
                // activation. Other v1 agents can use cumulative counters.
                usage: (v2 || self.agent == ExternalAgent::ClaudeCode)
                    .then(|| protocol::foreground_usage(&result["usage"]))
                    .flatten(),
                fork_cursor: lifecycle.into_fork_cursor(),
            }),
        }
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        updates: SessionUpdates<'_>,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        self.request_with_timeout(None, method, params, updates, cancel)
            .await
            .expect("unbounded ACP request cannot time out")
    }

    async fn request_with_timeout(
        &mut self,
        duration: Option<Duration>,
        method: &str,
        params: Value,
        updates: SessionUpdates<'_>,
        cancel: &CancellationToken,
    ) -> Result<Result<Value, AppError>, tokio::time::error::Elapsed> {
        let session = match &updates {
            SessionUpdates::Turn { session, .. } => Some(*session),
            _ => None,
        };
        let mut steering_requests = HashMap::new();
        let mut replies = std::mem::take(&mut self.opening_replies);
        let exchange = self.request_loop(
            method,
            params,
            updates,
            cancel,
            &mut steering_requests,
            &mut replies,
        );
        let result = match duration {
            Some(duration) => tokio::time::timeout(duration, exchange).await,
            None => Ok(exchange.await),
        };
        if matches!(&result, Ok(Ok(_))) {
            self.active_request = None;
            self.opening_replies = replies;
        } else {
            if (cancel.is_cancelled()
                || matches!(
                    &result,
                    Ok(Err(AppError::ExternalAgentPromptInactive { .. }))
                ))
                && let Some(session) = session
                && let Some(request) = self.active_request.take()
            {
                self.cancel_foreground(session, request, steering_requests.into_keys())
                    .await;
            } else {
                self.abandon_request().await;
            }
            // Client tools and passive inputs settle their local state after
            // withdrawal, even when there is no longer a peer to answer.
            while replies.join_next().await.is_some() {}
        }
        result
    }

    async fn request_for(
        &mut self,
        duration: Duration,
        method: &str,
        params: Value,
        updates: SessionUpdates<'_>,
        cancel: &CancellationToken,
    ) -> Result<Result<Value, AppError>, tokio::time::error::Elapsed> {
        self.request_with_timeout(Some(duration), method, params, updates, cancel)
            .await
    }

    async fn request_loop(
        &mut self,
        method: &str,
        params: Value,
        mut updates: SessionUpdates<'_>,
        cancel: &CancellationToken,
        steering_requests: &mut HashMap<u64, steering::Request>,
        replies: &mut PeerReplies,
    ) -> Result<Value, AppError> {
        if self.air_index && !index::allowed_request(method) {
            return Err(AppError::Internal(
                "Unexpected request on the ACP session index connection.".into(),
            ));
        }
        self.next_id += 1;
        let id = self.next_id;
        let request_input = match &updates {
            SessionUpdates::RequestInput(host) => Some(*host),
            _ => None,
        };
        let startup = matches!(&updates, SessionUpdates::Opening(..));
        let opening_target = params["sessionId"].as_str().map(str::to_owned);
        let opening = if startup {
            self.opening_turn.upgrade()
        } else {
            None
        };
        let opening_session = opening.as_ref().and(opening_target.as_deref());
        let opening_scope = opening.as_ref().zip(opening_session);
        if method == "session/resume"
            && self.uses_v2()
            && let Some(work) = &self.native_work
        {
            work.set_root(opening_session);
        }
        let (turn, mut steering, mut prompt) = match &mut updates {
            SessionUpdates::Turn {
                turn,
                session,
                steering,
                lifecycle,
            } => (Some((*turn, *session)), steering.take(), lifecycle.take()),
            _ => (None, None, None),
        };
        let v2 = self.uses_v2();
        let retain_opening_requests = v2 && opening_scope.is_some();
        let mut v1_result = None;
        {
            let mut request = json!({"jsonrpc":"2.0", "id":id, "method":method});
            request["params"] = params;
            self.write(&request, cancel).await?;
            self.active_request = Some(ActiveRequest {
                id,
                awaiting_reply: true,
            });
        }
        let mut inactivity = if turn.is_some() {
            Some(PromptIdle::after_send(self.mcp_activity.clone()))
        } else if startup {
            Some(PromptIdle::new(STARTUP_TIMEOUT, self.mcp_activity.clone()))
        } else {
            None
        };
        // Requests and updates share the transcript mutex. Let request tasks
        // make progress while the stream loop records updates.
        loop {
            let steering_pending = !steering_requests.is_empty();
            if v2
                && let Some(result) = prompt
                    .as_mut()
                    .and_then(|prompt| prompt.take_result(steering_pending))
            {
                self.finish_interactions(replies, cancel).await?;
                return Ok(result);
            }
            if v1_result.is_some()
                && !steering_pending
                && let Some((turn, _)) = turn
                && !turn.has_native_background().await
            {
                self.finish_interactions(replies, cancel).await?;
                return Ok(v1_result.take().expect("checked completed v1 result"));
            }
            let completion_deadline = v2
                .then(|| prompt.as_ref()?.completion_deadline(steering_pending))
                .flatten();
            // A silent background terminal can remain live after the Agent
            // completes its prompt. Inactivity applies to pending input and
            // foreground work, not to a reported independent task.
            let background_only = !steering_pending
                && (v1_result.is_some()
                    || (v2 && prompt.as_ref().is_some_and(|p| p.waiting_for_background())));
            let authorizing_until = self.url_elicitations.values().copied().max();
            let line = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                id = abandoned_steering(steering_requests), if !steering_requests.is_empty() => {
                    steering_requests.remove(&id);
                    self.abandon_rpc(id).await;
                    continue;
                }
                Some(reply) = replies.join_next(), if !replies.is_empty() => {
                    self.reply_to_interaction(reply, cancel).await?;
                    if let Some(inactivity) = &mut inactivity {
                        inactivity.record();
                    }
                    if let Some((turn, _)) = turn
                        && self.pending.is_empty()
                    {
                        turn.control.phase(agena_domain::ExecutionPhase::StreamingModel);
                    }
                    continue;
                }
                Some(request) = async {
                    match steering.as_mut() {
                        Some((_, receiver)) => receiver.recv().await,
                        None => std::future::pending().await,
                    }
                }, if steering_requests.len() < 8 && v1_result.is_none() => {
                    if request.answer.is_closed() {
                        continue;
                    }
                    let Some((_, root)) = turn else { unreachable!("steering is scoped to a prompt") };
                    let steer_method = steering.as_ref().expect("steering receiver").0;
                    self.next_id += 1;
                    let request_id = self.next_id;
                    let mut params = json!({"sessionId": root, "prompt": [{"type":"text", "text": request.text}]});
                    if v2 {
                        params["idempotencyKey"] = json!(request.id.to_string());
                    } else if steer_method == "_session/steering" {
                        params["_meta"] = json!({"steering":{"idleBehavior":"promptRequired"}});
                    }
                    let send = self.write(
                        &json!({"jsonrpc":"2.0", "id": request_id, "method": steer_method, "params": params}),
                        cancel,
                    ).await;
                    if let Err(error) = send {
                        let _ = request.answer.send(Err(error));
                    } else {
                        steering_requests.insert(request_id, request);
                    }
                    continue;
                }
                Some(event) = cancellation::receive(&mut self.native_work) => {
                    self.handle_work_event(event, cancel).await;
                    continue;
                }
                _ = async {
                    inactivity.as_mut().expect("enabled for a session operation").expired(authorizing_until).await;
                }, if inactivity.is_some() && !background_only && self.pending.is_empty() => {
                    if startup {
                        return Err(AppError::Config(format!(
                            "{} timed out during {method}.",
                            self.agent.label(),
                        )));
                    }
                    return Err(inactivity.as_ref().expect("enabled for a prompt").error(self.agent.label()));
                }
                line = self.output.next() => line,
                _ = async { tokio::time::sleep_until(completion_deadline.expect("completion is pending")).await }, if completion_deadline.is_some() => {
                    continue;
                }
            };
            let line = line
                .ok_or_else(|| {
                    let hint = if self.isolation.is_some() {
                        "Check the MXC prerequisites, ACP sandbox policy, and agent installation."
                    } else {
                        "Check the official agent's sign-in and version."
                    };
                    AppError::Config(format!(
                        "{} closed its ACP connection during {method}. {hint}",
                        self.agent.label()
                    ))
                })?
                .map_err(|error| AppError::Internal(format!("invalid ACP frame: {error}")))?;
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line).map_err(|error| {
                AppError::Internal(format!("external agent sent invalid ACP JSON: {error}"))
            })?;
            if let Some(peer_method) = message.get("method").and_then(Value::as_str) {
                let params = &message["params"];
                if peer_method == index::CHANGES_METHOD
                    && message.get("id").is_none()
                    && let SessionUpdates::IndexChanges(changes) = &mut updates
                {
                    changes.observe(params);
                    continue;
                }
                if let Some(peer_id) = message.get("id") {
                    if self.pending.contains_key(peer_id) {
                        continue;
                    }
                    if self.pending.len() >= MAX_PEER_REQUESTS {
                        self.write(
                            &Reply::Error(-32603, "Too many active ACP client requests")
                                .message(peer_id),
                            cancel,
                        )
                        .await?;
                        continue;
                    }
                    if peer_method == "mcp/message" {
                        self.receive_mcp(
                            peer_id,
                            params,
                            turn.or(opening_scope.filter(|_| v2)),
                            replies,
                            cancel,
                        )
                        .await?;
                        continue;
                    }
                    let kind = match peer_method {
                        "session/request_permission" => InteractionKind::Permission,
                        "elicitation/create" => InteractionKind::Form,
                        _ => {
                            let operation = (!v2)
                                .then(|| client_tools::Operation::parse(peer_method))
                                .flatten();
                            let Some(operation) = operation else {
                                self.write(
                                    &Reply::Error(-32601, "ACP client method is not supported")
                                        .message(peer_id),
                                    cancel,
                                )
                                .await?;
                                continue;
                            };
                            InteractionKind::ClientTool(operation)
                        }
                    };
                    if let Some(host) = request_input
                        && kind == InteractionKind::Form
                        && params["sessionId"].is_null()
                        && params["requestId"].as_u64() == Some(id)
                    {
                        let input =
                            input_requests::NativeInput::elicitation(params, self.agent.label());
                        self.begin_passive_input(
                            peer_id, kind, params, input, host, replies, cancel,
                        )
                        .await?;
                        continue;
                    }
                    let request_matches = params["requestId"].as_u64().is_some_and(|request| {
                        self.active_request
                            .as_ref()
                            .is_some_and(|active| active.awaiting_reply && request == active.id)
                            || steering_requests.contains_key(&request)
                    });
                    let interaction_turn = match (turn, params["sessionId"].as_str()) {
                        (Some((turn, root)), Some(session)) => turn
                            .accepts_native_session(root, session)
                            .await
                            .then_some(turn),
                        (Some((turn, _)), None)
                            if kind == InteractionKind::Form && request_matches =>
                        {
                            Some(turn)
                        }
                        (None, Some(session))
                            if opening_session == Some(session)
                                && (kind == InteractionKind::Form
                                    || (v2 && kind == InteractionKind::Permission)) =>
                        {
                            opening.as_ref()
                        }
                        (None, None) if kind == InteractionKind::Form && request_matches => {
                            opening.as_ref()
                        }
                        _ => None,
                    };
                    let Some(turn) = interaction_turn else {
                        self.write(&kind.cancelled().message(peer_id), cancel)
                            .await?;
                        continue;
                    };
                    if v2
                        && kind == InteractionKind::Permission
                        && let SessionUpdates::Turn { session: root, .. } = &updates
                        && let Some(source) = params["sessionId"].as_str()
                        && let Some(update) = protocol::permission_tool_update(params)
                    {
                        self.observe_native_work(source, &update);
                        turn.update_native(root, source, &update).await?;
                    }
                    let url_elicitation = match self.pending_url(kind, params) {
                        Ok(url) => url,
                        Err(reply) => {
                            self.write(&reply.message(peer_id), cancel).await?;
                            continue;
                        }
                    };
                    let withdrawal = if matches!(kind, InteractionKind::ClientTool(_)) {
                        turn.control.cancel.child_token()
                    } else {
                        CancellationToken::new()
                    };
                    self.pending.insert(
                        peer_id.clone(),
                        PendingInteraction {
                            kind,
                            withdrawal: withdrawal.clone(),
                            mcp_request: None,
                            url_elicitation,
                        },
                    );
                    let peer_id = peer_id.clone();
                    let params = if v2 && kind == InteractionKind::Permission {
                        protocol::permission_params(params)
                    } else {
                        params.clone()
                    };
                    let turn = Arc::clone(turn);
                    replies.spawn(async move {
                        let reply = kind.request(&turn, &params, &withdrawal).await?;
                        Ok((peer_id, reply))
                    });
                } else if peer_method == "$/cancel_request" {
                    if let Some(pending) =
                        params.get("requestId").and_then(|id| self.pending.get(id))
                    {
                        pending.withdrawal.cancel();
                    }
                } else if peer_method == "elicitation/complete" {
                    if let Some(id) = elicitation_id(params)
                        && self.url_elicitations.remove(id).is_some()
                        && let Some(turn) = turn.map(|(turn, _)| turn).or(opening.as_ref())
                    {
                        turn.complete_url_elicitation().await?;
                        if let Some(inactivity) = &mut inactivity {
                            inactivity.record();
                        }
                    }
                } else if peer_method == "auth/status_update" {
                    self.observe_auth_status(params);
                } else if peer_method == "session/update"
                    && let Some(session) = params["sessionId"].as_str()
                    && !session.is_empty()
                    && session.len() <= 4096
                {
                    let mut update = params["update"].clone();
                    self.normalize_session_update(&mut update);
                    self.observe_native_work(session, &update);
                    match &mut updates {
                        SessionUpdates::Turn {
                            turn,
                            session: root,
                            ..
                        } => {
                            if let Some(inactivity) = &mut inactivity
                                && (session == *root
                                    || turn.accepts_native_session(root, session).await)
                            {
                                inactivity.observe_update(session == *root, &update);
                            }
                            if session == *root
                                && let Some(lifecycle) = &mut prompt
                            {
                                lifecycle.observe(&update);
                            }
                            if session == *root
                                && let Some(change) = cursors::change(
                                    &update,
                                    self.capabilities.is_some_and(|caps| caps.air_rewind),
                                )
                            {
                                let request = super::RequestId::new();
                                super::persist(|| {
                                    turn.round.update_external_agent_cursor(
                                        request,
                                        root,
                                        change.clone(),
                                    )
                                })
                                .await
                                .map_err(super::kernel_error)?;
                            }
                            turn.update_native(root, session, &update).await?;
                        }
                        SessionUpdates::Replay(replay, expected) => {
                            replay.update(expected, session, &update, v2)?;
                        }
                        SessionUpdates::Opening(states, messages) => {
                            if opening_target.as_deref() == Some(session)
                                && let Some(inactivity) = &mut inactivity
                            {
                                inactivity.observe_update(true, &update);
                            }
                            if let Some(state) = states.get_mut(session) {
                                state.observe(session, session, &update);
                            } else if let Some((root, state)) =
                                states.iter_mut().find(|(_, state)| {
                                    state.state.details.subagents.contains_key(session)
                                })
                            {
                                state.observe(root, session, &update);
                            } else if states.len() < MAX_OPENING_SESSIONS {
                                let mut state = OpeningState::default();
                                if state.observe(session, session, &update) {
                                    states.insert(session.to_owned(), state);
                                }
                            }
                            if v2
                                && let Some((root, messages)) = messages
                                && session == *root
                            {
                                messages.record(update, line.len());
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            }
            if self.complete_native_cancellation(&message) {
                if let Some(inactivity) = &mut inactivity {
                    inactivity.record();
                }
                continue;
            }
            let response_id = message.get("id").and_then(Value::as_u64);
            if let Some(request) = response_id.and_then(|id| steering_requests.remove(&id)) {
                if let Some(inactivity) = &mut inactivity {
                    inactivity.record();
                }
                let Some((turn, _)) = turn else {
                    unreachable!("steering reply without prompt")
                };
                let result = if let Some(error) = message.get("error") {
                    Err(self.peer_error("steering", error))
                } else if message["result"]["outcome"] == "promptRequired" {
                    Err(AppError::Config("The external turn ended before it could accept the additional instruction.".into()))
                } else if message.get("result").is_some()
                    && (!v2
                        || bounded_string(&message["result"]["messageId"], MAX_CURSOR_BYTES)
                            .is_some())
                {
                    turn.record_steer(&request).await
                } else {
                    Err(AppError::Internal(
                        "The external agent did not acknowledge the additional instruction.".into(),
                    ))
                };
                let _ = request.answer.send(result);
                continue;
            }
            if response_id != Some(id)
                || !self
                    .active_request
                    .as_ref()
                    .is_some_and(|request| request.awaiting_reply)
            {
                continue;
            }
            self.active_request
                .as_mut()
                .expect("current ACP request")
                .awaiting_reply = false;
            if let Some(inactivity) = &mut inactivity {
                inactivity.record();
            }
            if let Some(error) = message.get("error") {
                self.finish_interactions(replies, cancel).await?;
                if let Some((turn, _)) = turn
                    && method == "session/prompt"
                    && (v2
                        || matches!(
                            error["code"].as_i64(),
                            Some(-32600 | -32601 | -32602 | -32000)
                        ))
                    && !prompt
                        .as_ref()
                        .is_some_and(|prompt| prompt.replayed_users.is_some())
                    && self
                        .mcp_activity
                        .as_ref()
                        .is_none_or(mcp::ToolProgress::revoke_if_idle)
                {
                    // This reply establishes rejection before insertion. Stop
                    // admitting tools before the kernel checks for effects.
                    turn.record_prompt_delivery(super::ExternalAgentPromptDelivery::Unsent)
                        .await?;
                }
                if matches!(method, "session/resume" | "session/load")
                    && let SessionUpdates::Opening(_, Some((session, captured))) = &mut updates
                {
                    let missing_id = &error["data"]["sessionId"];
                    captured.resume_rejection = match error["code"].as_i64() {
                        Some(-32002)
                            if missing_id.as_str() == Some(*session)
                                || (!v2 && missing_id.is_null()) =>
                        {
                            Some(ResumeRejection::Missing)
                        }
                        Some(-32601) => Some(ResumeRejection::Unsupported),
                        _ => None,
                    };
                }
                if matches!(method, "session/close" | "session/delete")
                    && error["code"].as_i64() == Some(-32002)
                {
                    return Ok(json!({}));
                }
                return Err(self.peer_error(method, error));
            }
            let mut result = message.get("result").cloned().ok_or_else(|| {
                AppError::Internal("external agent returned an invalid ACP response".into())
            })?;
            if v2 && let Some(prompt) = &mut prompt {
                let (turn, _) = turn.expect("ACP prompt has a turn");
                turn.record_prompt_delivery(super::ExternalAgentPromptDelivery::Accepted)
                    .await?;
                let message_id = bounded_string(&result["messageId"], MAX_CURSOR_BYTES)
                    .ok_or_else(|| AppError::Config(
                        "The external agent accepted the prompt without a valid ACP message ID.".into(),
                    ))?;
                if prompt
                    .replayed_users
                    .as_ref()
                    .is_some_and(|users| users.contains(&message_id))
                {
                    return Err(AppError::ExternalAgentPromptAlreadyAccepted);
                }
                prompt.acknowledge(message_id);
                continue;
            }
            if turn.is_some() && method == "session/prompt" {
                v1_result = Some(result);
                continue;
            }
            if !retain_opening_requests {
                self.finish_interactions(replies, cancel).await?;
            }
            if v2 {
                protocol::normalize_config_options(&mut result);
            }
            self.filter_skill_commands(&mut result);
            return Ok(result);
        }
    }

    fn pending_url(&self, kind: InteractionKind, params: &Value) -> Result<Option<String>, Reply> {
        if kind != InteractionKind::Form || params["mode"] != "url" {
            return Ok(None);
        }
        let id =
            elicitation_id(params).ok_or(Reply::Error(-32602, "Invalid ACP elicitation ID"))?;
        if self.url_elicitations.contains_key(id)
            || self
                .pending
                .values()
                .any(|pending| pending.url_elicitation.as_deref() == Some(id))
            || self.url_elicitations.len() + self.pending.len() >= MAX_PEER_REQUESTS
        {
            return Err(Reply::Error(
                -32602,
                "Duplicate or excessive ACP URL elicitations",
            ));
        }
        Ok(Some(id.to_owned()))
    }

    async fn begin_passive_input(
        &mut self,
        peer_id: &Value,
        kind: InteractionKind,
        params: &Value,
        input: Option<input_requests::NativeInput>,
        host: &input_requests::Host,
        replies: &mut PeerReplies,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        let Some(input) = input else {
            let reply = if kind == InteractionKind::Form {
                Reply::Error(-32602, "Invalid or unsupported ACP elicitation request")
            } else {
                kind.cancelled()
            };
            return self.write(&reply.message(peer_id), cancel).await;
        };
        let url_elicitation = match self.pending_url(kind, params) {
            Ok(url) => url,
            Err(reply) => return self.write(&reply.message(peer_id), cancel).await,
        };
        let withdrawal = CancellationToken::new();
        self.pending.insert(
            peer_id.clone(),
            PendingInteraction {
                kind,
                withdrawal: withdrawal.clone(),
                mcp_request: None,
                url_elicitation,
            },
        );
        let id = peer_id.clone();
        let host = host.clone();
        replies
            .spawn(async move { Ok((id, Reply::Result(host.request(input, withdrawal).await?))) });
        Ok(())
    }

    async fn cancel_native_work(
        &mut self,
        request: cancellation::Request,
        cancel: &CancellationToken,
    ) {
        if request.answer.is_closed() {
            return;
        }
        let work = self
            .native_work
            .as_mut()
            .expect("cancellation receiver is registered");
        if work.pending.len() >= cancellation::MAX_REQUESTS {
            if let Some(id) = work
                .pending
                .iter()
                .find_map(|(&id, (request, _))| request.is_none().then_some(id))
            {
                work.pending.remove(&id);
            } else {
                let _ = request.answer.send(Err(cancellation::busy()));
                return;
            }
        }
        let Some((target, params)) = work.claim(&request) else {
            let _ = request.answer.send(Err(cancellation::unavailable()));
            return;
        };
        self.next_id += 1;
        let id = target.rpc_id(self.next_id);
        let mut message = json!({"jsonrpc":"2.0", "method":target.method(), "params":params});
        if let Some(id) = id {
            message["id"] = json!(id);
        }
        match (self.write(&message, cancel).await, id) {
            (Ok(()), Some(id)) => {
                self.native_work
                    .as_mut()
                    .expect("live work registry")
                    .pending
                    .insert(id, (Some(request), target));
            }
            (result, _) => {
                let _ = request.answer.send(result);
            }
        }
    }

    async fn handle_work_event(&mut self, event: cancellation::Event, cancel: &CancellationToken) {
        match event {
            cancellation::Event::Cancel(request) => self.cancel_native_work(request, cancel).await,
            cancellation::Event::Abandoned(id) => {
                if let Some((request, _)) = self
                    .native_work
                    .as_mut()
                    .and_then(|work| work.pending.get_mut(&id))
                {
                    request.take();
                }
                self.abandon_rpc(id).await;
            }
        }
    }

    fn observe_native_work(&self, session: &str, update: &Value) {
        if let Some(work) = &self.native_work {
            work.observe(session, update);
        }
    }

    fn normalize_session_update(&self, update: &mut Value) {
        if update["sessionUpdate"] == "available_commands_update" {
            self.filter_skill_commands(update);
        }
        if self.uses_v2() && update["sessionUpdate"] == "config_option_update" {
            protocol::normalize_config_options(update);
        } else if self.capabilities.is_some_and(|caps| caps.air_tasks) {
            protocol::normalize_air_subagent_update(update);
        }
    }

    fn filter_skill_commands(&self, message: &mut Value) {
        let Some(commands) = message
            .get_mut("availableCommands")
            .and_then(Value::as_array_mut)
        else {
            return;
        };
        if self.capabilities.is_some_and(|caps| caps.skill) {
            let inline = self.capabilities.is_some_and(|caps| caps.skill_inline);
            for command in commands.iter_mut().filter(|entry| entry["type"] == "skill") {
                command["inline"] = json!(inline);
            }
        } else {
            commands.retain(|command| command["type"] != "skill");
        }
    }

    fn complete_native_cancellation(&mut self, message: &Value) -> bool {
        let Some(id) = message["id"].as_u64() else {
            return false;
        };
        let Some((request, target)) = self
            .native_work
            .as_mut()
            .and_then(|work| work.pending.remove(&id))
        else {
            return false;
        };
        let error = message.get("error");
        let declined = target.declined(&message["result"]);
        let result = if let Some(error) = error {
            Err(self.peer_error(target.method(), error))
        } else if declined {
            Err(AppError::Config(
                "The external agent could not stop the background task.".into(),
            ))
        } else if target.accepted(&message["result"]) {
            Ok(())
        } else {
            Err(AppError::Internal(
                "The external agent returned an invalid cancellation response.".into(),
            ))
        };
        if result.is_err()
            && !declined
            && !matches!(message["error"]["code"].as_i64(), Some(-32602 | -32002))
        {
            self.native_work
                .as_ref()
                .expect("live work registry")
                .reoffer(&target);
        }
        if let Some(request) = request {
            let _ = request.answer.send(result);
        }
        true
    }

    fn peer_error(&self, method: &str, error: &Value) -> AppError {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        if code == -32800 {
            return AppError::Cancelled;
        }
        // The agent's error payload can carry authentication URLs or tokens.
        if matches!(code, -32000 | -32001) {
            return AppError::ProviderClassified {
                provider: self.agent.label().into(),
                message: format!(
                    "Rejected {method} (ACP error {code}). Check the agent's native sign-in."
                ),
                kind: ProviderErrorKind::Authentication,
                retryable: false,
            };
        }
        AppError::Config(format!(
            "{} rejected {method} (ACP error {code}).",
            self.agent.label()
        ))
    }

    async fn finish_interactions(
        &mut self,
        replies: &mut PeerReplies,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        self.withdraw_interactions();
        while let Some(reply) = replies.join_next().await {
            self.reply_to_interaction(reply, cancel).await?;
        }
        Ok(())
    }

    async fn write_cancel_request(
        &mut self,
        request_id: u64,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        self.write(
            &json!({"jsonrpc":"2.0", "method":"$/cancel_request", "params":{"requestId":request_id}}),
            cancel,
        )
        .await
    }

    async fn abandon_rpc(&mut self, id: u64) {
        let cancel = CancellationToken::new();
        let _ =
            tokio::time::timeout(CANCELLATION_GRACE, self.write_cancel_request(id, &cancel)).await;
    }

    async fn abandon_request(&mut self) {
        let active = self.active_request.take();
        if active.is_none() && self.pending.is_empty() {
            return;
        }
        self.withdraw_interactions();
        let cancel = CancellationToken::new();
        let cleanup = async {
            if let Some(request) = active.filter(|request| request.awaiting_reply) {
                self.write_cancel_request(request.id, &cancel).await?;
            }
            self.cancel_peer_requests(&cancel).await
        };
        let _ = tokio::time::timeout(CANCELLATION_GRACE, cleanup).await;
        self.pending.clear();
    }

    fn withdraw_interactions(&self) {
        for pending in self.pending.values() {
            pending.withdrawal.cancel();
        }
    }

    async fn cancel_peer_requests(&mut self, cancel: &CancellationToken) -> Result<(), AppError> {
        self.withdraw_interactions();
        let pending = std::mem::take(&mut self.pending);
        for (id, request) in pending {
            self.write(&request.kind.cancelled().message(&id), cancel)
                .await?;
        }
        Ok(())
    }

    async fn cancel_foreground(
        &mut self,
        session: &str,
        main: ActiveRequest,
        steering_ids: impl Iterator<Item = u64>,
    ) {
        let v2 = self.uses_v2();
        let cancel = CancellationToken::new();
        let stop = async {
            self.write(
                &json!({"jsonrpc":"2.0", "method":"session/cancel", "params":{"sessionId":session}}),
                &cancel,
            )
            .await?;
            let mut awaiting: HashSet<_> = steering_ids.collect();
            if !v2 {
                // v1 requires a cancellation reply for pending permission requests.
                self.cancel_peer_requests(&cancel).await?;
                for &id in &awaiting {
                    self.write_cancel_request(id, &cancel).await?;
                }
            }
            if main.awaiting_reply {
                awaiting.insert(main.id);
            }
            let mut idle = !v2;
            while !idle || !awaiting.is_empty() {
                let Some(Ok(line)) = self.output.next().await else {
                    break;
                };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let params = &message["params"];
                match message["method"].as_str() {
                    Some("$/cancel_request") if message.get("id").is_none() => {
                        // In v2 only the Agent can identify the callbacks its
                        // cancelled work was waiting for.
                        if let Some(id) = params.get("requestId")
                            && let Some(request) = self.pending.remove(id)
                        {
                            request.withdrawal.cancel();
                            self.write(&request.kind.cancelled().message(id), &cancel)
                                .await?;
                        }
                    }
                    Some("session/request_permission") if !v2 => {
                        if let Some(id) = message.get("id") {
                            self.write(
                                &InteractionKind::Permission.cancelled().message(id),
                                &cancel,
                            )
                            .await?;
                        }
                    }
                    Some("session/update") => {
                        if let Some(source) = params["sessionId"].as_str() {
                            let mut update = params["update"].clone();
                            self.normalize_session_update(&mut update);
                            self.observe_native_work(source, &update);
                            if source == session && update["sessionUpdate"] == "state_update" {
                                idle = update["state"] == "idle";
                            }
                        }
                    }
                    Some("auth/status_update") => self.observe_auth_status(params),
                    None => {
                        if let Some(id) = message["id"].as_u64() {
                            awaiting.remove(&id);
                            if id == main.id && message["error"]["code"] == -32800 {
                                idle = true;
                            }
                        }
                        self.complete_native_cancellation(&message);
                    }
                    _ => {}
                }
            }
            Ok::<(), AppError>(())
        };
        let _ = tokio::time::timeout(CANCELLATION_GRACE, stop).await;
        self.withdraw_interactions();
        self.pending.clear();
    }

    async fn receive_mcp(
        &mut self,
        id: &Value,
        params: &Value,
        turn: Option<(&Arc<ExternalTurn>, &str)>,
        replies: &mut PeerReplies,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        let error = |message| Reply::Error(-32602, message).message(id);
        let Some(endpoint) = self
            .mcp
            .clone()
            .filter(|bridge| bridge.matches(&params["serverId"]))
        else {
            return self.write(&error("Unknown ACP MCP server"), cancel).await;
        };
        let Some(request_id) = params["requestId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 4096)
        else {
            return self
                .write(&error("Invalid ACP MCP request ID"), cancel)
                .await;
        };
        if self
            .pending
            .values()
            .any(|request| request.mcp_request.as_deref() == Some(request_id))
        {
            return self
                .write(&error("Duplicate ACP MCP request ID"), cancel)
                .await;
        }
        let Some(method) = params["method"].as_str() else {
            return self.write(&error("Missing ACP MCP method"), cancel).await;
        };
        if method == "tools/call" {
            let Some((turn, root)) = turn else {
                return self
                    .write(&error("The ACP MCP server has no active turn"), cancel)
                    .await;
            };
            if let Some(session) = params["sessionId"].as_str()
                && !turn.accepts_native_session(root, session).await
            {
                return self
                    .write(
                        &error("The ACP MCP server does not serve that session"),
                        cancel,
                    )
                    .await;
            }
        }
        let withdrawal = CancellationToken::new();
        self.pending.insert(
            id.clone(),
            PendingInteraction {
                kind: InteractionKind::Mcp,
                withdrawal: withdrawal.clone(),
                mcp_request: Some(request_id.to_owned()),
                url_elicitation: None,
            },
        );
        let id = id.clone();
        let method = method.to_owned();
        let args = params["params"].clone();
        replies.spawn(async move {
            let reply = match endpoint.respond(&method, args, withdrawal).await {
                Ok(result) => Reply::Result(result),
                Err(AppError::Cancelled) => InteractionKind::Mcp.cancelled(),
                Err(error) => return Err(error),
            };
            Ok((id, reply))
        });
        Ok(())
    }

    async fn reply_to_interaction(
        &mut self,
        completed: Result<Result<(Value, Reply), AppError>, JoinError>,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        let (id, reply) = completed.map_err(|error| {
            AppError::Internal(format!("ACP interaction task stopped: {error}"))
        })??;
        let pending = &self.pending[&id];
        // A tool can commit before the withdrawal reaches its executor.
        // Return the actual outcome instead of implying it did not execute.
        let reply = if pending.withdrawal.is_cancelled()
            && matches!(
                pending.kind,
                InteractionKind::Permission | InteractionKind::Form
            ) {
            pending.kind.cancelled()
        } else {
            reply
        };
        let accepted = matches!(&reply, Reply::Result(result) if result["action"] == "accept");
        self.write(&reply.message(&id), cancel).await?;
        if let Some(url_id) = self
            .pending
            .remove(&id)
            .and_then(|pending| pending.url_elicitation)
            && accepted
        {
            self.url_elicitations
                .insert(url_id, Instant::now() + input_requests::INPUT_TIMEOUT);
        }
        Ok(())
    }

    async fn write(&mut self, message: &Value, cancel: &CancellationToken) -> Result<(), AppError> {
        if self.incomplete_write {
            return Err(AppError::Internal(
                "the ACP stream has an incomplete frame".into(),
            ));
        }
        let mut frame = serde_json::to_vec(message)?;
        frame.push(b'\n');
        if frame.len() > MAX_FRAME_BYTES {
            return Err(AppError::Config(
                "The external agent request exceeds its size limit.".into(),
            ));
        }
        let deadline = tokio::time::sleep(STARTUP_TIMEOUT);
        tokio::pin!(deadline);
        let mut written = 0;
        while written < frame.len() {
            let bytes = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                _ = &mut deadline => return Err(AppError::Config(format!(
                    "{} did not read an ACP message in time.", self.agent.label()
                ))),
                result = self.input.write(&frame[written..]) => result.map_err(|error|
                    AppError::Internal(format!("could not write the ACP connection: {error}")))?,
            };
            if bytes == 0 {
                return Err(AppError::Internal(
                    "the ACP connection stopped accepting data".into(),
                ));
            }
            written += bytes;
            self.incomplete_write = written < frame.len();
        }
        Ok(())
    }

    pub(super) async fn watch_idle(
        &mut self,
        kernel: &agena_storage::backend::kernel::KernelPorts,
        resume: &agena_storage::kernel::ExternalAgentResume,
        state: ExternalAgentState,
        host: &input_requests::Host,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        self.mcp = None;
        self.mcp_activity = None;
        let mut replies = JoinSet::new();
        let result = self
            .read_idle(kernel, resume, state, host, &mut replies, cancel)
            .await;
        self.native_work = None;
        self.live_auth = None;
        self.withdraw_interactions();
        while replies.join_next().await.is_some() {}
        result
    }

    async fn read_idle(
        &mut self,
        kernel: &agena_storage::backend::kernel::KernelPorts,
        resume: &agena_storage::kernel::ExternalAgentResume,
        mut state: ExternalAgentState,
        host: &input_requests::Host,
        replies: &mut PeerReplies,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        let still_owned = || async {
            let current = kernel
                .reader
                .configuration(resume.session)
                .await
                .map_err(super::kernel_error)?;
            Ok::<_, AppError>(current.external_agent_resume.as_ref() == Some(resume))
        };
        let mut ownership = tokio::time::interval(Duration::from_secs(15));
        let mut messages = NativeMessages::default();
        loop {
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(()),
                Some(reply) = replies.join_next(), if !replies.is_empty() => {
                    self.reply_to_interaction(reply, cancel).await?;
                    continue;
                }
                _ = ownership.tick() => {
                    if !still_owned().await? {
                        return Ok(());
                    }
                    continue;
                }
                Some(event) = cancellation::receive(&mut self.native_work) => {
                    if !still_owned().await? {
                        if let cancellation::Event::Cancel(request) = event {
                            let _ = request.answer.send(Err(cancellation::unavailable()));
                        }
                        return Ok(());
                    }
                    self.handle_work_event(event, cancel).await;
                    continue;
                }
                next = self.output.next() => next,
            };
            let Some(line) = next else {
                return Ok(());
            };
            let line = line
                .map_err(|error| AppError::Internal(format!("invalid idle ACP frame: {error}")))?;
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line)?;
            let Some(method) = message["method"].as_str() else {
                self.complete_native_cancellation(&message);
                continue;
            };
            let params = &message["params"];
            if let Some(id) = message.get("id") {
                if self.pending.contains_key(id) {
                    continue;
                }
                if self.pending.len() >= MAX_PEER_REQUESTS {
                    self.write(
                        &Reply::Error(-32603, "Too many active ACP client requests").message(id),
                        cancel,
                    )
                    .await?;
                    continue;
                }
                if let Some(session) = params["sessionId"].as_str()
                    && super::state::accepts_session(&state, &resume.native_id, session)
                {
                    let label = state
                        .details
                        .subagents
                        .get(session)
                        .and_then(|child| child.title.as_ref())
                        .map_or_else(
                            || self.agent.label().to_owned(),
                            |child| format!("{} · {child}", self.agent.label()),
                        );
                    let input = match method {
                        "elicitation/create" => Some((
                            InteractionKind::Form,
                            input_requests::NativeInput::elicitation(params, &label),
                        )),
                        "session/request_permission" => {
                            let normalized =
                                self.uses_v2().then(|| protocol::permission_params(params));
                            Some((
                                InteractionKind::Permission,
                                input_requests::NativeInput::permission(
                                    normalized.as_ref().unwrap_or(params),
                                    &label,
                                ),
                            ))
                        }
                        _ => None,
                    };
                    if let Some((kind, input)) = input {
                        self.begin_passive_input(id, kind, params, input, host, replies, cancel)
                            .await?;
                        continue;
                    }
                }
                let reply = match method {
                    "session/request_permission" => InteractionKind::Permission.cancelled(),
                    "elicitation/create" => InteractionKind::Form.cancelled(),
                    "mcp/message" => Reply::Error(-32602, "The Agena MCP grant has expired"),
                    _ => Reply::Error(-32601, "This ACP client request requires an active turn"),
                };
                self.write(&reply.message(id), cancel).await?;
                continue;
            }
            let changed = match method {
                "$/cancel_request" => {
                    if let Some(pending) =
                        params.get("requestId").and_then(|id| self.pending.get(id))
                    {
                        pending.withdrawal.cancel();
                    }
                    false
                }
                "elicitation/complete"
                    if elicitation_id(params)
                        .is_some_and(|id| self.url_elicitations.remove(id).is_some()) =>
                {
                    super::state::apply_update(
                        &mut state,
                        &json!({
                            "sessionUpdate": "notice",
                            "severity": "info",
                            "title": "Authorization completed",
                            "description": self.agent.label(),
                        }),
                    )
                }
                "auth/status_update" => {
                    self.observe_auth_status(params);
                    false
                }
                "session/update" => {
                    let Some(session) = params["sessionId"].as_str() else {
                        continue;
                    };
                    let mut update = params["update"].clone();
                    self.normalize_session_update(&mut update);
                    self.observe_native_work(session, &update);
                    if session == resume.native_id
                        && let Some(change) = cursors::change(
                            &update,
                            self.capabilities.is_some_and(|caps| caps.air_rewind),
                        )
                    {
                        let request = super::RequestId::new();
                        super::persist(|| {
                            kernel.executor.update_idle_external_agent_cursor(
                                request,
                                resume,
                                change.clone(),
                            )
                        })
                        .await
                        .map_err(super::kernel_error)?;
                        continue;
                    }
                    if self.uses_v2()
                        && session == resume.native_id
                        && let Some(message) = MessageUpdate::parse(&update)
                    {
                        let applied = tokio::select! {
                            biased;
                            _ = cancel.cancelled() => return Ok(()),
                            result = messages.apply(MessageTarget::Parked(kernel, resume, self.agent), message, false) => result?,
                        };
                        if applied {
                            continue;
                        }
                    }
                    super::state::apply_native_update(
                        &mut state,
                        &resume.native_id,
                        session,
                        &update,
                    ) || (session == resume.native_id
                        && super::state::record_idle_message(&mut state, &update))
                }
                _ => false,
            };
            if changed {
                let request = super::RequestId::new();
                let snapshot = state.bounded_snapshot(super::state::MAX_STATE_SNAPSHOT_BYTES);
                super::persist(|| {
                    kernel.executor.refresh_idle_external_agent_state(
                        request,
                        resume,
                        snapshot.clone(),
                    )
                })
                .await
                .map_err(super::kernel_error)?;
            }
        }
    }

    pub(super) async fn close_native_session_if_supported(&mut self, session: &str) {
        if self.capabilities.is_some_and(|caps| caps.close) {
            let cancel = CancellationToken::new();
            let _ = self
                .request_for(
                    Duration::from_secs(2),
                    "session/close",
                    json!({"sessionId": session}),
                    SessionUpdates::Ignore,
                    &cancel,
                )
                .await;
        }
    }

    pub(super) async fn compact_antigravity(
        &mut self,
        native_id: &str,
        cancel: &CancellationToken,
    ) -> Result<u64, AppError> {
        self.release_native(native_id, false, cancel).await?;
        antigravity::compact(
            &self.cwd,
            &self.additional_directories,
            native_id,
            self.previous_antigravity_history,
            self.isolation.is_some(),
            cancel,
        )
        .await
    }

    pub(super) async fn close(mut self) {
        self.native_work = None;
        self.live_auth = None;
        self.abandon_request().await;
        while self.opening_replies.join_next().await.is_some() {}
        drop(self.input);
        if tokio::time::timeout(Duration::from_millis(300), self.child.wait())
            .await
            .is_ok_and(|result| result.is_ok())
        {
            return;
        }
        if let Err(error) = self.child.terminate(Duration::from_millis(500)).await {
            tracing::debug!(%error, "external agent process cleanup");
        }
    }
}
