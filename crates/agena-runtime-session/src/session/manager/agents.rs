//! Durable agent identity, launch and completion. Plugins own no parallel registry.
use super::results_text::truncate;
use super::*;
use agena_domain::{ActivityPayload, AgentStatus, CommandReferenceActivity, ComposerActivity};
use agena_plugin_host::sdk::host_api::{AgentHandle, AgentSpawnRequest};
use sha2::{Digest, Sha256};

mod messaging;

const MAX_AGENTS_PER_PARENT: usize = 8;

struct AgentLaunch {
    parent_session_id: i64,
    launch_call_id: Option<i64>,
    name: String,
    prompt: String,
    commands: Vec<String>,
    selection: agena_domain::ModelSelectionConfig,
    timeout_ms: Option<u64>,
    max_tokens: Option<u64>,
    max_cost_microusd: Option<u64>,
    existing: Option<SessionId>,
}
struct PreparingAgent {
    child: SessionId,
    name: String,
    started: i64,
    origin: Option<EntryId>,
    activity: Option<ActivityHandle>,
}
struct AgentSettlement {
    child: SessionId,
    name: String,
    started: i64,
    timed_out: bool,
    control: Arc<RunControl>,
    activity: Option<ActivityHandle>,
    execution: ExecutionId,
    origin: Option<EntryId>,
}

impl SessionManager {
    pub async fn spawn_agent(
        &self,
        caller: i64,
        call: Option<i64>,
        request: AgentSpawnRequest,
    ) -> Result<AgentHandle, AppError> {
        let selection = request.selection.unwrap_or_default();
        self.launch_agent(AgentLaunch {
            parent_session_id: caller,
            launch_call_id: call,
            name: request.name,
            prompt: request.prompt,
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
            existing: None,
        })
        .await
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
                if agent.status != AgentStatus::Interrupted {
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
                    let payload = serde_json::json!({"kind":"agent_result","name":agent.name,"agent_id":summary.id.get(),"status":"interrupted","text":format!("Agent '{}' was interrupted before its result was delivered. Use agent.followup with explicit recovery instructions.",agent.name)});
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

    async fn launch_agent(&self, request: AgentLaunch) -> Result<AgentHandle, AppError> {
        let manager = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let preparation = StdMutex::new(None);
            let result = manager.launch_agent_inner(request, &preparation).await;
            if let Err(error) = &result {
                let pending = preparation.lock().unwrap_or_else(|p| p.into_inner()).take();
                if let Some(pending) = pending
                    && let Err(settle) = manager
                        .fail_agent_preparation(pending, error.failure())
                        .await
                {
                    tracing::error!(%settle,"agent launch failure could not be settled");
                }
            }
            let _ = tx.send(result);
        });
        rx.await
            .map_err(|_| AppError::Internal("agent preparation supervisor stopped".into()))?
    }
    async fn launch_agent_inner(
        &self,
        request: AgentLaunch,
        preparing: &StdMutex<Option<PreparingAgent>>,
    ) -> Result<AgentHandle, AppError> {
        if request.name.trim().is_empty() || request.prompt.trim().is_empty() {
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
        let permit = self.reserve_execution()?;
        let settlement = self
            .agent_settlements
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Config("the agent settlement queue is full".into()))?;
        let parent = self.get_session(request.parent_session_id).await?;
        if parent.is_subagent() {
            return Err(AppError::Config(
                "delegated agents cannot create nested agents".into(),
            ));
        }
        let name = request.name.trim().to_owned();
        if name.is_empty()
            || name == "parent"
            || name == "self"
            || name.bytes().all(|b| b.is_ascii_digit())
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(AppError::Config(
                "agent name must contain 1–64 lowercase letters, digits or underscores".into(),
            ));
        }
        let mut denials = parent.execution.capability_denied_tool_names.clone();
        denials.extend(agent_child_capability_denials());
        let state = self.state.load_full();
        let selected = request.selection;
        let selection = parent
            .execution
            .selection
            .overlay_with_cascade(&ExecutionSelection {
                provider: selected.provider,
                adapter: selected.adapter,
                model: selected.model,
                thinking_mode: selected.thinking_mode,
                speed_mode: selected.speed_mode,
                verbosity: selected.verbosity,
                parallel_tool_calls: selected.parallel_tool_calls,
                ..Default::default()
            });
        let mut options = self.options_for_selection(&selection)?;
        let references = resolve_agent_command_references(
            &state.executor.plugin_manager().command_catalog(),
            &request.commands,
        )?;
        let launch = match request.launch_call_id {
            Some(call) => Some(self.live_tool(parent.id, call)?),
            None => None,
        };
        let origin = match &launch {
            Some((_, control)) => Some(
                self.kernel()
                    .reader
                    .execution(control.session, control.execution)
                    .await
                    .map_err(kernel_error)?
                    .message_id,
            ),
            None => self.latest_origin_message(session_id(parent.id)?).await?,
        };
        let preparation = self.agent_preparation.lock().await;
        let existing = request.existing;
        if existing.is_none() {
            if self
                .kernel()
                .reader
                .find_agent(session_id(parent.id)?, &name)
                .await
                .map_err(kernel_error)?
                .is_some()
            {
                return Err(AppError::Config(
                    "agent name already exists; use agent.followup".into(),
                ));
            }
            let members = self
                .kernel()
                .reader
                .query_summaries(SummaryQuery {
                    parent_id: Some(session_id(parent.id)?),
                    agents_only: true,
                    limit: MAX_AGENTS_PER_PARENT + 1,
                    ..Default::default()
                })
                .await
                .map_err(kernel_error)?;
            if members.sessions.len() >= MAX_AGENTS_PER_PARENT {
                return Err(AppError::Config("a parent may have at most 8 agents; reuse an existing agent with agent.followup".into()));
            }
        }
        let previous = if let Some(id) = existing {
            let configuration = self
                .kernel()
                .reader
                .configuration(id)
                .await
                .map_err(kernel_error)?;
            // Another followup may have launched and finished between this
            // request's optimistic check and the serialized launch. Wait for
            // that execution's settlement before replacing its agent state.
            if configuration
                .agent
                .as_ref()
                .is_some_and(|a| a.status == AgentStatus::Running)
                || self.is_run_active(id.get()).await
            {
                return Err(AppError::ExecutionAlreadyActive(id.get()));
            }
            configuration.agent.as_ref().and_then(|a| a.started_at_ms)
        } else {
            None
        };
        let now = Utc::now()
            .timestamp_millis()
            .max(previous.unwrap_or(i64::MIN).saturating_add(1));
        let agent = SessionAgentConfiguration {
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
        let child = match existing {
            Some(id) => {
                let previous = self
                    .kernel()
                    .reader
                    .configuration(id)
                    .await
                    .map_err(kernel_error)?;
                denials.extend(previous.capability_denied_tool_names.iter().cloned());
                let change = SessionConfigurationChange {
                    agent: Some(Some(agent.clone())),
                    message_queue_paused: Some(false),
                    capability_denied_tool_names: Some(denials.clone()),
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
                let configuration = SessionConfiguration {
                    parent_session_id: Some(session_id(parent.id)?),
                    relation_kind: agena_domain::SessionRelationKind::Subagent,
                    permission: parent.execution.effective_permission.clone(),
                    permission_ceiling: parent.execution.effective_permission.clone(),
                    capability_denied_tool_names: denials.clone(),
                    effective_workspace_root: parent.execution.effective_workspace_root.clone(),
                    agent: Some(agent.clone()),
                    ..Default::default()
                };
                persist(|| {
                    self.kernel().sessions.create_configured(
                        command,
                        parent.workspace_id,
                        request.name.trim().into(),
                        configuration.clone(),
                    )
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
        });
        self.update_session_selection(child.get(), options.clone())
            .await?;
        self.prepare_options(child, &mut options).await?;
        let activity = match launch {
            Some((tool, _)) => {
                let activity_request = RequestId::new();
                let mut label = format!("Agent {name}");
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
        let mut document = ComposerDocument(vec![ComposerNode::Text {
            text: request.prompt,
        }]);
        for reference in references {
            document.0.push(ComposerNode::Activity {
                activity: Box::new(ComposerActivity {
                    id: agena_domain::ActivityId::new(),
                    payload: ActivityPayload::CommandReference(reference),
                    provenance: Default::default(),
                }),
            });
        }
        let command = RequestId::new();
        let execution = persist(|| {
            self.kernel()
                .sessions
                .send(command, child, document.clone())
        })
        .await
        .map_err(kernel_error)?
        .value;
        let execution_id = execution.id;
        let control = self.supervise_with_budget(
            execution,
            options.clone(),
            false,
            permit,
            Some(AgentBudget {
                max_tokens: request.max_tokens,
                max_cost_microusd: request.max_cost_microusd,
            }),
        );
        preparing.lock().unwrap_or_else(|p| p.into_inner()).take();
        drop(preparation);
        let manager = self.clone();
        let agent_name = name.clone();
        let timeout = request.timeout_ms;
        tokio::spawn(async move {
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
                    .cancel_execution_with_outcome(child.get(), execution_id)
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
                    control,
                    activity,
                    execution: execution_id,
                    origin,
                })
                .await;
            if let Err(error) = result {
                tracing::error!(%error, agent_id = child.get(), "agent settlement failed");
            }
        });
        Ok(AgentHandle {
            agent_id: child.get(),
            name,
            status: AgentStatus::Running,
        })
    }

    async fn settle_agent(&self, settlement: AgentSettlement) -> Result<(), AppError> {
        let AgentSettlement {
            child,
            name,
            started,
            timed_out,
            control,
            activity,
            execution,
            origin,
        } = settlement;
        let _preparation = self.agent_preparation.lock().await;
        let view = persist(|| self.kernel().reader.execution(child, execution))
            .await
            .map_err(kernel_error)?;
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
        let limits = self
            .kernel()
            .reader
            .configuration(child)
            .await
            .map_err(kernel_error)?
            .agent
            .as_ref()
            .map(|a| a.limits.clone())
            .unwrap_or_default();
        let agent = SessionAgentConfiguration {
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
        let value = bounded_agent_payload(
            serde_json::json!({"kind":"agent_result","message_id":origin.map(EntryId::get),"agent_id":child.get(),"name":name,"run_id":execution.to_string(),"status":status,"text":format!("Agent {} ({}) {}:\n{}",name,child.get(),status.as_ref(),text.unwrap_or_default()),"failure":failure}),
        )?;
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
        let command = request_key("agent-completed", &execution.to_string());
        persist(|| {
            self.kernel().sessions.finish_agent(
                command,
                child,
                agent.clone(),
                format!("agent:{execution}"),
                value.clone(),
            )
        })
        .await
        .map_err(kernel_error)?;
        Ok(())
    }
    async fn fail_agent_preparation(
        &self,
        pending: PreparingAgent,
        failure: agena_failure::Failure,
    ) -> Result<(), AppError> {
        let limits = self
            .kernel()
            .reader
            .configuration(pending.child)
            .await
            .map_err(kernel_error)?
            .agent
            .as_ref()
            .map(|a| a.limits.clone())
            .unwrap_or_default();
        let agent = SessionAgentConfiguration {
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

#[cfg(test)]
#[test]
fn agent_delivery_fits_activity_limit_after_json_escaping() {
    let original = "\u{0000}\"\\界".repeat(8000);
    let value=bounded_agent_payload(serde_json::json!({"kind":"agent_result","agent_id":2,"name":"worker","status":"completed","text":original,"failure":null})).unwrap();
    assert_eq!(value["text_truncated"], true);
    assert!(original.starts_with(value["text"].as_str().unwrap()));
    assert!(
        serde_json::to_vec(&BackgroundActivityOutcome::Succeeded { value })
            .unwrap()
            .len()
            < 64 * 1024
    );
}
