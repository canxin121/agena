//! Host callbacks use the currently running tool capability. They never find
//! a mutable transcript part and cannot recover authority from an integer ID.
use super::execution::PermissionInvocation;
use super::*;
use agena_domain::{
    ToolInvocation, UserInputReply, UserInputReplyKind, UserInputRequest, UserInputSource,
};
use agena_plugin_host::sdk::host_api::AskUserResponse;
use tokio::sync::Semaphore;

mod callback_executor;
mod cordis;
mod mcp_apps;
mod mcp_elicitation;
mod mcp_sampling;
mod operator;
mod process_sandbox;
mod session_scope;

// ToolInvokeInput uses this existing wire sentinel for a call without a
// session. It never identifies a kernel session or a live transcript tool.
const OPERATOR_SESSION: i64 = -1;

// Each context can use two of the kernel's eight control reads. Keep slots
// available for status and cancellation when a tool fans out many requests.
pub(super) static HOST_POLICY_READS: Semaphore = Semaphore::const_new(3);

pub(super) fn parent_interaction_required(
    reason: &str,
) -> Box<agena_domain::CapabilityUnavailableResult> {
    Box::new(agena_domain::CapabilityUnavailableResult {
        capability: "user_interaction".into(),
        tool_name: None,
        reason: format!(
            "Subagents cannot request user decisions directly. {reason} Report the blocked operation and required decision or authorization to the parent agent."
        ),
        source: agena_domain::CapabilitySourceKind::AgentProfile,
        retryable: false,
    })
}

fn require_live_tool_for_hook_approval(
    prepared: &agena_tool::PreparedToolInvocation,
) -> Result<(), ToolError> {
    if let Some(reason) = &prepared.permission_ask {
        return Err(ToolError::CapabilityUnavailable(Box::new(
            agena_domain::CapabilityUnavailableResult {
                capability: "user_interaction".into(),
                tool_name: Some(prepared.invocation.name.clone()),
                reason: format!("{reason}. This tool request has no approval interaction."),
                source: agena_domain::CapabilitySourceKind::RuntimeConfiguration,
                retryable: false,
            },
        )));
    }
    Ok(())
}

impl SessionManager {
    pub(super) fn live_tool(
        &self,
        session: i64,
        call: i64,
    ) -> Result<(ToolHandle, Arc<RunControl>), AppError> {
        let live = self
            .tools
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&(session, call))
            .cloned()
            .ok_or_else(|| {
                AppError::Config("the host callback has no active tool authority".into())
            })?;
        if live.control.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        Ok((live.tool, live.control))
    }
    pub async fn require_host_invocation_permission(
        &self,
        session: i64,
        call: i64,
        invocation: ToolInvocation,
    ) -> Result<(), AppError> {
        if session == OPERATOR_SESSION {
            let authority = self.operator_invocation(call).map_err(tool_error)?;
            let checks = authority
                .executor
                .collect_permission_checks_for_invocation_in_session(&invocation, None)
                .await
                .map_err(tool_error)?;
            return self
                .authorize_operator_checks(&authority, checks)
                .await
                .map_err(tool_error);
        }
        let (tool, control) = self.live_tool(session, call)?;
        let state = control.state.clone();
        let context = self.execution_context_value(session_id(session)?).await?;
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(control.cancel.clone()));
        self.authorize_tool(
            &tool,
            PermissionInvocation {
                tool: &invocation,
                run_mod_check: false,
                hook_ask: None,
            },
            &executor,
            &control,
            &state,
        )
        .await
        .map_err(tool_error)
    }

    /// Read-only resolution of remembered rules. This never classifies an
    /// automatic action, creates an interaction, or turns `Ask` into a grant.
    pub async fn query_host_permission(
        &self,
        session: Option<i64>,
        check: agena_tool::ToolPermissionCheck,
    ) -> Result<agena_domain::PermissionDecision, AppError> {
        let key = super::execution::permission_key(&check.action)?;
        // Read-only background/plugin checks use the same owning-workspace
        // rule scope as foreground tool authorization.
        let workspace = match session {
            Some(id) if id != OPERATOR_SESSION => {
                self.kernel()
                    .reader
                    .metadata(session_id(id)?)
                    .await
                    .map_err(kernel_error)?
                    .workspace_id
            }
            _ => Some(self.current_workspace_id().await?),
        };
        if workspace.is_none()
            && let agena_domain::PermissionAction::PathAccess { target_path, .. } = &check.action
            && !std::path::Path::new(target_path).is_absolute()
        {
            return Err(AppError::Config(
                "Global sessions require a full absolute path".into(),
            ));
        }
        let rules = self
            .kernel_runtime
            .permissions
            .resolve(&key, session, workspace)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let rules = rules
            .into_iter()
            .map(|rule| agena_permission::RuleEntry {
                id: rule.id,
                revision_ms: rule.updated_at_ms.or(rule.created_at_ms),
                scope: rule.scope,
                source: rule.source,
                reason: rule.reason,
                operator: rule.operator,
                mode: rule.mode,
            })
            .collect::<Vec<_>>();
        Ok(agena_permission::apply_rules(&check.decision, &rules).decision)
    }

    /// Authorize plugin-owned I/O under the current live tool capability.
    /// Integer IDs alone cannot create an approval authority.
    pub async fn require_host_path_permission(
        &self,
        session: i64,
        call: i64,
        kind: agena_domain::PathKind,
        path: String,
    ) -> Result<(), AppError> {
        self.require_host_permission(session, call, move |executor| async move {
            if executor.is_global() && !std::path::Path::new(&path).is_absolute() {
                return Err(ToolError::invalid_input(
                    "Global sessions require a full absolute path",
                ));
            }
            let access = match kind {
                agena_domain::PathKind::Read => agena_domain::AccessKind::Read,
                agena_domain::PathKind::Write => agena_domain::AccessKind::Write,
            };
            Ok(executor.path_permission_check(access, &path))
        })
        .await
    }

    pub async fn require_host_network_permission(
        &self,
        session: i64,
        call: i64,
        target: String,
    ) -> Result<(), AppError> {
        self.require_host_network_check(session, call, move |executor| async move {
            executor.network_permission_check_async(&target).await
        })
        .await
    }

    pub async fn require_host_pinned_http_permission(
        &self,
        session: i64,
        call: i64,
        target: String,
    ) -> Result<(), AppError> {
        self.require_host_network_check(session, call, move |executor| async move {
            executor.pinned_http_permission_check_async(target).await
        })
        .await
    }

    async fn require_host_network_check<F>(
        &self,
        session: i64,
        call: i64,
        check: impl FnOnce(ToolExecutor) -> F,
    ) -> Result<(), AppError>
    where
        F: std::future::Future<Output = Result<agena_tool::ToolPermissionCheck, ToolError>>,
    {
        let admission = HOST_POLICY_READS
            .acquire()
            .await
            .expect("host policy admission is never closed");
        self.require_host_permission(session, call, move |executor| {
            // DNS preflight and user review can outlast the policy read.
            drop(admission);
            check(executor)
        })
        .await
    }

    async fn require_host_permission<F>(
        &self,
        session: i64,
        call: i64,
        check: impl FnOnce(ToolExecutor) -> F,
    ) -> Result<(), AppError>
    where
        F: std::future::Future<Output = Result<agena_tool::ToolPermissionCheck, ToolError>>,
    {
        if session == OPERATOR_SESSION {
            let authority = self.operator_invocation(call).map_err(tool_error)?;
            let executor = self
                .sandbox_executor_for_operator(&authority)
                .await
                .map_err(tool_error)?;
            let check = check(executor).await.map_err(tool_error)?;
            return self
                .authorize_operator_checks(&authority, vec![check])
                .await
                .map_err(tool_error);
        }
        // A session-scoped plugin callback has no live tool slot to hold the
        // grant cache or the approval interaction. Authorize each concrete
        // target against the session policy instead.
        if call < 0 {
            let key = session_id(session)?;
            let state = self.state_for_session(key).await?;
            let context = self.execution_context_with_state(key, &state).await?;
            let executor = state
                .executor
                .for_session_resource_context(&context)
                .with_cancellation_token(Some(self.shutdown.child_token()));
            let executor = self
                .sandbox_executor_for_session(&executor, session)
                .await
                .map_err(tool_error)?;
            let check = check(executor.clone()).await.map_err(tool_error)?;
            return self
                .authorize_session_scoped_checks(session, &executor, vec![check])
                .await;
        }
        let (tool, control) = self.live_tool(session, call)?;
        let state = control.state.clone();
        let context = self
            .execution_context_for_execution(session_id(session)?, &state, Some(control.execution))
            .await?;
        let executor = state
            .executor
            .for_session_resource_context(&context)
            .with_cancellation_token(Some(control.cancel.clone()));
        let executor = self
            .sandbox_executor_for_session(&executor, session)
            .await
            .map_err(tool_error)?;
        let check = check(executor.clone()).await.map_err(tool_error)?;
        self.authorize_checks(&tool, vec![check], &executor, &control, &state)
            .await
            .map_err(tool_error)
    }

    pub async fn request_host_user_input(
        &self,
        session: i64,
        call: i64,
        input: crate::part::AskUserToolInput,
    ) -> Result<AskUserResponse, AppError> {
        self.request_host_user_input_inner(session, call, input, None)
            .await
    }

    async fn request_cancellable_host_user_input(
        &self,
        session: i64,
        call: i64,
        input: crate::part::AskUserToolInput,
        cancellation: &CancellationToken,
    ) -> Result<AskUserResponse, AppError> {
        let cancellation = cancellation.child_token();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        let manager = self.clone();
        // Persistence may already be in progress when the callback is dropped.
        // The worker owns the request until it can durably close the dialog.
        let worker = self.control_tasks.spawn(async move {
            manager
                .request_host_user_input_inner(session, call, input, Some(cancellation))
                .await
        });
        agena_async::permission_wait::waiting(worker)
            .await
            .map_err(|error| AppError::Internal(format!("MCP user input worker failed: {error}")))?
    }

    async fn request_host_user_input_inner(
        &self,
        session: i64,
        call: i64,
        input: crate::part::AskUserToolInput,
        cancellation: Option<CancellationToken>,
    ) -> Result<AskUserResponse, AppError> {
        let (tool, control) = self.live_tool(session, call)?;
        let configuration = self
            .kernel()
            .reader
            .configuration(control.session)
            .await
            .map_err(kernel_error)?;
        if configuration.relation_kind.is_subagent() {
            return Err(AppError::CapabilityUnavailable(
                parent_interaction_required(
                    "The parent handles user questions and approval workflows.",
                ),
            ));
        }
        if cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Ok(AskUserResponse {
                cancelled: true,
                ..Default::default()
            });
        }
        let sequence = {
            let mut sequences = self
                .host_sequences
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let sequence = sequences.entry((session, call)).or_default();
            let result = *sequence;
            *sequence += 1;
            result
        };
        let command = request_key("host-input", &format!("{session}:{call}:{sequence}"));
        let timeout =
            Duration::from_millis(input.auto_resolution_ms.unwrap_or(600_000).min(86_400_000));
        let request = UserInputRequest {
            request_id: command.to_string(),
            session_id: Some(session),
            title: input.title,
            body_markdown: input.body_markdown,
            kind: input.kind.into(),
            source: UserInputSource::Host,
            auto_resolution_ms: Some(timeout.as_millis() as u64),
            presented_at: None,
            questions: input.questions,
            created_at: Utc::now(),
        };
        let interaction = InteractionRequest::Input {
            request: request.clone(),
        };
        let id = persist(|| tool.request_interaction(command, interaction.clone()))
            .await
            .map_err(kernel_error)?
            .value;
        control.phase(agena_domain::ExecutionPhase::AwaitingInteraction);
        let wait = tokio::time::timeout(
            timeout,
            self.wait_interaction(control.session, id, Some(&control.cancel)),
        );
        let decision = if let Some(cancellation) = cancellation {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => None,
                result = wait => Some(result),
            }
        } else {
            Some(wait.await)
        };
        control.phase(agena_domain::ExecutionPhase::ExecutingTools);
        let decision = match decision {
            Some(Ok(decision)) => decision?,
            other => {
                let timed_out = other.is_some();
                let command = request_key("host-input-closed", &id.get().to_string());
                // A racing user reply remains authoritative and is read again
                // after the close command.
                match persist(|| {
                    self.kernel().sessions.reply(
                        command,
                        control.session,
                        id,
                        InteractionDecision::Cancel,
                        None,
                    )
                })
                .await
                {
                    Ok(_) => {
                        return Ok(AskUserResponse {
                            cancelled: !timed_out,
                            timed_out,
                            ..Default::default()
                        });
                    }
                    Err(KernelError::Conflict(_)) => self
                        .kernel()
                        .reader
                        .interaction(control.session, id)
                        .await
                        .map_err(kernel_error)?
                        .decision
                        .clone()
                        .ok_or_else(|| {
                            AppError::Internal("the input interaction could not be closed".into())
                        })?,
                    Err(error) => return Err(kernel_error(error)),
                }
            }
        };
        match decision {
            InteractionDecision::Input(text) => {
                let reply: UserInputReply = serde_json::from_str(&text)?;
                super::commands::validate_answers(&request, &reply)?;
                Ok(AskUserResponse {
                    reply: reply
                        .answers
                        .get("0")
                        .and_then(|a| a.first())
                        .cloned()
                        .unwrap_or_default(),
                    cancelled: reply.kind == UserInputReplyKind::Cancel,
                    timed_out: reply.kind == UserInputReplyKind::Timeout,
                    answers: reply.answers,
                })
            }
            InteractionDecision::Cancel => Ok(AskUserResponse {
                cancelled: true,
                ..Default::default()
            }),
            _ => Err(AppError::Internal(
                "an input interaction received an incompatible decision".into(),
            )),
        }
    }
    pub async fn execute_host_invoked_tool(
        &self,
        session: i64,
        call: i64,
        invocation: ToolInvocation,
        run_mod_hooks: bool,
    ) -> Result<ToolInvocationExecution, AppError> {
        if session == OPERATOR_SESSION {
            return self
                .execute_operator_invocation(call, &invocation)
                .await
                .map_err(tool_error);
        }
        let (tool, control) = self.live_tool(session, call)?;
        let key = session_id(session)?;
        let state = if run_mod_hooks {
            control.state.clone()
        } else {
            self.state_for_session(key).await?
        };
        let context = self
            .execution_context_for_execution(key, &state, Some(control.execution))
            .await?;
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(control.cancel.clone()));
        let manager = self.clone();
        let authorizer = executor.clone();
        let authority_tool = tool.clone();
        let authority_control = control.clone();
        let authority_state = state.clone();
        let runtime = tokio::runtime::Handle::current();
        let executor = executor.with_path_effect_authorizer(Arc::new(move |access, path| {
            let check = authorizer.path_permission_check(access, &path.to_string_lossy());
            runtime.block_on(agena_async::permission_wait::scope(
                Some((session, call)),
                agena_async::permission_wait::waiting(manager.authorize_checks(
                    &authority_tool,
                    vec![check],
                    &authorizer,
                    &authority_control,
                    &authority_state,
                )),
            ))
        }));
        let prepared = executor
            .prepare_invocation(&invocation, session, call)
            .await
            .map_err(tool_error)?;
        let (invocation, shell) = executor
            .prepare_shell_invocation(&prepared.invocation, session, call)
            .await
            .map_err(tool_error)?;
        self.authorize_tool(
            &tool,
            PermissionInvocation {
                tool: &invocation,
                run_mod_check: run_mod_hooks,
                hook_ask: prepared.permission_ask.as_deref(),
            },
            &executor,
            &control,
            &state,
        )
        .await
        .map_err(tool_error)?;
        let executor = self
            .sandbox_executor_for_session(&executor, session)
            .await
            .map_err(tool_error)?;
        let output = match executor
            .streaming_content_kind(&invocation)
            .map_err(tool_error)?
        {
            Some(kind) => Some(
                crate::tool::ToolOutputStreams::open(&tool, kind)
                    .await
                    .map_err(kernel_error)?,
            ),
            None => None,
        };
        let context = crate::tool::ToolRuntimeContext {
            session_id: Some(session),
            call_id: Some(call),
            prepared_shell_command: shell,
            launch_provenance: None,
            output,
            tool: Some(tool),
        };
        let result = if run_mod_hooks {
            executor.run_authorized_tool(&invocation, context).await
        } else {
            executor.execute_invocation(&invocation, context).await
        };
        match result {
            Ok(mut execution) => {
                execution
                    .view
                    .additional_context
                    .splice(..0, prepared.additional_context);
                Ok(execution)
            }
            Err(ToolError::UserInputRequired(input)) => {
                let response = self.request_host_user_input(session, call, *input).await?;
                Ok(input_execution(response))
            }
            Err(error) => Err(tool_error(error)),
        }
    }
}
pub(super) fn input_execution(response: AskUserResponse) -> ToolInvocationExecution {
    let text = if response.timed_out {
        "User input timed out; no approval was given.".to_owned()
    } else if response.cancelled {
        "User input was cancelled.".to_owned()
    } else {
        response
            .answers
            .iter()
            .map(|(question, answers)| format!("{question}: {}", answers.join(", ")))
            .collect::<Vec<_>>()
            .join("\n")
    };
    ToolInvocationExecution::new(
        crate::tool::ToolPayloadOutput::AskUser {
            answers: response.answers,
            timed_out: response.timed_out,
        }
        .into_tool_output(),
        crate::tool::ToolExecutionView::simple(
            "Ask user",
            if response.timed_out {
                "Timed out"
            } else if response.cancelled {
                "Cancelled"
            } else {
                "Answered"
            },
            text,
        ),
    )
}
#[async_trait::async_trait]
impl crate::SessionToolExecutionService for SessionManager {
    fn mcp_app_views(&self) -> Option<Arc<crate::McpAppViews>> {
        Some(self.mcp_app_views.clone())
    }

    async fn execute_mcp_app_tool(
        &self,
        id: i64,
        invocation: ToolInvocation,
    ) -> Result<agena_tool::ToolExecutionSummary, crate::SessionToolExecutionError> {
        self.execute_session_scoped_app_tool(id, invocation)
            .await
            .map(|execution| execution.summary())
            .map_err(|error| crate::SessionToolExecutionError::Execution(error.to_string()))
    }

    async fn sample_mcp_app(
        &self,
        scope: agena_domain::mcp_app_tools::McpAppViewScope,
        params: serde_json::Value,
        cancellation: CancellationToken,
    ) -> agena_mcp_client::McpResult<serde_json::Value> {
        self.sample_from_mcp_app(scope, params, cancellation).await
    }

    async fn execute_session_tool(
        &self,
        id: i64,
        invocation: ToolInvocation,
    ) -> Result<crate::SessionToolExecutionOutcome, crate::SessionToolExecutionError> {
        let result = async {
            let session = session_id(id)?;
            let state = self.state_for_session(session).await?;
            let context = self.execution_context_with_state(session, &state).await?;
            self.prepare_imported_mod_session(&state.executor, id, &context)
                .await;
            let executor = state.executor.for_session_context_async(&context).await;
            if executor.is_claude_mod_tool(&invocation) {
                return self.execute_session_scoped_mod_tool(id, invocation, false).await;
            }
            let prepared = executor
                .prepare_invocation(&invocation, id, -1)
                .await
                .map_err(tool_error)?;
            require_live_tool_for_hook_approval(&prepared).map_err(tool_error)?;
            let (invocation, shell) = executor
                .prepare_shell_invocation(&prepared.invocation, id, -1)
                .await
                .map_err(tool_error)?;
            let executor = self
                .sandbox_executor_for_session(&executor, id)
                .await
                .map_err(tool_error)?;
            executor
                .execute_invocation(
                    &invocation,
                    crate::tool::ToolRuntimeContext {
                        session_id: Some(id),
                        call_id: None,
                        prepared_shell_command: shell,
                        launch_provenance: None,
                        output: None,
                        tool: None,
                    },
                )
                .await
                .map_err(tool_error)
        }
        .await;
        match result {
            Ok(execution) => Ok(crate::SessionToolExecutionOutcome::Completed(
                execution.summary(),
            )),
            Err(AppError::CapabilityUnavailable(value)) => Ok(
                crate::SessionToolExecutionOutcome::CapabilityUnavailable(value),
            ),
            Err(AppError::ToolUnavailable(value)) => {
                Ok(crate::SessionToolExecutionOutcome::ToolUnavailable(value))
            }
            Err(error) => Err(crate::SessionToolExecutionError::Execution(
                error.to_string(),
            )),
        }
    }
}

impl SessionManager {
    pub async fn resolve_plugin_tool_for_session(
        &self,
        id: i64,
        plugin_id: Option<&str>,
        tool_name: &str,
    ) -> Result<Option<agena_plugin_host::registry::RegisteredTool>, AppError> {
        let session = session_id(id)?;
        let state = self.state_for_session(session).await?;
        let plugins = state.executor.plugin_manager();
        if plugins.has_imported_mod_session_start() {
            let context = self.execution_context_with_state(session, &state).await?;
            self.prepare_imported_mod_session(&state.executor, id, &context).await;
        }
        let scope = agena_plugin_host::PluginScopeKey::session(id);
        Ok(match plugin_id {
            Some(plugin_id) => plugins.resolve_registered_tool_for_plugin_tool(
                plugin_id, tool_name, Some(&scope),
            ),
            None => plugins.lookup_tool_for_name_scope(tool_name, Some(&scope)),
        })
    }

    pub async fn prepare_imported_mod_commands(&self, id: i64) -> Result<(), AppError> {
        let state = self.state_for_session(session_id(id)?).await?;
        if !state.executor.plugin_manager().has_imported_mod_session_start() {
            return Ok(());
        }
        let session = self.get_session(id).await?;
        self.prepare_imported_mod_session(&state.executor, id, &session.execution)
            .await;
        Ok(())
    }

    pub(super) async fn prepare_imported_mod_session(
        &self,
        executor: &ToolExecutor,
        id: i64,
        context: &crate::session::model::SessionExecutionContext,
    ) {
        use agena_plugin_host::sdk::{SessionStartInput, SessionStartSource};

        let plugins = executor.plugin_manager();
        if !plugins.has_imported_mod_session_start()
            || context.global
            || context.owner.is_some()
            || context.conversation.as_ref().is_some_and(|session| session.is_read_only())
        {
            return;
        }
        let executor = executor.for_session_resource_context(context);
        plugins
            .prepare_imported_mod_session(
                &SessionStartInput {
                    session_id: id,
                    workspace_root: executor.workspace_root().to_string_lossy().into_owned(),
                    source: SessionStartSource::Resume,
                    model: context.selection.model.clone(),
                    session_title: None,
                    agent_type: None,
                    resumed_cache: None,
                },
                |plugin| executor.plugin_is_enabled(plugin),
            )
            .await;
    }
}
