use super::{
    AppError, Arc, EventKind, ExecutionControl, ExecutionSource, FinishReason, HistoryMessageId,
    HistoryRunId, MessageMetadata, MessageSource, MessageStatus, PartContent, Role, RunCompleted,
    RunStarted, SessionExecutionRequest, SessionManager, SessionSubtaskRequest,
    SessionSubtaskResponse, SessionUserMessageRequest, TranscriptContent, UserMessageAppended,
    build_message, mpsc,
};
use crate::event::SubtaskStatusChangedEvent;
use crate::session::Session;

impl SessionManager {
    #[tracing::instrument(skip(self, request), fields(session_id = request.run.session_id))]
    pub async fn submit_user_message(
        &self,
        request: SessionUserMessageRequest,
    ) -> Result<Session, AppError> {
        let session_id = request.run.session_id;
        self.execute_registered(
            session_id,
            ExecutionSource::User,
            "user execution",
            move |manager, control, steer_rx| async move {
                manager
                    .submit_user_message_inner(request, control, steer_rx)
                    .await
            },
        )
        .await
    }

    async fn submit_user_message_inner(
        &self,
        mut request: SessionUserMessageRequest,
        control: Arc<ExecutionControl>,
        steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();

        // Plugin chain: user.prompt.submit. Plugins can rewrite or block the
        // user's prompt before it enters the session.
        let prompt_text = request
            .parts
            .iter()
            .filter_map(|p| p.text_value())
            .collect::<Vec<_>>()
            .join("\n");
        if !prompt_text.is_empty() {
            let input = crate::plugin::UserPromptSubmitInput {
                session_id: request.run.session_id,
                prompt: prompt_text,
            };
            match state
                .tool_executor
                .plugin_manager()
                .dispatch_user_prompt_submit_cancellable(input, Some(control.cancel.clone()))
                .await
            {
                Ok(updated) => {
                    // Replace text parts with the (potentially rewritten) prompt.
                    let mut replaced = false;
                    for part in &mut request.parts {
                        if part.text_value().is_some() {
                            *part = PartContent::text(updated.prompt.clone());
                            replaced = true;
                            break;
                        }
                    }
                    if !replaced {
                        request.parts.push(PartContent::text(updated.prompt));
                    }
                }
                Err(err) => {
                    if control.cancel.is_cancelled() {
                        return Err(AppError::Cancelled);
                    }
                    return Err(AppError::Internal(format!(
                        "prompt blocked by plugin: {}",
                        err.message
                    )));
                }
            }
        }

        let mut session = self
            .store
            .load_session(request.run.session_id, state.cache_policy())
            .await?;
        session = self
            .apply_requested_agent_profile(session, &mut request.run.options, state.clone())
            .await?;
        let options = self.apply_execution_context_to_run_options(&session, request.run.options)?;
        self.apply_run_selection_to_session(&mut session, &options);
        let ids = self.store.reserve_message_ids(request.parts.len()).await?;
        let user_message = build_message(
            ids,
            Role::User,
            MessageStatus::Completed,
            request.parts,
            MessageMetadata {
                source: MessageSource::User,
                parent_message_id: session
                    .last_conversation_message()
                    .map(|message| message.id),
                generated_by_call_id: None,
                model_provider_id: options.model.provider_id.to_string(),
                model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
                model_id: options.model.model_id.to_string(),
                model_thinking_mode: options.thinking_mode.clone(),
                model_speed_mode: options.speed_mode.clone(),
            },
        );
        session.messages.push(user_message.clone());
        session = self
            .persist_session_changes(
                session,
                vec![user_message.clone()],
                Vec::new(),
                None,
                state.clone(),
            )
            .await?;

        // Append-only history: persist the user-authored message as its own
        // closed run batch so it remains addressable in fork/rewind flows.
        let user_run_id = HistoryRunId::new();
        let user_history_items = vec![
            EventKind::RunStarted(RunStarted {
                execution_id: control.execution_id(),
                run_id: user_run_id,
                source: ExecutionSource::User,
                model_id: options.model.model_id.as_ref().into(),
                provider_id: options.model.provider_id.as_ref().into(),
                request_digest: None,
            }),
            EventKind::UserMessageAppended(UserMessageAppended {
                execution_id: control.execution_id(),
                message_id: HistoryMessageId(user_message.id),
                run_id: user_run_id,
                created_at: user_message.created_at,
                content: TranscriptContent::from_message_lossy(&user_message),
                parts: user_message.parts.clone(),
                metadata: user_message.metadata.clone(),
                provider_state: user_message.provider_state.clone(),
            }),
            EventKind::RunCompleted(RunCompleted {
                run_id: user_run_id,
                finish_reason: FinishReason::Stop,
            }),
        ];
        session = self
            .store
            .append_history_items(session, user_history_items, state.cache_policy())
            .await?;

        self.run_until_stable(
            session,
            &options,
            false,
            ExecutionSource::User,
            state,
            control,
            steer_rx,
        )
        .await
    }

    pub async fn continue_session(
        &self,
        request: SessionExecutionRequest,
    ) -> Result<Session, AppError> {
        let session_id = request.session_id;
        self.execute_registered(
            session_id,
            ExecutionSource::Continue,
            "continuation execution",
            move |manager, control, steer_rx| async move {
                manager
                    .continue_session_inner(request, control, steer_rx)
                    .await
            },
        )
        .await
    }

    async fn continue_session_inner(
        &self,
        mut request: SessionExecutionRequest,
        control: Arc<ExecutionControl>,
        steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        session = self
            .apply_requested_agent_profile(session, &mut request.options, state.clone())
            .await?;
        let options = self.apply_execution_context_to_run_options(&session, request.options)?;
        if self.apply_run_selection_to_session(&mut session, &options) {
            session = self
                .persist_session_changes(session, Vec::new(), Vec::new(), None, state.clone())
                .await?;
        }
        self.run_until_stable(
            session,
            &options,
            true,
            ExecutionSource::Continue,
            state,
            control,
            steer_rx,
        )
        .await
    }

    pub async fn run_subtask(
        &self,
        request: SessionSubtaskRequest,
    ) -> Result<SessionSubtaskResponse, AppError> {
        let task_started = tokio::time::Instant::now();
        let task_timeout = request.timeout_ms.map(std::time::Duration::from_millis);
        let state = self.execution_state();
        let description = request.description.trim();
        if description.is_empty() {
            return Err(AppError::Config(
                "subtask description must not be empty".to_string(),
            ));
        }
        let delegated_prompt = request.prompt.trim();
        if delegated_prompt.is_empty() {
            return Err(AppError::Config(
                "subtask prompt must not be empty".to_string(),
            ));
        }
        let requested_profile_name = request.profile_name.trim();
        if requested_profile_name.is_empty() {
            return Err(AppError::Config(
                "subtask profile name must not be empty".to_string(),
            ));
        }
        if request.timeout_ms == Some(0) {
            return Err(AppError::Config(
                "subtask timeout_ms must be greater than zero".to_string(),
            ));
        }
        let profile = state
            .tool_executor
            .subagent_registry()
            .require(requested_profile_name)
            .map_err(|error| AppError::Config(error.to_string()))?;
        let parent = self
            .store
            .load_session(request.parent_session_id, state.cache_policy())
            .await?;
        if parent.is_subagent {
            return Err(AppError::Config(
                "delegated subtasks cannot create nested subtasks".to_string(),
            ));
        }
        let options = self.subtask_run_options(
            &parent,
            &state,
            &profile.frontmatter.defaults,
            &request.requested_selection,
        )?;
        let task_id = match request.task_id.as_deref().map(str::trim) {
            Some("") => {
                return Err(AppError::Config(
                    "subtask task_id must not be empty when supplied".to_string(),
                ));
            }
            Some(value) if value.len() > 128 => {
                return Err(AppError::Config(
                    "subtask task_id must not exceed 128 bytes".to_string(),
                ));
            }
            Some(value) => value.to_string(),
            None => format!("task_{}", uuid::Uuid::new_v4().simple()),
        };

        // Serialize preparation for a direct parent so `(parent_id, task_id)`
        // remains deterministic before the database uniqueness constraint is
        // reached. The lock is released before model work starts.
        let preparation_lock = self.reply_session_lock(parent.id).await;
        let preparation_guard = preparation_lock.lock().await;
        let existing = self
            .store
            .find_subagent_by_task_id(parent.id, task_id.as_str(), state.cache_policy())
            .await?;
        let resumed = existing.is_some();
        let mut child = match existing {
            Some(existing) => {
                if self.execution_registry.is_active(existing.id).await {
                    return Err(AppError::ExecutionAlreadyActive(existing.id));
                }
                existing
            }
            None => {
                self.store
                    .create_subagent_session(
                        description.to_string(),
                        parent.id,
                        task_id.clone(),
                        state.cache_policy(),
                    )
                    .await?
            }
        };

        child.runtime.execution.selection.agent = Some(profile.name.clone());
        child.runtime.execution.agent_system_prompt = Some(profile.prompt.trim().to_string());
        child
            .runtime
            .set_allowed_tools(effective_subtask_allowed_tools(
                parent.runtime.allowed_tools(),
                crate::agents::allowed_tools(&profile).as_slice(),
            ));
        let mut child_permission = self.resolve_effective_session_permission(
            &child,
            &state,
            Some(&profile.frontmatter.permission),
        );
        let parent_permission = if parent.runtime.execution.effective_permission.is_empty() {
            self.resolve_effective_session_permission(&parent, &state, None)
        } else {
            parent.runtime.execution.effective_permission.clone()
        };
        child_permission.merge_from(non_recursive_subtask_permission_ceiling());
        child.runtime.execution.effective_permission = child_permission;
        child.runtime.execution.permission_ceiling = parent_permission;
        child.runtime.execution.effective_workspace_root = Some(
            parent
                .runtime
                .execution
                .effective_workspace_root
                .clone()
                .unwrap_or_else(|| state.tool_executor.workspace_root().to_path_buf()),
        );
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let baseline_message_id = child.messages.iter().map(|message| message.id).max();
        let baseline_usage = child.aggregate_usage();
        child.runtime.subtask.status = crate::session::SubtaskStatus::Running;
        child.runtime.subtask.started_at_ms = Some(started_at_ms);
        child.runtime.subtask.finished_at_ms = None;
        child.runtime.subtask.error = None;
        self.apply_run_selection_to_session(&mut child, &options);
        child = self
            .store
            .update_subtask_state(
                child,
                crate::session::SubtaskRuntimeState {
                    status: crate::session::SubtaskStatus::Running,
                    started_at_ms: Some(started_at_ms),
                    finished_at_ms: None,
                    error: None,
                },
                state.cache_policy(),
            )
            .await?;
        let child_id = child.id;
        self.persist_session_changes(
            child,
            Vec::new(),
            vec![EventKind::SubtaskStatusChanged(SubtaskStatusChangedEvent {
                session_id: child_id,
                parent_session_id: parent.id,
                task_id: task_id.clone(),
                profile: profile.name.clone(),
                status: crate::session::SubtaskStatus::Running,
                resumed,
                started_at_ms: Some(started_at_ms),
                finished_at_ms: None,
                error: None,
                ts_ms: started_at_ms,
            })],
            None,
            state.clone(),
        )
        .await?;
        drop(preparation_guard);

        let manager = self.background_handle();
        let run_options = options.clone();
        let prompt = delegated_prompt.to_string();
        let mut run = tokio::task::spawn(async move {
            manager
                .submit_user_message(SessionUserMessageRequest::new(
                    child_id,
                    run_options,
                    vec![PartContent::text(prompt)],
                ))
                .await
        });

        // Ensure the spawned owner has either registered its execution or
        // already finished before a very short deadline can fire. Otherwise a
        // timeout could attempt cancellation just before registration, miss
        // the execution, and leave an unbounded detached child running.
        while !run.is_finished() && !self.execution_registry.is_active(child_id).await {
            tokio::task::yield_now().await;
        }

        let mut timed_out = false;
        let run_result = if let Some(timeout) = task_timeout {
            let remaining = timeout.saturating_sub(task_started.elapsed());
            match tokio::time::timeout(remaining, &mut run).await {
                Ok(joined) => joined
                    .map_err(|error| {
                        AppError::Internal(format!("subtask run task failed: {error}"))
                    })
                    .and_then(std::convert::identity),
                Err(_) => {
                    timed_out = true;
                    let _ = self.cancel_active_execution(child_id).await;
                    match tokio::time::timeout(std::time::Duration::from_secs(5), &mut run).await {
                        Ok(joined) => joined
                            .map_err(|error| {
                                AppError::Internal(format!(
                                    "timed-out subtask failed while cancelling: {error}"
                                ))
                            })
                            .and_then(std::convert::identity),
                        // Dropping a JoinHandle detaches the task. The
                        // execution owns its registry cleanup and will finish
                        // once the provider observes cancellation, while the
                        // caller still receives a bounded timeout response.
                        Err(_) => Err(AppError::Cancelled),
                    }
                }
            }
        } else {
            run.await
                .map_err(|error| AppError::Internal(format!("subtask run task failed: {error}")))
                .and_then(std::convert::identity)
        };

        let (status, error, mut session) = match run_result {
            Ok(session) if timed_out => (
                crate::session::SubtaskStatus::TimedOut,
                Some("subtask exceeded its configured timeout".to_string()),
                session,
            ),
            Ok(session) => (crate::session::SubtaskStatus::Completed, None, session),
            Err(error) => {
                let status = if timed_out {
                    crate::session::SubtaskStatus::TimedOut
                } else if matches!(&error, AppError::Cancelled) {
                    crate::session::SubtaskStatus::Cancelled
                } else {
                    crate::session::SubtaskStatus::Failed
                };
                let message = error.to_string();
                let session = self
                    .store
                    .load_session(child_id, state.cache_policy())
                    .await?;
                (status, Some(message), session)
            }
        };
        let finished_at_ms = chrono::Utc::now().timestamp_millis();
        session.runtime.subtask.status = status;
        session.runtime.subtask.finished_at_ms = Some(finished_at_ms);
        session.runtime.subtask.error = error.clone();
        let subtask_started_at_ms = session.runtime.subtask.started_at_ms;
        session = self
            .store
            .update_subtask_state(
                session,
                crate::session::SubtaskRuntimeState {
                    status,
                    started_at_ms: subtask_started_at_ms,
                    finished_at_ms: Some(finished_at_ms),
                    error: error.clone(),
                },
                state.cache_policy(),
            )
            .await?;
        session = self
            .persist_session_changes(
                session,
                Vec::new(),
                vec![EventKind::SubtaskStatusChanged(SubtaskStatusChangedEvent {
                    session_id: child_id,
                    parent_session_id: parent.id,
                    task_id: task_id.clone(),
                    profile: profile.name.clone(),
                    status,
                    resumed,
                    started_at_ms: subtask_started_at_ms,
                    finished_at_ms: Some(finished_at_ms),
                    error: error.clone(),
                    ts_ms: finished_at_ms,
                })],
                None,
                state,
            )
            .await?;
        let usage = session.aggregate_usage().saturating_sub(&baseline_usage);

        Ok(SessionSubtaskResponse {
            task_id,
            parent_session_id: parent.id,
            profile_name: profile.name,
            status,
            resumed,
            final_text: session.last_assistant_text_after(baseline_message_id),
            error,
            usage,
            model_provider_id: Some(options.model.provider_id.to_string()),
            model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
            model_id: Some(options.model.model_id.to_string()),
            session,
        })
    }
}

fn effective_subtask_allowed_tools(parent: &[String], profile: &[String]) -> Vec<String> {
    if parent.is_empty() {
        return profile.to_vec();
    }
    if profile.is_empty() {
        return parent.to_vec();
    }
    let profile = profile.iter().collect::<std::collections::HashSet<_>>();
    let intersection = parent
        .iter()
        .filter(|tool| profile.contains(tool))
        .cloned()
        .collect::<Vec<_>>();
    if intersection.is_empty() {
        // An empty allowlist means "unrestricted" to the tool executor. Keep a
        // disjoint parent/profile intersection explicitly restrictive instead.
        vec!["__agena_no_tools__".to_string()]
    } else {
        intersection
    }
}

pub(in crate::session::manager) fn non_recursive_subtask_permission_ceiling()
-> crate::agent::PermissionConfig {
    let deny = crate::permission::PermissionMode::Deny;
    crate::agent::PermissionConfig {
        tools: Some(crate::agent::ToolPermissionConfig {
            names: std::collections::BTreeMap::from([
                ("task".to_string(), deny),
                ("tasks.run".to_string(), deny),
                ("agena.tasks.run".to_string(), deny),
                ("agena_tasks_run".to_string(), deny),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_subtask_allowed_tools, non_recursive_subtask_permission_ceiling};
    use crate::permission::PermissionMode;

    #[test]
    fn subtask_tool_allowlists_intersect_parent_and_profile_boundaries() {
        assert_eq!(
            effective_subtask_allowed_tools(
                &["read".to_string(), "shell".to_string()],
                &["read".to_string(), "web".to_string()],
            ),
            vec!["read"]
        );
        assert_eq!(
            effective_subtask_allowed_tools(&[], &["read".to_string()]),
            vec!["read"]
        );
        assert_eq!(
            effective_subtask_allowed_tools(&["read".to_string()], &[]),
            vec!["read"]
        );
        assert_eq!(
            effective_subtask_allowed_tools(&["read".to_string()], &["shell".to_string()]),
            vec!["__agena_no_tools__"]
        );
    }

    #[test]
    fn delegated_agents_cannot_recursively_run_tasks() {
        let permission = non_recursive_subtask_permission_ceiling();
        let names = &permission.tools.expect("task ceiling").names;
        for name in ["task", "tasks.run", "agena.tasks.run", "agena_tasks_run"] {
            assert_eq!(names.get(name), Some(&PermissionMode::Deny));
        }

        let agent = crate::agent::Agent::new(
            "delegated",
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        )
        .try_apply_permission_config(&non_recursive_subtask_permission_ceiling())
        .expect("valid non-recursive policy");
        assert!(matches!(
            agent.authorize_tool_name("agena.tasks.run"),
            crate::permission::PermissionDecision::Deny { .. }
        ));
    }
}
