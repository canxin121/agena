use super::{
    AppError, Arc, EventKind, ExecutionControl, ExecutionSource, ExecutionStatus, FinishReason,
    HistoryMessageId, HistoryRunId, MessageMetadata, MessageSource, PartContent, Role,
    RunCompleted, RunStarted, SessionExecutionRequest, SessionManager, SessionSubtaskRequest,
    SessionSubtaskResponse, SessionUserMessageRequest, StableRunContext, TranscriptContent,
    UserInputPart, UserMessageAppended, build_message, mpsc,
};
use crate::session::Session;
use agena_domain::SubtaskStatusChangedEvent;

impl SessionManager {
    async fn require_subtask_session(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Session, AppError> {
        let task_id = task_id.trim();
        if task_id.is_empty() {
            return Err(AppError::Config(
                "subtask task_id must not be empty".to_string(),
            ));
        }
        self.store
            .find_subagent_by_task_id(
                parent_session_id,
                task_id,
                self.execution_state().cache_policy(),
            )
            .await?
            .ok_or_else(|| {
                AppError::Config(format!(
                    "subtask '{task_id}' does not exist under session {parent_session_id}"
                ))
            })
    }

    pub async fn cancel_subtask(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<i64, AppError> {
        let child = self
            .require_subtask_session(parent_session_id, task_id)
            .await?;
        self.cancel_active_execution(child.id).await?;
        Ok(child.id)
    }

    pub async fn message_subtask(
        &self,
        parent_session_id: i64,
        task_id: &str,
        message: String,
    ) -> Result<i64, AppError> {
        if message.trim().is_empty() {
            return Err(AppError::Config(
                "subtask message must not be empty".to_string(),
            ));
        }
        let child = self
            .require_subtask_session(parent_session_id, task_id)
            .await?;
        self.steer_input(child.id, vec![PartContent::text(message)])
            .await?;
        Ok(child.id)
    }

    pub async fn read_subtask_output(
        &self,
        parent_session_id: i64,
        task_id: &str,
        after_cursor: i64,
        limit: u32,
    ) -> Result<crate::SessionSubtaskOutput, AppError> {
        let child = self
            .require_subtask_session(parent_session_id, task_id)
            .await?;
        let limit = limit.clamp(1, 500) as usize;
        let mut messages = child
            .messages
            .iter()
            .filter(|message| message.id > after_cursor)
            .filter_map(|message| {
                let text = message.visible_text_lossy();
                (!text.trim().is_empty()).then_some(crate::SessionSubtaskOutputChunk {
                    cursor: message.id,
                    role: message.role,
                    text,
                    created_at_ms: message.created_at.timestamp_millis(),
                })
            })
            .collect::<Vec<_>>();
        messages.sort_by_key(|message| message.cursor);
        let has_more = messages.len() > limit;
        messages.truncate(limit);
        let next_cursor = messages
            .last()
            .map_or(after_cursor, |message| message.cursor);
        Ok(crate::SessionSubtaskOutput {
            session_id: child.id,
            chunks: messages,
            next_cursor,
            has_more,
        })
    }

    #[tracing::instrument(skip(self, request), fields(session_id = request.run.session_id))]
    pub(in crate::session::manager) async fn submit_user_message_parts(
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
                    .submit_user_message_inner(request, control, steer_rx, None)
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
        usage_budget: Option<super::SubtaskUsageBudget>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();

        // Plugin chain: user.prompt.submit. Plugins can rewrite or block the
        // user's prompt before it enters the session.
        let prompt_text = request
            .parts
            .iter()
            .filter_map(|p| p.content.text_value())
            .collect::<Vec<_>>()
            .join("\n");
        if !prompt_text.is_empty() {
            let input = agena_plugin_host::UserPromptSubmitInput {
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
                        if part.content.text_value().is_some() {
                            part.content = PartContent::text(updated.prompt.clone());
                            replaced = true;
                            break;
                        }
                    }
                    if !replaced {
                        request.parts.push(UserInputPart {
                            activity_id: None,
                            content: PartContent::text(updated.prompt),
                        });
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
        self.refresh_execution_policy(&mut session, &state);
        let options = self.apply_execution_context_to_run_options(&session, request.run.options)?;
        self.apply_run_selection_to_session(&mut session, &options);
        let ids = self.store.reserve_message_ids(request.parts.len()).await?;
        let user_turn_id = ids.message_id;
        let input_parts = request.parts;
        let activity_ids = input_parts
            .iter()
            .map(|part| part.activity_id)
            .collect::<Vec<_>>();
        let mut user_message = build_message(
            ids,
            Role::User,
            ExecutionStatus::Completed,
            input_parts.into_iter().map(|part| part.content).collect(),
            MessageMetadata {
                source: MessageSource::User,
                idempotency_key: request.idempotency_key.clone(),
                turn_id: Some(user_turn_id),
                parent_message_id: session
                    .last_conversation_message()
                    .map(|message| message.id),
                generated_by_call_id: None,
                externally_initiated_tool: false,
                model_provider_id: options.model.provider_id.to_string(),
                model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
                model_id: options.model.model_id.to_string(),
                model_thinking_mode: options.thinking_mode.clone(),
                model_speed_mode: options.speed_mode.clone(),
            },
        );
        for (part, activity_id) in user_message.parts.iter_mut().zip(activity_ids) {
            if let Some(activity_id) = activity_id {
                part.bind_activity(activity_id);
            }
        }
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
            StableRunContext {
                allow_goal_continuation: false,
                base_run_source: ExecutionSource::User,
                active_turn_id: Some(user_message.id),
                state,
                control,
                steer_rx,
                usage_budget,
            },
        )
        .await
    }

    /// Same execution lifecycle as an ordinary user message, with an
    /// optional child-relative budget checked before every model turn. This
    /// stays private to delegated tasks so normal interactive sessions do not
    /// inherit a task's accounting boundary.
    pub(in crate::session::manager) async fn submit_subtask_user_message(
        &self,
        request: SessionUserMessageRequest,
        usage_budget: Option<super::SubtaskUsageBudget>,
    ) -> Result<Session, AppError> {
        let session_id = request.run.session_id;
        self.execute_registered(
            session_id,
            ExecutionSource::User,
            "subtask execution",
            move |manager, control, steer_rx| async move {
                manager
                    .submit_user_message_inner(request, control, steer_rx, usage_budget)
                    .await
            },
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
        request: SessionExecutionRequest,
        control: Arc<ExecutionControl>,
        steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
    ) -> Result<Session, AppError> {
        let state = self.execution_state();
        let mut session = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        self.refresh_execution_policy(&mut session, &state);
        let options = self.apply_execution_context_to_run_options(&session, request.options)?;
        if self.apply_run_selection_to_session(&mut session, &options) {
            session = self
                .persist_session_changes(session, Vec::new(), Vec::new(), None, state.clone())
                .await?;
        }
        self.run_until_stable(
            session,
            &options,
            StableRunContext {
                allow_goal_continuation: true,
                base_run_source: ExecutionSource::Continue,
                active_turn_id: None,
                state,
                control,
                steer_rx,
                usage_budget: None,
            },
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
        if request.timeout_ms == Some(0) {
            return Err(AppError::Config(
                "subtask timeout_ms must be greater than zero".to_string(),
            ));
        }
        if request.max_tokens == Some(0) {
            return Err(AppError::Config(
                "subtask max_tokens must be greater than zero".to_string(),
            ));
        }
        if request.max_cost_microusd == Some(0) {
            return Err(AppError::Config(
                "subtask max_cost_microusd must be greater than zero".to_string(),
            ));
        }
        let parent = self
            .store
            .load_session(request.parent_session_id, state.cache_policy())
            .await?;
        if parent.is_subagent() {
            return Err(AppError::Config(
                "delegated subtasks cannot create nested subtasks".to_string(),
            ));
        }
        let options =
            self.subtask_run_options(&parent, &state, &request.requested_model_selection)?;
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

        child.runtime.execution.access =
            if parent.runtime.execution.access == agena_domain::ExecutionAccess::ReadOnly {
                agena_domain::ExecutionAccess::ReadOnly
            } else {
                request.access
            };
        let child_permission = self.resolve_effective_session_permission(&child, &state);
        let parent_permission = if parent.runtime.execution.effective_permission.is_empty() {
            self.resolve_effective_session_permission(&parent, &state)
        } else {
            parent.runtime.execution.effective_permission.clone()
        };
        child.runtime.execution.effective_permission = child_permission;
        child.runtime.execution.permission_ceiling = parent_permission;
        child.runtime.execution.capability_denied_tool_names =
            non_recursive_subtask_capability_denials();
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
        let usage_budget = super::SubtaskUsageBudget::new(
            baseline_usage.clone(),
            request.max_tokens,
            request.max_cost_microusd,
        );
        child.runtime.subtask.status = agena_domain::SubtaskStatus::Running;
        child.runtime.subtask.started_at_ms = Some(started_at_ms);
        child.runtime.subtask.finished_at_ms = None;
        child.runtime.subtask.error = None;
        self.apply_run_selection_to_session(&mut child, &options);
        child = self
            .store
            .update_subtask_state(
                child,
                crate::session::SubtaskRuntimeState {
                    status: agena_domain::SubtaskStatus::Running,
                    started_at_ms: Some(started_at_ms),
                    finished_at_ms: None,
                    error: None,
                },
                state.cache_policy(),
            )
            .await?;
        let child_id = child.id;
        let child_access = child.runtime.execution.access;
        self.persist_session_changes(
            child,
            Vec::new(),
            vec![EventKind::SubtaskStatusChanged(SubtaskStatusChangedEvent {
                session_id: child_id,
                parent_session_id: parent.id,
                task_id: task_id.clone(),
                access: child_access,
                status: agena_domain::SubtaskStatus::Running,
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
                .submit_subtask_user_message(
                    SessionUserMessageRequest::new(
                        child_id,
                        run_options,
                        vec![PartContent::text(prompt)],
                    ),
                    usage_budget,
                )
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

        let (status, error, budget_exceeded, mut session) = match run_result {
            Ok(session) if timed_out => (
                agena_domain::SubtaskStatus::TimedOut,
                Some("subtask exceeded its configured timeout".to_string()),
                false,
                session,
            ),
            Ok(session) => (agena_domain::SubtaskStatus::Completed, None, false, session),
            Err(error) => {
                let status = if timed_out {
                    agena_domain::SubtaskStatus::TimedOut
                } else if matches!(&error, AppError::Cancelled) {
                    agena_domain::SubtaskStatus::Cancelled
                } else {
                    agena_domain::SubtaskStatus::Failed
                };
                let budget_exceeded = matches!(&error, AppError::SubtaskBudgetExceeded(_));
                let message = error.to_string();
                let session = self
                    .store
                    .load_session(child_id, state.cache_policy())
                    .await?;
                (status, Some(message), budget_exceeded, session)
            }
        };
        let finished_at_ms = chrono::Utc::now().timestamp_millis();
        session.runtime.subtask.status = status;
        session.runtime.subtask.finished_at_ms = Some(finished_at_ms);
        session.runtime.subtask.error = error.clone();
        let subtask_started_at_ms = session.runtime.subtask.started_at_ms;
        let subtask_access = session.runtime.execution.access;
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
                    access: subtask_access,
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
            status,
            resumed,
            final_text: session.last_assistant_text_after(baseline_message_id),
            error,
            usage,
            model_provider_id: Some(options.model.provider_id.to_string()),
            model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
            model_id: Some(options.model.model_id.to_string()),
            budget_exceeded,
            session,
        })
    }
}

pub(in crate::session::manager) fn non_recursive_subtask_capability_denials()
-> std::collections::BTreeSet<String> {
    [
        "task",
        "tasks.run",
        "agena.tasks.run",
        "agena_tasks_run",
        "agena.tasks.create",
        "agena.tasks.followup",
        "agena.tasks.message",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::non_recursive_subtask_capability_denials;

    #[test]
    fn delegated_instances_cannot_recursively_run_tasks() {
        let names = non_recursive_subtask_capability_denials();
        for name in [
            "task",
            "tasks.run",
            "agena.tasks.run",
            "agena_tasks_run",
            "agena.tasks.create",
            "agena.tasks.followup",
            "agena.tasks.message",
        ] {
            assert!(names.contains(name));
        }
    }
}
