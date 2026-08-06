use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::message::{TaskAccess, TaskToolInput};
use crate::plugins::provided::workflow::{WorkflowPlugin, WorkflowPluginConfig};
use agena_macros::ToolInput;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::host_api::{
    CancelSubtaskRequest, HostCallbackContext, HostStorageGetRequest, HostStorageListRequest,
    HostStorageScope, HostStorageSetRequest, MessageSubtaskRequest, ReadSubtaskOutputRequest,
    RunSubtaskAccess, RunSubtaskModelSelection, RunSubtaskRequest, RunSubtaskResponse,
    RunSubtaskStatus, run_in_host_callback_context,
};
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, Result as SdkResult, SessionEndInput, ToolInvokeContext,
    ToolInvokeOutput,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

pub(crate) const TASKS_PLUGIN_ID: &str = "agena.tasks";
const TASK_STORAGE_NAMESPACE: &str = "async_tasks.v1";
/// Prevent one parent session from filling the runtime with unbounded child
/// executions. This is deliberately a per-parent admission boundary; global
/// provider capacity remains owned by the runtime/provider layer.
const MAX_ACTIVE_TASKS_PER_PARENT: usize = 4;

pub(crate) struct TasksPlugin {
    inner: WorkflowPlugin,
    tasks: Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>,
}

#[derive(Debug)]
struct AsyncTaskEntry {
    state: Mutex<AsyncTaskState>,
    notify: Arc<Notify>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AsyncTaskState {
    task_id: String,
    parent_session_id: i64,
    description: String,
    /// Original instruction is retained only in session-private storage so a
    /// user can make an explicit post-restart recovery decision. It is never
    /// replayed automatically, because the child session may already contain
    /// that user message when a process died before acknowledging completion.
    prompt: String,
    access: TaskAccess,
    status: String,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    response: Option<RunSubtaskResponse>,
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection: Option<RunSubtaskModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_cost_microusd: Option<u64>,
    #[serde(default)]
    budget_exceeded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput, Default)]
#[serde(deny_unknown_fields)]
struct TaskListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("task_id"), non_empty("task_id"))]
#[serde(deny_unknown_fields)]
struct TaskIdInput {
    task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("task_id", "message"), non_empty("task_id", "message"))]
#[serde(deny_unknown_fields)]
struct TaskMessageInput {
    task_id: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("task_id", "prompt"),
    non_empty("task_id", "prompt"),
    minimum("timeout_ms", 1),
    minimum("max_tokens", 1),
    minimum("max_cost_microusd", 1)
)]
#[serde(deny_unknown_fields)]
struct TaskFollowupInput {
    task_id: String,
    prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_cost_microusd: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("task_id"),
    non_empty("task_id"),
    minimum("cursor", 0),
    minimum("limit", 1),
    maximum("limit", 500)
)]
#[serde(deny_unknown_fields)]
struct TaskOutputInput {
    task_id: String,
    #[serde(default)]
    cursor: i64,
    #[serde(default = "default_output_limit")]
    limit: u32,
}

const fn default_output_limit() -> u32 {
    100
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum TaskWaitMode {
    Any,
    #[default]
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("task_ids[]"),
    non_empty("task_ids[]"),
    min_items("task_ids", 1),
    max_items("task_ids", 64),
    maximum("timeout_ms", 60000)
)]
#[serde(deny_unknown_fields)]
struct TaskWaitInput {
    task_ids: Vec<String>,
    #[serde(default)]
    mode: TaskWaitMode,
    #[serde(default = "default_wait_timeout_ms")]
    timeout_ms: u64,
}

const fn default_wait_timeout_ms() -> u64 {
    30_000
}

fn run_subtask_access(access: TaskAccess) -> RunSubtaskAccess {
    match access {
        TaskAccess::Inherit => RunSubtaskAccess::Inherit,
        TaskAccess::ReadOnly => RunSubtaskAccess::ReadOnly,
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "tasks",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Delegated subtask orchestration tools.",
    display = brief_detailed
)]
impl TasksPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
            tasks: Mutex::new(BTreeMap::new()),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(subtask, execute),
        summary = "Create or resume a delegated subagent task. Attach Skill names in `skills` so the child session applies them as task guidance.",
        help = "Use when the work is genuinely parallel, independent, or read-heavy across many files, or when a matching subagent type exists. Do small tasks yourself instead of delegating; do not fan out a single task into many subtasks; verify inline instead of delegating when you can; do not redo work you already delegated. Delegates a bounded task to a subagent session. Set `skills` to Skill names or aliases (for example a read-only review skill for a review task, or an explore skill for an exploration task); the child session receives the resolved Skill instructions and should follow them. Unknown Skill names are rejected before the subtask starts. Use `agena.skills.list` to discover available Skills.",
        task,
        subtask,
        display = detailed,

    )]
    async fn run(&self, input: &TaskToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_task(input).await
    }

    #[tool(
        tags(subtask, mutate),
        summary = "Create a delegated subagent task in the background. Attach Skill names in `skills` so the child session applies them as task guidance.",
        help = "Creates a delegated background task. Set `skills` to Skill names or aliases (for example a read-only review skill for a review task, or an explore skill for an exploration task); the child session receives the resolved Skill instructions and should follow them. Unknown Skill names are rejected before the subtask starts. Use `agena.skills.list` to discover available Skills.",
        task,
        subtask,
        display = detailed,

    )]
    async fn create(
        &self,
        input: &TaskToolInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        ensure_task_capacity(&self.tasks, context.session_id, None)?;
        let task_id = input
            .task_id
            .clone()
            .unwrap_or_else(|| format!("task_{}", uuid::Uuid::new_v4().simple()));
        let selection = input
            .selection
            .as_ref()
            .map(|selection| RunSubtaskModelSelection {
                provider: selection.provider.clone(),
                adapter: selection.adapter.clone(),
                model: selection.model.clone(),
                thinking_mode: selection.thinking_mode.clone(),
                speed_mode: selection.speed_mode.clone(),
                verbosity: selection.verbosity.clone(),
                parallel_tool_calls: selection.parallel_tool_calls,
            });
        let entry = Arc::new(AsyncTaskEntry {
            state: Mutex::new(AsyncTaskState {
                task_id: task_id.clone(),
                parent_session_id: context.session_id,
                description: input.description.clone(),
                prompt: input.prompt.clone(),
                access: input.access,
                status: "running".to_string(),
                started_at_ms: chrono::Utc::now().timestamp_millis(),
                finished_at_ms: None,
                response: None,
                error: None,
                selection: selection.clone(),
                timeout_ms: input.timeout_ms,
                max_tokens: input.max_tokens,
                max_cost_microusd: input.max_cost_microusd,
                budget_exceeded: false,
            }),
            notify: Arc::new(Notify::new()),
        });
        {
            let mut tasks = self.tasks.lock().map_err(|_| {
                agena_plugin_host::PluginError::internal("tasks registry lock poisoned")
            })?;
            if tasks.get(task_id.as_str()).is_some_and(|existing| {
                !is_terminal(&existing.state.lock().expect("task state").status)
            }) {
                return Err(agena_plugin_host::PluginError::invalid_params(format!(
                    "task '{task_id}' is already running"
                )));
            }
            tasks.insert(task_id.clone(), Arc::clone(&entry));
        }
        let host = self.inner.host()?;
        let request = RunSubtaskRequest {
            parent_session_id: Some(context.session_id),
            access: run_subtask_access(input.access),
            description: input.description.clone(),
            prompt: input.prompt.clone(),
            skills: input.skills.clone(),
            task_id: Some(task_id.clone()),
            selection,
            timeout_ms: input.timeout_ms,
            max_tokens: input.max_tokens,
            max_cost_microusd: input.max_cost_microusd,
        };
        persist_task_state(
            &host,
            callback_context(context),
            &entry_state(&self.tasks, &task_id)?,
        )
        .await?;
        spawn_task(host, Arc::clone(&entry), request, callback_context(context));
        Ok(task_output(
            "Start task",
            format!(
                "Started delegated task '{task_id}' in the background. Use tasks.get, tasks.output, or tasks.wait to inspect it."
            ),
            vec![entry_state(&self.tasks, task_id.as_str())?],
            false,
        ))
    }

    #[tool(
        tags(subtask, query, discovery),
        summary = "List delegated background tasks.",
        read_only,
        task,
        display = detailed,
        concurrency_safe,

    )]
    async fn list(
        &self,
        input: &TaskListInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let states = self
            .tasks
            .lock()
            .map_err(|_| agena_plugin_host::PluginError::internal("tasks registry lock poisoned"))?
            .values()
            .filter_map(|entry| entry.state.lock().ok().map(|state| state.clone()))
            .filter(|state| state.parent_session_id == context.session_id)
            .filter(|state| {
                input
                    .status
                    .as_ref()
                    .is_none_or(|status| state.status.eq_ignore_ascii_case(status.trim()))
            })
            .collect::<Vec<_>>();
        Ok(task_output(
            "List tasks",
            format!("{} delegated task(s).", states.len()),
            states,
            false,
        ))
    }

    #[tool(
        tags(subtask, query),
        summary = "Get delegated task metadata and terminal result.",
        read_only,
        task,
        display = detailed,
        concurrency_safe,

    )]
    async fn get(
        &self,
        input: &TaskIdInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let state =
            entry_state_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        Ok(task_output(
            "Task details",
            format!("Task '{}' is {}.", input.task_id, state.status),
            vec![state],
            false,
        ))
    }

    #[tool(
        tags(subtask, query),
        summary = "Read incremental delegated-task transcript output after a cursor.",
        read_only,
        task,
        display = detailed,
        concurrency_safe,

    )]
    async fn output(
        &self,
        input: &TaskOutputInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let state =
            entry_state_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        let output = self
            .inner
            .host()?
            .read_subtask_output(ReadSubtaskOutputRequest {
                parent_session_id: Some(state.parent_session_id),
                task_id: input.task_id.clone(),
                after_cursor: input.cursor,
                limit: input.limit,
            })
            .await?;
        let text = if output.chunks.is_empty() {
            state.error.clone().unwrap_or_else(|| {
                format!(
                    "No new task output after cursor {} (status: {}).",
                    input.cursor, state.status
                )
            })
        } else {
            output
                .chunks
                .iter()
                .map(|chunk| format!("[{}] {}", chunk.role, chunk.text))
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        Ok(ToolInvokeOutput::from_parts(
            "Task output",
            if output.has_more {
                format!(
                    "{} chunks · {} · more available",
                    output.chunks.len(),
                    state.status
                )
            } else {
                format!("{} chunks · {}", output.chunks.len(), state.status)
            },
            text,
            Some(serde_json::json!({
                "task": state,
                "chunks": output.chunks,
                "next_cursor": output.next_cursor,
                "has_more": output.has_more,
            })),
            BTreeMap::from([
                ("next_cursor".to_string(), output.next_cursor.to_string()),
                ("has_more".to_string(), output.has_more.to_string()),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        tags(subtask, mutate),
        summary = "Cancel a running delegated task and its child execution.",
        task,
        subtask,
        display = detailed,

    )]
    async fn cancel(
        &self,
        input: &TaskIdInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let entry = task_entry_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        let parent_session_id = lock_state(&entry)?.parent_session_id;
        let response = self
            .inner
            .host()?
            .cancel_subtask(CancelSubtaskRequest {
                parent_session_id: Some(parent_session_id),
                task_id: input.task_id.clone(),
            })
            .await?;
        {
            let mut state = lock_state(&entry)?;
            if !is_terminal(&state.status) {
                state.status = "cancelling".to_string();
            }
        }
        entry.notify.notify_waiters();
        entry.notify.notify_one();
        let state =
            entry_state_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        persist_task_state(&self.inner.host()?, callback_context(context), &state).await?;
        Ok(task_output(
            "Cancel task",
            format!("Cancellation requested for task '{}'.", input.task_id),
            vec![entry_state_for_parent(
                &self.tasks,
                input.task_id.as_str(),
                context.session_id,
            )?],
            !response.accepted,
        ))
    }

    #[tool(
        tags(subtask, mutate),
        summary = "Send additional guidance to a running delegated task.",
        task,
        subtask,
        display = detailed,

    )]
    async fn message(
        &self,
        input: &TaskMessageInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let state =
            entry_state_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        if is_terminal(&state.status) {
            return Err(agena_plugin_host::PluginError::invalid_params(format!(
                "task '{}' is terminal; use tasks.followup to resume it",
                input.task_id
            )));
        }
        self.inner
            .host()?
            .message_subtask(MessageSubtaskRequest {
                parent_session_id: Some(state.parent_session_id),
                task_id: input.task_id.clone(),
                message: input.message.clone(),
            })
            .await?;
        Ok(task_output(
            "Send task message",
            format!("Guidance delivered to task '{}'.", input.task_id),
            vec![state],
            false,
        ))
    }

    #[tool(
        tags(subtask, mutate),
        summary = "Resume a terminal delegated task with a follow-up prompt.",
        task,
        subtask,
        display = detailed,

    )]
    async fn followup(
        &self,
        input: &TaskFollowupInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let entry = task_entry_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?;
        ensure_task_capacity(
            &self.tasks,
            context.session_id,
            Some(input.task_id.as_str()),
        )?;
        let request = {
            let mut state = lock_state(&entry)?;
            if !is_terminal(&state.status) {
                return Err(agena_plugin_host::PluginError::invalid_params(format!(
                    "task '{}' is {}; use tasks.message while it is running",
                    input.task_id, state.status
                )));
            }
            state.status = "running".to_string();
            state.started_at_ms = chrono::Utc::now().timestamp_millis();
            state.finished_at_ms = None;
            state.response = None;
            state.error = None;
            state.budget_exceeded = false;
            state.max_tokens = input.max_tokens.or(state.max_tokens);
            state.max_cost_microusd = input.max_cost_microusd.or(state.max_cost_microusd);
            RunSubtaskRequest {
                parent_session_id: Some(state.parent_session_id),
                access: run_subtask_access(state.access),
                description: state.description.clone(),
                prompt: input.prompt.clone(),
                // Resuming an existing subtask keeps the Skill references
                // already attached to its first user message; do not re-inject.
                skills: None,
                task_id: Some(state.task_id.clone()),
                selection: state.selection.clone(),
                timeout_ms: input.timeout_ms.or(state.timeout_ms),
                max_tokens: state.max_tokens,
                max_cost_microusd: state.max_cost_microusd,
            }
        };
        let host = self.inner.host()?;
        persist_task_state(
            &host,
            callback_context(context),
            &entry_state_for_parent(&self.tasks, input.task_id.as_str(), context.session_id)?,
        )
        .await?;
        spawn_task(host, Arc::clone(&entry), request, callback_context(context));
        Ok(task_output(
            "Follow up task",
            format!("Resumed task '{}' with a follow-up prompt.", input.task_id),
            vec![entry_state_for_parent(
                &self.tasks,
                input.task_id.as_str(),
                context.session_id,
            )?],
            false,
        ))
    }

    #[tool(
        tags(subtask, query),
        summary = "Wait for any or all delegated tasks to finish.",
        read_only,
        task,
        display = detailed,
        concurrency_safe,

    )]
    async fn wait(
        &self,
        input: &TaskWaitInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        self.hydrate_session_tasks(context).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(input.timeout_ms);
        let (states, timed_out) = loop {
            let entries = input
                .task_ids
                .iter()
                .map(|task_id| task_entry_for_parent(&self.tasks, task_id, context.session_id))
                .collect::<SdkResult<Vec<_>>>()?;
            let states = input
                .task_ids
                .iter()
                .map(|task_id| entry_state_for_parent(&self.tasks, task_id, context.session_id))
                .collect::<SdkResult<Vec<_>>>()?;
            let complete = match input.mode {
                TaskWaitMode::Any => states
                    .iter()
                    .any(|state| is_terminal(state.status.as_str())),
                TaskWaitMode::All => states
                    .iter()
                    .all(|state| is_terminal(state.status.as_str())),
            };
            if complete {
                break (states, false);
            }
            if tokio::time::Instant::now() >= deadline {
                break (states, true);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let notifications = entries
                .iter()
                .map(|entry| Box::pin(Arc::clone(&entry.notify).notified_owned()))
                .collect::<Vec<_>>();
            if tokio::time::timeout(remaining, futures_util::future::select_all(notifications))
                .await
                .is_err()
            {
                break (states, true);
            }
        };
        Ok(task_output(
            "Wait for tasks",
            if timed_out {
                format!("Wait timed out after {} ms.", input.timeout_ms)
            } else {
                format!("Task wait condition {:?} completed.", input.mode)
            },
            states,
            timed_out,
        ))
    }

    /// Background tasks are attached to their parent session by default. When
    /// the parent ends, request cancellation of every nonterminal child rather
    /// than leaving unowned provider work running. The child session remains
    /// persisted for audit and `tasks.output`; its normal completion path
    /// writes the final task record if it can observe cancellation.
    #[hook(session.end)]
    async fn session_end(&self, input: SessionEndInput) -> SdkResult<()> {
        let entries = self
            .tasks
            .lock()
            .map_err(|_| agena_plugin_host::PluginError::internal("tasks registry lock poisoned"))?
            .values()
            .filter_map(|entry| {
                let state = entry.state.lock().ok()?.clone();
                (state.parent_session_id == input.session_id && !is_terminal(&state.status))
                    .then(|| (Arc::clone(entry), state))
            })
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return Ok(());
        }
        let host = self.inner.host()?;
        let callback = HostCallbackContext {
            plugin_id: Some(TASKS_PLUGIN_ID.to_string()),
            session_id: Some(input.session_id),
            ..Default::default()
        };
        for (entry, state) in entries {
            if let Err(error) = run_in_host_callback_context(
                callback.clone(),
                host.cancel_subtask(CancelSubtaskRequest {
                    parent_session_id: Some(input.session_id),
                    task_id: state.task_id.clone(),
                }),
            )
            .await
            {
                tracing::warn!(
                    target: "agena_tasks",
                    task_id = %state.task_id,
                    parent_session_id = input.session_id,
                    %error,
                    "failed to request child cancellation while parent session ended"
                );
                continue;
            }
            if let Ok(mut mutable) = entry.state.lock()
                && !is_terminal(&mutable.status)
            {
                mutable.status = "cancelling".to_string();
                mutable.error = Some("parent session ended; cancellation requested".to_string());
            }
            entry.notify.notify_waiters();
            entry.notify.notify_one();
        }
        Ok(())
    }

    /// Rebuild task handles for the invoking parent session from
    /// session-private plugin storage. The child session transcript remains
    /// the source of truth for output; this registry supplies task metadata
    /// after plugin reconstruction.
    ///
    /// A nonterminal persisted handle becomes `interrupted`, rather than
    /// automatically replaying its prompt. The child session might already
    /// contain that prompt when a process stopped before acknowledgement, so
    /// automatic restart could duplicate side effects. `tasks.output` and an
    /// explicit `tasks.followup` remain available for recovery.
    async fn hydrate_session_tasks(&self, context: &ToolInvokeContext<'_>) -> SdkResult<()> {
        let host = self.inner.host()?;
        let records = host
            .storage_list(HostStorageListRequest {
                scope: HostStorageScope::Session,
                visibility: Default::default(),
                namespace: Some(TASK_STORAGE_NAMESPACE.to_string()),
                prefix: None,
            })
            .await?;
        for record in records.records {
            let response = host
                .storage_get(HostStorageGetRequest {
                    scope: HostStorageScope::Session,
                    visibility: Default::default(),
                    namespace: TASK_STORAGE_NAMESPACE.to_string(),
                    key: record.key,
                })
                .await?;
            let Some(value) = response.value else {
                continue;
            };
            let mut state: AsyncTaskState = serde_json::from_str(&value).map_err(|error| {
                agena_plugin_host::PluginError::internal(format!(
                    "invalid persisted task registry entry: {error}"
                ))
            })?;
            if state.parent_session_id != context.session_id {
                continue;
            }
            let exists = self
                .tasks
                .lock()
                .map_err(|_| {
                    agena_plugin_host::PluginError::internal("tasks registry lock poisoned")
                })?
                .contains_key(state.task_id.as_str());
            if exists {
                continue;
            }
            if !is_terminal(&state.status) {
                state.status = "interrupted".to_string();
                state.finished_at_ms = Some(chrono::Utc::now().timestamp_millis());
                state.error = Some(
                    "Agena restarted before this delegated task reported a terminal result; inspect output and use tasks.followup for explicit recovery."
                        .to_string(),
                );
                persist_task_state(&host, callback_context(context), &state).await?;
            }
            let entry = Arc::new(AsyncTaskEntry {
                state: Mutex::new(state.clone()),
                notify: Arc::new(Notify::new()),
            });
            self.tasks
                .lock()
                .map_err(|_| {
                    agena_plugin_host::PluginError::internal("tasks registry lock poisoned")
                })?
                .insert(state.task_id.clone(), entry);
        }
        Ok(())
    }
}

fn callback_context(context: &ToolInvokeContext<'_>) -> HostCallbackContext {
    HostCallbackContext {
        plugin_id: Some(TASKS_PLUGIN_ID.to_string()),
        session_id: Some(context.session_id),
        call_id: Some(context.call_id),
        workspace_root: Some(context.workspace_root.to_string()),
        tool_name: Some(context.tool_name.to_string()),
    }
}

async fn persist_task_state(
    host: &Arc<dyn HostClient>,
    context: HostCallbackContext,
    state: &AsyncTaskState,
) -> SdkResult<()> {
    let value = serde_json::to_string(state).map_err(|error| {
        agena_plugin_host::PluginError::internal(format!("serialize task state: {error}"))
    })?;
    run_in_host_callback_context(
        context,
        host.storage_set(HostStorageSetRequest {
            scope: HostStorageScope::Session,
            visibility: Default::default(),
            namespace: TASK_STORAGE_NAMESPACE.to_string(),
            key: state.task_id.clone(),
            value,
        }),
    )
    .await
}

fn spawn_task(
    host: Arc<dyn HostClient>,
    entry: Arc<AsyncTaskEntry>,
    request: RunSubtaskRequest,
    context: HostCallbackContext,
) {
    tokio::spawn(async move {
        let result = run_in_host_callback_context(context.clone(), host.run_subtask(request)).await;
        let persisted = if let Ok(mut state) = entry.state.lock() {
            state.finished_at_ms = Some(chrono::Utc::now().timestamp_millis());
            match result {
                Ok(response) => {
                    state.status = status_name(response.status).to_string();
                    state.error = response
                        .problem
                        .as_ref()
                        .map(|failure| failure.user.fallback.clone());
                    state.budget_exceeded = response.budget_exceeded;
                    state.response = Some(response);
                }
                Err(error) => {
                    state.status = "failed".to_string();
                    state.error = Some(error.to_string());
                }
            }
            Some(state.clone())
        } else {
            None
        };
        if let Some(state) = persisted
            && let Err(error) = persist_task_state(&host, context, &state).await
        {
            tracing::warn!(target: "agena_tasks", %error, task_id = %state.task_id, "failed to persist terminal task state");
        }
        entry.notify.notify_waiters();
        // Preserve one permit for a waiter that checked state immediately
        // before this completion and had not yet polled its notification.
        entry.notify.notify_one();
    });
}

fn status_name(status: RunSubtaskStatus) -> &'static str {
    match status {
        RunSubtaskStatus::Created => "created",
        RunSubtaskStatus::Running => "running",
        RunSubtaskStatus::Completed => "completed",
        RunSubtaskStatus::Failed => "failed",
        RunSubtaskStatus::Cancelled => "cancelled",
        RunSubtaskStatus::TimedOut => "timed_out",
        RunSubtaskStatus::Interrupted => "interrupted",
    }
}

fn is_terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "cancelled" | "timed_out" | "interrupted"
    )
}

fn entry_state(
    tasks: &Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>,
    task_id: &str,
) -> SdkResult<AsyncTaskState> {
    let entry = tasks
        .lock()
        .map_err(|_| agena_plugin_host::PluginError::internal("tasks registry lock poisoned"))?
        .get(task_id)
        .cloned()
        .ok_or_else(|| {
            agena_plugin_host::PluginError::invalid_params(format!("unknown task '{task_id}'"))
        })?;
    let state = entry
        .state
        .lock()
        .map_err(|_| agena_plugin_host::PluginError::internal("task state lock poisoned"))?
        .clone();
    Ok(state)
}

fn entry_state_for_parent(
    tasks: &Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>,
    task_id: &str,
    parent_session_id: i64,
) -> SdkResult<AsyncTaskState> {
    let state = entry_state(tasks, task_id)?;
    if state.parent_session_id != parent_session_id {
        return Err(agena_plugin_host::PluginError::invalid_params(format!(
            "unknown task '{task_id}'"
        )));
    }
    Ok(state)
}

fn task_entry(
    tasks: &Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>,
    task_id: &str,
) -> SdkResult<Arc<AsyncTaskEntry>> {
    tasks
        .lock()
        .map_err(|_| agena_plugin_host::PluginError::internal("tasks registry lock poisoned"))?
        .get(task_id)
        .cloned()
        .ok_or_else(|| {
            agena_plugin_host::PluginError::invalid_params(format!("unknown task '{task_id}'"))
        })
}

fn task_entry_for_parent(
    tasks: &Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>,
    task_id: &str,
    parent_session_id: i64,
) -> SdkResult<Arc<AsyncTaskEntry>> {
    let entry = task_entry(tasks, task_id)?;
    if lock_state(&entry)?.parent_session_id != parent_session_id {
        return Err(agena_plugin_host::PluginError::invalid_params(format!(
            "unknown task '{task_id}'"
        )));
    }
    Ok(entry)
}

fn ensure_task_capacity(
    tasks: &Mutex<BTreeMap<String, Arc<AsyncTaskEntry>>>,
    parent_session_id: i64,
    exclude_task_id: Option<&str>,
) -> SdkResult<()> {
    let active = tasks
        .lock()
        .map_err(|_| agena_plugin_host::PluginError::internal("tasks registry lock poisoned"))?
        .values()
        .filter_map(|entry| entry.state.lock().ok().map(|state| state.clone()))
        .filter(|state| {
            state.parent_session_id == parent_session_id
                && !is_terminal(&state.status)
                && exclude_task_id != Some(state.task_id.as_str())
        })
        .count();
    if active >= MAX_ACTIVE_TASKS_PER_PARENT {
        return Err(agena_plugin_host::PluginError::invalid_params(format!(
            "at most {MAX_ACTIVE_TASKS_PER_PARENT} delegated tasks may run concurrently for one parent session"
        )));
    }
    Ok(())
}

fn lock_state(entry: &AsyncTaskEntry) -> SdkResult<std::sync::MutexGuard<'_, AsyncTaskState>> {
    entry
        .state
        .lock()
        .map_err(|_| agena_plugin_host::PluginError::internal("task state lock poisoned"))
}

fn task_output(
    title: impl Into<String>,
    text: impl Into<String>,
    tasks: Vec<AsyncTaskState>,
    timed_out: bool,
) -> ToolInvokeOutput {
    let summary = if timed_out {
        format!("Timed out · {} tasks", tasks.len())
    } else if let [task] = tasks.as_slice() {
        task.status.clone()
    } else {
        let terminal = tasks
            .iter()
            .filter(|task| is_terminal(task.status.as_str()))
            .count();
        format!("{} tasks · {terminal} terminal", tasks.len())
    };
    ToolInvokeOutput::from_parts(
        title,
        summary,
        text,
        Some(serde_json::json!({ "tasks": tasks, "timed_out": timed_out })),
        BTreeMap::from([
            ("task_count".to_string(), tasks.len().to_string()),
            ("timed_out".to_string(), timed_out.to_string()),
        ]),
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::message::{TaskAccess, TaskToolInput};
    use agena_plugin_host::sdk::Plugin;

    use super::{AsyncTaskEntry, AsyncTaskState, TasksPlugin, lock_state};

    #[test]
    fn task_contract_uses_execution_access_and_terminal_host_capability() {
        let manifest = TasksPlugin::new().manifest();
        let tool = manifest.tools.first().expect("task tool");
        assert_eq!(tool.name, "run");
        let schema = &tool.contract.input_schema;
        assert!(schema.pointer("/properties/access").is_some());
        assert!(schema.pointer("/properties/profile").is_none());
        assert!(schema.pointer("/properties/selection").is_some());
        assert!(schema.pointer("/properties/skills").is_some());
        assert!(schema.pointer("/properties/subagent_type").is_none());
        assert!(schema.pointer("/properties/command").is_none());
        assert_eq!(
            schema.pointer("/properties/timeout_ms/minimum"),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            schema.pointer("/properties/max_tokens/minimum"),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            schema.pointer("/properties/max_cost_microusd/minimum"),
            Some(&serde_json::json!(1))
        );
        let names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "run", "create", "list", "get", "output", "cancel", "message", "followup", "wait"
            ]
        );
        for name in ["create", "output", "cancel", "message", "followup"] {
            assert!(
                manifest.tools.iter().any(|tool| tool.name == name),
                "missing task lifecycle tool `{name}`"
            );
        }
    }

    #[test]
    fn task_input_rejects_zero_timeout_and_unknown_legacy_fields() {
                let valid = serde_json::json!({
            "description": "verify",
            "prompt": "run the checks",
            "access": "read_only",
            "skills": ["verify", "security-review"],
            "timeout_ms": 1
        });
        assert!(TaskToolInput::parse_input(valid).is_ok());

        for invalid in [
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "access": "read_only",
                "timeout_ms": 0
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "access": "read_only",
                "max_tokens": 0
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "access": "read_only",
                "max_cost_microusd": 0
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "access": "read_only",
                "task_id": "   "
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "profile": "verify",
                "subagent_type": "verify"
            }),
        ] {
            assert!(TaskToolInput::parse_input(invalid).is_err());
        }
    }

    #[tokio::test]
    async fn task_notification_preserves_a_completion_wakeup() {
        let entry = Arc::new(AsyncTaskEntry {
            state: std::sync::Mutex::new(AsyncTaskState {
                task_id: "task_wait".to_string(),
                parent_session_id: 7,
                description: "wait".to_string(),
                prompt: "wait".to_string(),
                access: TaskAccess::ReadOnly,
                status: "running".to_string(),
                started_at_ms: 1,
                finished_at_ms: None,
                response: None,
                error: None,
                selection: None,
                timeout_ms: None,
                max_tokens: None,
                max_cost_microusd: None,
                budget_exceeded: false,
            }),
            notify: Arc::new(tokio::sync::Notify::new()),
        });
        let waiter = {
            let entry = Arc::clone(&entry);
            tokio::spawn(async move {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    Arc::clone(&entry.notify).notified_owned(),
                )
                .await
            })
        };
        tokio::task::yield_now().await;
        lock_state(&entry).expect("task state").status = "completed".to_string();
        entry.notify.notify_waiters();
        entry.notify.notify_one();

        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter woke")
            .expect("waiter joined")
            .expect("notification completed");
    }
}
