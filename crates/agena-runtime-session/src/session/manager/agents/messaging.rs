//! Scoped durable mailboxes. Wait observes facts; provider boundaries deliver content.
use super::*;
use agena_plugin_host::sdk::host_api::{
    AgentMessageReceipt, AgentMessageRequest, AgentTargetRequest, AgentWaitRequest,
    AgentWaitResponse, AgentWakeReason, ImportedPluginAgentState,
};

impl SessionManager {
    async fn agent_group(&self, caller: SessionId) -> Result<SessionId, AppError> {
        let configuration = read_with_busy_retry(caller, "configuration", || {
            self.kernel().reader.configuration(caller)
        })
        .await?;
        if configuration.relation_kind == agena_domain::SessionRelationKind::Subagent {
            if configuration.agent.is_none() {
                return Err(AppError::Config("session has no agent identity".into()));
            }
            configuration
                .parent_session_id
                .ok_or_else(|| AppError::Config("agent has no parent".into()))
        } else {
            Ok(caller)
        }
    }

    async fn resolve_agent_target(
        &self,
        caller: SessionId,
        target: &str,
        direct_child: bool,
    ) -> Result<SessionId, AppError> {
        let owner = self.agent_group(caller).await?;
        if direct_child && owner != caller {
            return Err(AppError::Config(
                "only the direct parent can control an agent".into(),
            ));
        }
        let target = target.trim();
        let id = if target == "parent" && caller != owner {
            owner
        } else if let Ok(numeric) = target.parse::<i64>() {
            session_id(numeric)?
        } else {
            self.kernel()
                .reader
                .find_agent(owner, target)
                .await
                .map_err(kernel_error)?
                .ok_or_else(|| AppError::Config("target is not a visible agent".into()))?
        };
        if id == caller {
            return Err(AppError::Config(
                "send messages to another visible agent".into(),
            ));
        }
        if id != owner {
            let config = self
                .kernel()
                .reader
                .configuration(id)
                .await
                .map_err(kernel_error)?;
            let metadata = self
                .kernel()
                .reader
                .metadata(id)
                .await
                .map_err(kernel_error)?;
            let parent = self
                .kernel()
                .reader
                .metadata(owner)
                .await
                .map_err(kernel_error)?;
            if config.parent_session_id != Some(owner)
                || config.agent.is_none()
                || config.relation_kind != agena_domain::SessionRelationKind::Subagent
                || metadata.workspace_id != parent.workspace_id
            {
                return Err(AppError::Config(
                    "target is outside this agent group".into(),
                ));
            }
        } else if direct_child {
            return Err(AppError::Config(
                "target must be a direct child agent".into(),
            ));
        }
        Ok(id)
    }

    fn agent_callback(
        &self,
        caller: i64,
        call: Option<i64>,
    ) -> Result<Option<Arc<RunControl>>, AppError> {
        call.map(|call| self.live_tool(caller, call).map(|(_, control)| control))
            .transpose()
    }

    async fn agent_message_payload(
        &self,
        caller: SessionId,
        message: &str,
    ) -> Result<serde_json::Value, AppError> {
        let config = self
            .kernel()
            .reader
            .configuration(caller)
            .await
            .map_err(kernel_error)?;
        let name = config
            .agent
            .as_ref()
            .map(|agent| agent.name.as_str())
            .unwrap_or("parent");
        agent_message(caller, name, message, false)
    }

    pub async fn message_agent(
        &self,
        caller: i64,
        call: Option<i64>,
        request: AgentMessageRequest,
    ) -> Result<AgentMessageReceipt, AppError> {
        self.agent_callback(caller, call)?;
        let caller = session_id(caller)?;
        let target = self
            .resolve_agent_target(caller, &request.target, false)
            .await?;
        let payload = self.agent_message_payload(caller, &request.message).await?;
        let command = RequestId::new();
        persist(|| {
            self.kernel().executor.deliver(
                command,
                target,
                format!("agent-message:{command}"),
                payload.clone(),
            )
        })
        .await
        .map_err(kernel_error)?;
        Ok(AgentMessageReceipt {
            agent_id: target.get(),
            accepted: true,
            state: "queued".into(),
        })
    }

    pub async fn wait_agent(
        &self,
        caller: i64,
        call: Option<i64>,
        request: AgentWaitRequest,
    ) -> Result<AgentWaitResponse, AppError> {
        let timeout = request.timeout_ms.unwrap_or(60_000);
        if !(1..=300_000).contains(&timeout) {
            return Err(AppError::Config(
                "agent.wait timeout_ms must be between 1 and 300000".into(),
            ));
        }
        let control = self.agent_callback(caller, call)?;
        let session = session_id(caller)?;
        self.agent_group(session).await?;
        let reason = self
            .with_agent_wait_permit(
                caller,
                call,
                control.as_deref(),
                self.wait_agent_mailbox(
                    session,
                    control.as_deref(),
                    Duration::from_millis(timeout),
                ),
            )
            .await?;
        Ok(AgentWaitResponse { reason })
    }

    pub async fn find_imported_plugin_agent(
        &self,
        caller: i64,
        call: Option<i64>,
        name: &str,
    ) -> Result<Option<ImportedPluginAgentState>, AppError> {
        self.agent_callback(caller, call)?;
        let parent = session_id(caller)?;
        let Some(child) = *read_with_busy_retry(parent, "find_workflow_agent", || {
            self.kernel().reader.find_agent(parent, name)
        })
        .await?
        else {
            return Ok(None);
        };
        let configuration = read_with_busy_retry(child, "workflow_agent_configuration", || {
            self.kernel().reader.configuration(child)
        })
        .await?;
        let Some(agent) = &configuration.agent else {
            return Ok(None);
        };
        Ok(Some(match agent.status {
            AgentStatus::Completed => ImportedPluginAgentState::Completed(child.get()),
            status if status.is_terminal() => ImportedPluginAgentState::Failed,
            _ => ImportedPluginAgentState::Active,
        }))
    }

    pub async fn wait_imported_plugin_agent(
        &self,
        caller: i64,
        call: Option<i64>,
        child: i64,
        timeout_ms: u64,
    ) -> Result<agena_plugin_host::sdk::ToolInvokeOutput, AppError> {
        let control = self.agent_callback(caller, call)?;
        let cancel = control
            .as_ref()
            .map(|control| control.cancel.clone())
            .or_else(agena_plugin_host::sdk::current_cancellation_token)
            .unwrap_or_else(|| self.shutdown.child_token());
        let wait = self.wait_imported_child(caller, child, timeout_ms, cancel);
        self.with_agent_wait_permit(caller, call, control.as_deref(), wait)
            .await
    }

    pub(in crate::session::manager) async fn wait_imported_child(
        &self,
        caller: i64,
        child: i64,
        timeout_ms: u64,
        cancel: CancellationToken,
    ) -> Result<agena_plugin_host::sdk::ToolInvokeOutput, AppError> {
        let child = self
            .resolve_agent_target(session_id(caller)?, &child.to_string(), true)
            .await?;
        let wait = async {
            loop {
                let running = self
                    .active
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .values()
                    .find(|task| task.session == child)
                    .cloned();
                let config = self
                    .kernel()
                    .reader
                    .configuration(child)
                    .await
                    .map_err(kernel_error)?;
                let agent = config.agent.as_ref().ok_or_else(|| {
                    AppError::Config("the plugin task is not a child agent".into())
                })?;
                if agent.status.is_terminal() {
                    if agent.status != AgentStatus::Completed {
                        let failure = agent
                            .failure
                            .as_ref()
                            .map(|failure| failure.user.fallback.as_str())
                            .unwrap_or(agent.status.as_ref());
                        return Err(AppError::Internal(format!(
                            "imported plugin task failed: {failure}"
                        )));
                    }
                    if let Some(message) = self.latest_origin_message(child).await? {
                        let input = read_with_busy_retry(child, "plugin_agent_result", || {
                            self.kernel().reader.entry(child, message)
                        })
                        .await?;
                        if let Some(text) = self
                            .final_execution_text(
                                child,
                                input.execution_id,
                                Some(input.execution_generation),
                                64 * 1024,
                            )
                            .await?
                        {
                            return Ok(agena_plugin_host::sdk::ToolInvokeOutput::text(text));
                        }
                    }
                    return Err(AppError::Internal(
                        "imported plugin task completed without text output".into(),
                    ));
                }
                if let Some(task) = running {
                    task.wait_settled().await;
                } else {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AppError::Cancelled),
            result = tokio::time::timeout(Duration::from_millis(timeout_ms.clamp(1, 300_000)), wait) => {
                result.map_err(|_| AppError::Internal("imported plugin task timed out".into()))?
            }
        }
    }

    pub(in crate::session::manager) async fn with_agent_wait_permit<T>(
        &self,
        caller: i64,
        call: Option<i64>,
        control: Option<&RunControl>,
        wait: impl std::future::Future<Output = Result<T, AppError>>,
    ) -> Result<T, AppError> {
        // Gateway calls may contain nested tool invocations. Release the actual
        // outer tool permit, rather than recognizing just a top-level name.
        let slot = call.and_then(|call| {
            self.tools
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&(caller, call))
                .map(|live| live.permit.clone())
        });
        let mut slot_guard = match &slot {
            Some(slot) => Some(slot.lock().await),
            None => None,
        };
        let released = slot_guard
            .as_mut()
            .is_some_and(|slot| slot.take().is_some());
        let outcome = wait.await;
        if released {
            let control = control.expect("a live tool owns its permit");
            let permit = tokio::select! {
                biased;
                _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
                permit = control.state.tools.clone().acquire_owned() => permit.map_err(|_| AppError::Cancelled)?,
            };
            **slot_guard.as_mut().expect("released permit slot") = Some(permit);
        }
        outcome
    }

    pub(super) async fn wait_agent_mailbox(
        &self,
        session: SessionId,
        control: Option<&RunControl>,
        timeout: Duration,
    ) -> Result<AgentWakeReason, AppError> {
        // Subscribe before the snapshot, including when resubscribing after a
        // lag/reset. A message arriving in either interval cannot be missed.
        let mut watch = self
            .kernel()
            .reader
            .watch(session, None)
            .map_err(kernel_error)?;
        let deadline = tokio::time::Instant::now() + timeout;
        let cancel = control
            .map(|c| c.cancel.clone())
            .unwrap_or_else(|| self.shutdown.clone());
        loop {
            let pending = self
                .kernel()
                .reader
                .pending_deliveries(session, None, 128)
                .await
                .map_err(kernel_error)?;
            if pending.iter().any(|d| {
                d.target_execution
                    .is_none_or(|id| control.is_some_and(|c| c.execution == id))
            }) {
                return Ok(AgentWakeReason::Message);
            }
            if let Some(control) = control {
                let active = self
                    .kernel()
                    .reader
                    .active_executions(session, None, 128)
                    .await
                    .map_err(kernel_error)?;
                if active
                    .executions
                    .iter()
                    .any(|e| e.id != control.execution && e.phase == ExecutionPhase::Queued)
                {
                    return Ok(AgentWakeReason::UserInput);
                }
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => return Ok(AgentWakeReason::Timeout),
                event = watch.recv() => { if event.is_err() { watch = self.kernel().reader.watch(session,None).map_err(kernel_error)?; } }
            }
        }
    }

    pub async fn followup_agent(
        &self,
        caller: i64,
        call: Option<i64>,
        request: AgentMessageRequest,
    ) -> Result<AgentHandle, AppError> {
        self.agent_callback(caller, call)?;
        let parent = session_id(caller)?;
        let child = self
            .resolve_agent_target(parent, &request.target, true)
            .await?;
        let payload = self.agent_message_payload(parent, &request.message).await?;
        loop {
            self.agent_callback(caller, call)?;
            // Settlement and launch share this lock. Watch first, so the tiny
            // execution-finished / agent-settled gap cannot lose its wake.
            let mut watch = self
                .kernel()
                .reader
                .watch(child, None)
                .map_err(kernel_error)?;
            let preparation = self.agent_preparation.lock().await;
            let config = self
                .kernel()
                .reader
                .configuration(child)
                .await
                .map_err(kernel_error)?;
            let agent = config
                .agent
                .clone()
                .ok_or_else(|| AppError::Config("target has no agent identity".into()))?;
            let active = self
                .kernel()
                .reader
                .active_executions(child, None, 1)
                .await
                .map_err(kernel_error)?;
            if let Some(execution) = active.executions.first() {
                let execution = execution.id;
                let command = RequestId::new();
                match persist(|| {
                    self.kernel().executor.deliver_to_execution(
                        command,
                        child,
                        execution,
                        format!("agent-followup:{command}"),
                        payload.clone(),
                    )
                })
                .await
                {
                    Ok(_) => {
                        return Ok(AgentHandle {
                            agent_id: child.get(),
                            name: agent.name,
                            profile: Some(
                                config
                                    .profile
                                    .as_deref()
                                    .unwrap_or(agena_runtime_config::DEFAULT_SUBAGENT_PROFILE)
                                    .to_owned(),
                            ),
                            status: AgentStatus::Running,
                        });
                    }
                    Err(KernelError::Conflict(_)) => {}
                    Err(error) => return Err(kernel_error(error)),
                }
            } else if agent.status != AgentStatus::Running {
                drop(preparation);
                let launch = AgentLaunch {
                    parent_session_id: caller,
                    launch_call_id: call,
                    workflow: None,
                    name: agent.name,
                    prompt: request.message.clone(),
                    profile: None,
                    commands: Vec::new(),
                    selection: Default::default(),
                    timeout_ms: agent.limits.timeout_ms,
                    max_tokens: agent.limits.max_tokens,
                    max_cost_microusd: agent.limits.max_cost_microusd,
                    inherit_parent_context: false,
                    existing: Some(child),
                    cordis: None,
                };
                match self.launch_agent(launch).await {
                    Err(AppError::ExecutionAlreadyActive(_)) => continue,
                    result => return result.map(|launch| launch.handle),
                }
            }
            drop(preparation);
            let cancel = self
                .agent_callback(caller, call)?
                .map(|c| c.cancel.clone())
                .unwrap_or_else(|| self.shutdown.clone());
            tokio::select! { _ = cancel.cancelled() => return Err(AppError::Cancelled), _ = watch.recv() => {} }
        }
    }

    pub async fn cancel_agent(
        &self,
        caller: i64,
        call: Option<i64>,
        request: AgentTargetRequest,
    ) -> Result<AgentMessageReceipt, AppError> {
        self.agent_callback(caller, call)?;
        let child = self
            .resolve_agent_target(session_id(caller)?, &request.target, true)
            .await?;
        let _preparation = self.agent_preparation.lock().await;
        self.cancel_active_execution_with_outcome(child.get())
            .await?;
        Ok(AgentMessageReceipt {
            agent_id: child.get(),
            accepted: true,
            state: "cancelled".into(),
        })
    }

    /// Rebuilt on each model boundary, so compaction/restart never loses
    /// routing information. Contains identity and membership only, bounded to
    /// 8 peers.
    ///
    /// Live execution state is deliberately absent: a status line changes every
    /// time a peer starts or finishes, which would rewrite the cached prompt
    /// prefix on each turn. The model observes that state through the agent
    /// tools that return it — `agent.wait` reports why it woke, and
    /// `agent.spawn` / `agent.followup` / `agent.cancel` return the observed
    /// handle — while finished work is delivered automatically.
    pub(in crate::session::manager) async fn agent_roster(
        &self,
        caller: SessionId,
    ) -> Result<String, AppError> {
        let owner = self.agent_group(caller).await?;
        let page = read_with_busy_retry(caller, "agent_roster", || {
            self.kernel().reader.query_summaries(SummaryQuery {
                parent_id: Some(owner),
                agents_only: true,
                exclude_workflow_agents: true,
                limit: MAX_AGENTS_PER_PARENT,
                ..Default::default()
            })
        })
        .await?;
        if caller == owner && page.sessions.is_empty() {
            return Ok(String::new());
        }
        let mut text = format!(
            "\n\n# Agent roster\nSelf agent_id: {}. Parent/group owner agent_id: {}.\n",
            caller.get(),
            owner.get()
        );
        for member in &page.sessions {
            text.push_str(&format!(
                "{} agent_id={}{}\n",
                member.agent_name.as_deref().unwrap_or("agent"),
                member.id.get(),
                if member.id == caller { " (self)" } else { "" }
            ));
        }
        Ok(text)
    }
}

pub(super) fn agent_message(
    caller: SessionId, name: &str, message: &str, wake: bool,
) -> Result<serde_json::Value, AppError> {
    if message.trim().is_empty() || message.len() > 12 * 1024 {
        return Err(AppError::Config("agent message must contain 1–12288 bytes".into()));
    }
    bounded_agent_payload(serde_json::json!({
        "kind":"agent_message", "sender_agent_id":caller.get(), "sender_name":name,
        "text": format!("Message from {name} ({}):\n{message}", caller.get()), "wake":wake,
    }))
}
