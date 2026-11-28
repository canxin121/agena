//! Durable agent identity, launch and completion. Plugins own no parallel registry.
use super::results_text::truncate;
use super::*;
use agena_domain::{ActivityPayload, AgentStatus, CommandReferenceActivity, ComposerActivity};
use agena_plugin_host::sdk::host_api::{AgentHandle, AgentSpawnRequest};
use sha2::{Digest, Sha256};

mod cordis;
mod cordis_control;
mod messaging;
pub(super) use cordis::CordisExecution;

#[cfg(test)]
mod tests;

const MAX_AGENTS_PER_PARENT: usize = 8;

#[derive(Clone)]
pub(super) struct WorkflowOwner {
    pub id: agena_domain::ActivityId,
    pub origin: EntryId,
    pub model: ModelRef,
}

struct AgentLaunch {
    parent_session_id: i64,
    launch_call_id: Option<i64>,
    workflow: Option<WorkflowOwner>,
    name: String,
    prompt: String,
    profile: Option<String>,
    commands: Vec<String>,
    selection: agena_domain::ModelSelectionConfig,
    timeout_ms: Option<u64>,
    max_tokens: Option<u64>,
    max_cost_microusd: Option<u64>,
    inherit_parent_context: bool,
    existing: Option<SessionId>,
    cordis: Option<cordis::Launch>,
}

impl AgentLaunch {
    fn new(parent_session_id: i64, launch_call_id: Option<i64>, request: AgentSpawnRequest) -> Self {
        let selection = request.selection.unwrap_or_default();
        Self {
            parent_session_id,
            launch_call_id,
            workflow: None,
            name: request.name,
            prompt: request.prompt,
            profile: request.profile,
            commands: request.commands,
            selection: agena_domain::ModelSelectionConfig {
                provider: selection.provider,
                adapter: selection.adapter,
                model: selection.model,
                thinking_mode: selection.thinking_mode,
                speed_mode: selection.speed_mode,
                verbosity: selection.verbosity,
                parallel_tool_calls: selection.parallel_tool_calls,
            },
            timeout_ms: request.timeout_ms,
            max_tokens: request.max_tokens,
            max_cost_microusd: request.max_cost_microusd,
            inherit_parent_context: request.inherit_parent_context,
            existing: None,
            cordis: None,
        }
    }

    fn check_startup(&self) -> Result<(), AppError> {
        if self.cordis.as_ref().is_some_and(|launch| launch.cancellation.is_cancelled()) {
            return Err(AppError::Cancelled);
        }
        Ok(())
    }
}

struct LaunchedAgent {
    handle: AgentHandle,
    cordis: Option<agena_plugin_host::sdk::host_api::CordisSubagentStarted>,
}

struct PreparingAgent {
    child: SessionId,
    name: String,
    started: i64,
    origin: Option<EntryId>,
    activity: Option<ActivityHandle>,
    new_worktree: bool,
    provisional: bool,
}
struct AgentSettlement {
    child: SessionId,
    name: String,
    started: i64,
    timed_out: bool,
    control: Arc<RunControl>,
    activity: Option<ActivityHandle>,
    execution: ExecutionId,
    generation: AttemptGeneration,
    origin: Option<EntryId>,
}

pub(super) struct PendingAgentSettlement {
    started: i64,
    execution: ExecutionId,
    generation: AttemptGeneration,
    done: Arc<Notify>,
}

/// The registration describes only an actually owned settlement task. Dropping
/// that task, including unwinding it, must release any successor waiting on it.
struct AgentSettlementRegistration {
    pending: Arc<StdMutex<HashMap<SessionId, PendingAgentSettlement>>>,
    child: SessionId,
    started: i64,
    execution: ExecutionId,
    done: Arc<Notify>,
}

impl Drop for AgentSettlementRegistration {
    fn drop(&mut self) {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        if pending
            .get(&self.child)
            .is_some_and(|entry| entry.started == self.started && entry.execution == self.execution)
        {
            pending.remove(&self.child);
        }
        self.done.notify_waiters();
    }
}

pub(super) struct IndependentAgentRun {
    name: String,
    started: i64,
    origin: Option<EntryId>,
    generation: AttemptGeneration,
    pub(super) limits: AgentExecutionLimits,
}

impl SessionManager {
    pub async fn spawn_agent(
        &self,
        caller: i64,
        call: Option<i64>,
        request: AgentSpawnRequest,
    ) -> Result<AgentHandle, AppError> {
        self.spawn_agent_with_workflow(caller, call, request, None).await
    }

    pub(super) async fn spawn_agent_with_workflow(
        &self,
        caller: i64,
        call: Option<i64>,
        request: AgentSpawnRequest,
        workflow: Option<WorkflowOwner>,
    ) -> Result<AgentHandle, AppError> {
        let mut launch = AgentLaunch::new(caller, call, request);
        launch.workflow = workflow;
        self.launch_agent(launch).await.map(|launch| launch.handle)
    }
    /// The kernel interrupts provider attempts and external activities on open.
    /// Complete the corresponding agent state and reliable parent handoff.
    pub async fn reconcile_background_agents(&self, limit: usize) -> Result<usize, AppError> {
        let mut before = None;
        let mut reconciled = 0;
        loop {
            let page = persist(|| {
                self.kernel().reader.query_summaries(SummaryQuery {
                    workspace_id: None,
                    agents_only: true,
                    before,
                    limit: limit.clamp(1, 128),
                    ..Default::default()
                })
            })
            .await
            .map_err(kernel_error)?;
            for summary in &page.sessions {
                let _preparation = self.agent_preparation.lock().await;
                let configuration = persist(|| self.kernel().reader.configuration(summary.id))
                    .await
                    .map_err(kernel_error)?;
                let Some(agent) = configuration.agent.clone() else {
                    continue;
                };
                // The kernel fences interrupted work before the runtime is
                // published. A current Running identity may be preparing or
                // settling; guessing from an empty execution list races both.
                if agent.status != AgentStatus::Interrupted || !agent.delivers_to_parent() {
                    continue;
                }
                let active =
                    persist(|| self.kernel().reader.active_executions(summary.id, None, 1))
                        .await
                        .map_err(kernel_error)?;
                if !active.executions.is_empty() {
                    continue;
                }
                if let Some(parent) = configuration.parent_session_id {
                    let payload = serde_json::json!({"kind":"agent_result","message_id":agent.origin_message_id.map(EntryId::get),"name":agent.name,"agent_id":summary.id.get(),"status":"interrupted","text":format!("Agent '{}' was interrupted before its result was delivered. Use agent.followup with explicit recovery instructions.",agent.name)});
                    let key = format!(
                        "agent-recovered:{}:{}",
                        summary.id.get(),
                        agent.started_at_ms.unwrap_or_default()
                    );
                    let request = request_key("agent-recovery-delivery", &key);
                    persist(|| {
                        self.kernel().executor.deliver(
                            request,
                            parent,
                            key.clone(),
                            payload.clone(),
                        )
                    })
                    .await
                    .map_err(kernel_error)?;
                    reconciled += 1;
                }
            }
            if !page.has_more {
                break;
            }
            before = page.next;
        }
        Ok(reconciled)
    }

    async fn launch_agent(&self, request: AgentLaunch) -> Result<LaunchedAgent, AppError> {
        let manager = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let preparation = StdMutex::new(None);
            let mut tree_work = None;
            let result = manager.launch_agent_inner(request, &preparation, &mut tree_work).await;
            if let Err(error) = &result {
                let pending = preparation.lock().unwrap_or_else(|p| p.into_inner()).take();
                if let Some(pending) = pending {
                    if let Err(settle) = manager.abort_agent_preparation(pending, error.failure()).await {
                        tracing::error!(%settle, "agent launch failure could not be settled");
                    }
                }
            }
            let _ = tx.send(result);
        });
        rx.await
            .map_err(|_| AppError::Internal("agent preparation supervisor stopped".into()))?
    }
    async fn launch_agent_inner(
        &self,
        mut request: AgentLaunch,
        preparing: &StdMutex<Option<PreparingAgent>>,
        tree_work: &mut Option<cordis::TreeWork>,
    ) -> Result<LaunchedAgent, AppError> {
        request.check_startup()?;
        let auto_name = request.cordis.is_some() && request.name.is_empty();
        let has_cordis_attachments = request
            .cordis
            .as_ref()
            .is_some_and(|cordis| !cordis.attachments.is_empty());
        if (!auto_name && request.name.trim().is_empty())
            || (request.prompt.trim().is_empty() && !has_cordis_attachments)
        {
            return Err(AppError::Config(
                "agent name and prompt must not be empty".into(),
            ));
        }
        if request.timeout_ms == Some(0)
            || request.max_tokens == Some(0)
            || request.max_cost_microusd == Some(0)
        {
            return Err(AppError::Config(
                "agent budgets and timeout must be greater than zero".into(),
            ));
        }
        if request.prompt.len() > 512 * 1024 {
            return Err(AppError::Config(
                "agent prompt exceeds its byte budget".into(),
            ));
        }
        let parent_session = session_id(request.parent_session_id)?;
        let permit = self.reserve_execution()?;
        let settlement = self
            .agent_settlements
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Config("the agent settlement queue is full".into()))?;
        let launch = match request.launch_call_id {
            Some(call) => Some(self.live_tool(request.parent_session_id, call)?),
            None => None,
        };
        let state = match &launch {
            Some((_, control)) => control.state.clone(),
            None => self.state_for_session(parent_session).await?,
        };
        let parent = self
            .get_session_with_state(request.parent_session_id, Some(state.clone()))
            .await?;
        if parent.is_subagent() && request.cordis.is_none() {
            return Err(AppError::Config(
                "delegated agents cannot create nested agents".into(),
            ));
        }
        let cordis_parent = if let Some(cordis) = &request.cordis {
            let lineage = self.cordis_lineage(parent_session).await?;
            lineage.admit_child(cordis.max_depth)?;
            Some(lineage)
        } else {
            None
        };
        let name = request.name.trim().to_owned();
        let valid = if request.cordis.is_some() {
            auto_name || SessionAgentConfiguration::valid_cordis_name(&request.name)
        } else {
            SessionAgentConfiguration::valid_native_name(&name)
                || (request.existing.is_some() && SessionAgentConfiguration::valid_cordis_name(&name))
        };
        if !valid {
            let reason = if request.cordis.is_some() {
                "Cordis child names must be 1–32 lowercase letters, digits or hyphens, start with a letter and differ from lead"
            } else {
                "agent name must contain 1–64 lowercase letters, digits or underscores"
            };
            return Err(AppError::Config(reason.into()));
        }
        let mut denials = parent.execution.capability_denied_tool_names.clone();
        if request.cordis.is_none() {
            denials.extend(agent_child_capability_denials());
        }
        let selected = std::mem::take(&mut request.selection);
        let explicit_selection = ExecutionSelection {
            provider: selected.provider,
            adapter: selected.adapter,
            model: selected.model,
            thinking_mode: selected.thinking_mode,
            speed_mode: selected.speed_mode,
            verbosity: selected.verbosity,
            parallel_tool_calls: selected.parallel_tool_calls,
            ..Default::default()
        };
        let parent_configuration = self
            .kernel()
            .reader
            .configuration(parent_session)
            .await
            .map_err(kernel_error)?;
        let references = resolve_agent_command_references(
            &state.executor.plugin_manager().command_catalog(),
            &request.commands,
        )?;
        let origin = match &launch {
            Some((_, control)) => Some(
                self.kernel()
                    .reader
                    .execution(control.session, control.execution)
                    .await
                    .map_err(kernel_error)?
                    .message_id,
            ),
            None if request.workflow.is_some() => request.workflow.as_ref().map(|owner| owner.origin),
            None if parent.execution.channel_authority.is_some() => {
                let execution = self.running_execution(parent_session).await?.ok_or_else(|| {
                    AppError::Config(
                        "channel agent launch requires an active originating execution".into(),
                    )
                })?;
                Some(
                    self.kernel()
                        .reader
                        .execution(parent_session, execution)
                        .await
                        .map_err(kernel_error)?
                        .message_id,
                )
            }
            None => self.latest_origin_message(parent_session).await?,
        };
        let preparation = self.agent_preparation.lock().await;
        request.check_startup()?;
        let name = if auto_name {
            self.next_cordis_agent_name(parent_session).await?
        } else {
            name
        };
        let existing = request.existing;
        if existing.is_none() {
            if self
                .kernel()
                .reader
                .find_agent(parent_session, &name)
                .await
                .map_err(kernel_error)?
                .is_some()
            {
                return Err(AppError::Config(
                    "agent name already exists; use agent.followup".into(),
                ));
            }
            if let Some(cordis) = &request.cordis {
                let root = cordis_parent.as_ref().expect("Cordis lineage").root_id();
                if let Some(reserved) = cordis.identity.reserved_id.as_deref() {
                    if reserved == root.get().to_string()
                        || reserved == parent_session.get().to_string()
                    {
                        return Err(AppError::Config(format!(
                            "Cordis childId {reserved:?} is already in use in this tree"
                        )));
                    }
                    let catalog = read_with_busy_retry(root, "cordis_reserved_id", || {
                        self.kernel().reader.agent_catalog(root, 1025)
                    })
                    .await?;
                    if catalog.iter().any(|member| {
                        member.reserved_id.as_deref() == Some(reserved)
                            || member.id.get().to_string() == reserved
                    }) {
                        return Err(AppError::Config(format!(
                            "Cordis childId {reserved:?} is already in use in this tree"
                        )));
                    }
                }
                let limit = cordis.max_active_children;
                if self.cordis_active_agent_count(root).await? >= limit {
                    return Err(AppError::Config(format!(
                        "Cordis subagent concurrency limit reached ({limit} per tree)"
                    )));
                }
                *tree_work = Some(self.register_cordis_tree_work(root));
            } else if request.workflow.is_none() {
                let members = self
                    .kernel()
                    .reader
                    .query_summaries(SummaryQuery {
                        parent_id: Some(parent_session),
                        agents_only: true,
                        exclude_workflow_agents: true,
                        exclude_cordis_agents: true,
                        limit: MAX_AGENTS_PER_PARENT,
                        ..Default::default()
                    })
                    .await
                    .map_err(kernel_error)?;
                if members.sessions.len() >= MAX_AGENTS_PER_PARENT {
                    return Err(AppError::Config(
                        "a parent may have at most 8 agents; reuse an existing agent with agent.followup".into(),
                    ));
                }
            }
        }
        let existing_configuration = if let Some(id) = existing {
            let configuration = self
                .kernel()
                .reader
                .configuration(id)
                .await
                .map_err(kernel_error)?;
            if configuration
                .agent
                .as_ref()
                .and_then(|agent| agent.workflow)
                .is_some_and(|id| self.cordis_workflows.is_running(id))
            {
                return Err(AppError::Config("This agent is still owned by a running Cordis workflow".into()));
            }
            if configuration
                .agent
                .as_ref()
                .is_some_and(|a| a.status == AgentStatus::Running)
                || self.is_run_active(id.get()).await
            {
                return Err(AppError::ExecutionAlreadyActive(id.get()));
            }
            Some((*configuration).clone())
        } else {
            None
        };
        let previous_started = existing_configuration
            .as_ref()
            .and_then(|configuration| configuration.agent.as_ref())
            .and_then(|agent| agent.started_at_ms);
        let now = Utc::now()
            .timestamp_millis()
            .max(previous_started.unwrap_or(i64::MIN).saturating_add(1));
        let profile = match &existing_configuration {
            Some(configuration) => configuration.profile.clone(),
            None => Some(resolve_subagent_profile_name(
                &state.config,
                request.profile.as_deref(),
            )?),
        };
        let effective_profile = state
            .config
            .effective_profile_name(
                profile.as_deref(),
                agena_domain::SessionRelationKind::Subagent,
            )
            .to_owned();
        let resolved_profile = state
            .config
            .resolve_session_profile(
                profile.as_deref(),
                agena_domain::SessionRelationKind::Subagent,
            )
            .map_err(AppError::Config)?;
        let needs_worktree = resolved_profile.worktree
            && existing_configuration.as_ref().is_none_or(|configuration| configuration.worktree.is_none());
        if needs_worktree && (parent.workspace_id.is_none() || parent.execution.channel_authority.is_some()) {
            return Err(AppError::Config("worktree isolation requires a project session".into()));
        }
        let mut parent_selection = parent.execution.selection.clone();
        if let Some(model) = launch.as_ref().map(|(_, control)| &control.model)
            .or_else(|| request.workflow.as_ref().map(|owner| &owner.model))
        {
            parent_selection = parent_selection.overlay_with_cascade(&ExecutionSelection {
                provider: Some(model.provider_id.to_string()),
                adapter: model.adapter_id.as_ref().map(ToString::to_string),
                model: Some(model.model_id.to_string()),
                ..Default::default()
            });
        }
        let selection_defaults = existing_configuration
            .as_ref()
            .map(|configuration| configuration.selection_defaults.clone())
            .unwrap_or(parent_selection);
        let selected = if let Some(id) = existing {
            self.kernel()
                .reader
                .metadata(id)
                .await
                .map_err(kernel_error)?
                .selection
                .clone()
                .unwrap_or_default()
                .overlay_with_cascade(&explicit_selection)
        } else {
            explicit_selection.clone()
        };
        let cordis_identity = request.cordis.as_ref().map(|launch| launch.identity.clone()).or_else(|| {
            let mut identity = existing_configuration.as_ref()?.agent.as_ref()?.cordis.clone()?;
            identity.result_to_caller = false;
            Some(identity)
        });
        let agent = SessionAgentConfiguration {
            origin_message_id: origin,
            workflow: request.workflow.as_ref().map(|owner| owner.id),
            cordis: cordis_identity,
            name: name.clone(),
            status: AgentStatus::Running,
            started_at_ms: Some(now),
            finished_at_ms: None,
            failure: None,
            limits: AgentExecutionLimits {
                timeout_ms: request.timeout_ms,
                max_tokens: request.max_tokens,
                max_cost_microusd: request.max_cost_microusd,
            },
        };
        let mut selection_configuration = existing_configuration.clone().unwrap_or_default();
        selection_configuration.profile = profile.clone();
        selection_configuration.selection_defaults = selection_defaults.clone();
        selection_configuration.relation_kind = agena_domain::SessionRelationKind::Subagent;
        selection_configuration.agent = Some(agent.clone());
        let selection = state
            .config
            .session_selection(&selection_configuration, &selected)
            .map_err(AppError::Config)?;
        let mut options = self.options_for_selection_with_state(&selection, &state)?;
        options.selection_override = Some(selected);
        let child = match existing {
            Some(id) => {
                let mut previous = existing_configuration.expect("existing child configuration");
                if previous.channel_authority != parent.execution.channel_authority {
                    return Err(AppError::Config(
                        "a child from a different channel authority cannot be reused".into(),
                    ));
                }
                previous.permission_ceiling_layers =
                    super::permission_config::delegated_permission_ceiling_layers(
                        &parent_configuration,
                        &parent.execution.effective_permission,
                        &previous.permission_ceiling,
                        previous.permission_ceiling_layers,
                    )?;
                self.validate_session_permission_configuration_with_state(
                    parent.workspace_id,
                    &previous,
                    &state,
                )
                .await?;
                denials.extend(previous.capability_denied_tool_names.iter().cloned());
                let change = SessionConfigurationChange {
                    agent: Some(Some(agent.clone())),
                    message_queue_paused: Some(needs_worktree),
                    capability_denied_tool_names: Some(denials.clone()),
                    permission_ceiling_layers: Some(previous.permission_ceiling_layers),
                    ..Default::default()
                };
                let command = RequestId::new();
                persist(|| {
                    self.kernel()
                        .sessions
                        .configure(command, id, change.clone())
                })
                .await
                .map_err(kernel_error)?;
                id
            }
            None => {
                let command = RequestId::new();
                let permission_ceiling = if parent.execution.permission_ceiling.is_empty() {
                    parent.execution.effective_permission.clone()
                } else {
                    parent.execution.permission_ceiling.clone()
                };
                let mut permission_ceiling_layers =
                    super::permission_config::delegated_permission_ceiling_layers(
                        &parent_configuration,
                        &parent.execution.effective_permission,
                        &permission_ceiling,
                        Vec::new(),
                    )?;
                // The selected role is a delegation boundary as well as a
                // default. Broad parent/global overrides cannot expand it.
                super::permission_config::append_permission_ceiling_layer(
                    &permission_ceiling,
                    &mut permission_ceiling_layers,
                    &resolved_profile.permission,
                )?;
                let configuration = SessionConfiguration {
                    parent_session_id: Some(parent_session),
                    relation_kind: agena_domain::SessionRelationKind::Subagent,
                    profile: profile.clone(),
                    selection_defaults,
                    permission: Default::default(),
                    // Freeze both the current effective policy and every
                    // inherited boundary as independent intersections.
                    permission_ceiling,
                    permission_ceiling_layers,
                    channel_authority: parent.execution.channel_authority.clone(),
                    capability_denied_tool_names: denials.clone(),
                    effective_workspace_root: parent.execution.effective_workspace_root.clone(),
                    working_directory: request.cordis.as_ref().map_or_else(
                        || parent_configuration.working_directory.clone(),
                        |launch| launch.cwd.clone(),
                    ),
                    message_queue_paused: needs_worktree,
                    agent: Some(agent.clone()),
                    ..Default::default()
                };
                self.validate_session_permission_configuration_with_state(
                    parent.workspace_id,
                    &configuration,
                    &state,
                )
                .await?;
                request.check_startup()?;
                let title = request.cordis.as_ref().map(|launch| launch.label.as_str()).unwrap_or(&name);
                persist(|| async {
                    if request.inherit_parent_context {
                        return self
                            .kernel()
                            .sessions
                            .create_seeded_agent(
                                command,
                                parent_session,
                                title.into(),
                                configuration.clone(),
                            )
                            .await;
                    }
                    match parent.workspace_id {
                        Some(workspace) => {
                            self.kernel()
                                .sessions
                                .create_configured(
                                    command,
                                    workspace,
                                title.into(),
                                    configuration.clone(),
                                )
                                .await
                        }
                        None => {
                            self.kernel()
                                .sessions
                                .create_global(
                                    command,
                                title.into(),
                                    configuration.clone(),
                                )
                                .await
                        }
                    }
                })
                .await
                .map_err(kernel_error)?
                .value
                .id
            }
        };
        *preparing.lock().unwrap_or_else(|p| p.into_inner()) = Some(PreparingAgent {
            child,
            name: name.clone(),
            started: now,
            origin,
            activity: None,
            new_worktree: false,
            provisional: request.cordis.is_some(),
        });
        if needs_worktree {
            self.provision_worktree(child, &state).await?;
            if let Some(pending) = preparing.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
                pending.new_worktree = true;
            }
            let configuration = self.kernel().reader.configuration(child).await.map_err(kernel_error)?;
            self.validate_session_permission_configuration_with_state(parent.workspace_id, &configuration, &state).await?;
            if let Some(relative) = &configuration.working_directory
                && let Some(root) = &configuration.effective_workspace_root
            {
                let cwd = super::working_directories::canonical_directory(&root.join(relative)).await?;
                if !cwd.starts_with(root) {
                    return Err(AppError::Config("Agent working directory resolves outside its worktree".into()));
                }
            }
        }
        let plugins = state.executor.plugin_manager();
        let start_context = plugins
            .dispatch_subagent_start(agena_plugin_host::SubagentStartInput {
                session_id: request.parent_session_id,
                child_session_id: child.get(),
                name: name.clone(),
                agent_type: Some(plugins.imported_subagent_type(&effective_profile)),
                model: Some(options.model.model_id.to_string()),
            });
        let start_context = if let Some(launch) = &request.cordis {
            tokio::select! {
                biased;
                _ = launch.cancellation.cancelled() => return Err(AppError::Cancelled),
                result = start_context => result,
            }
        } else {
            start_context.await
        }.map_err(|error| AppError::Config(error.diagnostic_message().to_owned()))?;
        self.persist_submission_selection(child.get(), options.clone())
            .await?;
        self.prepare_options_with_state(child, &mut options, &state)
            .await?;
        if !start_context.is_empty() {
            let mut text = start_context.join("\n");
            truncate(&mut text, 16 * 1024);
            let command = request_key("subagent-start-context", &format!("{}:{text}", child.get()));
            let key = format!("subagent-start:{command}");
            let payload = serde_json::json!({"text":text,"wake":false});
            persist(|| {
                self.kernel()
                    .executor
                    .deliver(command, child, key.clone(), payload.clone())
            })
            .await
            .map_err(kernel_error)?;
        }
        let activity = match launch {
            Some((tool, _)) => {
                let activity_request = RequestId::new();
                let mut label = request.cordis.as_ref().map(|launch| launch.label.clone())
                    .unwrap_or_else(|| format!("Agent {name}"));
                truncate(&mut label, 1024);
                let start = BackgroundActivityStart {
                    kind: BackgroundActivityKind::Agent,
                    label,
                };
                Some(
                    persist(|| tool.begin_activity(activity_request, start.clone()))
                        .await
                        .map_err(kernel_error)?
                        .value,
                )
            }
            None => None,
        };
        if let Some(pending) = preparing.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            pending.activity = activity.clone();
        }
        // No child provider is polled before its durable configuration and
        // launch intent have committed. An uncertain send retains its identity.
        let mut document = ComposerDocument(Vec::new());
        let prompt_text = std::mem::take(&mut request.prompt);
        if !prompt_text.is_empty() {
            document.0.push(ComposerNode::Text { text: prompt_text });
        }
        let mut blobs = Vec::new();
        if let Some(cordis) = request.cordis.as_mut()
            && !cordis.attachments.is_empty()
        {
            use agena_domain::{
                ActivityProvenance, AttachmentKind, AttachmentSource, ResourceActivity,
                ResourceDelivery, ResourceKind, ResourceReference,
            };
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            use sha2::{Digest, Sha256};

            let route = state.providers.media_route_binding(&options.model).ok();
            let attachments = std::mem::take(&mut cordis.attachments);
            for (index, item) in attachments.into_iter().enumerate() {
                let kind = match item.kind {
                    AttachmentKind::Image => ResourceKind::Image,
                    AttachmentKind::Audio => ResourceKind::Audio,
                    AttachmentKind::Video => ResourceKind::Video,
                    AttachmentKind::Pdf => ResourceKind::Pdf,
                    AttachmentKind::File => ResourceKind::File,
                };
                let name = item
                    .filename
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| format!("attachment-{index}"));
                let mut bound_for_model = false;
                if let (Some(route), AttachmentSource::Base64 { data }) =
                    (route.as_ref(), &item.source)
                    && let Ok(bytes) = STANDARD.decode(data)
                {
                    let mut candidate = item.clone();
                    candidate.source = AttachmentSource::ProviderData {
                        route: route.clone(),
                        data: data.clone(),
                    };
                    if state
                        .providers
                        .validate_media_inputs(&options.model, std::slice::from_ref(&candidate))
                        .is_ok()
                    {
                        let sha256 = hex::encode(Sha256::digest(&bytes));
                        let blob_cmd = request_key(
                            &format!("cordis-subagent-media:{}:{now}", child.get()),
                            &index.to_string(),
                        );
                        let blob = self
                            .prepare_immutable_blob(blob_cmd, child, &bytes)
                            .await?;
                        let mut metadata = item.clone();
                        metadata.source = AttachmentSource::Base64 {
                            data: String::new(),
                        };
                        metadata.size_bytes = Some(bytes.len() as u64);
                        metadata.sha256 = Some(sha256.clone());
                        let binding = crate::session::kernel::media::MediaBinding {
                            version: 1,
                            model: options.model.clone(),
                            route: route.clone(),
                            blob: blob.descriptor().id,
                            attachment: metadata,
                        };
                        let uri = crate::session::kernel::media::uri(&binding)
                            .map_err(kernel_error)?;
                        blobs.push(blob);
                        document.0.push(ComposerNode::Activity {
                            activity: Box::new(ComposerActivity {
                                id: agena_domain::ActivityId::new(),
                                payload: ActivityPayload::Resource(ResourceActivity {
                                    delivery: ResourceDelivery::ModelInput,
                                    kind,
                                    reference: ResourceReference::Artifact { sha256, uri },
                                    name: name.clone(),
                                    media_type: Some(item.mime.clone()),
                                    size_bytes: Some(bytes.len() as u64),
                                    width: item.width,
                                    height: item.height,
                                    duration_ms: item.duration_ms,
                                    page_count: item.page_count,
                                }),
                                provenance: ActivityProvenance {
                                    source: Some("cordis.subagent".into()),
                                    content_hash: item.sha256.clone(),
                                    plugin_id: None,
                                },
                            }),
                        });
                        bound_for_model = true;
                    }
                }
                if !bound_for_model {
                    document.0.push(ComposerNode::Activity {
                        activity: Box::new(ComposerActivity {
                            id: agena_domain::ActivityId::new(),
                            payload: ActivityPayload::Resource(ResourceActivity {
                                delivery: ResourceDelivery::Reference,
                                kind,
                                reference: ResourceReference::WorkspacePath { path: name.clone() },
                                name,
                                media_type: Some(item.mime),
                                size_bytes: item.size_bytes,
                                width: item.width,
                                height: item.height,
                                duration_ms: item.duration_ms,
                                page_count: item.page_count,
                            }),
                            provenance: ActivityProvenance {
                                source: Some("cordis.subagent".into()),
                                content_hash: item.sha256,
                                plugin_id: None,
                            },
                        }),
                    });
                }
            }
        }
        for reference in references {
            document.0.push(ComposerNode::Activity {
                activity: Box::new(ComposerActivity {
                    id: agena_domain::ActivityId::new(),
                    payload: ActivityPayload::CommandReference(reference),
                    provenance: Default::default(),
                }),
            });
        }
        if needs_worktree {
            self.persist_message_queue_pause(child, false).await?;
        }
        request.check_startup()?;
        let command = RequestId::new();
        let execution = persist(|| async {
            if blobs.is_empty() {
                self.kernel()
                    .sessions
                    .send(command, child, document.clone())
                    .await
            } else {
                self.kernel()
                    .sessions
                    .send_with_blobs(command, child, document.clone(), &blobs)
                    .await
            }
        })
        .await
        .map_err(kernel_error)?
        .value;
        if let Some(pending) = preparing.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            pending.provisional = false;
        }
        let execution_id = execution.id;
        let generation = execution.generation;
        let message_id = execution.message_id;
        let (cordis, result_sender) = if let Some(parent) = cordis_parent {
            let (run, sender) = cordis::Run::new(self.clone(), child, execution_id, generation);
            (Some(agena_plugin_host::sdk::host_api::CordisSubagentStarted {
                child_id: child.get(),
                reserved_id: request
                    .cordis
                    .as_ref()
                    .and_then(|launch| launch.identity.reserved_id.clone()),
                message_id: message_id.get(),
                name: parent.child_name(&name, child),
                ancestor_sessions: parent.ancestor_ids(),
                run: Arc::new(run),
            }), Some(sender))
        } else {
            (None, None)
        };
        let registration = self.register_agent_settlement(child, now, execution_id, generation);
        let control = self.supervise_with_budget_and_state(
            execution,
            options,
            false,
            permit,
            Some(AgentBudget {
                max_tokens: request.max_tokens,
                max_cost_microusd: request.max_cost_microusd,
                cordis: request
                    .cordis
                    .as_ref()
                    .map(|launch| CordisExecution::new(launch.max_output_tokens)),
            }),
            state,
        );
        preparing.lock().unwrap_or_else(|p| p.into_inner()).take();
        drop(preparation);
        let manager = self.clone();
        let agent_name = name.clone();
        let timeout = request.timeout_ms;
        let tree_work = tree_work.take();
        tokio::spawn(async move {
            let _tree_work = tree_work;
            let _registration = registration;
            let _settlement = settlement;
            let wait = async {
                loop {
                    let notified = control.done.notified();
                    if control.settled.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    notified.await;
                }
            };
            let timed_out = match timeout {
                Some(ms) => tokio::time::timeout(Duration::from_millis(ms), wait)
                    .await
                    .is_err(),
                None => {
                    wait.await;
                    false
                }
            };
            if timed_out {
                let _ = manager
                    .cancel_execution_attempt_with_outcome(child.get(), execution_id, generation)
                    .await;
                loop {
                    let notified = control.done.notified();
                    if control.settled.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    notified.await;
                }
            }
            let result = manager
                .settle_agent(AgentSettlement {
                    child,
                    name: agent_name,
                    started: now,
                    timed_out,
                    control: control.clone(),
                    activity,
                    execution: execution_id,
                    generation,
                    origin,
                })
                .await;
            if let Err(error) = &result {
                tracing::error!(%error, agent_id = child.get(), "agent settlement failed");
            }
            if let Some(sender) = result_sender {
                let result = match result {
                    Ok(status) => manager.capture_cordis_subagent(&control, status).await,
                    Err(error) => Err(cordis::result_error(&error)),
                };
                let _ = sender.send(Some(result));
            }
        });
        Ok(LaunchedAgent {
            handle: AgentHandle {
                agent_id: child.get(),
                name,
                profile: Some(effective_profile),
                status: AgentStatus::Running,
            },
            cordis,
        })
    }

    pub(super) fn has_pending_agent_settlement(
        &self,
        child: SessionId,
        execution: ExecutionId,
        generation: AttemptGeneration,
    ) -> bool {
        self.pending_agent_settlements
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&child)
            .is_some_and(|pending| {
                pending.execution == execution && pending.generation == generation
            })
    }

    fn register_agent_settlement(
        &self,
        child: SessionId,
        started: i64,
        execution: ExecutionId,
        generation: AttemptGeneration,
    ) -> AgentSettlementRegistration {
        let done = Arc::new(Notify::new());
        self.pending_agent_settlements
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                child,
                PendingAgentSettlement {
                    started,
                    execution,
                    generation,
                    done: done.clone(),
                },
            );
        AgentSettlementRegistration {
            pending: self.pending_agent_settlements.clone(),
            child,
            started,
            execution,
            done,
        }
    }

    /// Direct user turns and continuations retain a subagent's identity and
    /// limits. Called after this execution is claimed, so no older attempt can
    /// still run. Only an exact, live settlement owner can delay the new turn.
    pub(super) async fn begin_independent_agent_run(
        &self,
        control: &RunControl,
        generation: AttemptGeneration,
    ) -> Result<Option<IndependentAgentRun>, AppError> {
        loop {
            let preparation = self.agent_preparation.lock().await;
            let configuration = persist(|| self.kernel().reader.configuration(control.session))
                .await
                .map_err(kernel_error)?;
            if !configuration.relation_kind.is_subagent() {
                return Ok(None);
            }
            let Some(mut agent) = configuration.agent.clone() else {
                return Err(AppError::Config(
                    "subagent session has no agent identity".into(),
                ));
            };
            let pending = {
                let settlements = self
                    .pending_agent_settlements
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                settlements
                    .get(&control.session)
                    .filter(|pending| {
                        pending.execution != control.execution
                            && Some(pending.started) == agent.started_at_ms
                    })
                    .map(|pending| {
                        let mut notified = Box::pin(pending.done.clone().notified_owned());
                        // Registration and removal use this same mutex, so a
                        // finishing or aborted owner cannot lose this wakeup.
                        notified.as_mut().enable();
                        notified
                    })
            };
            if let Some(notified) = pending {
                drop(preparation);
                tokio::select! {
                    _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
                    _ = notified => {}
                }
                continue;
            }
            let started = Utc::now()
                .timestamp_millis()
                .max(agent.started_at_ms.unwrap_or(i64::MIN).saturating_add(1));
            // Resolve fallible routing reads before publishing Running. Once
            // committed, return its settlement identity without another await.
            let origin = agent.origin_message_id;
            agent.status = AgentStatus::Running;
            agent.started_at_ms = Some(started);
            agent.finished_at_ms = None;
            agent.failure = None;
            if let Some(cordis) = &mut agent.cordis {
                cordis.result_to_caller = false;
            }
            let command = RequestId::new();
            persist(|| {
                self.kernel().sessions.configure(
                    command,
                    control.session,
                    SessionConfigurationChange {
                        agent: Some(Some(agent.clone())),
                        ..Default::default()
                    },
                )
            })
            .await
            .map_err(kernel_error)?;
            return Ok(Some(IndependentAgentRun {
                name: agent.name,
                started,
                origin,
                generation,
                limits: agent.limits,
            }));
        }
    }

    pub(super) async fn finish_independent_agent_run(
        &self,
        control: &Arc<RunControl>,
        agent: Option<IndependentAgentRun>,
        timed_out: bool,
        result: &Result<(), AppError>,
    ) -> Result<(), AppError> {
        let Some(agent) = agent else {
            return Ok(());
        };
        if let Err(error) = result {
            *control.failure.lock().unwrap_or_else(|p| p.into_inner()) = Some(error.failure());
        }
        self.settle_agent(AgentSettlement {
            child: control.session,
            name: agent.name,
            started: agent.started,
            timed_out,
            control: control.clone(),
            activity: None,
            execution: control.execution,
            generation: agent.generation,
            origin: agent.origin,
        })
        .await
        .map(|_| ())
    }

    /// Retrying reuses an execution ID with a new generation. Keep a terminal
    /// attempt's small settlement window intact before admitting that retry.
    /// Running/queued work still goes through the ordinary conflict check.
    pub(super) async fn wait_terminal_agent_settlement(
        &self,
        session: SessionId,
    ) -> Result<(), AppError> {
        let configuration = self
            .kernel()
            .reader
            .configuration(session)
            .await
            .map_err(kernel_error)?;
        if !configuration.relation_kind.is_subagent() {
            return Ok(());
        }
        'settlement: loop {
            let controls = self
                .active
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .values()
                .filter(|run| run.session == session)
                .cloned()
                .collect::<Vec<_>>();
            for control in controls {
                let execution = self
                    .kernel()
                    .reader
                    .execution(session, control.execution)
                    .await
                    .map_err(kernel_error)?;
                if execution.phase.is_terminal() {
                    tokio::select! {
                        _ = self.shutdown.cancelled() => return Err(AppError::Cancelled),
                        _ = control.wait_settled() => {}
                    }
                    continue 'settlement;
                }
            }
            let pending = self
                .pending_agent_settlements
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&session)
                .map(|pending| {
                    let mut notified = Box::pin(pending.done.clone().notified_owned());
                    notified.as_mut().enable();
                    (pending.execution, notified)
                });
            let Some((execution, notified)) = pending else {
                return Ok(());
            };
            let execution = self
                .kernel()
                .reader
                .execution(session, execution)
                .await
                .map_err(kernel_error)?;
            if !execution.phase.is_terminal() {
                return Ok(());
            }
            tokio::select! {
                _ = self.shutdown.cancelled() => return Err(AppError::Cancelled),
                _ = notified => {}
            }
        }
    }

    async fn settle_agent(&self, settlement: AgentSettlement) -> Result<AgentStatus, AppError> {
        let AgentSettlement {
            child,
            name,
            started,
            timed_out,
            control,
            activity,
            execution,
            generation,
            origin,
        } = settlement;
        // A terminal child reports only after its attempt's detached jobs
        // finish. Cleanup can itself use the agent preparation lock.
        control.cordis_child_jobs.finish().await;
        let _preparation = self.agent_preparation.lock().await;
        let view = persist(|| self.kernel().reader.execution(child, execution))
            .await
            .map_err(kernel_error)?;
        if view.generation != generation || !view.phase.is_terminal() {
            return Err(AppError::Internal(
                "agent settlement no longer owns the terminal attempt".into(),
            ));
        }
        let status = if timed_out {
            AgentStatus::TimedOut
        } else {
            match view.phase {
                ExecutionPhase::Succeeded => AgentStatus::Completed,
                ExecutionPhase::Cancelled => AgentStatus::Cancelled,
                ExecutionPhase::Interrupted => AgentStatus::Interrupted,
                _ => AgentStatus::Failed,
            }
        };
        let mut failure = control
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if timed_out {
            let mut timeout = AppError::Cancelled.failure();
            timeout.category = agena_failure::FailureCategory::Timeout;
            timeout.user.fallback = "The agent reached its execution time limit.".into();
            failure = Some(timeout);
        }
        let finished = Utc::now().timestamp_millis().max(started);
        let configuration = self
            .kernel()
            .reader
            .configuration(child)
            .await
            .map_err(kernel_error)?;
        let parent_session_id = configuration.parent_session_id.map(SessionId::get);
        let limits = configuration
            .agent
            .as_ref()
            .map(|a| a.limits.clone())
            .unwrap_or_default();
        let agent = SessionAgentConfiguration {
            origin_message_id: origin,
            workflow: configuration.agent.as_ref().and_then(|agent| agent.workflow),
            cordis: configuration.agent.as_ref().and_then(|agent| agent.cordis.clone()),
            name: name.clone(),
            status,
            started_at_ms: Some(started),
            finished_at_ms: Some(finished),
            failure: failure.clone(),
            limits,
        };
        let text = self
            .final_text_for(child, Some(execution), 12 * 1024)
            .await?;
        let mut value = bounded_agent_payload(
            serde_json::json!({"kind":"agent_result","message_id":origin.map(EntryId::get),"agent_id":child.get(),"name":name,"run_id":execution.to_string(),"attempt_generation":view.generation.get(),"status":status,"text":format!("Agent {} ({}) {}:\n{}",name,child.get(),status.as_ref(),text.unwrap_or_default()),"failure":failure}),
        )?;
        if agent.cordis.is_some() {
            value["wake"] = true.into();
        }
        if let Some(activity) = activity {
            let command = RequestId::new();
            let outcome = match status {
                AgentStatus::Completed => BackgroundActivityOutcome::Succeeded {
                    value: value.clone(),
                },
                AgentStatus::Cancelled => BackgroundActivityOutcome::Stopped,
                AgentStatus::Interrupted => BackgroundActivityOutcome::Interrupted {
                    diagnostic: "child execution was interrupted".into(),
                },
                _ => BackgroundActivityOutcome::Failed {
                    diagnostic: bounded_diagnostic(
                        failure
                            .as_ref()
                            .map(|f| f.user.fallback.clone())
                            .unwrap_or_else(|| "child agent did not complete".into()),
                    ),
                },
            };
            persist(|| activity.end(command, outcome.clone()))
                .await
                .map_err(kernel_error)?;
        }
        let cordis_root = if agent.cordis.is_some() {
            self.cordis_lineage(child)
                .await
                .ok()
                .map(|lineage| lineage.root_id())
        } else {
            None
        };
        let _tree_work = cordis_root.map(|root| self.register_cordis_tree_work(root));
        let run_key = format!("{execution}:{}", view.generation.get());
        let command = request_key("agent-completed", &run_key);
        persist(|| {
            self.kernel().sessions.finish_agent(
                command,
                child,
                agent.clone(),
                format!("agent:{run_key}"),
                value.clone(),
            )
        })
        .await
        .map_err(kernel_error)?;
        if let Some(parent) = parent_session_id {
            self.publish_agent_settled(session_id(parent)?, child, name, status)
                .await?;
        }
        drop(_preparation);
        if let Some(parent) = parent_session_id
            && agent.cordis.is_some() && agent.delivers_to_parent()
            && let Err(error) = self.wake_agent_inbox(session_id(parent)?).await
        {
            tracing::debug!(%error, "Cordis agent completion wake deferred");
        }
        if let Some(root) = cordis_root {
            self.wake_deferred_cordis_agents(root).await;
        }
        Ok(status)
    }

    async fn abort_agent_preparation(
        &self,
        pending: PreparingAgent,
        failure: agena_failure::Failure,
    ) -> Result<(), AppError> {
        let dispatch = self.session_dispatch(pending.child);
        let _preparation;
        let _submission;
        let discard = if pending.provisional {
            _preparation = self.agent_preparation.lock().await;
            _submission = dispatch.submission.lock().await;
            // A project operator may have submitted directly to the temporary
            // session during startup. Preserve that work.
            !self.is_run_active(pending.child.get()).await
                && self.latest_origin_message(pending.child).await?.is_none()
        } else {
            false
        };
        if pending.new_worktree && (!pending.provisional || discard)
            && let Err(error) = self.release_worktree(pending.child).await
        {
            tracing::warn!(%error, "failed agent launch retained its worktree");
            return self.fail_agent_preparation(pending, failure).await;
        }
        if !discard {
            return self.fail_agent_preparation(pending, failure).await;
        }
        if let Some(activity) = pending.activity {
            let command = RequestId::new();
            persist(|| activity.end(command, BackgroundActivityOutcome::Stopped))
                .await
                .map_err(kernel_error)?;
        }
        let command = RequestId::new();
        persist(|| self.kernel().sessions.delete(command, pending.child))
            .await
            .map_err(kernel_error)?;
        Ok(())
    }
    async fn fail_agent_preparation(
        &self,
        pending: PreparingAgent,
        failure: agena_failure::Failure,
    ) -> Result<(), AppError> {
        let configuration = self
            .kernel()
            .reader
            .configuration(pending.child)
            .await
            .map_err(kernel_error)?;
        let limits = configuration
            .agent
            .as_ref()
            .map(|agent| agent.limits.clone())
            .unwrap_or_default();
        let agent = SessionAgentConfiguration {
            origin_message_id: pending.origin,
            workflow: configuration.agent.as_ref().and_then(|agent| agent.workflow),
            cordis: configuration.agent.as_ref().and_then(|agent| agent.cordis.clone()),
            name: pending.name.clone(),
            status: AgentStatus::Failed,
            started_at_ms: Some(pending.started),
            finished_at_ms: Some(Utc::now().timestamp_millis().max(pending.started)),
            failure: Some(failure.clone()),
            limits,
        };
        if let Some(activity) = pending.activity {
            let command = RequestId::new();
            persist(|| {
                activity.end(
                    command,
                    BackgroundActivityOutcome::Failed {
                        diagnostic: bounded_diagnostic(failure.user.fallback.clone()),
                    },
                )
            })
            .await
            .map_err(kernel_error)?;
        }
        {
            let payload = bounded_agent_payload(
                serde_json::json!({"kind":"agent_result","message_id":pending.origin.map(EntryId::get),"name":pending.name,"agent_id":pending.child.get(),"status":AgentStatus::Failed,"text":format!("Agent launch failed: {}",failure.user.fallback),"failure":failure}),
            )?;
            let command = request_key(
                "agent-launch-delivery",
                &format!("{}:{}", pending.child.get(), pending.started),
            );
            persist(|| {
                self.kernel().sessions.finish_agent(
                    command,
                    pending.child,
                    agent.clone(),
                    format!("agent-launch:{}:{}", pending.child.get(), pending.started),
                    payload.clone(),
                )
            })
            .await
            .map_err(kernel_error)?;
        }
        if let Some(parent) = self
            .kernel()
            .reader
            .configuration(pending.child)
            .await
            .map_err(kernel_error)?
            .parent_session_id
        {
            self.publish_agent_settled(parent, pending.child, pending.name, AgentStatus::Failed)
                .await?;
        }
        Ok(())
    }

    async fn publish_agent_settled(
        &self,
        parent: SessionId,
        child: SessionId,
        name: String,
        status: AgentStatus,
    ) -> Result<(), AppError> {
        let state = self.state_for_session(parent).await?;
        let host = state.executor.plugin_manager();
        host.broadcast_subagent_stop(agena_plugin_host::SubagentStopInput {
            session_id: parent.get(),
            child_session_id: child.get(),
            name: name.clone(),
            status: status.as_ref().to_owned(),
        })
        .await;
        if host.has_imported_notification_hooks()
            && let Ok(context) = self.execution_context_with_state(parent, &state).await
        {
            let executor = state.executor.for_session_resource_context(&context);
            tokio::spawn(async move {
                executor
                    .plugin_manager()
                    .broadcast_imported_notification(
                        agena_plugin_host::NotificationInput {
                            kind: "agent_completed".into(),
                            session_id: Some(parent.get()),
                            title: "Agent finished".into(),
                            message: format!("Agent {name} {}.", status.as_ref()),
                            payload: serde_json::json!({"agent_id":child.get(), "status":status}),
                        },
                        |plugin| executor.plugin_is_enabled(plugin),
                    )
                    .await;
            });
        }
        Ok(())
    }
}

fn bounded_diagnostic(mut text: String) -> String {
    truncate(&mut text, 4096);
    text
}

/// Both the activity outcome and the reliable delivery must fit 64 KiB after
/// JSON escaping. Full output and failure remain available on the child.
fn bounded_agent_payload(mut value: serde_json::Value) -> Result<serde_json::Value, AppError> {
    while serde_json::to_vec(&value)?.len() > 60 * 1024 {
        if let Some(text) = value
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .filter(|text| !text.is_empty())
        {
            let mut preview = text;
            let limit = preview.len() / 2;
            truncate(&mut preview, limit);
            value["text"] = preview.into();
            value["text_truncated"] = true.into();
        } else if !value["failure"].is_null() {
            let summary = value["failure"]["user"]["fallback"]
                .as_str()
                .map(str::to_owned)
                .map(bounded_diagnostic);
            value["failure"] = serde_json::Value::Null;
            value["failure_summary"] = summary.into();
            value["failure_truncated"] = true.into();
        } else {
            return Err(AppError::Config(
                "agent delivery envelope exceeds its byte budget".into(),
            ));
        }
    }
    Ok(value)
}

fn resolve_agent_command_references(
    catalog: &[agena_plugin_host::CommandCatalogItem],
    requested: &[String],
) -> Result<Vec<CommandReferenceActivity>, AppError> {
    if requested.len() > 32 {
        return Err(AppError::Config(
            "agent command references exceed their limit".into(),
        ));
    }
    requested
        .iter()
        .map(|name| {
            let query = name.trim().trim_start_matches('/');
            let item = catalog
                .iter()
                .find(|item| {
                    item.command.id.eq_ignore_ascii_case(query)
                        || item.command.slash.as_deref().is_some_and(|slash| {
                            slash.trim_start_matches('/').eq_ignore_ascii_case(query)
                        })
                        || item
                            .command
                            .aliases
                            .iter()
                            .any(|alias| alias.eq_ignore_ascii_case(query))
                })
                .ok_or_else(|| AppError::Config(format!("unknown command '{name}' for agent")))?;
            let hash = Sha256::digest(serde_json::to_vec(&item.command)?);
            Ok(CommandReferenceActivity {
                name: item.command.id.clone(),
                description: item.command.docs.summary.clone().unwrap_or_default(),
                content_hash: hex::encode(hash),
                source: item.plugin_id.to_string(),
                skill_id: None,
                aliases: item.command.aliases.clone(),
            })
        })
        .collect()
}

fn agent_child_capability_denials() -> std::collections::BTreeSet<String> {
    ["spawn", "followup", "cancel"]
        .into_iter()
        .flat_map(|name| {
            [
                format!("agent.{name}"),
                format!("agena.agent.{name}"),
                format!("agena_agent_{name}"),
            ]
        })
        .collect()
}

fn resolve_subagent_profile_name(
    config: &RuntimeSessionManagerConfig,
    requested: Option<&str>,
) -> Result<String, AppError> {
    let name = match requested.map(str::trim) {
        Some("") => agena_runtime_config::DEFAULT_SUBAGENT_PROFILE,
        Some(name) => name,
        None => config
            .default_subagent_profile
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(agena_runtime_config::DEFAULT_SUBAGENT_PROFILE),
    };
    if let Err(reason) =
        config.validate_session_profile(Some(name), agena_domain::SessionRelationKind::Subagent)
    {
        let available = config
            .profiles
            .keys()
            .filter_map(|name| {
                let profile =
                    agena_runtime_config::resolve_agent_profile(&config.profiles, name).ok()?;
                (profile.mode.allows_subagent() && !profile.hidden).then_some(name.as_str())
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::Config(format!(
            "{reason}. Available subagent profiles: {available}"
        )));
    }
    Ok(name.to_owned())
}
