//! Single-instance durable task controller. Models run in existing supervised
//! sessions, never inside the dispatcher. One worker is admitted at a time
//! until task-specific worktree/isolation policies are available.
use std::{sync::Arc, time::Duration};

use agena_storage::kernel::{ExecutionPhase, SessionId};
use agena_superagent::{
    ChatCommand, ChatMessage, ChatReply, CreateTask, Error, Result, ResumeTask, SuperagentService,
    Task, TaskQuery, TaskRepository, TaskState,
};
use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

mod channel_access;
mod companion;
mod conversation;
mod integrations;

use super::builder::WeakAgenaRuntime;
use crate::session::SessionManager;

pub(super) struct SuperagentRuntime {
    runtime: WeakAgenaRuntime,
    repository: Arc<dyn TaskRepository>,
    companion: Arc<dyn agena_superagent::CompanionRepository>,
    companion_gate: Mutex<()>,
    integrations: Arc<dyn agena_superagent::IntegrationRepository>,
    integration_gate: Mutex<()>,
    /// Serializes only admission/control, not model execution. A cancellation
    /// cannot race between the final launch check and durable message submit.
    gate: Mutex<()>,
    wake: Notify,
}

impl SuperagentRuntime {
    pub(super) fn new(
        runtime: WeakAgenaRuntime,
        repository: Arc<dyn TaskRepository>,
        companion: Arc<dyn agena_superagent::CompanionRepository>,
        integrations: Arc<dyn agena_superagent::IntegrationRepository>,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            repository,
            companion,
            companion_gate: Mutex::new(()),
            integrations,
            integration_gate: Mutex::new(()),
            gate: Mutex::new(()),
            wake: Notify::new(),
        })
    }

    pub(super) fn start(self: &Arc<Self>, control: Arc<crate::TaskControl>) {
        self.install_channel_verifier();
        self.start_integrations(control.clone());
        let service = self.clone();
        control.spawn(async move {
            loop {
                if let Err(error) = service.tick().await {
                    tracing::warn!(%error, "superagent reconciliation deferred; durable state retained");
                }
                if let Err(error) = service.deliver_work_notices().await {
                    tracing::warn!(%error, "conversation work reports remain queued for recovery");
                }
                if let Err(error) = service.deliver_companion_reminders().await {
                    tracing::warn!(%error, "assistant reminder processing deferred");
                }
                tokio::select! {
                    _ = service.wake.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
        });
    }

    fn manager(&self) -> Result<Arc<SessionManager>> {
        let runtime = self
            .runtime
            .upgrade()
            .ok_or(Error::Unavailable("runtime is unavailable"))?;
        if runtime.is_shutdown() {
            return Err(Error::Unavailable("runtime is stopping"));
        }
        runtime
            .session_manager()
            .ok_or(Error::Unavailable("session runtime is unavailable"))
    }

    async fn save(&self, mut task: Task) -> Result<Task> {
        let expected = task.revision;
        task.revision = expected
            .checked_add(1)
            .ok_or(Error::Conflict("task revision exhausted"))?;
        task.updated_at_ms = Utc::now().timestamp_millis().max(task.updated_at_ms);
        self.repository.replace(task, expected).await
    }

    async fn tick(&self) -> Result<()> {
        let _gate = self.gate.lock().await;
        let manager = self.manager()?;
        let tasks = self.repository.pending(128).await?;
        let mut busy = false;
        for task in tasks.iter().filter(|task| task.state.is_active()) {
            if !self.automation_valid(task).await? {
                let mut stopped = task.clone();
                stopped.state = TaskState::Cancelling;
                stopped.note = Some(
                    "Automation grant was paused, revoked, expired or exceeded its task deadline."
                        .into(),
                );
                let stopped = self.save(stopped).await?;
                self.finish_cancel(&manager, stopped).await?;
                continue;
            }
            let current = match task.state {
                TaskState::Starting => self.launch(&manager, task.clone()).await?,
                TaskState::Cancelling => self.finish_cancel(&manager, task.clone()).await?,
                _ => self.reconcile(&manager, task.clone()).await?,
            };
            busy |= current.state.is_active();
        }
        if !busy
            && let Some(task) = tasks
                .into_iter()
                .find(|task| task.state == TaskState::Queued)
        {
            let mut task = task;
            if !self.automation_valid(&task).await? {
                task.state = TaskState::Cancelled;
                task.note =
                    Some("Automation authorization no longer permits this queued task.".into());
                self.save(task).await?;
                return Ok(());
            }
            task.state = TaskState::Starting;
            let task = self.save(task).await?;
            self.launch(&manager, task).await?;
        }
        Ok(())
    }

    async fn launch(&self, manager: &SessionManager, mut task: Task) -> Result<Task> {
        // A deterministic create command closes the crash window between
        // creating the session and attaching its identity to the task.
        if task.worker_session_id.is_none() {
            match manager
                .ensure_task_worker(task.source_session_id, &task.id.to_string())
                .await
            {
                Ok(id) => {
                    task.worker_session_id = Some(id);
                    task = self.save(task).await?;
                }
                Err(error) => {
                    tracing::warn!(task_id=%task.id, %error, "task worker preparation failed");
                    task.state = TaskState::Failed;
                    task.note = Some("Worker preparation failed. Check the source session, model and permissions, then explicitly resume.".into());
                    return self.save(task).await;
                }
            }
        }
        let id = task
            .worker_session_id
            .ok_or(Error::Conflict("worker session is missing"))?;
        manager
            .set_message_queue_paused(id, false)
            .await
            .map_err(runtime_error)?;
        let text = if task.attempt == 1 {
            format!(
                "Work on this explicitly requested task. Keep the existing permission boundaries. Report results and verification evidence; do not claim human acceptance.\n\n{}",
                task.instruction
            )
        } else {
            format!(
                "Continue the same task using this explicit feedback. First inspect previous execution and external results; do not blindly repeat operations after interruption.\n\n{}",
                task.instruction
            )
        };
        match manager
            .submit_managed_message(id, task.delivery_key(), text)
            .await
        {
            Ok(outcome) => {
                let receipt = outcome
                    .receipt
                    .ok_or(Error::Unavailable("worker submission has no receipt"))?;
                task.execution_id = Some(receipt.execution_id.to_string());
                task.attempt_generation = Some(receipt.attempt_generation.get());
                task.state = TaskState::Running;
                task.note = None;
                // A crash/lost save here replays the SAME submit key, never
                // inventing another execution. Terminal attempts are read back.
                self.save(task).await
            }
            Err(error) => {
                tracing::warn!(task_id=%task.id, %error, "task submission needs explicit recovery");
                task.state = TaskState::Interrupted;
                task.note = Some("Submission did not return a confirmed receipt. Inspect the worker before explicitly resuming; external work may already exist.".into());
                self.save(task).await
            }
        }
    }

    async fn reconcile(&self, manager: &SessionManager, mut task: Task) -> Result<Task> {
        let Some(worker) = task.worker_session_id else {
            return Err(Error::Conflict("active task has no worker"));
        };
        let Some(execution) = task.execution_id.as_deref() else {
            return Err(Error::Conflict("active task has no execution receipt"));
        };
        let session =
            SessionId::try_from(worker).map_err(|_| Error::Conflict("invalid worker identity"))?;
        let execution = execution
            .parse()
            .map_err(|_| Error::Conflict("invalid execution identity"))?;
        let view = match manager
            .session_kernel()
            .reader
            .execution(session, execution)
            .await
        {
            Ok(view) => view,
            Err(agena_storage::kernel::KernelError::NotFound(_)) => {
                task.state = TaskState::Interrupted;
                task.note = Some(
                    "Worker execution is no longer available. Inspect the worker before recovery."
                        .into(),
                );
                return self.save(task).await;
            }
            Err(error) => return Err(runtime_error(error)),
        };
        if view.phase == ExecutionPhase::Succeeded
            && task.attempt_generation == Some(view.generation.get())
            && task.advance_conversation()?
        {
            self.wake.notify_one();
            return self.save(task).await;
        }
        let previous_attention = task
            .conversation
            .as_ref()
            .and_then(|work| work.attention_id.clone());
        let (state, note) = if task.attempt_generation != Some(view.generation.get()) {
            (TaskState::Interrupted, Some("The worker was advanced outside the task controller. Inspect its current execution before recovery.".to_owned()))
        } else {
            match view.phase {
                ExecutionPhase::Succeeded => (
                    if task.conversation.as_ref().is_some_and(|work| !work.require_review) { TaskState::Completed } else { TaskState::Review }, None),
                ExecutionPhase::Failed => (TaskState::Failed, Some("Execution failed; inspect the worker transcript.".into())),
                ExecutionPhase::Interrupted => (TaskState::Interrupted, Some("Execution was interrupted. Review existing effects before explicitly resuming.".into())),
                ExecutionPhase::Cancelled => (TaskState::Cancelled, None),
                ExecutionPhase::Running | ExecutionPhase::Queued => {
                    let worker = manager.get_session(worker).await.map_err(runtime_error)?;
                    let paused = manager.session_kernel().reader.configuration(session).await.map_err(runtime_error)?.message_queue_paused;
                    if worker.blocked() || paused {
                        (TaskState::Waiting, Some("Reply to the pending interaction or explicitly resume the worker queue.".into()))
                    } else {
                        (TaskState::Running, None)
                    }
                }
            }
        };
        if state == TaskState::Waiting
            && let Some(work) = &mut task.conversation
        {
            let questions = manager
                .conversation_work_questions(worker)
                .await
                .map_err(runtime_error)?;
            work.attention_id = questions
                .first()
                .map(|question| question.id.get().to_string())
                .or_else(|| Some("queue-paused".into()));
        }
        let attention_changed = task
            .conversation
            .as_ref()
            .and_then(|work| work.attention_id.clone())
            != previous_attention;
        if task.state != state || task.note != note || attention_changed {
            task.state = state;
            task.note = note;
            return self.save(task).await;
        }
        Ok(task)
    }

    async fn finish_cancel(&self, manager: &SessionManager, mut task: Task) -> Result<Task> {
        if let Some(worker) = task.worker_session_id {
            manager
                .cancel_active_execution_with_outcome(worker)
                .await
                .map_err(runtime_error)?;
            // Cancellation fences claims immediately, but physical execution
            // cleanup can still be settling. Do not report that as completed.
            if manager.is_run_active(worker).await {
                return Ok(task);
            }
            // A cancelled parent can settle before its delegated children.
            // Keep the task slot until their supervised effects have exited.
            // Nested subagents remain prohibited by the existing runtime.
            let session = SessionId::try_from(worker)
                .map_err(|_| Error::Conflict("invalid worker identity"))?;
            let children = manager
                .session_kernel()
                .reader
                .query_summaries(agena_storage::kernel::SummaryQuery {
                    parent_id: Some(session),
                    agents_only: true,
                    limit: 128,
                    ..Default::default()
                })
                .await
                .map_err(runtime_error)?;
            for child in &children.sessions {
                if manager.is_run_active(child.id.get()).await {
                    return Ok(task);
                }
            }
        }
        task.state = TaskState::Cancelled;
        task.note = None;
        self.save(task).await
    }

    async fn scoped_task(&self, id: Uuid, source: i64) -> Result<Task> {
        let task = self.repository.get(id).await?;
        if task.source_session_id != source {
            return Err(Error::NotFound);
        }
        Ok(task)
    }
}

fn runtime_error(error: impl std::fmt::Display) -> Error {
    tracing::warn!(diagnostic=%error, "superagent runtime operation failed");
    Error::Unavailable("runtime operation could not be completed; inspect server diagnostics")
}

#[async_trait]
impl SuperagentService for SuperagentRuntime {
    async fn update_channel_policy(
        &self,
        source: i64,
        id: Uuid,
        r: agena_superagent::ChannelPolicyUpdate,
    ) -> Result<agena_superagent::IntegrationBinding> {
        self.update_channel_access(source, id, r).await
    }

    async fn integrations(&self, source: i64) -> Result<agena_superagent::IntegrationsSnapshot> {
        self.integration_snapshot(source).await
    }
    async fn create_integration(
        &self,
        source: i64,
        request: agena_superagent::CreateIntegration,
    ) -> Result<agena_superagent::IntegrationBinding> {
        self.add_integration(source, request).await
    }
    async fn integration_detail(
        &self,
        source: i64,
        id: Uuid,
    ) -> Result<agena_superagent::IntegrationDetail> {
        self.read_integration(source, id).await
    }
    async fn control_integration(
        &self,
        source: i64,
        id: Uuid,
        request: agena_superagent::IntegrationControl,
    ) -> Result<agena_superagent::IntegrationBinding> {
        self.change_integration(source, id, request).await
    }

    async fn assistant(&self, source: i64) -> Result<agena_superagent::AssistantSnapshot> {
        self.assistant_snapshot(source).await
    }
    async fn update_assistant(
        &self,
        source: i64,
        update: agena_superagent::AssistantUpdate,
    ) -> Result<agena_superagent::AssistantProfile> {
        self.change_assistant_profile(source, update).await
    }
    async fn control_reminder(
        &self,
        source: i64,
        id: Uuid,
        control: agena_superagent::ReminderControl,
    ) -> Result<agena_superagent::Reminder> {
        self.change_reminder(source, id, control).await
    }
    async fn change_assistant_memory(
        &self,
        source: i64,
        update: agena_superagent::AssistantMemoryUpdate,
    ) -> Result<Option<agena_superagent::AssistantMemory>> {
        self.edit_assistant_memory(source, update).await
    }

    async fn create(&self, request: CreateTask) -> Result<Task> {
        request.validate()?;
        let source = self
            .manager()?
            .get_session(request.source_session_id)
            .await
            .map_err(runtime_error)?;
        if source.is_subagent() {
            return Err(Error::Invalid(
                "subagents cannot authorize durable task workers",
            ));
        }
        let task = Task::new(request, source.workspace_id, Utc::now().timestamp_millis())?;
        let task = self.repository.insert(task).await?;
        self.wake.notify_one();
        Ok(task)
    }

    async fn get(&self, id: Uuid) -> Result<Task> {
        self.repository.get(id).await
    }

    async fn list(&self, query: TaskQuery) -> Result<Vec<Task>> {
        self.repository.list(query).await
    }

    async fn cancel(&self, id: Uuid, expected_revision: Option<i64>) -> Result<Task> {
        let _gate = self.gate.lock().await;
        let mut task = self.repository.get(id).await?;
        if matches!(task.state, TaskState::Completed | TaskState::Cancelled) {
            return Ok(task);
        }
        if task.state == TaskState::Cancelling {
            return Ok(task);
        }
        if expected_revision.is_some_and(|revision| revision != task.revision) {
            return Err(Error::Conflict("task changed; refresh before cancelling"));
        }
        task.state = TaskState::Cancelling;
        task = self.save(task).await?;
        self.wake.notify_one();
        self.finish_cancel(self.manager()?.as_ref(), task).await
    }

    async fn resume(&self, id: Uuid, request: ResumeTask) -> Result<Task> {
        let _gate = self.gate.lock().await;
        let mut task = self.repository.get(id).await?;
        if !task.apply_resume(&request)? {
            return Ok(task);
        }
        if let Some(worker) = task.worker_session_id {
            let manager = self.manager()?;
            let session = SessionId::try_from(worker)
                .map_err(|_| Error::Conflict("invalid worker identity"))?;
            let active = manager
                .session_kernel()
                .reader
                .active_executions(session, None, 1)
                .await
                .map_err(runtime_error)?;
            if manager.is_run_active(worker).await || !active.executions.is_empty() {
                return Err(Error::Conflict(
                    "worker is still active; inspect or cancel it before resuming",
                ));
            }
        }
        let task = self.save(task).await?;
        self.wake.notify_one();
        Ok(task)
    }

    async fn accept(&self, id: Uuid, expected_revision: i64) -> Result<Task> {
        let _gate = self.gate.lock().await;
        let mut task = self.repository.get(id).await?;
        if task.state == TaskState::Completed {
            return Ok(task);
        }
        if task.state != TaskState::Review || task.revision != expected_revision {
            return Err(Error::Conflict(
                "only the current reviewed result can be accepted",
            ));
        }
        task.state = TaskState::Completed;
        self.save(task).await
    }

    async fn chat(&self, message: ChatMessage) -> Result<ChatReply> {
        message.validate()?;
        // Verify the conversation exists even for list/control-only messages.
        self.manager()?
            .get_session(message.session_id)
            .await
            .map_err(runtime_error)?;
        let command = ChatCommand::parse(&message.text)?;
        if let Some(reply) = self.repository.reserve_chat(&message).await? {
            return Ok(reply);
        }
        let reply: Result<ChatReply> = match command {
            ChatCommand::Create(goal) => {
                let task = self
                    .create(CreateTask {
                        source_session_id: message.session_id,
                        goal,
                        request_key: message.message_id.clone(),
                    })
                    .await?;
                Ok(ChatReply::Task {
                    task: Box::new(task),
                })
            }
            ChatCommand::List => Ok(ChatReply::Tasks {
                items: self
                    .list(TaskQuery {
                        source_session_id: Some(message.session_id),
                        limit: Some(20),
                        ..Default::default()
                    })
                    .await?,
            }),
            ChatCommand::Status(id) => Ok(ChatReply::Task {
                task: Box::new(self.scoped_task(id, message.session_id).await?),
            }),
            ChatCommand::Cancel(id) => {
                self.scoped_task(id, message.session_id).await?;
                Ok(ChatReply::Task {
                    task: Box::new(self.cancel(id, None).await?),
                })
            }
            ChatCommand::Conversation => {
                let receipt = self
                    .manager()?
                    .submit_managed_message(
                        message.session_id,
                        format!("chatbot:{}", message.message_id),
                        message.text.clone(),
                    )
                    .await
                    .map_err(runtime_error)?
                    .receipt
                    .ok_or(Error::Unavailable("chat submission has no receipt"))?;
                Ok(ChatReply::Conversation {
                    session_id: message.session_id,
                    execution_id: receipt.execution_id.to_string(),
                    attempt_generation: receipt.attempt_generation.get(),
                })
            }
        };
        self.repository.finish_chat(&message, reply?).await
    }
}
