//! An installed ACP agent drives inference. Its MCP tool calls are ordinary
//! session tool effects, owned by the current kernel attempt.
use super::*;
use crate::session::kernel::{KernelProviderState, input::read_text};
use agena_domain::{
    ActivityPayload, ExternalAgent, MessageAuthor, MessageOrigin, ResourceDelivery, ResourceKind,
    ResourceReference, UserInputOption, UserInputQuestion,
};
use agena_storage::kernel::{ExternalAgentPendingTurn, ExternalAgentPromptDelivery};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod acp;
pub(super) mod auth;
pub(super) mod cancellation;
mod client_tools;
mod elicitation;
pub(super) mod idle;
pub(super) mod input_requests;
mod lifecycle;
mod mcp;
mod options;
mod prompt;
mod state;
pub(super) mod steering;
mod transcript;

pub(super) use elicitation::FORM_INPUT_KIND;

pub(super) fn require_direct_user(
    author: Option<&MessageAuthor>,
    origin: Option<&MessageOrigin>,
) -> Result<(), AppError> {
    if author.is_some_and(MessageAuthor::is_instance_operator)
        && origin.is_some_and(|origin| origin.platform.is_local())
    {
        Ok(())
    } else {
        Err(AppError::Config(
            "External agent sessions accept direct user messages.".into(),
        ))
    }
}

pub(super) fn validate_document(document: &ComposerDocument) -> Result<(), AppError> {
    let supported = document.0.iter().all(|node| match node {
        ComposerNode::Text { .. } => true,
        ComposerNode::Activity { activity } => match &activity.payload {
            ActivityPayload::SkillReference(_) | ActivityPayload::TextArtifact(_) => true,
            ActivityPayload::Resource(resource) => match resource.delivery {
                ResourceDelivery::Reference => matches!(
                    resource.reference,
                    ResourceReference::WorkspacePath { .. } | ResourceReference::Url { .. }
                ),
                ResourceDelivery::ModelInput => {
                    !matches!(resource.kind, ResourceKind::Directory | ResourceKind::Url)
                }
            },
            _ => false,
        },
    });
    if !supported {
        return Err(AppError::Config(
            "External agent sessions accept text, attachments, and references to project files or skills.".into(),
        ));
    }
    Ok(())
}

impl SessionManager {
    pub(super) fn list_external_agents(&self) -> Vec<agena_domain::ExternalAgentCatalogEntry> {
        self.global_state
            .load_full()
            .config
            .acp
            .resolve_catalog()
            .into_iter()
            .map(|entry| entry.catalog_entry())
            .collect()
    }

    pub(super) fn resolve_external_agent(
        &self,
        agent: &ExternalAgent,
    ) -> Result<agena_domain::ResolvedExternalAgent, AppError> {
        self.global_state
            .load_full()
            .config
            .acp
            .resolve(agent)
            .ok_or_else(|| {
                AppError::Config(format!(
                    "External agent `{}` is not configured or is disabled. Configure it under acp.agents in global settings.",
                    agent.id()
                ))
            })
    }

    pub(super) async fn external_agent(
        &self,
        session: SessionId,
    ) -> Result<Option<ExternalAgent>, AppError> {
        let agent = read_with_busy_retry(session, "external_agent", || {
            self.kernel().reader.configuration(session)
        })
        .await?
        .external_agent
        .clone();
        if agent.is_some() && !self.is_acp_scope() {
            return Err(AppError::Config(
                "Open the native conversation in ACP sessions.".into(),
            ));
        }
        Ok(agent)
    }

    pub(super) async fn external_agent_prompt_preview(
        &self,
        session: SessionId,
        agent: ExternalAgent,
    ) -> Result<agena_prompt_layers::PromptResolution, AppError> {
        use agena_prompt_layers::{
            FragmentId, PromptResolution, PromptScope, PromptWarning, PromptWarningKind,
        };

        let _permit = self.reserve_execution()?;
        let state = self.state_for_session(session).await?;
        let context = self.execution_context_with_state(session, &state).await?;
        let executor = state.executor.for_session_context_async(&context).await;
        let cwd = lifecycle::canonical_directory(executor.workspace_root()).await?;
        let spec = self.resolve_external_agent(&agent)?;
        let mut client = acp::AcpClient::spawn(spec.clone(), &cwd)?;
        let supports_prompt = client
            .lifecycle_capabilities(&self.shutdown.child_token())
            .await;
        client.close().await;
        if !supports_prompt?.system_prompt {
            return Ok(PromptResolution::from_fragments(
                Vec::new(),
                vec![PromptWarning {
                    id: FragmentId::parse("agena.runtime.acp-system-prompt")
                        .expect("valid ACP fragment id"),
                    scope: PromptScope::Runtime,
                    kind: PromptWarningKind::ConditionNotMatched,
                    detail: format!(
                        "{} does not advertise appended system instructions over ACP.",
                        spec.name
                    ),
                }],
            ));
        }
        Ok(super::session_prompt::external_agent_system_prompt(
            &executor, &state, &context, agent, session, None,
        )
        .await)
    }

    pub(super) async fn inspect_external_agent_providers(
        &self,
        id: i64,
    ) -> Result<agena_domain::ExternalAgentProviders, AppError> {
        let session = session_id(id)?;
        self.require_writable_bot_history(id).await?;
        let agent = self
            .external_agent(session)
            .await?
            .ok_or_else(|| AppError::Config("This session has no external agent.".into()))?;
        let spec = self.resolve_external_agent(&agent)?;
        let _permit = self.reserve_execution()?;
        let state = self.state_for_session(session).await?;
        let context = self
            .execution_context_for_execution(session, &state, None)
            .await?;
        let executor = state.executor.for_session_context_async(&context).await;
        let cwd = lifecycle::canonical_directory(executor.workspace_root()).await?;
        let cancel = self.shutdown.child_token();
        let mut client = acp::AcpClient::spawn(spec, &cwd)?;
        let result = client.provider_catalog(&cancel).await;
        client.close().await;
        result
    }

    pub(super) async fn run_external_agent(
        &self,
        agent: ExternalAgent,
        attempt: &AttemptHandle,
        execution: &ExecutionView,
        control: Arc<RunControl>,
        state: Arc<ExecutionState>,
        system: Option<&str>,
    ) -> Result<(), AppError> {
        self.close_idle_external_agent(control.session).await;
        control.phase(agena_domain::ExecutionPhase::PreparingModel);
        let context = self
            .execution_context_for_execution(control.session, &state, Some(control.execution))
            .await?;
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(control.cancel.clone()));
        let cwd = lifecycle::canonical_directory(executor.workspace_root()).await?;
        let configuration = read_with_busy_retry(control.session, "external_agent_context", || {
            self.kernel().reader.configuration(control.session)
        })
        .await?;
        let directories = lifecycle::directories_for_agent(
            &executor,
            &cwd,
            &configuration.external_agent_directories,
        )
        .await?;
        let mut session_context = lifecycle::session_context(
            agent.clone(),
            control.session,
            &executor,
            &state,
            &context,
            directories,
            system,
        )
        .await;

        let command = RequestId::new();
        let opened = persist(|| attempt.begin_external_agent_round(command))
            .await
            .map_err(kernel_error)?
            .value;
        session_context
            .selected_options
            .clone_from(&opened.selection);
        let round = opened.round;
        let facts =
            read_with_busy_retry(control.session, "external_agent_prompt", || round.prompt())
                .await?;
        let input = prompt::TurnInput::new(&facts, execution)?;
        let retry = opened
            .pending
            .as_ref()
            .filter(|prior| prior.message == execution.message_id);
        let delivery = retry.map_or(ExternalAgentPromptDelivery::Unsent, |prior| prior.delivery);
        let keyed_retry = match delivery {
            ExternalAgentPromptDelivery::Accepted => {
                return Err(AppError::ExternalAgentPromptAlreadyAccepted);
            }
            ExternalAgentPromptDelivery::Unconfirmed => {
                return Err(AppError::ExternalAgentPromptUncertain);
            }
            ExternalAgentPromptDelivery::Keyed { prompt } => Some(prompt),
            ExternalAgentPromptDelivery::Unsent => None,
        };
        let previous = opened
            .previous
            .filter(|prior| prior.cwd == cwd && input.can_resume(prior));
        let native_resume = previous
            .as_ref()
            .map(|prior| prior.native_id.as_str())
            .or_else(|| {
                opened
                    .link
                    .as_ref()
                    .filter(|link| {
                        link.cwd == cwd
                            && (opened
                                .pending
                                .as_ref()
                                .is_some_and(|pending| input.can_resume_pending(pending))
                                || link
                                    .resume_at
                                    .as_ref()
                                    .is_some_and(|anchor| input.can_resume_anchor(anchor)))
                    })
                    .map(|link| link.native_id.as_str())
            });
        if keyed_retry.is_some() && native_resume.is_none() {
            return Err(AppError::ExternalAgentPromptUncertain);
        }
        let spec = self.resolve_external_agent(&agent)?;
        let mut client = acp::AcpClient::spawn_with_directories(
            spec.clone(),
            &cwd,
            &session_context.additional_directories,
        )?;
        client.track_auth_status(auth::LiveStatus::register(self, control.session));
        let capabilities = client.prompt_capabilities(&control.cancel).await?;
        if keyed_retry.is_some() && !client.uses_v2() {
            return Err(AppError::ExternalAgentPromptUncertain);
        }
        let (mut blocks, media_leases) = input
            .blocks_with_capabilities(
                &self.kernel().reader,
                control.session,
                &agent.run_identity(),
                agent.clone(),
                &cwd,
                capabilities,
            )
            .await?;
        if !session_context.system_prompt.trim().is_empty()
            && !client
                .lifecycle_capabilities(&control.cancel)
                .await?
                .system_prompt
        {
            let notice = serde_json::to_string(&agena_runtime_contracts::NoticePart {
                kind: "external_agent_system_prompt_unavailable".into(),
                summary: format!(
                    "{} cannot accept Agena's additional system instructions in this session.",
                    spec.name
                ),
                detail: None,
                title: Some("Agena system instructions unavailable".into()),
            })?;
            let request = RequestId::new();
            persist(|| attempt.notice(request, notice.clone()))
                .await
                .map_err(kernel_error)?;
        }
        let has_images = blocks.iter().any(|block| block["type"] == "image");
        let turn = Arc::new(ExternalTurn {
            manager: self.clone(),
            control: control.clone(),
            attempt: attempt.clone(),
            executor,
            round: round.clone(),
            agent: agent.clone(),
            transcript: Default::default(),
            terminals: Default::default(),
        });
        let bridge = client.bridge(turn.clone()).await?;
        let mut native = None;
        let outcome = async {
            let mut session = client
                .start_with_context(
                    &cwd,
                    bridge.as_ref().map(mcp::Bridge::descriptor),
                    native_resume.map(|id| (id, &opened.state)),
                    has_images,
                    if keyed_retry.is_some() {
                        acp::OpenPurpose::RetryPrompt
                    } else {
                        acp::OpenPurpose::Prompt
                    },
                    &session_context,
                    &control.cancel,
                )
                .await?;
            let incarnation = opened
                .link
                .as_ref()
                .filter(|link| session.resumed && link.native_id == session.id)
                .and_then(|link| link.incarnation)
                .unwrap_or_else(|| round.id());
            let link = native.insert(ExternalAgentLink {
                native_id: session.id.clone(),
                cwd: cwd.clone(),
                incarnation: Some(incarnation),
                resume_at: None,
            });
            let command = RequestId::new();
            let pending = input.pending_turn(round.id(), delivery)?;
            persist(|| round.bind_external_agent_link(command, link.clone(), pending.clone()))
                .await
                .map_err(kernel_error)?;
            for (kind, value) in &opened.selection {
                client
                    .set_option(&mut session, kind, value, &control.cancel)
                    .await?;
            }
            let routed_command =
                prompt::route_command(&mut blocks, &session.state.commands, capabilities)?;
            let delivery = if client.uses_v2() && !routed_command {
                let prompt = Sha256::digest(serde_json::to_vec(&blocks)?).into();
                if keyed_retry.is_some_and(|previous| prompt != previous) {
                    return Err(AppError::ExternalAgentPromptUncertain);
                }
                ExternalAgentPromptDelivery::Keyed { prompt }
            } else {
                ExternalAgentPromptDelivery::Unconfirmed
            };
            let replayed_users = session.replacements.take_replayed_users();
            let activity = session.activity;
            session.state.options = session.options.clone();
            turn.transcript
                .lock()
                .await
                .start_native_session(
                    &turn,
                    link.clone(),
                    &opened.state,
                    session.state,
                    session.replacements.into_updates(),
                )
                .await?;
            // ACP owns the conversation history. A new native session receives
            // the current user's composer document.
            if !session.resumed && (input.has_prior_turns() || session.missing_previous) {
                let summary = if session.missing_previous {
                    format!(
                        "{} no longer has the linked conversation and opened a new one.",
                        spec.name
                    )
                } else {
                    format!(
                        "{} opened a new conversation with fresh context.",
                        spec.name
                    )
                };
                let notice = serde_json::to_string(&agena_runtime_contracts::NoticePart {
                    kind: "external_agent_context_reset".into(),
                    summary,
                    detail: None,
                    title: Some("External agent context".into()),
                })?;
                let command = RequestId::new();
                persist(|| attempt.notice(command, notice.clone()))
                    .await
                    .map_err(kernel_error)?;
            }
            control.phase(agena_domain::ExecutionPhase::StreamingModel);
            turn.record_prompt_delivery(delivery).await?;
            client
                .prompt(&link.native_id, blocks, activity, replayed_users, &turn)
                .await
        }
        .await;
        drop(media_leases);
        turn.close_native_terminals().await;
        // Revoke the tools grant before releasing the attempt.
        if let Some(bridge) = bridge {
            bridge.close().await;
        }
        let completed = match outcome {
            Ok(completed) if !control.cancel.is_cancelled() => completed,
            outcome => {
                let cancelled =
                    control.cancel.is_cancelled() || matches!(outcome, Err(AppError::Cancelled));
                client.close().await;
                return if cancelled {
                    Err(AppError::Cancelled)
                } else {
                    outcome.map(|_| ())
                };
            }
        };
        let finish_reason = match completed.stop_reason.as_deref() {
            Some("end_turn") => agena_domain::FinishReason::Stop,
            Some("refusal") => agena_domain::FinishReason::ContentFilter,
            Some("max_tokens") => agena_domain::FinishReason::MaxTokens,
            Some("max_turn_requests") => {
                let notice = serde_json::to_string(&agena_runtime_contracts::NoticePart {
                    kind: "external_agent_turn_limit".into(),
                    summary: format!("{} reached its request limit for this turn.", spec.name),
                    detail: None,
                    title: None,
                })?;
                let command = RequestId::new();
                persist(|| attempt.notice(command, notice.clone()))
                    .await
                    .map_err(kernel_error)?;
                agena_domain::FinishReason::Other
            }
            _ => agena_domain::FinishReason::Other,
        };
        let link = native.ok_or_else(|| AppError::Internal("missing native session".into()))?;
        let replay = if let Some(cursor) = completed.fork_cursor {
            serde_json::to_value(ExternalAgentForkCursor {
                native_id: link.native_id.clone(),
                incarnation: link.incarnation,
                message_id: execution.message_id,
                native_message_id: cursor.message_id,
                cursor_id: cursor.cursor_id,
                fork: cursor.fork,
                rewind: cursor.rewind,
            })?
        } else {
            Value::Null
        };
        let mut provider = KernelProviderState::new(agent.run_identity(), replay);
        provider.finish_reason = Some(finish_reason);
        provider.usage = completed.usage;
        let provider = serde_json::to_value(provider)?;
        let command = RequestId::new();
        persist(|| round.finish(command, Some(provider.clone()), true))
            .await
            .map_err(kernel_error)?;

        // Finish the exact attempt after draining its deliveries. A racing
        // targeted delivery is written to history for the next user turn.
        let resume = input.resume_state(control.session, round.id(), link)?;
        loop {
            self.consume_boundary_deliveries(
                control.session,
                control.execution,
                execution.message_id,
            )
            .await?;
            let command = RequestId::new();
            persist(|| attempt.prepare_stop(command, StopConditions::default()))
                .await
                .map_err(kernel_error)?;
            let command = RequestId::new();
            match persist(|| attempt.finish_external_agent(command, resume.clone())).await {
                Ok(receipt) if receipt.value.phase == ExecutionPhase::Succeeded => break,
                Ok(_) => return Err(AppError::Cancelled),
                Err(KernelError::PendingDelivery(execution)) if execution == control.execution => {
                    continue;
                }
                Err(error) => return Err(kernel_error(error)),
            }
        }
        let state = turn.transcript.lock().await.snapshot_state();
        self.park_external_agent(client, resume, state).await;
        Ok(())
    }
}

struct ExternalTurn {
    manager: SessionManager,
    control: Arc<RunControl>,
    attempt: AttemptHandle,
    executor: ToolExecutor,
    round: RoundHandle,
    agent: ExternalAgent,
    transcript: tokio::sync::Mutex<transcript::Transcript>,
    terminals: client_tools::Terminals,
}

impl ExternalTurn {
    async fn record_prompt_delivery(
        &self,
        delivery: ExternalAgentPromptDelivery,
    ) -> Result<(), AppError> {
        let command = RequestId::new();
        persist(|| {
            self.round
                .record_external_agent_prompt_delivery(command, delivery)
        })
        .await
        .map_err(kernel_error)
    }

    async fn update_native(
        &self,
        root: &str,
        source: &str,
        update: &Value,
    ) -> Result<(), AppError> {
        self.transcript
            .lock()
            .await
            .update(self, root, source, update)
            .await
    }

    async fn accepts_native_session(&self, root: &str, source: &str) -> bool {
        self.transcript.lock().await.accepts_session(root, source)
    }

    async fn has_native_background(&self) -> bool {
        self.transcript.lock().await.has_native_background()
    }

    async fn native_child_label(&self, source: &str) -> Option<String> {
        self.transcript
            .lock()
            .await
            .child_label(source)
            .map(str::to_owned)
    }

    async fn finish_transcript(&self) -> Result<(), AppError> {
        self.transcript.lock().await.finish_turn(self).await
    }

    async fn begin_tool(&self, name: String, input: Value) -> Result<ToolHandle, AppError> {
        let mut transcript = self.transcript.lock().await;
        transcript.flush_messages(self).await?;
        transcript.finish_text().await?;
        let command = RequestId::new();
        let call = ToolStart {
            call_id: command.to_string(),
            name,
            input,
        };
        Ok(persist(|| self.round.begin_tool(command, call.clone()))
            .await
            .map_err(kernel_error)?
            .value)
    }

    async fn request_permission(
        &self,
        params: &Value,
        withdrawal: &CancellationToken,
    ) -> Result<Value, AppError> {
        let cancelled = || json!({"outcome": {"outcome": "cancelled"}});
        if withdrawal.is_cancelled() {
            return Ok(cancelled());
        }
        let label = self.interaction_agent_label(params).await;
        let Some(PermissionInput {
            title,
            details,
            choices,
            prompt,
        }) = permission_input(params, &label)
        else {
            return Ok(cancelled());
        };
        let tool = self
            .begin_tool(
                "external_agent_permission".into(),
                json!({
                    "agent": self.agent, "operation": title, "details": details,
                }),
            )
            .await?;
        let result = match self
            .manager
            .request_input_for_tool(
                &tool,
                &self.control,
                RequestId::new(),
                prompt,
                Some(withdrawal),
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                let outcome = ToolResult {
                    phase: ToolPhase::Cancelled,
                    value: json!({"title": title, "summary": "Permission cancelled"}),
                };
                let command = RequestId::new();
                let _ = persist(|| tool.complete(command, outcome.clone())).await;
                return Err(error);
            }
        };
        let selection = if result.cancelled
            || result.timed_out
            || withdrawal.is_cancelled()
            || self.control.cancel.is_cancelled()
        {
            None
        } else {
            choices.iter().find(|choice| choice.label == result.reply)
        };
        let phase = match selection {
            None
            | Some(PermissionChoice {
                kind: "reject_once" | "reject_always",
                ..
            }) => ToolPhase::Cancelled,
            Some(_) => ToolPhase::Succeeded,
        };
        let summary = selection.map_or("Permission declined", |choice| choice.label.as_str());
        let outcome = ToolResult {
            phase,
            value: json!({"title": title, "summary": summary}),
        };
        let command = RequestId::new();
        persist(|| tool.complete(command, outcome.clone()))
            .await
            .map_err(kernel_error)?;
        Ok(selection.map_or_else(
            cancelled,
            |choice| json!({"outcome": {"outcome": "selected", "optionId": choice.id}}),
        ))
    }
}

struct PermissionInput<'a> {
    title: String,
    details: String,
    choices: Vec<PermissionChoice<'a>>,
    prompt: crate::part::AskUserToolInput,
}

fn permission_input<'a>(params: &'a Value, label: &str) -> Option<PermissionInput<'a>> {
    let choices = permission_choices(params["options"].as_array()?);
    if choices.is_empty() {
        return None;
    }
    let call = &params["toolCall"];
    let mut title: String = call["title"]
        .as_str()
        .unwrap_or("Tool request")
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    super::results_text::truncate(&mut title, 512);
    let mut details: String = if call["rawInput"].is_null() {
        call.get("locations")
            .map(Value::to_string)
            .unwrap_or_default()
    } else {
        call["rawInput"].to_string()
    }
    .chars()
    .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
    .collect();
    super::results_text::truncate(&mut details, 6 * 1024);
    let prompt = crate::part::AskUserToolInput {
        title: format!("{label} tool permission"),
        questions: vec![UserInputQuestion {
            question: format!("{label} requests permission for {title}.\n{details}"),
            options: choices
                .iter()
                .map(|choice| UserInputOption {
                    label: choice.label.clone(),
                    description: match choice.kind {
                        "allow_always" => "Always allow".into(),
                        "allow_once" => "Allow once".into(),
                        "reject_always" => "Always reject".into(),
                        "reject_once" => "Reject once".into(),
                        _ => String::new(),
                    },
                    custom: false,
                })
                .collect(),
            header: String::new(),
            multiple: false,
            allow_custom: false,
            optional: false,
        }],
        kind: String::new(),
        body_markdown: String::new(),
        auto_resolution_ms: None,
    };
    Some(PermissionInput {
        title,
        details,
        choices,
        prompt,
    })
}

struct PermissionChoice<'a> {
    id: &'a str,
    label: String,
    kind: &'a str,
}

fn permission_choices(options: &[Value]) -> Vec<PermissionChoice<'_>> {
    let mut used_ids = std::collections::HashSet::new();
    let mut labels = std::collections::HashSet::new();
    let mut choices = Vec::new();
    for option in options.iter().take(16) {
        let Some(id) = option["optionId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 1024)
        else {
            continue;
        };
        let Some(name) = option["name"].as_str() else {
            continue;
        };
        if !used_ids.insert(id) {
            continue;
        }
        let mut base: String = name.trim().chars().filter(|ch| !ch.is_control()).collect();
        super::results_text::truncate(&mut base, 256);
        if base.is_empty() {
            continue;
        }
        let mut label = base.clone();
        let mut suffix = 2;
        while !labels.insert(label.clone()) {
            label = format!("{base} ({suffix})");
            suffix += 1;
        }
        choices.push(PermissionChoice {
            id,
            label,
            kind: option["kind"].as_str().unwrap_or_default(),
        });
    }
    choices
}
