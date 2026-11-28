//! Run an App tool and follow its continuations on the owning view.

use agena_domain::mcp_app_tools::{McpAppTaskMethod, McpAppTaskSupport};
use agena_mcp_client::manager::respond_to_app_tool_inputs;
use agena_mcp_client::protocol::{
    CreateTaskResult, DEFAULT_MRTR_MAX_ROUNDS, GetTaskResult, InputRequests, InputRequiredResult,
    InputResponses, Task, TaskAckResult, TaskPayload, TaskStatus,
};
use serde::Deserialize;

use super::*;

const TASK_TIMEOUT: Duration = Duration::from_secs(300);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(3);
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_POLL_MS: u64 = 250;
const MAX_CALL_INPUTS: usize = 128;
const RELATED_TASK: &str = "io.modelcontextprotocol/related-task";

pub(super) struct RunningCall {
    view: Arc<View>,
    task_id: Option<String>,
    cancel_task: bool,
    input_count: usize,
}

impl RunningCall {
    pub(super) fn new(view: Arc<View>) -> Self {
        Self {
            view,
            task_id: None,
            cancel_task: false,
            input_count: 0,
        }
    }

    pub(super) async fn run(
        mut self,
        model_call: i64,
        definition: &McpAppToolDefinition,
        arguments: Map<String, Value>,
        cancellation: &CancellationToken,
        interactions: &dyn McpElicitationHandler,
    ) -> Result<Value, McpAppViewError> {
        let deadline = Instant::now() + TASK_TIMEOUT;
        let support = definition
            .execution
            .as_ref()
            .map_or(McpAppTaskSupport::Forbidden, |execution| {
                execution.task_support
            });
        let extension = self.view.task_extension;
        let augment = !extension && support != McpAppTaskSupport::Forbidden;
        let handoff = (extension || augment).then_some(Handoff {
            call: model_call,
            extension,
        });
        let result = self
            .call_tool(
                McpAppToolRequest::Tool {
                    name: definition.name.clone(),
                    arguments,
                    task: augment.then(Map::new),
                    request_state: None,
                    input_responses: None,
                },
                deadline,
                cancellation,
                handoff,
                interactions,
            )
            .await?;
        let result = match result {
            ViewResponse::ServerTask(task) => return task.result(deadline, cancellation).await,
            frame => frame.value()?,
        };
        if result["resultType"] == "task" {
            if !extension {
                return Err(McpAppViewError::InvalidResult);
            }
            let created: CreateTaskResult =
                serde_json::from_value(result).map_err(|_| McpAppViewError::InvalidResult)?;
            self.bind_task(&created.task)?;
            self.follow_extension(created.task, deadline, cancellation, interactions)
                .await
        } else if let Some(task) = legacy_handle(&result) {
            if !augment {
                return Err(McpAppViewError::InvalidResult);
            }
            let task = Task::deserialize(task).map_err(|_| McpAppViewError::InvalidResult)?;
            self.bind_task(&task)?;
            self.follow_legacy(task, deadline, cancellation).await
        } else if augment && support == McpAppTaskSupport::Required {
            Err(McpAppViewError::InvalidResult)
        } else {
            Ok(result)
        }
    }

    async fn call_tool(
        &mut self,
        mut request: McpAppToolRequest,
        deadline: Instant,
        cancellation: &CancellationToken,
        handoff: Option<Handoff>,
        interactions: &dyn McpElicitationHandler,
    ) -> Result<ViewResponse, McpAppViewError> {
        let mut state_only_rounds = 0;
        for round in 1..=DEFAULT_MRTR_MAX_ROUNDS {
            bounded_message(&request)?;
            let response = self
                .view
                .request(
                    request.clone(),
                    (Instant::now() + CALL_TIMEOUT).min(deadline),
                    cancellation,
                    handoff,
                )
                .await?;
            let ViewResponse::Frame(McpAppToolReply::Result { result }) = &response else {
                return Ok(response);
            };
            if result["resultType"] != "input_required" {
                return Ok(response);
            }
            if round == DEFAULT_MRTR_MAX_ROUNDS {
                break;
            }
            let required = InputRequiredResult::deserialize(result)
                .map_err(|_| McpAppViewError::InvalidResult)?;
            let inputs = required.input_requests.unwrap_or_default();
            let responses = if inputs.is_empty() {
                if required.request_state.is_none() {
                    return Err(McpAppViewError::InvalidResult);
                }
                let delay = Duration::from_millis((50_u64 << state_only_rounds.min(3)).min(250));
                self.wait_until((Instant::now() + delay).min(deadline), cancellation)
                    .await?;
                state_only_rounds += 1;
                None
            } else {
                state_only_rounds = 0;
                Some(
                    self.resolve_inputs(inputs, interactions, deadline, cancellation)
                        .await?,
                )
            };
            let McpAppToolRequest::Tool {
                request_state,
                input_responses,
                ..
            } = &mut request
            else {
                unreachable!("view tool request");
            };
            *request_state = required.request_state;
            *input_responses = responses;
        }
        Err(McpAppViewError::Rejected(format!(
            "The App tool exceeded {DEFAULT_MRTR_MAX_ROUNDS} continuation rounds."
        )))
    }

    fn bind_task(&mut self, task: &Task) -> Result<(), McpAppViewError> {
        let id = &task.task_id;
        if id.is_empty() || !self.view.live_state()?.task_ids.insert(id.clone()) {
            return Err(McpAppViewError::InvalidResult);
        }
        self.task_id = Some(id.clone());
        self.cancel_task = !task.status.is_terminal();
        Ok(())
    }

    fn observe_task(&mut self, task: &Task) -> Result<(), McpAppViewError> {
        if self.task_id.as_deref() != Some(task.task_id.as_str()) {
            return Err(McpAppViewError::InvalidResult);
        }
        if task.status.is_terminal() {
            self.cancel_task = false;
        }
        Ok(())
    }

    async fn follow_legacy(
        &mut self,
        mut task: Task,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Value, McpAppViewError> {
        let mut interval = task.poll_interval_ms;
        while task.status == TaskStatus::Working {
            self.wait_to_poll(interval, deadline, cancellation).await?;
            task = serde_json::from_value(
                self.task_request(McpAppTaskMethod::Get, None, deadline, cancellation)
                    .await?,
            )
            .map_err(|_| McpAppViewError::InvalidResult)?;
            self.observe_task(&task)?;
            interval = task.poll_interval_ms.or(interval);
        }
        // App-local interactions may settle while tasks/result is outstanding.
        let result = self
            .task_request(McpAppTaskMethod::Result, None, deadline, cancellation)
            .await?;
        if is_intermediate(&result) {
            return Err(McpAppViewError::InvalidResult);
        }
        if let Some(related) = result.get("_meta").and_then(|meta| meta.get(RELATED_TASK))
            && related["taskId"].as_str() != self.task_id.as_deref()
        {
            return Err(McpAppViewError::InvalidResult);
        }
        self.cancel_task = false;
        Ok(result)
    }

    async fn follow_extension(
        &mut self,
        mut task: Task,
        deadline: Instant,
        cancellation: &CancellationToken,
        interactions: &dyn McpElicitationHandler,
    ) -> Result<Value, McpAppViewError> {
        let mut interval = task.poll_interval_ms;
        let mut wait = task.status == TaskStatus::Working;
        let mut answered = BTreeSet::new();
        loop {
            if wait {
                self.wait_to_poll(interval, deadline, cancellation).await?;
            }
            let progress: GetTaskResult = serde_json::from_value(
                self.task_request(McpAppTaskMethod::Get, None, deadline, cancellation)
                    .await?,
            )
            .map_err(|_| McpAppViewError::InvalidResult)?;
            if !progress.result_type.is_complete() {
                return Err(McpAppViewError::InvalidResult);
            }
            task = progress.task.task;
            self.observe_task(&task)?;
            interval = task.poll_interval_ms.or(interval);
            wait = true;
            match progress.task.payload {
                TaskPayload::Working => {}
                TaskPayload::Completed { result } => return Ok(Value::Object(result)),
                TaskPayload::Failed { error } => {
                    let reason = error
                        .get("message")
                        .and_then(Value::as_str)
                        .ok_or(McpAppViewError::InvalidResult)?;
                    return Err(McpAppViewError::Rejected(
                        reason.chars().take(512).collect(),
                    ));
                }
                TaskPayload::Cancelled => {
                    return Err(McpAppViewError::Rejected(
                        "The App cancelled the task.".into(),
                    ));
                }
                TaskPayload::InputRequired { mut input_requests } => {
                    // The App may still report these IDs immediately after acknowledging the update.
                    input_requests.retain(|key, _| !answered.contains(key));
                    if !input_requests.is_empty() {
                        let responses = self
                            .resolve_inputs(input_requests, interactions, deadline, cancellation)
                            .await?;
                        answered.extend(responses.keys().cloned());
                        let ack = self
                            .task_request(
                                McpAppTaskMethod::Update,
                                Some(responses),
                                deadline,
                                cancellation,
                            )
                            .await?;
                        if !ack.as_object().is_some_and(Map::is_empty)
                            && serde_json::from_value::<TaskAckResult>(ack).is_err()
                        {
                            return Err(McpAppViewError::InvalidResult);
                        }
                    }
                }
                _ => return Err(McpAppViewError::InvalidResult),
            }
        }
    }

    async fn resolve_inputs(
        &mut self,
        inputs: InputRequests,
        interactions: &dyn McpElicitationHandler,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<InputResponses, McpAppViewError> {
        if inputs.len() > MAX_CALL_INPUTS - self.input_count {
            return Err(McpAppViewError::Rejected(
                "The App tool requested too many interactions.".into(),
            ));
        }
        self.input_count += inputs.len();
        let scoped = cancellation.child_token();
        // Dropping the interaction also closes any durable user request it opened.
        let _cancel_on_drop = scoped.clone().drop_guard();
        let work = respond_to_app_tool_inputs(&self.view.name, inputs, interactions, &scoped);
        tokio::pin!(work);
        let stopped = tokio::select! {
            biased;
            _ = cancellation.cancelled() => McpAppViewError::Cancelled,
            _ = self.view.closed.cancelled() => McpAppViewError::Unavailable,
            _ = tokio::time::sleep_until(deadline) => McpAppViewError::Timeout,
            result = &mut work => {
                return result.map_err(|error| McpAppViewError::Rejected(error.user_message()));
            }
        };
        scoped.cancel();
        let _ = work.await;
        Err(stopped)
    }

    async fn wait_to_poll(
        &self,
        interval: Option<u64>,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<(), McpAppViewError> {
        let delay =
            Duration::from_millis(interval.unwrap_or(DEFAULT_POLL_MS)).max(MIN_POLL_INTERVAL);
        let now = Instant::now();
        if delay >= deadline.saturating_duration_since(now) {
            self.wait_until(deadline, cancellation).await?;
            return Err(McpAppViewError::Timeout);
        }
        self.wait_until(now + delay, cancellation).await
    }

    async fn wait_until(
        &self,
        time: Instant,
        cancellation: &CancellationToken,
    ) -> Result<(), McpAppViewError> {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(McpAppViewError::Cancelled),
            _ = self.view.closed.cancelled() => Err(McpAppViewError::Unavailable),
            _ = tokio::time::sleep_until(time) => Ok(()),
        }
    }

    async fn task_request(
        &self,
        method: McpAppTaskMethod,
        input_responses: Option<InputResponses>,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Value, McpAppViewError> {
        let request = McpAppToolRequest::Task {
            method,
            task_id: self.task_id.clone().ok_or(McpAppViewError::InvalidResult)?,
            input_responses,
        };
        bounded_message(&request)?;
        self.view
            .request(request, deadline, cancellation, None)
            .await?
            .value()
    }
}

impl Drop for RunningCall {
    fn drop(&mut self) {
        if self.cancel_task
            && !self.view.closed.is_cancelled()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            // Keep the slot and task ID reserved until cancellation is attempted.
            let cleanup = Self {
                view: self.view.clone(),
                task_id: self.task_id.take(),
                cancel_task: false,
                input_count: 0,
            };
            runtime.spawn(async move {
                let _ = cleanup
                    .task_request(
                        McpAppTaskMethod::Cancel,
                        None,
                        Instant::now() + CANCEL_TIMEOUT,
                        &CancellationToken::new(),
                    )
                    .await;
            });
            return;
        }
        let mut state = self.view.state.lock();
        state.active_calls -= 1;
        if let Some(id) = self.task_id.as_ref() {
            state.task_ids.remove(id);
        }
    }
}

pub(super) fn legacy_handle(result: &Value) -> Option<&Value> {
    let object = result.as_object()?;
    (!object.contains_key("content") && !object.contains_key("structuredContent"))
        .then(|| object.get("task"))
        .flatten()
}

pub(super) fn is_intermediate(result: &Value) -> bool {
    result
        .get("resultType")
        .is_some_and(|kind| kind != "complete")
        || legacy_handle(result).is_some()
}
