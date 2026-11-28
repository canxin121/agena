//! MCP tool tasks for the current extension and the 2025-11-25 wire format.

use std::collections::{BTreeMap, HashMap, HashSet, hash_map::Entry};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agena_domain::mcp_app_tools::{MCP_APP_TOOL_MESSAGE_LIMIT, McpAppTaskInput};
use rmcp::ErrorData;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelTaskParams, CancelTaskRequest,
    ClientRequest, CustomRequest, GetExtensions, GetTaskParams, GetTaskRequest, InputResponses,
    MetaObject, ProtocolVersion, ResultType, ServerResult, Task, TaskPayload, TaskStatus,
    UpdateTaskParams, UpdateTaskRequest,
};
use rmcp::service::{
    AssociatedRequestData, InboundStreamOrigin, Peer, RequestContext, RoleClient, ServiceError,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::elicitation::{self, IncomingRoute, McpElicitationHandler, RequestError, WireInput};
use super::{ConnectedServer, ServerEventState};
use crate::McpError;

const RELATED_TASK: &str = "io.modelcontextprotocol/related-task";
const DEFAULT_POLL_MS: u64 = 250;
const MIN_POLL_MS: u64 = 10;
const CANCEL_TIMEOUT: Duration = Duration::from_millis(500);
pub(super) const MAX_APP_TASK_INPUTS: usize = 128;

/// Observe an asynchronous server call without moving it off its live peer.
pub struct McpTaskObserver<'a> {
    pub legacy_ttl_ms: Option<u64>,
    pub cancellation: &'a CancellationToken,
    pub progress: &'a (dyn Fn(Task) + Send + Sync),
    pub view: Option<&'a dyn McpTaskViewInputs>,
}

#[async_trait::async_trait]
pub trait McpTaskViewInputs: Send + Sync {
    fn clear_inputs(&self);

    /// Wait for input from the App while it owns the task. Returning no
    /// answers hands the outstanding requests to the host interaction path.
    async fn wait_for_inputs(
        &self,
        requests: BTreeMap<String, McpAppTaskInput>,
        cancellation: &CancellationToken,
    ) -> Option<InputResponses>;
}

fn observe(observer: Option<&McpTaskObserver<'_>>, server: &ConnectedServer, task: &Task) {
    if let Some(observer) = observer {
        let mut task = task.clone();
        // The calling bridge issues its own task ID. Keep the server ID on the
        // original peer and redact credentials from user-visible metadata.
        task.task_id.clear();
        task.created_at = server
            .redactions
            .text(task.created_at)
            .chars()
            .take(64)
            .collect();
        task.last_updated_at = server
            .redactions
            .text(task.last_updated_at)
            .chars()
            .take(64)
            .collect();
        task.status_message = task
            .status_message
            .map(|text| server.redactions.text(text).chars().take(512).collect());
        (observer.progress)(task);
    }
}

#[derive(Default)]
pub(super) struct ToolCall {
    pub(super) continuation_started: bool,
    active_task: Option<ActiveTask>,
}

impl ToolCall {
    pub(super) async fn run(
        &mut self,
        server: &ConnectedServer,
        params: CallToolRequestParams,
        handler: Option<&dyn McpElicitationHandler>,
        observer: Option<&McpTaskObserver<'_>>,
    ) -> Result<CallToolResult, RequestError> {
        let legacy_cancel = legacy_support(server, &params.name).await;
        let request = if legacy_cancel.is_some() {
            let mut wire = serde_json::to_value(params).map_err(McpError::from)?;
            wire["task"] = observer
                .and_then(|observer| observer.legacy_ttl_ms)
                .map_or_else(|| json!({}), |ttl| json!({"ttl": ttl}));
            ClientRequest::CustomRequest(CustomRequest::new("tools/call", Some(wire)))
        } else {
            ClientRequest::CallToolRequest(CallToolRequest::new(params))
        };
        let response =
            elicitation::request(server, request, handler, &mut self.continuation_started).await?;
        match response {
            ServerResult::CallToolResult(result) => Ok(result),
            ServerResult::CreateTaskResult(created)
                if server.peer.peer_info().is_some_and(|info| {
                    info.protocol_version >= ProtocolVersion::V_2026_07_28
                        && info.capabilities.supports_tasks()
                }) =>
            {
                self.continuation_started = true;
                let active =
                    self.active_task
                        .insert(ActiveTask::new(server, &created.task, None, true)?);
                observe(observer, server, &created.task);
                follow_modern(server, created.task, active, handler, observer).await
            }
            ServerResult::CustomResult(result)
                if legacy_cancel.is_some() && result.0.get("task").is_some() =>
            {
                self.continuation_started = true;
                let created: LegacyCreateResult =
                    serde_json::from_value(result.0).map_err(McpError::from)?;
                let (sender, incoming) = mpsc::channel(8);
                let route = LegacyTaskRoute {
                    id: created.task.task_id.clone(),
                    incoming: IncomingRoute::new(server, sender),
                };
                let active = self.active_task.insert(ActiveTask::new(
                    server,
                    &created.task,
                    Some(route.clone()),
                    legacy_cancel == Some(true),
                )?);
                observe(observer, server, &created.task);
                elicitation::with_incoming(
                    server,
                    handler,
                    observer.and_then(|observer| observer.view),
                    incoming,
                    follow_legacy(server, created.task, active, route, observer),
                    &mut self.continuation_started,
                )
                .await
            }
            _ => Err(ServiceError::UnexpectedResponse.into()),
        }
    }

    pub(super) async fn settle(self) {
        if let Some(mut task) = self.active_task
            && let Some(request) = task.cancel.take()
        {
            // Keep the originating call's authority alive while delivering
            // cancellation after a local timeout or protocol failure.
            send_cancellation(&task.peer, request).await;
        }
    }
}

// A legacy server must opt in both at initialization and for this tool. The
// return value also records whether it implements the optional cancel RPC.
async fn legacy_support(server: &ConnectedServer, name: &str) -> Option<bool> {
    let info = server.peer.peer_info()?;
    if info.protocol_version != ProtocolVersion::V_2025_11_25 {
        return None;
    }
    let capabilities = info.capabilities.legacy_tasks.as_ref()?;
    if !capabilities.get("requests")?["tools"]["call"].is_object() {
        return None;
    }
    let tools = server.events.tools.read().await;
    let execution = tools
        .iter()
        .find(|tool| tool.name == name)?
        .execution
        .as_ref()?;
    match execution["taskSupport"].as_str()? {
        "optional" | "required" => Some(capabilities.get("cancel").is_some_and(Value::is_object)),
        _ => None,
    }
}

#[derive(Deserialize)]
struct LegacyCreateResult {
    task: Task,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskWithCustomInput {
    #[serde(flatten)]
    task: Task,
    result_type: ResultType,
    input_requests: BTreeMap<String, WireInput>,
}

async fn follow_modern(
    server: &ConnectedServer,
    seed: Task,
    active: &mut ActiveTask,
    handler: Option<&dyn McpElicitationHandler>,
    observer: Option<&McpTaskObserver<'_>>,
) -> Result<CallToolResult, RequestError> {
    let id = seed.task_id;
    let mut interval = seed.poll_interval_ms;
    let mut wait = seed.status == TaskStatus::Working;
    let mut answered = HashSet::new();
    loop {
        if wait {
            wait_to_poll(server, interval).await?;
        }
        let result = elicitation::send(
            &server.peer,
            GetTaskRequest::new(GetTaskParams::new(&id)).into(),
        )
        .await?;
        let (task, mut inputs) = match result {
            ServerResult::GetTaskResult(result) if result.result_type.is_complete() => {
                let task = result.task.task;
                require_id(&id, &task)?;
                observe(observer, server, &task);
                if task.status.is_terminal() {
                    active.cancel = None;
                }
                let inputs = match result.task.payload {
                    TaskPayload::Working => BTreeMap::new(),
                    TaskPayload::InputRequired { input_requests } => input_requests
                        .into_iter()
                        .map(|(key, request)| (key, WireInput::Standard(request)))
                        .collect(),
                    TaskPayload::Completed { result } => {
                        return Ok(serde_json::from_value(Value::Object(result))
                            .map_err(McpError::from)?);
                    }
                    TaskPayload::Failed { error } => {
                        let error: ErrorData =
                            serde_json::from_value(Value::Object(error)).map_err(McpError::from)?;
                        return Err(McpError::from(error).into());
                    }
                    TaskPayload::Cancelled => {
                        return Err(ServiceError::Cancelled {
                            reason: task.status_message,
                        }
                        .into());
                    }
                    _ => return Err(ServiceError::UnexpectedResponse.into()),
                };
                (task, inputs)
            }
            // The standard rmcp union only recognizes core input methods.
            // Use the same custom input decoder as ordinary MCP continuations.
            ServerResult::CustomResult(result) if result.0["status"] == "input_required" => {
                let input: TaskWithCustomInput =
                    serde_json::from_value(result.0).map_err(McpError::from)?;
                if !input.result_type.is_complete() {
                    return Err(ServiceError::UnexpectedResponse.into());
                }
                require_id(&id, &input.task)?;
                observe(observer, server, &input.task);
                (input.task, input.input_requests)
            }
            _ => return Err(ServiceError::UnexpectedResponse.into()),
        };
        interval = task.poll_interval_ms.or(interval);
        wait = true;
        // A poll after an acknowledged update can still return the old input.
        inputs.retain(|key, _| !answered.contains(key));
        if inputs.is_empty()
            && task.status == TaskStatus::InputRequired
            && let Some(view) = observer.and_then(|observer| observer.view)
        {
            view.clear_inputs();
        }
        if !inputs.is_empty() {
            if observer.is_some() && answered.len() + inputs.len() > MAX_APP_TASK_INPUTS {
                return Err(
                    McpError::Malformed("Too many MCP App task input requests".into()).into(),
                );
            }
            let responses = match observer {
                Some(observer) if observer.view.is_some() => {
                    app_task_inputs(server, inputs, handler, observer).await?
                }
                _ => elicitation::respond_to_inputs(server, inputs, handler).await?,
            };
            answered.extend(responses.keys().cloned());
            let ack = elicitation::send(
                &server.peer,
                UpdateTaskRequest::new(UpdateTaskParams::new(&id, responses)).into(),
            )
            .await?;
            if !matches!(
                ack,
                ServerResult::TaskAckResult(_) | ServerResult::EmptyResult(_)
            ) {
                return Err(ServiceError::UnexpectedResponse.into());
            }
        }
    }
}

async fn app_task_inputs(
    server: &ConnectedServer,
    inputs: BTreeMap<String, WireInput>,
    handler: Option<&dyn McpElicitationHandler>,
    observer: &McpTaskObserver<'_>,
) -> Result<InputResponses, RequestError> {
    let mut host = BTreeMap::new();
    let mut app = BTreeMap::new();
    let mut projected = BTreeMap::new();
    // Allow room for the proxy IDs and the task response envelope.
    let mut size = 2048;
    for (id, input) in elicitation::decode_inputs(inputs)? {
        if let Some(visible) = input.for_app(&server.redactions)? {
            size += id.len().max(64)
                + serde_json::to_vec(&visible.request)
                    .map_err(McpError::from)?
                    .len();
            if size > MCP_APP_TOOL_MESSAGE_LIMIT {
                return Err(McpError::Malformed(
                    "MCP App task inputs exceed their size limit".into(),
                )
                .into());
            }
            projected.insert(id.clone(), visible);
            app.insert(id, input);
        } else {
            host.insert(id, input);
        }
    }
    let view = observer.view.expect("App input observer");
    // Roots and declined elicitations can be answered before waiting on the App.
    if !host.is_empty() {
        if projected.is_empty() {
            view.clear_inputs();
        }
        return elicitation::respond_to_decoded_inputs(server, host, handler).await;
    }
    let reply = tokio::select! {
        biased;
        _ = server.events.shutdown.cancelled() => return Err(ServiceError::TransportClosed.into()),
        _ = server.peer.transport_closed() => return Err(ServiceError::TransportClosed.into()),
        result = view.wait_for_inputs(projected, observer.cancellation) => result,
    };
    match reply {
        Some(replies) => Ok(replies),
        None => elicitation::respond_to_decoded_inputs(server, app, handler).await,
    }
}

async fn follow_legacy(
    server: &ConnectedServer,
    mut task: Task,
    active: &mut ActiveTask,
    route: LegacyTaskRoute,
    observer: Option<&McpTaskObserver<'_>>,
) -> Result<CallToolResult, RequestError> {
    let id = task.task_id.clone();
    let mut interval = task.poll_interval_ms;
    while task.status == TaskStatus::Working {
        wait_to_poll(server, interval).await?;
        let result = legacy_request(
            server,
            &route,
            GetTaskRequest::new(GetTaskParams::new(&id)).into(),
        )
        .await?;
        task = match result {
            ServerResult::GetTaskResult(result) => result.task.task,
            ServerResult::CustomResult(result) => {
                serde_json::from_value(result.0).map_err(McpError::from)?
            }
            _ => return Err(ServiceError::UnexpectedResponse.into()),
        };
        require_id(&id, &task)?;
        observe(observer, server, &task);
        interval = task.poll_interval_ms.or(interval);
    }
    if task.status.is_terminal() {
        active.cancel = None;
    }
    // For input_required the server can keep this request open and send
    // elicitation on its POST stream, a standalone GET, or the stdio channel.
    let request = CustomRequest::new("tasks/result", Some(json!({"taskId": id})));
    let result = legacy_request(server, &route, request.into()).await?;
    let ServerResult::CallToolResult(result) = result else {
        return Err(ServiceError::UnexpectedResponse.into());
    };
    if let Some(related) = result.meta.as_ref().and_then(|meta| meta.get(RELATED_TASK))
        && related["taskId"].as_str() != Some(id.as_str())
    {
        return Err(task_id_mismatch().into());
    }
    active.cancel = None;
    Ok(result)
}

async fn legacy_request(
    server: &ConnectedServer,
    route: &LegacyTaskRoute,
    mut request: ClientRequest,
) -> Result<ServerResult, ServiceError> {
    request
        .extensions_mut()
        .insert(AssociatedRequestData::new(route.clone()));
    elicitation::send(&server.peer, request).await
}

async fn wait_to_poll(
    server: &ConnectedServer,
    interval_ms: Option<u64>,
) -> Result<(), ServiceError> {
    let interval = Duration::from_millis(interval_ms.unwrap_or(DEFAULT_POLL_MS).max(MIN_POLL_MS));
    tokio::select! {
        biased;
        _ = server.events.shutdown.cancelled() => Err(ServiceError::TransportClosed),
        _ = server.peer.transport_closed() => Err(ServiceError::TransportClosed),
        _ = tokio::time::sleep(interval) => Ok(()),
    }
}

fn require_id(id: &str, task: &Task) -> Result<(), McpError> {
    if task.task_id == id {
        Ok(())
    } else {
        Err(task_id_mismatch())
    }
}

fn task_id_mismatch() -> McpError {
    McpError::Malformed("MCP task response referred to a different task".into())
}

#[derive(Clone)]
pub(super) struct LegacyTaskRoute {
    id: String,
    pub(super) incoming: IncomingRoute,
}

impl LegacyTaskRoute {
    pub(super) fn metadata(&self) -> MetaObject {
        let mut metadata = MetaObject::new();
        metadata.insert(RELATED_TASK.into(), json!({"taskId": self.id}));
        metadata
    }
}

#[derive(Default)]
pub(super) struct TaskRegistry(Mutex<HashMap<String, Option<LegacyTaskRoute>>>);

impl TaskRegistry {
    pub(super) fn legacy_route(
        &self,
        context: &RequestContext<RoleClient>,
    ) -> Result<Option<LegacyTaskRoute>, ErrorData> {
        let bound = context
            .extensions
            .get::<AssociatedRequestData>()
            .and_then(|data| data.get::<LegacyTaskRoute>());
        let origin = context.extensions.get::<InboundStreamOrigin>();
        let Some(related) = context.meta.get(RELATED_TASK) else {
            // Shared transports cannot assign an untagged callback to an
            // ordinary in-flight request while a legacy task can also ask.
            let ambiguous = origin.is_none()
                && self
                    .0
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .values()
                    .any(Option::is_some);
            if bound.is_some() || ambiguous {
                return Err(ErrorData::invalid_params(
                    "missing MCP task association",
                    None,
                ));
            }
            return Ok(None);
        };
        let id = related["taskId"]
            .as_str()
            .ok_or_else(|| ErrorData::invalid_params("invalid MCP task association", None))?;
        let tasks = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let route = tasks
            .get(id)
            .and_then(Option::as_ref)
            .ok_or_else(|| ErrorData::invalid_params("unknown MCP task association", None))?;
        // On stdio and standalone SSE, the legacy wire task ID is the only
        // association. A POST stream specifically tied to another request
        // cannot use this task's interaction handler.
        if matches!(origin, Some(InboundStreamOrigin::OutboundRequest(_)))
            && !bound.is_some_and(|bound| bound.incoming.sender.same_channel(&route.incoming.sender))
        {
            return Err(ErrorData::invalid_params(
                "MCP task association does not match the stream",
                None,
            ));
        }
        route.incoming.attach_reply_scope(context);
        Ok(Some(route.clone()))
    }
}

struct TaskBinding {
    events: Arc<ServerEventState>,
    id: String,
}

impl Drop for TaskBinding {
    fn drop(&mut self) {
        self.events
            .tasks
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
    }
}

struct ActiveTask {
    binding: Option<TaskBinding>,
    peer: Peer<RoleClient>,
    cancel: Option<ClientRequest>,
}

impl ActiveTask {
    fn new(
        server: &ConnectedServer,
        task: &Task,
        route: Option<LegacyTaskRoute>,
        cancellable: bool,
    ) -> Result<Self, McpError> {
        let id = &task.task_id;
        if id.is_empty() {
            return Err(McpError::Malformed(
                "MCP server returned an empty task ID".into(),
            ));
        }
        {
            let mut tasks = server
                .events
                .tasks
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Entry::Vacant(entry) = tasks.entry(id.clone()) else {
                return Err(McpError::Malformed(
                    "MCP server reused an active task ID".into(),
                ));
            };
            entry.insert(route);
        }
        Ok(Self {
            binding: Some(TaskBinding {
                events: Arc::clone(&server.events),
                id: id.clone(),
            }),
            peer: server.peer.clone(),
            cancel: (cancellable && !task.status.is_terminal())
                .then(|| CancelTaskRequest::new(CancelTaskParams::new(id)).into()),
        })
    }
}

impl Drop for ActiveTask {
    fn drop(&mut self) {
        if let Some(request) = self.cancel.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let peer = self.peer.clone();
            let binding = self.binding.take();
            runtime.spawn(async move {
                let _binding = binding;
                send_cancellation(&peer, request).await;
            });
        }
    }
}

async fn send_cancellation(peer: &Peer<RoleClient>, request: ClientRequest) {
    let _ = tokio::time::timeout(CANCEL_TIMEOUT, elicitation::send(peer, request)).await;
}
