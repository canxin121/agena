//! Server tasks have one origin view and can transfer to one live model tool.

use std::future::Future;
use std::sync::Weak;

use agena_plugin_host::sdk::host_api::{McpAppLegacyTaskView, McpAppServerTaskPort};
use serde_json::json;

use super::*;

const MAX_SERVER_TASKS: usize = 128;
const MAX_VIEW_SERVER_TASKS: usize = 16;
const SERVER_TASK_LIFETIME: Duration = Duration::from_secs(300);
const PROXY_ID_PREFIX: &str = "agena-app-task:";
const RELATED_TASK_ID: &str = "/_meta/io.modelcontextprotocol~1related-task/taskId";
const RELATED_TASK_META: &str = "io.modelcontextprotocol/related-task";

#[derive(Clone, Copy)]
pub enum McpAppServerTaskProtocol {
    Extension,
    Legacy { ttl_ms: Option<u64> },
}

impl McpAppServerTaskProtocol {
    fn extension(self) -> bool {
        matches!(self, Self::Extension)
    }
}

#[derive(Clone)]
enum Outcome {
    Result(Arc<Value>),
    Failed(Arc<str>),
    Cancelled,
}

impl Outcome {
    fn result(self) -> Result<Value, McpAppViewError> {
        match self {
            Self::Result(value) => Ok((*value).clone()),
            Self::Failed(message) => Err(McpAppViewError::Rejected(message.to_string())),
            Self::Cancelled => Err(McpAppViewError::Rejected(
                "The MCP server task was cancelled.".into(),
            )),
        }
    }
}

pub struct McpAppServerTask {
    id: Uuid,
    pub(super) view: Uuid,
    pub(super) session: i64,
    pub(super) expires: Instant,
    protocol: McpAppServerTaskProtocol,
    legacy_result_requested: CancellationToken,
    port: McpAppServerTaskPort,
    outcome: watch::Sender<Option<Outcome>>,
}

struct LegacyAppTaskView {
    view: Weak<View>,
    task_id: String,
    expires: Instant,
    result_requested: CancellationToken,
}

#[async_trait::async_trait]
impl McpAppLegacyTaskView for LegacyAppTaskView {
    async fn request_input(
        &self,
        request: Value,
        cancellation: &CancellationToken,
    ) -> Option<Value> {
        let view = self.view.upgrade()?;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return None,
            _ = view.closed.cancelled() => return None,
            result = tokio::time::timeout_at(self.expires, self.result_requested.cancelled()) => result.ok()?,
        }
        let McpAppToolRequest::Input { method, mut params } =
            serde_json::from_value(request).ok()?
        else {
            return None;
        };
        params.insert(
            "_meta".into(),
            json!({(RELATED_TASK_META): {"taskId":self.task_id}}),
        );
        let request = McpAppToolRequest::Input { method, params };
        match view
            .request(request, self.expires, cancellation, None)
            .await
            .ok()?
        {
            ViewResponse::Frame(McpAppToolReply::Result { result }) => Some(result),
            _ => None,
        }
    }
}

impl McpAppServerTask {
    fn wire_id(&self) -> String {
        format!("{PROXY_ID_PREFIX}{}", self.id)
    }

    pub fn port(&self) -> &McpAppServerTaskPort {
        &self.port
    }

    pub(super) fn has_handle(&self) -> bool {
        self.port.initial().is_some()
    }

    pub async fn run(&self, work: impl Future<Output = Result<Value, String>>) {
        tokio::pin!(work);
        let result = tokio::select! {
            biased;
            result = &mut work => result,
            _ = tokio::time::sleep_until(self.expires) => {
                self.port.cancellation().cancel();
                work.await
            }
        };
        self.finish(result);
    }

    fn finish(&self, result: Result<Value, String>) {
        let outcome = match result {
            Ok(mut value) => {
                if let Some(id) = value.pointer_mut(RELATED_TASK_ID) {
                    *id = Value::String(self.wire_id());
                }
                Outcome::Result(Arc::new(value))
            }
            Err(_)
                if self.port.cancellation().is_cancelled()
                    || self
                        .port
                        .progress()
                        .borrow()
                        .as_ref()
                        .is_some_and(|task| task["status"] == "cancelled") =>
            {
                Outcome::Cancelled
            }
            Err(message) => Outcome::Failed(Arc::from(message)),
        };
        self.port.finish_view_inputs();
        self.outcome.send_replace(Some(outcome));
    }

    pub(super) fn is_finished(&self) -> bool {
        self.outcome.borrow().is_some()
    }

    pub async fn initial_reply(&self) -> Result<Value, McpAppViewError> {
        let mut progress = self.port.progress();
        let mut outcome = self.outcome.subscribe();
        let wait = async {
            loop {
                if let Some(task) = self.port.initial() {
                    let info = self.public_info(task);
                    return Ok(if self.protocol.extension() {
                        let mut info = info;
                        info["resultType"] = json!("task");
                        info
                    } else {
                        json!({"task": info})
                    });
                }
                if let Some(outcome) = outcome.borrow_and_update().clone() {
                    return outcome.result();
                }
                tokio::select! {
                    _ = progress.changed() => {},
                    _ = outcome.changed() => {},
                }
            }
        };
        tokio::select! {
            biased;
            _ = self.port.cancellation().cancelled() => Err(McpAppViewError::Cancelled),
            result = tokio::time::timeout_at(self.expires, wait) => result.map_err(|_| McpAppViewError::Timeout)?,
        }
    }

    pub fn status(&self) -> Value {
        let progress = self.port.progress();
        let current = progress.borrow();
        let source = current
            .as_ref()
            .or_else(|| self.port.initial())
            .expect("a published task has a seed");
        let mut info = self.public_info(source);
        if self.protocol.extension() {
            info["resultType"] = json!("complete");
        }
        match self.outcome.borrow().as_ref() {
            Some(Outcome::Result(value)) => {
                info["status"] = json!("completed");
                if self.protocol.extension() {
                    info["result"] = (**value).clone();
                }
            }
            Some(Outcome::Failed(message)) => {
                info["status"] = json!("failed");
                if self.protocol.extension() {
                    info["error"] = json!({"code":-32000,"message":message.as_ref()});
                }
            }
            Some(Outcome::Cancelled) => info["status"] = json!("cancelled"),
            None => {
                let inputs = self.port.pending_view_inputs();
                if inputs.is_empty() {
                    info["status"] = json!("working");
                } else {
                    info["status"] = json!("input_required");
                    if self.protocol.extension() {
                        info["inputRequests"] = json!(inputs);
                    }
                }
            }
        }
        info
    }

    pub async fn legacy_result(&self) -> Result<Value, McpAppViewError> {
        if self.protocol.extension() {
            return Err(McpAppViewError::Invalid(
                "Use tasks/get for this MCP App task.".into(),
            ));
        }
        self.legacy_result_requested.cancel();
        if let Some(outcome) = self.outcome.borrow().clone() {
            return outcome.result();
        }
        self.result(self.expires, self.port.cancellation()).await
    }

    pub async fn update(
        self: Arc<Self>,
        inputs: BTreeMap<String, Value>,
    ) -> Result<Value, McpAppViewError> {
        if !self.protocol.extension() {
            return Err(McpAppViewError::Invalid(
                "This MCP App task uses legacy callbacks.".into(),
            ));
        }
        SCHEMA_WORK
            .run(move || self.port.apply_view_inputs(inputs))
            .await
            .map_err(|_| McpAppViewError::Unavailable)?
            .map_err(|message| McpAppViewError::Invalid(message.into()))?;
        Ok(json!({"resultType":"complete"}))
    }

    pub fn request_cancel(&self) -> Result<Value, McpAppViewError> {
        if !self.port.cancel_from_view() && self.port.is_claimed() {
            return Err(McpAppViewError::Unavailable);
        }
        Ok(if self.protocol.extension() {
            json!({"resultType":"complete"})
        } else {
            self.status()
        })
    }

    fn public_info(&self, task: &Value) -> Value {
        let mut public = task.clone();
        public["taskId"] = json!(self.wire_id());
        let max_ttl = SERVER_TASK_LIFETIME.as_millis() as u64;
        public["ttlMs"] = json!(public["ttlMs"].as_u64().unwrap_or(max_ttl).min(max_ttl));
        if !self.protocol.extension()
            && let Some(object) = public.as_object_mut()
        {
            for (modern, legacy) in [("ttlMs", "ttl"), ("pollIntervalMs", "pollInterval")] {
                if let Some(value) = object.remove(modern) {
                    object.insert(legacy.into(), value);
                }
            }
        }
        public
    }

    async fn result(
        &self,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Value, McpAppViewError> {
        let mut result = self.outcome.subscribe();
        let wait = async {
            result
                .wait_for(Option::is_some)
                .await
                .map_err(|_| McpAppViewError::Unavailable)?
                .clone()
                .expect("task is settled")
                .result()
        };
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(McpAppViewError::Cancelled),
            result = tokio::time::timeout_at(deadline.min(self.expires), wait) => result.map_err(|_| McpAppViewError::Timeout)?,
        }
    }

    pub(super) fn claim(self: &Arc<Self>, handoff: Handoff) -> Option<ClaimedTask> {
        (handoff.extension == self.protocol.extension() && self.port.claim(handoff.call))
            .then(|| ClaimedTask(self.clone()))
    }
}

#[derive(Clone, Copy)]
pub(super) struct Handoff {
    pub(super) call: i64,
    pub(super) extension: bool,
}

pub(super) struct ClaimedTask(Arc<McpAppServerTask>);

impl ClaimedTask {
    pub(super) async fn result(
        self,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Value, McpAppViewError> {
        self.0.result(deadline, cancellation).await
    }
}

impl Drop for ClaimedTask {
    fn drop(&mut self) {
        if self.0.outcome.borrow().is_none() {
            self.0.port.cancellation().cancel();
        }
    }
}

pub(super) fn reply_task_id(result: &Value) -> Option<Result<Uuid, McpAppViewError>> {
    let info = if result["resultType"] == "task" {
        Some(result)
    } else {
        super::tasks::legacy_handle(result)
    }?;
    info["taskId"]
        .as_str()?
        .strip_prefix(PROXY_ID_PREFIX)
        .map(|id| id.parse().map_err(|_| McpAppViewError::InvalidResult))
}

impl McpAppViews {
    pub fn reserve_server_task(
        &self,
        binding: &McpAppViewBinding,
        view_id: Uuid,
        protocol: McpAppServerTaskProtocol,
    ) -> Result<Arc<McpAppServerTask>, McpAppViewError> {
        let mut registry = self.0.lock();
        registry.prune();
        let view = registry.bound_view(binding, view_id)?;
        if protocol.extension() && !view.task_extension {
            return Err(McpAppViewError::Invalid(
                "The App has not negotiated the MCP Tasks extension.".into(),
            ));
        }
        let state = view.live_state()?;
        if registry.server_tasks.len() >= MAX_SERVER_TASKS
            || registry
                .server_tasks
                .values()
                .filter(|task| task.view == view_id)
                .count()
                >= MAX_VIEW_SERVER_TASKS
        {
            return Err(McpAppViewError::Capacity);
        }
        let legacy_ttl_ms = match protocol {
            McpAppServerTaskProtocol::Legacy { ttl_ms } => ttl_ms,
            McpAppServerTaskProtocol::Extension => None,
        };
        let id = Uuid::new_v4();
        let expires = Instant::now() + SERVER_TASK_LIFETIME;
        let legacy_result_requested = CancellationToken::new();
        let legacy_view = (!protocol.extension()).then(|| {
            Arc::new(LegacyAppTaskView {
                view: Arc::downgrade(&view),
                task_id: format!("{PROXY_ID_PREFIX}{id}"),
                expires,
                result_requested: legacy_result_requested.clone(),
            }) as Arc<dyn McpAppLegacyTaskView>
        });
        let task = Arc::new(McpAppServerTask {
            id,
            view: view_id,
            session: binding.session_id,
            expires,
            protocol,
            legacy_result_requested,
            port: McpAppServerTaskPort::new(
                legacy_ttl_ms,
                state
                    .pending
                    .values()
                    .any(|pending| pending.handoff.is_some()),
                legacy_view,
            ),
            outcome: watch::channel(None).0,
        });
        registry.server_tasks.insert(id, task.clone());
        Ok(task)
    }

    pub fn server_task(
        &self,
        binding: &McpAppViewBinding,
        view_id: Uuid,
        wire_id: &str,
    ) -> Result<Arc<McpAppServerTask>, McpAppViewError> {
        let mut registry = self.0.lock();
        registry.prune();
        drop(registry.bound_view(binding, view_id)?.live_state()?);
        let id = wire_id
            .strip_prefix(PROXY_ID_PREFIX)
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or(McpAppViewError::Unavailable)?;
        registry
            .server_tasks
            .get(&id)
            .filter(|task| task.view == view_id && task.has_handle())
            .cloned()
            .ok_or(McpAppViewError::Unavailable)
    }
}
