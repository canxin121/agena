//! Delegated runs own their child execution and lifecycle after the caller
//! returns. Parent cancellation never grants a child write authority.
use super::*;
use agena_domain::{
    ActivityPayload, CommandReferenceActivity, ComposerActivity, Role, SubtaskStatus,
};
use agena_provider::CompletionUsage;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct SessionSubtaskRequest {
    pub parent_session_id: i64,
    pub run_in_background: bool,
    pub launch_call_id: Option<i64>,
    pub description: String,
    pub prompt: String,
    pub commands: Option<Vec<String>>,
    pub task_id: Option<String>,
    pub requested_model_selection: agena_domain::ModelSelectionConfig,
    pub timeout_ms: Option<u64>,
    pub max_tokens: Option<u64>,
    pub max_cost_microusd: Option<u64>,
}
#[derive(Debug, Clone)]
pub struct SessionSubtaskResponse {
    pub session: Session,
    pub task_id: String,
    pub parent_session_id: i64,
    pub status: SubtaskStatus,
    pub resumed: bool,
    pub final_text: Option<String>,
    pub failure: Option<agena_failure::Failure>,
    pub usage: CompletionUsage,
    pub model_provider_id: Option<String>,
    pub model_adapter_id: Option<String>,
    pub model_id: Option<String>,
    pub budget_exceeded: bool,
}
#[derive(Debug, Clone)]
pub struct SessionSubtaskOutputChunk {
    pub cursor: i64,
    pub role: Role,
    pub text: String,
    pub created_at_ms: i64,
}
#[derive(Debug, Clone)]
pub struct SessionSubtaskOutput {
    pub session_id: i64,
    pub chunks: Vec<SessionSubtaskOutputChunk>,
    pub next_cursor: i64,
    pub has_more: bool,
}

struct PreparingTask {
    child: SessionId,
    parent: SessionId,
    task: String,
    started: i64,
    origin: Option<EntryId>,
    activity: Option<ActivityHandle>,
}

struct TaskSettlement {
    child: SessionId,
    parent: i64,
    task: String,
    started: i64,
    resumed: bool,
    timed_out: bool,
    control: Arc<RunControl>,
    activity: Option<ActivityHandle>,
    model: ModelRef,
    execution: ExecutionId,
    origin: Option<EntryId>,
}

impl SessionManager {
    /// The kernel interrupts provider attempts and external activities on open.
    /// Complete the corresponding task projection and reliable parent handoff.
    pub async fn reconcile_background_tasks(&self, limit: usize) -> Result<usize, AppError> {
        let mut before = None;
        let mut reconciled = 0;
        loop {
            let page = persist(|| {
                self.kernel().reader.query_summaries(SummaryQuery {
                    workspace_id: None,
                    tasks_only: true,
                    before,
                    limit: limit.clamp(1, 128),
                    ..Default::default()
                })
            })
            .await
            .map_err(kernel_error)?;
            for summary in &page.sessions {
                let configuration = persist(|| self.kernel().reader.configuration(summary.id))
                    .await
                    .map_err(kernel_error)?;
                let Some(mut task) = configuration.task.clone() else {
                    continue;
                };
                if !matches!(
                    task.status,
                    SubtaskStatus::Running | SubtaskStatus::Interrupted
                ) {
                    continue;
                }
                let active =
                    persist(|| self.kernel().reader.active_executions(summary.id, None, 1))
                        .await
                        .map_err(kernel_error)?;
                if !active.executions.is_empty() {
                    continue;
                }
                if task.status == SubtaskStatus::Running {
                    task.status = SubtaskStatus::Interrupted;
                    task.finished_at_ms = Some(
                        Utc::now()
                            .timestamp_millis()
                            .max(task.started_at_ms.unwrap_or_default()),
                    );
                    let request = request_key(
                        "task-recovery-state",
                        &format!(
                            "{}:{}",
                            summary.id.get(),
                            task.started_at_ms.unwrap_or_default()
                        ),
                    );
                    let change = SessionConfigurationChange {
                        task: Some(Some(task.clone())),
                        ..Default::default()
                    };
                    persist(|| {
                        self.kernel()
                            .sessions
                            .configure(request, summary.id, change.clone())
                    })
                    .await
                    .map_err(kernel_error)?;
                    reconciled += 1;
                }
                if let Some(parent) = configuration.parent_session_id {
                    let payload = serde_json::json!({"kind":"task","task_id":task.id,"child_session_id":summary.id.get(),"status":"interrupted","text":format!("Delegated task '{}' was interrupted before its result was delivered. Inspect the task output and use tasks.followup to resume it.",task.id)});
                    let key = format!(
                        "task-recovered:{}:{}",
                        summary.id.get(),
                        task.started_at_ms.unwrap_or_default()
                    );
                    let request = request_key("task-recovery-delivery", &key);
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
                }
            }
            if !page.has_more {
                break;
            }
            before = page.next;
        }
        Ok(reconciled)
    }

    pub async fn task_session(&self, parent: i64, task: &str) -> Result<Session, AppError> {
        let id = (*self
            .kernel()
            .reader
            .find_task(session_id(parent)?, task.trim())
            .await
            .map_err(kernel_error)?)
        .ok_or_else(|| {
            AppError::Config(format!(
                "subtask '{task}' does not exist under session {parent}"
            ))
        })?;
        self.get_session(id.get()).await
    }
    pub async fn run_subtask(
        &self,
        request: SessionSubtaskRequest,
    ) -> Result<SessionSubtaskResponse, AppError> {
        let manager = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let preparation = StdMutex::new(None);
            let result = manager.run_subtask_inner(request, &preparation).await;
            if let Err(error) = &result {
                let pending = preparation.lock().unwrap_or_else(|p| p.into_inner()).take();
                if let Some(pending) = pending
                    && let Err(settle) = manager
                        .fail_task_preparation(pending, error.failure())
                        .await
                {
                    tracing::error!(%settle,"subtask launch failure could not be settled");
                }
            }
            let _ = tx.send(result);
        });
        rx.await
            .map_err(|_| AppError::Internal("subtask preparation supervisor stopped".into()))?
    }
    async fn run_subtask_inner(
        &self,
        request: SessionSubtaskRequest,
        preparing: &StdMutex<Option<PreparingTask>>,
    ) -> Result<SessionSubtaskResponse, AppError> {
        if request.description.trim().is_empty() || request.prompt.trim().is_empty() {
            return Err(AppError::Config(
                "subtask description and prompt must not be empty".into(),
            ));
        }
        if request.timeout_ms == Some(0)
            || request.max_tokens == Some(0)
            || request.max_cost_microusd == Some(0)
        {
            return Err(AppError::Config(
                "subtask budgets and timeout must be greater than zero".into(),
            ));
        }
        if request.description.len() > 4096 || request.prompt.len() > 512 * 1024 {
            return Err(AppError::Config(
                "subtask description or prompt exceeds its byte budget".into(),
            ));
        }
        let permit = self.reserve_execution()?;
        let settlement = self
            .task_settlements
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Config("the subtask settlement queue is full".into()))?;
        let parent = self.get_session(request.parent_session_id).await?;
        if parent.is_subagent() {
            return Err(AppError::Config(
                "delegated subtasks cannot create nested subtasks".into(),
            ));
        }
        let task_id = request
            .task_id
            .as_deref()
            .map(str::trim)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("task_{}", uuid::Uuid::new_v4().simple()));
        if task_id.is_empty() || task_id.len() > 128 {
            return Err(AppError::Config(
                "subtask task_id must contain 1 through 128 bytes".into(),
            ));
        }
        let state = self.state.load_full();
        let selected = request.requested_model_selection;
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
        let references = resolve_subtask_command_references(
            &state.executor.plugin_manager().command_catalog(),
            request.commands.as_deref().unwrap_or_default(),
        )?;
        let launch = match request.launch_call_id {
            Some(call) => Some(self.live_tool(parent.id, call)?),
            None => None,
        };
        if request.run_in_background && launch.is_none() {
            return Err(AppError::Config(
                "background subtasks require the active launch tool capability".into(),
            ));
        }
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
        let preparation = self.task_preparation.lock().await;
        let existing = *self
            .kernel()
            .reader
            .find_task(session_id(parent.id)?, &task_id)
            .await
            .map_err(kernel_error)?;
        let resumed = existing.is_some();
        if let Some(id) = existing
            && self.is_run_active(id.get()).await
        {
            return Err(AppError::ExecutionAlreadyActive(id.get()));
        }
        let previous = match existing {
            Some(id) => self
                .kernel()
                .reader
                .configuration(id)
                .await
                .map_err(kernel_error)?
                .task
                .as_ref()
                .and_then(|task| task.started_at_ms),
            None => None,
        };
        let now = Utc::now()
            .timestamp_millis()
            .max(previous.unwrap_or(i64::MIN).saturating_add(1));
        let task = SessionTaskConfiguration {
            id: task_id.clone(),
            status: SubtaskStatus::Running,
            started_at_ms: Some(now),
            finished_at_ms: None,
            failure: None,
        };
        let child = match existing {
            Some(id) => {
                let change = SessionConfigurationChange {
                    task: Some(Some(task.clone())),
                    permission_ceiling: Some(parent.execution.effective_permission.clone()),
                    capability_denied_tool_names: Some(non_recursive_subtask_capability_denials()),
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
                    capability_denied_tool_names: non_recursive_subtask_capability_denials(),
                    effective_workspace_root: parent.execution.effective_workspace_root.clone(),
                    task: Some(task.clone()),
                    ..Default::default()
                };
                persist(|| {
                    self.kernel().sessions.create_configured(
                        command,
                        parent.workspace_id,
                        request.description.trim().into(),
                        configuration.clone(),
                    )
                })
                .await
                .map_err(kernel_error)?
                .value
                .id
            }
        };
        *preparing.lock().unwrap_or_else(|p| p.into_inner()) = Some(PreparingTask {
            child,
            parent: session_id(parent.id)?,
            task: task_id.clone(),
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
                let mut label = format!("{task_id}: {}", request.description.trim());
                truncate(&mut label, 1024);
                let start = BackgroundActivityStart {
                    kind: BackgroundActivityKind::Task,
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
            Some(TaskBudget {
                max_tokens: request.max_tokens,
                max_cost_microusd: request.max_cost_microusd,
            }),
        );
        preparing.lock().unwrap_or_else(|p| p.into_inner()).take();
        drop(preparation);
        let manager = self.clone();
        let task_key = task_id.clone();
        let parent_id = parent.id;
        let timeout = request.timeout_ms;
        let model = options.model.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
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
                .settle_subtask(TaskSettlement {
                    child,
                    parent: parent_id,
                    task: task_key,
                    started: now,
                    resumed,
                    timed_out,
                    control,
                    activity,
                    model,
                    execution: execution_id,
                    origin,
                })
                .await;
            let _ = tx.send(result);
        });
        if request.run_in_background {
            Ok(SessionSubtaskResponse {
                session: self.get_session(child.get()).await?,
                task_id,
                parent_session_id: parent.id,
                status: SubtaskStatus::Running,
                resumed,
                final_text: None,
                failure: None,
                usage: Default::default(),
                model_provider_id: Some(options.model.provider_id.to_string()),
                model_adapter_id: options.model.adapter_id.map(|a| a.to_string()),
                model_id: Some(options.model.model_id.to_string()),
                budget_exceeded: false,
            })
        } else {
            rx.await.map_err(|_| {
                AppError::Internal("subtask supervisor stopped before settlement".into())
            })?
        }
    }
    async fn settle_subtask(
        &self,
        settlement: TaskSettlement,
    ) -> Result<SessionSubtaskResponse, AppError> {
        let TaskSettlement {
            child,
            parent,
            task,
            started,
            resumed,
            timed_out,
            control,
            activity,
            model,
            execution,
            origin,
        } = settlement;
        let view = persist(|| self.kernel().reader.execution(child, execution))
            .await
            .map_err(kernel_error)?;
        let status = if timed_out {
            SubtaskStatus::TimedOut
        } else {
            match view.phase {
                ExecutionPhase::Succeeded => SubtaskStatus::Completed,
                ExecutionPhase::Cancelled => SubtaskStatus::Cancelled,
                ExecutionPhase::Interrupted => SubtaskStatus::Interrupted,
                _ => SubtaskStatus::Failed,
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
            timeout.user.fallback = "The subtask reached its execution time limit.".into();
            failure = Some(timeout);
        }
        let finished = Utc::now().timestamp_millis().max(started);
        let change = SessionConfigurationChange {
            task: Some(Some(SessionTaskConfiguration {
                id: task.clone(),
                status,
                started_at_ms: Some(started),
                finished_at_ms: Some(finished),
                failure: failure.clone(),
            })),
            ..Default::default()
        };
        let command = RequestId::new();
        persist(|| {
            self.kernel()
                .sessions
                .configure(command, child, change.clone())
        })
        .await
        .map_err(kernel_error)?;
        let text = self
            .final_text_for(child, Some(execution), 64 * 1024)
            .await?;
        let value = bounded_task_payload(
            serde_json::json!({"kind":"task","message_id":origin.map(EntryId::get),"task_id":task,"child_session_id":child.get(),"status":status,"text":text,"failure":failure}),
        )?;
        if let Some(activity) = activity {
            let command = RequestId::new();
            let outcome = match status {
                SubtaskStatus::Completed => BackgroundActivityOutcome::Succeeded {
                    value: value.clone(),
                },
                SubtaskStatus::Cancelled => BackgroundActivityOutcome::Stopped,
                SubtaskStatus::Interrupted => BackgroundActivityOutcome::Interrupted {
                    diagnostic: "child execution was interrupted".into(),
                },
                _ => BackgroundActivityOutcome::Failed {
                    diagnostic: bounded_diagnostic(
                        failure
                            .as_ref()
                            .map(|f| f.user.fallback.clone())
                            .unwrap_or_else(|| "child task did not complete".into()),
                    ),
                },
            };
            persist(|| activity.end(command, outcome.clone()))
                .await
                .map_err(kernel_error)?;
        }
        if origin.is_some() {
            let delivery = request_key("task-completed", &execution.to_string());
            persist(|| {
                self.kernel().executor.deliver(
                    delivery,
                    session_id(parent).expect("validated parent"),
                    format!("task:{execution}"),
                    value.clone(),
                )
            })
            .await
            .map_err(kernel_error)?;
        }
        let usage = self.execution_usage(child, Some(execution)).await?;
        Ok(SessionSubtaskResponse {
            session: self.get_session(child.get()).await?,
            task_id: task,
            parent_session_id: parent,
            status,
            resumed,
            final_text: text,
            failure,
            usage,
            model_provider_id: Some(model.provider_id.to_string()),
            model_adapter_id: model.adapter_id.map(|a| a.to_string()),
            model_id: Some(model.model_id.to_string()),
            budget_exceeded: control
                .budget_exceeded
                .load(std::sync::atomic::Ordering::Acquire),
        })
    }
    async fn fail_task_preparation(
        &self,
        pending: PreparingTask,
        failure: agena_failure::Failure,
    ) -> Result<(), AppError> {
        let change = SessionConfigurationChange {
            task: Some(Some(SessionTaskConfiguration {
                id: pending.task.clone(),
                status: SubtaskStatus::Failed,
                started_at_ms: Some(pending.started),
                finished_at_ms: Some(Utc::now().timestamp_millis().max(pending.started)),
                failure: Some(failure.clone()),
            })),
            ..Default::default()
        };
        let command = request_key(
            "task-preparation-failed",
            &format!("{}:{}", pending.child.get(), pending.started),
        );
        persist(|| {
            self.kernel()
                .sessions
                .configure(command, pending.child, change.clone())
        })
        .await
        .map_err(kernel_error)?;
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
        if let Some(origin) = pending.origin {
            let payload = bounded_task_payload(
                serde_json::json!({"kind":"task","message_id":origin.get(),"task_id":pending.task,"child_session_id":pending.child.get(),"status":SubtaskStatus::Failed,"text":format!("Subtask launch failed: {}",failure.user.fallback),"failure":failure}),
            )?;
            let command = request_key(
                "task-launch-delivery",
                &format!("{}:{}", pending.child.get(), pending.started),
            );
            persist(|| {
                self.kernel().executor.deliver(
                    command,
                    pending.parent,
                    format!("task-launch:{}:{}", pending.child.get(), pending.started),
                    payload.clone(),
                )
            })
            .await
            .map_err(kernel_error)?;
        }
        Ok(())
    }
    pub async fn cancel_subtask(&self, parent: i64, task: &str) -> Result<i64, AppError> {
        let child = self.task_session(parent, task).await?;
        self.cancel_active_execution_with_outcome(child.id).await?;
        Ok(child.id)
    }
    pub async fn message_subtask(
        &self,
        parent: i64,
        task: &str,
        message: String,
    ) -> Result<i64, AppError> {
        if message.trim().is_empty() {
            return Err(AppError::Config("subtask message must not be empty".into()));
        }
        let child = self.task_session(parent, task).await?;
        if child.subtask_status != Some(SubtaskStatus::Running) {
            return Err(AppError::PersistenceConflict(
                "the delegated task has ended; use tasks.followup to resume it".into(),
            ));
        }
        let session = session_id(child.id)?;
        let active = persist(|| self.kernel().reader.active_executions(session, None, 128))
            .await
            .map_err(kernel_error)?;
        let execution = active
            .executions
            .iter()
            .find(|execution| execution.phase == ExecutionPhase::Running)
            .or_else(|| active.executions.first())
            .ok_or_else(|| {
                AppError::PersistenceConflict(
                    "the delegated execution has ended; use tasks.followup to resume it".into(),
                )
            })?;
        let payload = serde_json::json!({"kind":"task_guidance","message_id":execution.message_id.get(),"task_id":task,"text":message,"wake":false});
        if serde_json::to_vec(&payload)?.len() > 64 * 1024 {
            return Err(AppError::Config(
                "subtask guidance exceeds its byte budget".into(),
            ));
        }
        let request = RequestId::new();
        let key = format!("task-guidance:{request}");
        persist(|| {
            self.kernel().executor.deliver_to_execution(
                request,
                session,
                execution.id,
                key.clone(),
                payload.clone(),
            )
        })
        .await
        .map_err(|error| match error {
            KernelError::Conflict(message) => AppError::PersistenceConflict(message),
            other => kernel_error(other),
        })?;
        Ok(child.id)
    }
    pub async fn read_subtask_output(
        &self,
        parent: i64,
        task: &str,
        after: i64,
        limit: u32,
    ) -> Result<SessionSubtaskOutput, AppError> {
        self.read_subtask_output_bounded(parent, task, after, limit)
            .await
    }
    pub async fn read_subtask_output_bounded(
        &self,
        parent: i64,
        task: &str,
        after: i64,
        limit: u32,
    ) -> Result<SessionSubtaskOutput, AppError> {
        let child = self.task_session(parent, task).await?;
        let id = session_id(child.id)?;
        let after = if after > 0 {
            Some(entry_id(after)?)
        } else {
            None
        };
        let page = self
            .kernel()
            .reader
            .history_after(id, after, limit.clamp(1, 200) as usize)
            .await
            .map_err(kernel_error)?;
        let mut chunks = Vec::new();
        let mut bytes = 0usize;
        let mut next_cursor = after.map_or(0, EntryId::get);
        let mut has_more = page.has_more;
        for entry in &page.entries {
            if bytes >= 128 * 1024 {
                has_more = true;
                break;
            }
            let text = self
                .entry_text(id, entry, (128 * 1024 - bytes).min(64 * 1024))
                .await?;
            next_cursor = entry.id.get();
            if let Some((role, text)) = text {
                bytes += text.len();
                chunks.push(SessionSubtaskOutputChunk {
                    cursor: entry.id.get(),
                    role,
                    text,
                    created_at_ms: entry.created_at_ms,
                });
            }
        }
        Ok(SessionSubtaskOutput {
            session_id: child.id,
            chunks,
            next_cursor,
            has_more,
        })
    }
    async fn entry_text(
        &self,
        session: SessionId,
        entry: &EntryView,
        max: usize,
    ) -> Result<Option<(Role, String)>, AppError> {
        match &entry.payload {
            EntryPayload::User { document } => {
                let mut text = String::new();
                for node in &document.0 {
                    if let ComposerNode::Text { text: value } = node {
                        text.push_str(value);
                        text.push('\n');
                    }
                }
                truncate(&mut text, max);
                Ok(Some((Role::User, text)))
            }
            EntryPayload::Text { content_id } => {
                let range = persist(|| {
                    self.kernel()
                        .reader
                        .text_range(session, *content_id, None, 1)
                })
                .await
                .map_err(kernel_error)?;
                let head = &range.stream;
                let text = persist(|| async {
                    let mut remaining = max.saturating_mul(3).saturating_add(4096);
                    super::super::kernel::input::read_text(
                        &self.kernel().reader,
                        session,
                        head,
                        &mut remaining,
                        Some(max),
                    )
                    .await
                })
                .await
                .map_err(kernel_error)?;
                Ok(Some((Role::Assistant, text)))
            }
            EntryPayload::Notice { text } => {
                let mut text = text.clone();
                truncate(&mut text, max);
                Ok(Some((Role::System, text)))
            }
            _ => Ok(None),
        }
    }
    pub async fn final_text(&self, id: i64, max: usize) -> Result<Option<String>, AppError> {
        self.final_text_for(session_id(id)?, None, max).await
    }
    pub(super) async fn final_text_for(
        &self,
        session: SessionId,
        execution: Option<ExecutionId>,
        max: usize,
    ) -> Result<Option<String>, AppError> {
        let mut before = None;
        for _ in 0..32 {
            let page = persist(|| self.kernel().reader.history(session, before, 128))
                .await
                .map_err(kernel_error)?;
            // Pages select the newest window but expose it chronologically.
            // A continued execution can have several completed model rounds;
            // its final response is the newest text, not the first round.
            for entry in page.entries.iter().rev() {
                if execution.is_none_or(|id| entry.execution_id == id)
                    && matches!(entry.payload, EntryPayload::Text { .. })
                    && let Some((_, text)) = self
                        .entry_text(session, entry, max.min(1024 * 1024))
                        .await?
                    && !text.trim().is_empty()
                {
                    return Ok(Some(text));
                }
            }
            if !page.has_more {
                return Ok(None);
            }
            before = page.next;
        }
        Ok(None)
    }
    pub async fn aggregate_usage(&self, id: i64) -> Result<CompletionUsage, AppError> {
        self.execution_usage(session_id(id)?, None).await
    }
    async fn execution_usage(
        &self,
        session: SessionId,
        execution: Option<ExecutionId>,
    ) -> Result<CompletionUsage, AppError> {
        let mut after = None;
        let mut usage = CompletionUsage::default();
        loop {
            let page = persist(|| self.kernel().reader.rounds(session, after, 200))
                .await
                .map_err(kernel_error)?;
            if page.is_empty() {
                break;
            }
            for round in page.iter() {
                if execution.is_none_or(|id| round.execution_id == id)
                    && let Some(state) = &round.provider_state
                {
                    let state: super::super::kernel::KernelProviderState =
                        serde_json::from_value(state.clone())?;
                    if let Some(observed) = state.usage {
                        usage.add_assign(&observed);
                    }
                }
            }
            after = page.last().map(|r| r.id);
        }
        Ok(usage)
    }
}
fn truncate(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

fn bounded_diagnostic(mut text: String) -> String {
    truncate(&mut text, 4096);
    text
}

/// Both the activity outcome and the reliable delivery must fit 64 KiB after
/// JSON escaping. Full output and failure remain available on the child.
fn bounded_task_payload(mut value: serde_json::Value) -> Result<serde_json::Value, AppError> {
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
                "subtask delivery envelope exceeds its byte budget".into(),
            ));
        }
    }
    Ok(value)
}

#[cfg(test)]
#[test]
fn task_output_delivery_fits_activity_limit_after_json_escaping() {
    let original = "\u{0000}\"\\界".repeat(8000);
    let value = bounded_task_payload(serde_json::json!({
        "kind": "task", "task_id": "task", "child_session_id": 2,
        "status": "completed", "text": original, "failure": null,
    }))
    .unwrap();
    assert_eq!(value["text_truncated"], true);
    let preview = value["text"].as_str().unwrap();
    assert!(!preview.is_empty());
    assert!(original.starts_with(preview));
    assert!(serde_json::to_vec(&value).unwrap().len() < 64 * 1024);
    assert!(
        serde_json::to_vec(&BackgroundActivityOutcome::Succeeded { value })
            .unwrap()
            .len()
            < 64 * 1024
    );
}
fn non_recursive_subtask_capability_denials() -> std::collections::BTreeSet<String> {
    [
        "task",
        "tasks.run",
        "tasks.followup",
        "tasks.message",
        "agena.tasks.run",
        "agena.tasks.followup",
        "agena.tasks.message",
        "agena_tasks_run",
        "agena_tasks_followup",
        "agena_tasks_message",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
fn resolve_subtask_command_references(
    catalog: &[agena_plugin_host::CommandCatalogItem],
    requested: &[String],
) -> Result<Vec<CommandReferenceActivity>, AppError> {
    if requested.len() > 32 {
        return Err(AppError::Config(
            "subtask command references exceed their limit".into(),
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
                .ok_or_else(|| AppError::Config(format!("unknown command '{name}' for subtask")))?;
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
