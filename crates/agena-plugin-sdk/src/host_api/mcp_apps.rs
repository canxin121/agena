use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};

use agena_domain::mcp_app_tools::{MCP_APP_TOOL_MESSAGE_LIMIT, McpAppTaskInput, McpAppViewScope};
use serde_json::Value;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{PluginError, Result, ToolInvokeInput};

/// In-process authority resolved from an authenticated session's tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpAppHostBinding {
    pub source_call_id: i64,
    pub reference: String,
    pub catalogs: McpAppCatalogWatches,
    pub server_task: Option<McpAppServerTaskPort>,
    pub view: Option<McpAppViewScope>,
}

/// The original server invocation publishes task metadata here while it owns
/// the transport, including after the browser has returned the task to Agena.
#[derive(Clone)]
pub struct McpAppServerTaskPort(Arc<McpAppServerTaskState>);

#[async_trait::async_trait]
pub trait McpAppLegacyTaskView: Send + Sync {
    async fn request_input(
        &self,
        request: Value,
        cancellation: &CancellationToken,
    ) -> Option<Value>;
}

struct McpAppServerTaskState {
    legacy_ttl_ms: Option<u64>,
    legacy_view: Option<Arc<dyn McpAppLegacyTaskView>>,
    may_handoff: bool,
    initial: OnceLock<Value>,
    progress: watch::Sender<Option<Value>>,
    inputs: watch::Sender<TaskInputs>,
    cancellation: CancellationToken,
}

#[derive(Default)]
struct TaskInputs {
    parent: Option<i64>,
    finished: bool,
    pending: BTreeMap<String, PendingInput>,
    accepted: BTreeMap<String, AcceptedInput>,
}

struct PendingInput {
    server_id: String,
    input: McpAppTaskInput,
}

struct AcceptedInput {
    request: Value,
    response: Value,
}

impl fmt::Debug for McpAppServerTaskPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpAppServerTaskPort")
            .finish_non_exhaustive()
    }
}

impl PartialEq for McpAppServerTaskPort {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for McpAppServerTaskPort {}

impl McpAppServerTaskPort {
    pub fn new(
        legacy_ttl_ms: Option<u64>,
        may_handoff: bool,
        legacy_view: Option<Arc<dyn McpAppLegacyTaskView>>,
    ) -> Self {
        Self(Arc::new(McpAppServerTaskState {
            legacy_ttl_ms,
            legacy_view,
            may_handoff,
            initial: OnceLock::new(),
            progress: watch::channel(None).0,
            inputs: watch::channel(TaskInputs::default()).0,
            cancellation: CancellationToken::new(),
        }))
    }

    pub fn legacy_ttl_ms(&self) -> Option<u64> {
        self.0.legacy_ttl_ms
    }

    pub fn observe(&self, task: Value) {
        self.0.initial.get_or_init(|| task.clone());
        if task["status"] != "input_required" {
            self.0.inputs.send_if_modified(|inputs| {
                if inputs.pending.is_empty() {
                    return false;
                }
                inputs.pending.clear();
                true
            });
        }
        self.0.progress.send_replace(Some(task));
    }

    pub fn initial(&self) -> Option<&Value> {
        self.0.initial.get()
    }

    pub fn progress(&self) -> watch::Receiver<Option<Value>> {
        self.0.progress.subscribe()
    }

    pub fn cancellation(&self) -> &CancellationToken {
        &self.0.cancellation
    }

    pub fn claim(&self, call: i64) -> bool {
        self.initial().is_some()
            && self.0.inputs.send_if_modified(|inputs| {
                if inputs.parent.is_some() || self.cancellation().is_cancelled() {
                    return false;
                }
                inputs.parent = Some(call);
                inputs.pending.clear();
                true
            })
    }

    pub fn is_claimed(&self) -> bool {
        self.0.inputs.borrow().parent.is_some()
    }

    pub async fn parent_tool(&self, cancellation: &CancellationToken) -> Option<i64> {
        let mut inputs = self.0.inputs.subscribe();
        if let Some(call) = inputs.borrow_and_update().parent {
            return Some(call);
        }
        if !self.0.may_handoff || self.initial().is_none() {
            return None;
        }
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => None,
            _ = self.cancellation().cancelled() => None,
            owner = inputs.wait_for(|inputs| inputs.parent.is_some()) => owner.ok().and_then(|inputs| inputs.parent),
        }
    }

    pub fn pending_view_inputs(&self) -> BTreeMap<String, Value> {
        let inputs = self.0.inputs.borrow();
        if self.cancellation().is_cancelled() {
            return BTreeMap::new();
        }
        inputs
            .pending
            .iter()
            .map(|(id, pending)| (id.clone(), pending.input.request.clone()))
            .collect()
    }

    pub fn finish_view_inputs(&self) {
        self.0.inputs.send_if_modified(|inputs| {
            if inputs.finished {
                return false;
            }
            inputs.finished = true;
            inputs.pending.clear();
            inputs.accepted.clear();
            true
        });
    }

    pub fn clear_view_inputs(&self) {
        self.0.inputs.send_if_modified(|inputs| {
            if inputs.pending.is_empty() && inputs.accepted.is_empty() {
                return false;
            }
            inputs.pending.clear();
            inputs.accepted.clear();
            true
        });
    }

    pub fn cancel_from_view(&self) -> bool {
        self.0.inputs.send_if_modified(|inputs| {
            if inputs.parent.is_some() || inputs.finished {
                return false;
            }
            self.cancellation().cancel();
            inputs.pending.clear();
            inputs.accepted.clear();
            true
        })
    }

    /// Accept only the outstanding IDs on this task. Replayed or stale IDs
    /// are harmless. A handoff and an update acquire the same ownership lock.
    pub fn apply_view_inputs(
        &self,
        responses: BTreeMap<String, Value>,
    ) -> std::result::Result<(), &'static str> {
        let mut result = Ok(());
        self.0.inputs.send_if_modified(|inputs| {
            if inputs.parent.is_some() || self.cancellation().is_cancelled() {
                result = Err("The MCP App cannot update this task.");
                return false;
            }
            let matched = responses.into_iter().filter_map(|(id, value)| {
                let pending = inputs.pending.get(&id)?;
                Some(
                    pending
                        .input
                        .validate_response(value)
                        .map(|response| (id, response)),
                )
            });
            let Some(matched) = matched.collect::<Option<Vec<_>>>() else {
                result = Err("An MCP task response does not match its input request.");
                return false;
            };
            if matched.is_empty() {
                return false;
            }
            // Parts of one input round can arrive in multiple HTTP requests.
            let combined: BTreeMap<_, _> = inputs
                .accepted
                .iter()
                .map(|(id, input)| (id, &input.response))
                .chain(
                    matched
                        .iter()
                        .map(|(id, value)| (&inputs.pending[id].server_id, value)),
                )
                .collect();
            if !serde_json::to_vec(&combined)
                .is_ok_and(|bytes| bytes.len() <= MCP_APP_TOOL_MESSAGE_LIMIT)
            {
                result = Err("The pending MCP task input responses exceed their size limit.");
                return false;
            }
            for (id, response) in matched {
                let pending = inputs.pending.remove(&id).expect("matched pending request");
                inputs.accepted.insert(
                    pending.server_id,
                    AcceptedInput {
                        request: pending.input.request,
                        response,
                    },
                );
            }
            true
        });
        result
    }

    /// Supply only the inputs already answered. The MCP task driver forwards
    /// a partial update immediately and polls for the remaining requests.
    pub async fn wait_for_view_inputs(
        &self,
        mut requests: BTreeMap<String, McpAppTaskInput>,
        cancellation: &CancellationToken,
    ) -> Option<BTreeMap<String, Value>> {
        if self.initial().is_none() || requests.is_empty() {
            return None;
        }
        self.0.inputs.send_if_modified(|inputs| {
            if inputs.finished || self.cancellation().is_cancelled() {
                return false;
            }
            // Retain answers submitted during the preceding MCP update.
            inputs.accepted.retain(|id, accepted| {
                requests
                    .get(id)
                    .is_some_and(|pending| pending.request == accepted.request)
            });
            requests.retain(|id, _| !inputs.accepted.contains_key(id));
            if inputs.parent.is_none() {
                // Keep public IDs stable while a request remains outstanding.
                // The server's IDs stay on its original connection.
                inputs.pending.retain(|_, pending| {
                    if requests
                        .get(&pending.server_id)
                        .is_some_and(|input| input.request == pending.input.request)
                    {
                        requests.remove(&pending.server_id);
                        true
                    } else {
                        false
                    }
                });
                inputs
                    .pending
                    .extend(requests.into_iter().map(|(server_id, input)| {
                        (
                            Uuid::new_v4().to_string(),
                            PendingInput { server_id, input },
                        )
                    }));
            }
            true
        });
        let mut updates = self.0.inputs.subscribe();
        let ready = tokio::select! {
            biased;
            _ = cancellation.cancelled() => false,
            _ = self.cancellation().cancelled() => false,
            changed = updates.wait_for(|inputs| inputs.finished || inputs.parent.is_some() || !inputs.accepted.is_empty()) => changed.is_ok(),
            delivered = self.request_legacy_input(cancellation), if self.0.legacy_view.is_some() => delivered.is_some(),
        };
        if !ready {
            self.clear_view_inputs();
            return None;
        }
        let mut ready = None;
        self.0.inputs.send_if_modified(|inputs| {
            if inputs.accepted.is_empty() {
                return false;
            }
            ready = Some(
                std::mem::take(&mut inputs.accepted)
                    .into_iter()
                    .map(|(id, accepted)| (id, accepted.response))
                    .collect(),
            );
            true
        });
        ready
    }

    async fn request_legacy_input(&self, cancellation: &CancellationToken) -> Option<()> {
        let legacy_view = self.0.legacy_view.as_ref()?;
        let (id, request) = {
            let inputs = self.0.inputs.borrow();
            let (id, pending) = inputs.pending.first_key_value()?;
            (id.clone(), pending.input.request.clone())
        };
        let answer = legacy_view.request_input(request, cancellation).await?;
        self.apply_view_inputs(BTreeMap::from([(id, answer)])).ok()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpAppCatalogWatches {
    pub tools: Option<McpAppCatalogWatch>,
    pub resources: Option<McpAppCatalogWatch>,
}

/// The catalog returned by an authorized App lookup.
#[derive(Debug, Clone)]
pub struct McpAppCatalogSnapshot {
    pub revision: String,
    pub changes: Option<(watch::Receiver<u64>, CancellationToken)>,
    /// The Streamable HTTP receive operation already waited under this call.
    pub waited: bool,
}

#[derive(Debug, Clone, Default)]
pub struct McpAppCatalogWatch(Arc<McpAppCatalogWatchState>);

#[derive(Debug, Default)]
struct McpAppCatalogWatchState {
    snapshot: OnceLock<McpAppCatalogSnapshot>,
    wait: Option<(String, Duration)>,
}

impl PartialEq for McpAppCatalogWatch {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for McpAppCatalogWatch {}

impl McpAppCatalogWatch {
    pub fn wait_after(revision: String, timeout: Duration) -> Self {
        Self(Arc::new(McpAppCatalogWatchState {
            wait: Some((revision, timeout)),
            ..Default::default()
        }))
    }

    pub fn requested_wait(&self) -> Option<(&str, Duration)> {
        self.0
            .wait
            .as_ref()
            .map(|(revision, timeout)| (revision.as_str(), *timeout))
    }

    pub fn publish(&self, snapshot: McpAppCatalogSnapshot) {
        let _ = self.0.snapshot.set(snapshot);
    }

    pub fn snapshot(&self) -> Option<McpAppCatalogSnapshot> {
        self.0.snapshot.get().cloned()
    }
}

pub struct McpAppHostRequest {
    pub session_id: i64,
    pub tool_name: &'static str,
    pub input: serde_json::Value,
    pub binding: McpAppHostBinding,
}

tokio::task_local! {
    static MCP_APP_REQUEST: McpAppHostRequest;
}

pub async fn scope_mcp_app_request<F: Future>(request: McpAppHostRequest, work: F) -> F::Output {
    MCP_APP_REQUEST.scope(request, work).await
}

/// The App HTTP handler has reserved a catalog wait slot for this internal
/// read. It runs without occupying an agent's model or tool execution slot.
pub fn mcp_app_resource_catalog_wait() -> bool {
    MCP_APP_REQUEST
        .try_with(|request| {
            request.tool_name == "resources.list"
                && request
                    .binding
                    .catalogs
                    .resources
                    .as_ref()
                    .is_some_and(|watch| watch.requested_wait().is_some())
        })
        .unwrap_or(false)
}

pub fn mcp_app_server_task() -> Option<McpAppServerTaskPort> {
    MCP_APP_REQUEST
        .try_with(|request| request.binding.server_task.clone())
        .ok()
        .flatten()
}

pub fn mcp_app_request_binding(
    plugin_id: &str,
    input: &ToolInvokeInput,
) -> Result<Option<McpAppHostBinding>> {
    MCP_APP_REQUEST
        .try_with(|request| {
            if plugin_id == "agena.mcp"
                && input.session_id == request.session_id
                && input.call_id < 0
                && input.tool_name == request.tool_name
                && input.input == request.input
            {
                Ok(Some(request.binding.clone()))
            } else {
                Err(PluginError::invalid_params(
                    "The MCP App invocation changed.",
                ))
            }
        })
        .unwrap_or(Ok(None))
}
