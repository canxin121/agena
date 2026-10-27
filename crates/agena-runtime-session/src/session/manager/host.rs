//! Host callbacks use the currently running tool capability. They never find
//! a mutable transcript part and cannot recover authority from an integer ID.
use super::*;
use agena_domain::{
    ToolInvocation, UserInputReply, UserInputReplyKind, UserInputRequest, UserInputSource,
};
use agena_plugin_host::sdk::host_api::AskUserResponse;

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

impl SessionManager {
    pub async fn execute_unscoped_tool(
        &self,
        invocation: ToolInvocation,
        call_id: i64,
    ) -> Result<crate::SessionToolExecutionOutcome, AppError> {
        let executor = self.tool_executor();
        let execution = executor
            .execute_invocation(
                &invocation,
                crate::tool::ToolRuntimeContext {
                    session_id: None,
                    call_id: Some(call_id),
                    prepared_shell_command: None,
                    launch_provenance: None,
                    output: None,
                    tool: None,
                },
            )
            .await
            .map_err(tool_error)?;
        Ok(crate::SessionToolExecutionOutcome::Completed(
            execution.summary(),
        ))
    }
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
        let (tool, control) = self.live_tool(session, call)?;
        let state = control.state.clone();
        let context = self.execution_context_value(session_id(session)?).await?;
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(control.cancel.clone()));
        self.authorize_tool(&tool, &invocation, &executor, &control, &state)
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
        let workspace = self.current_workspace_id().await?;
        let rules = self
            .kernel_runtime
            .permissions
            .resolve(&key, session, Some(workspace))
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
        self.require_host_permission(session, call, move |executor| {
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
        self.require_host_permission(session, call, move |executor| {
            executor.network_permission_check(&target)
        })
        .await
    }

    async fn require_host_permission(
        &self,
        session: i64,
        call: i64,
        check: impl FnOnce(&ToolExecutor) -> Result<agena_tool::ToolPermissionCheck, ToolError>,
    ) -> Result<(), AppError> {
        let (tool, control) = self.live_tool(session, call)?;
        let state = control.state.clone();
        let context = self.execution_context_value(session_id(session)?).await?;
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(control.cancel.clone()));
        let check = check(&executor).map_err(tool_error)?;
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
        let decision = tokio::time::timeout(
            timeout,
            self.wait_interaction(control.session, id, Some(&control.cancel)),
        )
        .await;
        control.phase(agena_domain::ExecutionPhase::ExecutingTools);
        match decision {
            Ok(Ok(InteractionDecision::Input(text))) => {
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
            Ok(Ok(InteractionDecision::Cancel)) => Ok(AskUserResponse {
                cancelled: true,
                ..Default::default()
            }),
            Ok(Ok(_)) => Err(AppError::Internal(
                "an input interaction received an incompatible decision".into(),
            )),
            Ok(Err(error)) => Err(error),
            Err(_) => {
                let command = request_key("host-input-timeout", &id.get().to_string());
                // A timeout is a durable negative decision. A racing user reply
                // remains authoritative and is read again after the command.
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
                    Ok(_) => Ok(AskUserResponse {
                        timed_out: true,
                        ..Default::default()
                    }),
                    Err(KernelError::Conflict(_)) => {
                        let view = self
                            .kernel()
                            .reader
                            .interaction(control.session, id)
                            .await
                            .map_err(kernel_error)?;
                        match view.decision.as_ref() {
                            Some(InteractionDecision::Input(text)) => {
                                let reply: UserInputReply = serde_json::from_str(text)?;
                                super::commands::validate_answers(&request, &reply)?;
                                Ok(AskUserResponse {
                                    reply: reply
                                        .answers
                                        .get("0")
                                        .and_then(|a| a.first())
                                        .cloned()
                                        .unwrap_or_default(),
                                    answers: reply.answers,
                                    ..Default::default()
                                })
                            }
                            _ => Ok(AskUserResponse {
                                cancelled: true,
                                ..Default::default()
                            }),
                        }
                    }
                    Err(error) => Err(kernel_error(error)),
                }
            }
        }
    }
    pub async fn execute_host_invoked_tool(
        &self,
        session: i64,
        call: i64,
        invocation: ToolInvocation,
    ) -> Result<ToolInvocationExecution, AppError> {
        let (tool, control) = self.live_tool(session, call)?;
        let state = self.state.load_full();
        let context = self.execution_context_value(session_id(session)?).await?;
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(control.cancel.clone()));
        let prepared = executor
            .prepare_invocation(&invocation, session, call)
            .await
            .map_err(tool_error)?;
        let (invocation, shell) = executor
            .prepare_shell_invocation(&prepared.invocation, session, call)
            .await
            .map_err(tool_error)?;
        self.authorize_tool(&tool, &invocation, &executor, &control, &state)
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
        match executor
            .execute_invocation(
                &invocation,
                crate::tool::ToolRuntimeContext {
                    session_id: Some(session),
                    call_id: Some(call),
                    prepared_shell_command: shell,
                    launch_provenance: None,
                    output,
                    tool: Some(tool),
                },
            )
            .await
        {
            Ok(execution) => Ok(execution),
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
    async fn execute_session_tool(
        &self,
        id: i64,
        invocation: ToolInvocation,
    ) -> Result<crate::SessionToolExecutionOutcome, crate::SessionToolExecutionError> {
        let result = async {
            let context = self.execution_context_value(session_id(id)?).await?;
            let executor = self
                .tool_executor()
                .for_session_context_async(&context)
                .await;
            let prepared = executor
                .prepare_invocation(&invocation, id, -1)
                .await
                .map_err(tool_error)?;
            let (invocation, shell) = executor
                .prepare_shell_invocation(&prepared.invocation, id, -1)
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
#[async_trait::async_trait]
impl crate::SessionPluginCommandService for SessionManager {
    async fn invoke_session_plugin_command(
        &self,
        request: crate::SessionPluginCommandRequest,
    ) -> Result<agena_plugin_host::sdk::CommandResult, crate::SessionPluginCommandError> {
        self.get_session(request.session_id)
            .await
            .map_err(|e| crate::SessionPluginCommandError::Execution(e.to_string()))?;
        self.tool_executor()
            .plugin_manager()
            .invoke_plugin_command_async(
                &request.plugin_id,
                agena_plugin_host::sdk::CommandInvokeInput {
                    session_id: Some(request.session_id),
                    call_id: None,
                    workspace_root: request.workspace_root,
                    command_id: request.command_id,
                    slash: request.slash,
                    raw: request.raw,
                    input: request.input,
                },
            )
            .await
            .map_err(|e| crate::SessionPluginCommandError::Execution(e.to_string()))
    }
}
