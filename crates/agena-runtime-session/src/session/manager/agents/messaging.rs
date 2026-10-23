//! Scoped durable mailboxes. Wait observes facts; provider boundaries deliver content.
use super::*;
use agena_plugin_host::sdk::host_api::{
    AgentMessageReceipt, AgentMessageRequest, AgentTargetRequest, AgentWaitRequest,
    AgentWaitResponse, AgentWakeReason,
};

impl SessionManager {
    async fn agent_group(&self, caller: SessionId) -> Result<SessionId, AppError> {
        let configuration = self
            .kernel()
            .reader
            .configuration(caller)
            .await
            .map_err(kernel_error)?;
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
        if message.trim().is_empty() || message.len() > 12 * 1024 {
            return Err(AppError::Config(
                "agent message must contain 1–12288 bytes".into(),
            ));
        }
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
        bounded_agent_payload(serde_json::json!({
            "kind":"agent_message", "sender_agent_id":caller.get(), "sender_name":name,
            "text": format!("Message from {name} ({}):\n{message}", caller.get()), "wake":false
        }))
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
        let outcome = self
            .wait_agent_mailbox(session, control.as_deref(), Duration::from_millis(timeout))
            .await;
        if released {
            let control = control.as_ref().expect("a live tool owns its permit");
            let permit = tokio::select! {
                biased;
                _ = control.cancel.cancelled() => return Err(AppError::Cancelled),
                permit = control.state.tools.clone().acquire_owned() => permit.map_err(|_| AppError::Cancelled)?,
            };
            **slot_guard.as_mut().expect("released permit slot") = Some(permit);
        }
        outcome.map(|reason| AgentWaitResponse { reason })
    }

    async fn wait_agent_mailbox(
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
                            status: AgentStatus::Running,
                        });
                    }
                    Err(KernelError::Conflict(_)) => {}
                    Err(error) => return Err(kernel_error(error)),
                }
            } else if agent.status != AgentStatus::Running {
                let metadata = self
                    .kernel()
                    .reader
                    .metadata(child)
                    .await
                    .map_err(kernel_error)?;
                let selection = metadata.selection.clone().unwrap_or_default();
                drop(preparation);
                let launch = AgentLaunch {
                    parent_session_id: caller,
                    launch_call_id: call,
                    name: agent.name,
                    prompt: request.message.clone(),
                    commands: Vec::new(),
                    selection: agena_domain::ModelSelectionConfig {
                        provider: selection.provider,
                        adapter: selection.adapter,
                        model: selection.model,
                        thinking_mode: selection.thinking_mode,
                        speed_mode: selection.speed_mode,
                        verbosity: selection.verbosity,
                        parallel_tool_calls: selection.parallel_tool_calls,
                    },
                    timeout_ms: agent.limits.timeout_ms,
                    max_tokens: agent.limits.max_tokens,
                    max_cost_microusd: agent.limits.max_cost_microusd,
                    existing: Some(child),
                };
                match self.launch_agent(launch).await {
                    Err(AppError::ExecutionAlreadyActive(_)) => continue,
                    result => return result,
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
    /// routing information. Contains identity and state only, bounded to 8 peers.
    pub(in crate::session::manager) async fn agent_roster(
        &self,
        caller: SessionId,
    ) -> Result<String, AppError> {
        let owner = self.agent_group(caller).await?;
        let page = self
            .kernel()
            .reader
            .query_summaries(SummaryQuery {
                parent_id: Some(owner),
                agents_only: true,
                limit: MAX_AGENTS_PER_PARENT,
                ..Default::default()
            })
            .await
            .map_err(kernel_error)?;
        if caller == owner && page.sessions.is_empty() {
            return Ok(String::new());
        }
        let mut text = format!(
            "\n\n# Agent roster\nSelf agent_id: {}. Parent/group owner agent_id: {}.\n",
            caller.get(),
            owner.get()
        );
        if caller != owner {
            text.push_str("Use target `parent` to message the parent. You may message/wait; only the parent may spawn/followup/cancel.\n");
        }
        for member in &page.sessions {
            text.push_str(&format!(
                "{} agent_id={} status={}{}\n",
                member.agent_name.as_deref().unwrap_or("agent"),
                member.id.get(),
                member.agent_status.unwrap_or_default().as_ref(),
                if member.id == caller { " (self)" } else { "" }
            ));
        }
        Ok(text)
    }
}
