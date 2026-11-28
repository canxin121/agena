//! In-process ownership of tool catalogs and calls for open browser views.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::Arc,
    time::Duration,
};

use agena_domain::mcp_app_tools::{
    MCP_APP_TOOL_MESSAGE_LIMIT, McpAppPrompt, McpAppToolCall, McpAppToolCalls, McpAppToolDefinition,
    McpAppToolReply, McpAppToolRequest, McpAppToolResult, McpAppViewBinding, McpAppViewScope,
};
use agena_mcp_client::McpElicitationHandler;
use agena_plugin_host::sdk::schema_validation::CompiledSchema;
use parking_lot::Mutex;
use serde_json::{Map, Value};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, oneshot, watch},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

mod server_tasks;
mod tasks;

use server_tasks::{ClaimedTask, Handoff};
pub use server_tasks::{McpAppServerTask, McpAppServerTaskProtocol};

const MAX_VIEWS: usize = 64;
const MAX_SESSION_VIEWS: usize = 8;
const MAX_VIEW_TOOLS: usize = 50;
const MAX_VIEW_CALLS: usize = 4;
const VIEW_LEASE: Duration = Duration::from_secs(40);
const POLL_WAIT: Duration = Duration::from_secs(12);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const APP_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
static SCHEMA_WORK: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

#[derive(Debug, thiserror::Error)]
pub enum McpAppViewError {
    #[error("The MCP App view or tool is no longer available.")]
    Unavailable,
    #[error("The MCP App view or session is at capacity.")]
    Capacity,
    #[error("{0}")]
    Invalid(String),
    #[error("The MCP App tool did not answer before its deadline.")]
    Timeout,
    #[error("The MCP App tool call was cancelled.")]
    Cancelled,
    #[error("The MCP App tool returned an invalid result.")]
    InvalidResult,
    #[error("The MCP App tool failed: {0}")]
    Rejected(String),
}

#[derive(Default)]
pub struct McpAppViews(Mutex<Registry>);

#[derive(Default)]
struct Registry {
    generation: u64,
    shutdown: bool,
    views: BTreeMap<Uuid, Arc<View>>,
    server_tasks: BTreeMap<Uuid, Arc<McpAppServerTask>>,
}

struct View {
    binding: McpAppViewBinding,
    name: String,
    task_extension: bool,
    state: Mutex<ViewState>,
    changes: watch::Sender<u64>,
    closed: CancellationToken,
    guest_requests: Arc<Semaphore>,
}

struct ViewState {
    deadline: Instant,
    connected: bool,
    revision: u64,
    tools: BTreeMap<String, Arc<PublishedTool>>,
    active_calls: usize,
    task_ids: BTreeSet<String>,
    pending: BTreeMap<Uuid, Pending>,
}

struct Pending {
    call: McpAppToolCall,
    handoff: Option<Handoff>,
    reply: oneshot::Sender<ViewResponse>,
}

enum ViewResponse {
    Frame(McpAppToolReply),
    ServerTask(ClaimedTask),
}

impl ViewResponse {
    fn value(self) -> Result<Value, McpAppViewError> {
        match self {
            Self::Frame(McpAppToolReply::Result { result }) => Ok(result),
            Self::Frame(McpAppToolReply::Error { message }) => Err(McpAppViewError::Rejected(
                message.chars().take(512).collect(),
            )),
            Self::Frame(McpAppToolReply::Input { .. }) | Self::ServerTask(_) => {
                Err(McpAppViewError::InvalidResult)
            }
        }
    }
}

pub struct McpAppViewOperation {
    view: Arc<View>,
    _permit: OwnedSemaphorePermit,
}

impl McpAppViewOperation {
    pub fn task_extension(&self) -> bool {
        self.view.task_extension
    }

    pub fn cancellation(&self) -> &CancellationToken {
        &self.view.closed
    }

    pub async fn run<T>(&self, work: impl Future<Output = T>) -> Result<T, McpAppViewError> {
        tokio::select! {
            biased;
            _ = self.cancellation().cancelled() => Err(McpAppViewError::Unavailable),
            result = tokio::time::timeout(APP_REQUEST_TIMEOUT, work) => result.map_err(|_| McpAppViewError::Timeout),
        }
    }
}

struct PendingCall {
    view: Arc<View>,
    id: Uuid,
}

impl Drop for PendingCall {
    fn drop(&mut self) {
        self.view.remove_call(self.id);
    }
}

pub(crate) struct PublishedTool {
    pub(crate) id: Uuid,
    pub(crate) definition: McpAppToolDefinition,
    input: CompiledSchema,
    output: Option<CompiledSchema>,
}

impl PublishedTool {
    fn compile(definition: McpAppToolDefinition) -> Result<Self, McpAppViewError> {
        let invalid = || McpAppViewError::Invalid("Invalid MCP App tool definition.".into());
        if definition.name.is_empty()
            || definition.name.len() > 128
            || !definition
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
            || definition
                .title
                .as_ref()
                .is_some_and(|title| title.len() > 512)
            || definition
                .description
                .as_ref()
                .is_some_and(|text| text.len() > 8192)
            || !definition.input_schema.is_object()
            || definition
                .output_schema
                .as_ref()
                .is_some_and(|schema| !schema.is_object())
        {
            return Err(invalid());
        }
        let input = CompiledSchema::new(&definition.input_schema).map_err(|_| invalid())?;
        let output = definition
            .output_schema
            .as_ref()
            .map(CompiledSchema::new)
            .transpose()
            .map_err(|_| invalid())?;
        Ok(Self {
            id: Uuid::new_v4(),
            definition,
            input,
            output,
        })
    }

    pub(crate) fn alias(&self, part: i64) -> String {
        format!(
            "mcp_app__{part}__{}__{}",
            self.definition.name,
            self.id.simple()
        )
    }
}

async fn compile_tools(
    definitions: Vec<McpAppToolDefinition>,
) -> Result<BTreeMap<String, Arc<PublishedTool>>, McpAppViewError> {
    if definitions.len() > MAX_VIEW_TOOLS {
        return Err(McpAppViewError::Invalid(
            "An MCP App can publish up to 50 tools.".into(),
        ));
    }
    SCHEMA_WORK
        .run(move || {
            bounded_message(&definitions)?;
            let mut tools = BTreeMap::new();
            for definition in definitions {
                let tool = Arc::new(PublishedTool::compile(definition)?);
                if tools.insert(tool.definition.name.clone(), tool).is_some() {
                    return Err(McpAppViewError::Invalid(
                        "The MCP App has duplicate tool names.".into(),
                    ));
                }
            }
            Ok(tools)
        })
        .await
        .map_err(|_| McpAppViewError::Unavailable)?
}

impl McpAppViews {
    /// Capture the runtime generation before authorizing an App's source.
    pub fn admission_generation(&self) -> u64 {
        self.0.lock().generation
    }

    pub async fn open(
        &self,
        generation: u64,
        binding: McpAppViewBinding,
        name: String,
        definitions: Vec<McpAppToolDefinition>,
        task_extension: bool,
    ) -> Result<Uuid, McpAppViewError> {
        let tools = compile_tools(definitions).await?;
        let mut registry = self.0.lock();
        registry.prune();
        if registry.shutdown || registry.generation != generation {
            return Err(McpAppViewError::Unavailable);
        }
        let session_views = registry
            .views
            .values()
            .filter(|view| view.binding.session_id == binding.session_id);
        if registry.views.len() >= MAX_VIEWS || session_views.count() >= MAX_SESSION_VIEWS {
            return Err(McpAppViewError::Capacity);
        }
        let id = Uuid::new_v4();
        registry.views.insert(
            id,
            Arc::new(View {
                binding,
                name,
                task_extension,
                state: Mutex::new(ViewState {
                    deadline: Instant::now() + VIEW_LEASE,
                    connected: false,
                    revision: 0,
                    tools,
                    active_calls: 0,
                    task_ids: BTreeSet::new(),
                    pending: BTreeMap::new(),
                }),
                changes: watch::channel(0).0,
                closed: CancellationToken::new(),
                guest_requests: Arc::new(Semaphore::new(MAX_VIEW_CALLS)),
            }),
        );
        Ok(id)
    }

    pub fn begin_request(
        &self,
        scope: &McpAppViewScope,
    ) -> Result<McpAppViewOperation, McpAppViewError> {
        let view = self.get(&scope.binding, scope.view)?;
        drop(view.live_state()?);
        let permit = view.guest_requests.clone().try_acquire_owned()
            .map_err(|_| McpAppViewError::Capacity)?;
        Ok(McpAppViewOperation { view, _permit: permit })
    }

    pub(crate) fn view_cancellation(
        &self,
        scope: &McpAppViewScope,
    ) -> Result<CancellationToken, McpAppViewError> {
        let view = self.get(&scope.binding, scope.view)?;
        drop(view.live_state()?);
        Ok(view.closed.clone())
    }

    pub(crate) fn view_name(
        &self,
        scope: &McpAppViewScope,
    ) -> Result<String, McpAppViewError> {
        let view = self.get(&scope.binding, scope.view)?;
        drop(view.live_state()?);
        Ok(view.name.clone())
    }

    pub(crate) async fn prompt(
        &self,
        scope: &McpAppViewScope,
        prompt: McpAppPrompt,
        cancellation: &CancellationToken,
    ) -> Result<Option<String>, McpAppViewError> {
        bounded_message(&prompt)?;
        let view = self.get(&scope.binding, scope.view)?;
        let reply = view.request(
            McpAppToolRequest::Prompt { prompt },
            Instant::now() + APP_REQUEST_TIMEOUT,
            cancellation,
            None,
        ).await?;
        match reply {
            ViewResponse::Frame(McpAppToolReply::Input { answer }) => Ok(answer),
            _ => Err(McpAppViewError::InvalidResult),
        }
    }

    pub async fn publish(
        &self,
        binding: &McpAppViewBinding,
        view: Uuid,
        definitions: Vec<McpAppToolDefinition>,
    ) -> Result<(), McpAppViewError> {
        let view = self.get(binding, view)?;
        let mut tools = compile_tools(definitions).await?;
        let mut state = view.live_state()?;
        // Preserve the help contract for definitions which were present and
        // identical in both snapshots.
        for (name, published) in &mut tools {
            if let Some(previous) = state.tools.get(name)
                && previous.definition == published.definition
            {
                *published = previous.clone();
            }
        }
        state.tools = tools;
        Ok(())
    }

    pub async fn poll(
        &self,
        binding: &McpAppViewBinding,
        view: Uuid,
        after: Option<u64>,
    ) -> Result<McpAppToolCalls, McpAppViewError> {
        let view = self.get(binding, view)?;
        let mut changes = view.changes.subscribe();
        {
            let mut state = view.live_state()?;
            state.connected = true;
            state.deadline = Instant::now() + VIEW_LEASE;
            if after != Some(state.revision) {
                return Ok(state.calls());
            }
        }
        tokio::select! {
            biased;
            _ = view.closed.cancelled() => return Err(McpAppViewError::Unavailable),
            changed = changes.changed() => changed.map_err(|_| McpAppViewError::Unavailable)?,
            _ = tokio::time::sleep(POLL_WAIT) => {},
        }
        Ok(view.live_state()?.calls())
    }

    pub fn reply(
        &self,
        binding: &McpAppViewBinding,
        view_id: Uuid,
        id: Uuid,
        reply: McpAppToolReply,
    ) -> Result<(), McpAppViewError> {
        bounded_message(&reply)?;
        let mut registry = self.0.lock();
        registry.prune();
        let view = registry.bound_view(binding, view_id)?;
        let mut state = view.live_state()?;
        let Some(pending) = state.pending.get(&id) else {
            // Retrying an already delivered reply or a cancelled request is harmless.
            return Ok(());
        };
        match (&pending.call.request, &reply) {
            (McpAppToolRequest::Prompt { prompt }, McpAppToolReply::Input { answer })
                if answer.as_ref().is_none_or(|answer| valid_prompt_answer(prompt, answer)) => {}
            (McpAppToolRequest::Prompt { .. }, _) | (_, McpAppToolReply::Input { .. }) => {
                return Err(McpAppViewError::Invalid("Invalid MCP App answer.".into()));
            }
            _ => {}
        }
        let pending = state.pending.remove(&id).expect("pending reply");
        view.signal(&mut state);
        let handle = match &reply {
            McpAppToolReply::Result { result } => server_tasks::reply_task_id(result),
            McpAppToolReply::Error { .. } | McpAppToolReply::Input { .. } => None,
        };
        let response = if let Some(handle) = handle {
            let claim = handle.ok().and_then(|task_id| {
                let task = registry.server_tasks.get(&task_id)?;
                let handoff = pending.handoff?;
                (task.view == view_id)
                    .then(|| task.claim(handoff))
                    .flatten()
            });
            let Some(claim) = claim else {
                let _ = pending
                    .reply
                    .send(ViewResponse::Frame(McpAppToolReply::Error {
                        message: "The MCP server task is unavailable to this App tool call.".into(),
                    }));
                return Err(McpAppViewError::InvalidResult);
            };
            ViewResponse::ServerTask(claim)
        } else {
            ViewResponse::Frame(reply)
        };
        let _ = pending.reply.send(response);
        Ok(())
    }

    pub fn close(&self, binding: &McpAppViewBinding, id: Uuid) {
        let mut registry = self.0.lock();
        if registry
            .views
            .get(&id)
            .is_some_and(|view| view.binding == *binding)
            && let Some(view) = registry.views.remove(&id)
        {
            view.closed.cancel();
            registry.prune_server_tasks();
        }
    }

    pub(crate) fn end_session(&self, session_id: i64) {
        let mut registry = self.0.lock();
        registry.generation = registry.generation.wrapping_add(1);
        registry.retain_views(|view| view.binding.session_id != session_id);
        registry.retain_server_tasks(|task| task.session != session_id);
    }

    pub(crate) fn reset(&self) {
        let mut registry = self.0.lock();
        registry.generation = registry.generation.wrapping_add(1);
        registry.retain_views(|_| false);
        registry.retain_server_tasks(|_| false);
    }

    pub(crate) fn shutdown(&self) {
        let mut registry = self.0.lock();
        registry.shutdown = true;
        registry.retain_views(|_| false);
        registry.retain_server_tasks(|_| false);
    }

    pub(crate) fn prune(&self) {
        self.0.lock().prune();
    }

    pub(crate) fn has_tools(&self, session_id: i64) -> bool {
        let mut registry = self.0.lock();
        registry.prune();
        registry.views.values().any(|view| {
            if view.binding.session_id != session_id {
                return false;
            }
            let state = view.state.lock();
            state.connected && !state.tools.is_empty()
        })
    }

    pub(crate) fn tools(&self, session_id: i64) -> Vec<(i64, String, Arc<PublishedTool>)> {
        let mut registry = self.0.lock();
        registry.prune();
        let mut tools = Vec::new();
        for view in registry
            .views
            .values()
            .filter(|view| view.binding.session_id == session_id)
        {
            let state = view.state.lock();
            if state.connected {
                tools.extend(
                    state
                        .tools
                        .values()
                        .map(|tool| (view.binding.part_id, view.name.clone(), tool.clone())),
                );
            }
        }
        tools
    }

    pub(crate) async fn call(
        &self,
        session_id: i64,
        model_call: i64,
        target: Uuid,
        arguments: Map<String, Value>,
        cancellation: &CancellationToken,
        interactions: &dyn McpElicitationHandler,
    ) -> Result<McpAppToolResult, McpAppViewError> {
        let (view, tool) = self.find_tool(session_id, target)?;
        let contract = tool.clone();
        let arguments = SCHEMA_WORK
            .run(move || {
                bounded_message(&arguments)?;
                let value = Value::Object(arguments);
                contract.input.validate(&value).map_err(|_| {
                    McpAppViewError::Invalid(
                        "Arguments do not match the MCP App tool's input schema.".into(),
                    )
                })?;
                Ok::<_, McpAppViewError>(value)
            })
            .await
            .map_err(|_| McpAppViewError::Unavailable)??;
        let Value::Object(arguments) = arguments else {
            unreachable!("validated object")
        };

        let name = tool.definition.name.clone();
        {
            let mut state = view.live_state()?;
            if cancellation.is_cancelled() {
                return Err(McpAppViewError::Cancelled);
            }
            if !state.tools.get(&name).is_some_and(|live| live.id == target) {
                return Err(McpAppViewError::Unavailable);
            }
            if state.active_calls >= MAX_VIEW_CALLS {
                return Err(McpAppViewError::Capacity);
            }
            state.active_calls += 1;
        }
        let result = tasks::RunningCall::new(view.clone())
            .run(
                model_call,
                &tool.definition,
                arguments,
                cancellation,
                interactions,
            )
            .await?;
        let result = SCHEMA_WORK
            .run(move || {
                if !result.is_object() || tasks::is_intermediate(&result) {
                    return Err(McpAppViewError::InvalidResult);
                }
                bounded_message(&result)?;
                if let Some(output) = &tool.output
                    && result.get("isError").and_then(Value::as_bool) != Some(true)
                {
                    let structured = result
                        .get("structuredContent")
                        .ok_or(McpAppViewError::InvalidResult)?;
                    output
                        .validate(structured)
                        .map_err(|_| McpAppViewError::InvalidResult)?;
                }
                Ok(result)
            })
            .await
            .map_err(|_| McpAppViewError::InvalidResult)??;
        Ok(McpAppToolResult {
            app: view.name.clone(),
            name,
            result,
        })
    }

    fn get(&self, binding: &McpAppViewBinding, id: Uuid) -> Result<Arc<View>, McpAppViewError> {
        let mut registry = self.0.lock();
        registry.prune();
        registry.bound_view(binding, id)
    }

    fn find_tool(
        &self,
        session_id: i64,
        target: Uuid,
    ) -> Result<(Arc<View>, Arc<PublishedTool>), McpAppViewError> {
        let mut registry = self.0.lock();
        registry.prune();
        for view in registry
            .views
            .values()
            .filter(|view| view.binding.session_id == session_id)
        {
            let state = view.state.lock();
            if state.connected
                && let Some(tool) = state.tools.values().find(|tool| tool.id == target)
            {
                return Ok((view.clone(), tool.clone()));
            }
        }
        Err(McpAppViewError::Unavailable)
    }
}

fn valid_prompt_answer(prompt: &McpAppPrompt, answer: &str) -> bool {
    !answer.trim().is_empty()
        && (prompt.question.custom_option_index().is_some()
            || prompt.question.options.iter().any(|option| option.label == answer))
}

impl Registry {
    fn bound_view(
        &self,
        binding: &McpAppViewBinding,
        id: Uuid,
    ) -> Result<Arc<View>, McpAppViewError> {
        self.views
            .get(&id)
            .filter(|view| view.binding == *binding)
            .cloned()
            .ok_or(McpAppViewError::Unavailable)
    }

    fn prune(&mut self) {
        let now = Instant::now();
        self.retain_views(|view| view.state.lock().deadline > now);
        self.prune_server_tasks();
    }

    fn retain_views(&mut self, keep: impl Fn(&View) -> bool) {
        self.views.retain(|_, view| {
            if keep(view) {
                true
            } else {
                view.closed.cancel();
                false
            }
        });
    }

    fn prune_server_tasks(&mut self) {
        let now = Instant::now();
        let views = &self.views;
        self.server_tasks.retain(|_, task| {
            let view_open = views.contains_key(&task.view);
            if task.is_finished() && (!view_open || !task.has_handle()) {
                return false;
            }
            let keep = task.expires > now && (task.port().is_claimed() || view_open);
            if !keep {
                task.port().cancellation().cancel();
            }
            keep
        });
    }

    fn retain_server_tasks(&mut self, keep: impl Fn(&McpAppServerTask) -> bool) {
        self.server_tasks.retain(|_, task| {
            if keep(task) {
                true
            } else {
                task.port().cancellation().cancel();
                false
            }
        });
    }
}

impl View {
    async fn request(
        self: &Arc<Self>,
        request: McpAppToolRequest,
        deadline: Instant,
        cancellation: &CancellationToken,
        handoff: Option<Handoff>,
    ) -> Result<ViewResponse, McpAppViewError> {
        let id = Uuid::new_v4();
        let (send, receive) = oneshot::channel();
        {
            let mut state = self.live_state()?;
            if cancellation.is_cancelled() {
                return Err(McpAppViewError::Cancelled);
            }
            if deadline <= Instant::now() {
                return Err(McpAppViewError::Timeout);
            }
            state.pending.insert(
                id,
                Pending {
                    call: McpAppToolCall { id, request },
                    handoff,
                    reply: send,
                },
            );
            self.signal(&mut state);
        }
        let _pending = PendingCall {
            view: self.clone(),
            id,
        };
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(McpAppViewError::Cancelled),
            // A reply already accepted by the host can transfer the task even
            // if the iframe closes before this waiter runs again.
            result = tokio::time::timeout_at(deadline, receive) => result
                .map_err(|_| McpAppViewError::Timeout)?
                .map_err(|_| McpAppViewError::Unavailable),
            _ = self.closed.cancelled() => Err(McpAppViewError::Unavailable),
        }
    }

    fn live_state(&self) -> Result<parking_lot::MutexGuard<'_, ViewState>, McpAppViewError> {
        let state = self.state.lock();
        if self.closed.is_cancelled() || state.deadline <= Instant::now() {
            return Err(McpAppViewError::Unavailable);
        }
        Ok(state)
    }

    fn signal(&self, state: &mut ViewState) {
        state.revision = state.revision.wrapping_add(1);
        self.changes.send_replace(state.revision);
    }

    fn remove_call(&self, id: Uuid) {
        let mut state = self.state.lock();
        if state.pending.remove(&id).is_some() {
            self.signal(&mut state);
        }
    }
}

impl ViewState {
    fn calls(&self) -> McpAppToolCalls {
        McpAppToolCalls {
            revision: self.revision,
            calls: self
                .pending
                .values()
                .map(|pending| pending.call.clone())
                .collect(),
        }
    }
}

fn bounded_message(value: &impl serde::Serialize) -> Result<(), McpAppViewError> {
    if serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= MCP_APP_TOOL_MESSAGE_LIMIT) {
        Ok(())
    } else {
        Err(McpAppViewError::Invalid(
            "The MCP App message exceeds its size limit.".into(),
        ))
    }
}
