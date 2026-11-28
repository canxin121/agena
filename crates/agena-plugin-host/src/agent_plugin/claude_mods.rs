//! Claude function hooks run in one isolated ES module context per Agena session.

mod http;
mod modules;
mod tools;

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agena_plugin_sdk::{
    CommandInvokeInput, CommandResult, HostClient, PluginKey, SessionStartInput,
    host_api::{
        HostCommandRegisterRequest, HostStorageDeleteRequest, HostStorageGetRequest,
        HostStorageListRequest, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
        ImportedHookTranscripts, LogLevel, current_host_callback_context,
    },
};
use rquickjs::{CatchResultExt, Context, Function, Object};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use url::Url;

use self::modules::{ModuleSources, PackageModules};
pub(crate) use self::tools::CLAUDE_MOD_TOOL_TAG;
use super::hooks::{ModProcessOptions, run_mod_process};
use crate::host::ImportedPromptScope;
pub(super) use modules::is_module;

const BRIDGE: &str = include_str!("claude_mods/bridge.js");
pub(super) const MAX_ENTRIES: usize = 16;
pub(crate) const COMMAND_HANDLER: &str = "claude_mod";
const JS_BUDGET: Duration = Duration::from_secs(2);
const MAX_SESSIONS: usize = 256;
const MAX_IN_FLIGHT: usize = 32;
const MAX_MESSAGES: usize = 128;
const MAX_FRAME_BYTES: usize = 1024 * 1024;
static WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(12);

tokio::task_local! {
    static STARTING_MODS: i64;
}

#[derive(Clone)]
pub(super) struct Bundle {
    sources: Arc<ModuleSources>,
    entries: Arc<[String]>,
    events: BTreeSet<String>,
    /// Per exact event name (e.g. `tool.offer`, `tool.describe`), `Some(set)` if
    /// every matching registration narrows on an exact string `tool` matcher, or
    /// `None` if any matching registration is a wildcard.
    tool_filters: BTreeMap<String, Option<BTreeSet<String>>>,
    options: Value,
}

#[derive(Deserialize)]
struct RegisteredHook {
    event: String,
    #[serde(default)]
    tool: Option<String>,
}

impl Bundle {
    pub(super) fn discover(
        root: &Path,
        paths: &[PathBuf],
        options: Value,
        watch: &mut BTreeSet<PathBuf>,
        diagnostics: &mut Vec<String>,
    ) -> Option<Self> {
        if paths.is_empty() {
            return None;
        }
        let sources = Rc::new(RefCell::new(BTreeMap::new()));
        let loader = PackageModules::Discover {
            root: root.to_owned(),
            sources: sources.clone(),
        };
        let vm = match Vm::new(loader.clone()) {
            Ok(vm) => vm,
            Err(error) => {
                diagnostics.push(format!("Claude mod runtime: {error}"));
                return None;
            }
        };
        let mut events = BTreeSet::new();
        let mut registrations = Vec::new();
        let mut entries = Vec::new();
        for path in paths {
            let result = (|| {
                let url = Url::from_file_path(path).map_err(|()| "invalid hook module path")?;
                let source = loader.source(url.as_str())?;
                let hooks = vm.register(url.as_str(), &source, &options)?;
                Ok::<_, String>((url, hooks))
            })();
            match result {
                Ok((url, hooks)) => {
                    for hook in hooks {
                        events.insert(hook.event.clone());
                        registrations.push(hook);
                    }
                    entries.push(url.into());
                }
                Err(error) => diagnostics.push(format!("Claude mod {}: {error}", path.display())),
            }
        }
        let mut tool_filters = BTreeMap::new();
        for event in ["tool.offer", "tool.describe"] {
            let mut matched_any = false;
            let mut exact = Some(BTreeSet::new());
            for hook in &registrations {
                let matches_event = hook.event == event
                    || hook.event == "*"
                    || hook.event.strip_suffix(".*").is_some_and(|prefix| {
                        event
                            .strip_prefix(prefix)
                            .is_some_and(|suffix| suffix.starts_with('.'))
                    });
                if !matches_event {
                    continue;
                }
                matched_any = true;
                match (&mut exact, &hook.tool) {
                    (Some(set), Some(tool)) => {
                        set.insert(tool.clone());
                    }
                    _ => {
                        exact = None;
                    }
                }
            }
            if matched_any {
                tool_filters.insert(event.to_owned(), exact);
            }
        }
        let sources = std::mem::take(&mut *sources.borrow_mut());
        watch.extend(
            sources
                .keys()
                .filter_map(|name| Url::parse(name).ok()?.to_file_path().ok()),
        );
        (!entries.is_empty()).then(|| Self {
            sources: Arc::new(sources),
            entries: entries.into(),
            events,
            tool_filters,
            options,
        })
    }

    pub(super) fn events(&self) -> &BTreeSet<String> {
        &self.events
    }

    pub(super) fn matches(&self, event: &str) -> bool {
        self.events.iter().any(|name| {
            name == event
                || name == "*"
                || name.strip_suffix(".*").is_some_and(|prefix| {
                    event
                        .strip_prefix(prefix)
                        .is_some_and(|suffix| suffix.starts_with('.'))
                })
        })
    }

    pub(super) fn matches_tool(&self, event: &str, tool: &str) -> bool {
        match self.tool_filters.get(event) {
            Some(Some(names)) => names.contains(tool),
            Some(None) => true,
            None => self.matches(event),
        }
    }

    fn new_session(&self) -> Result<Vm, String> {
        let loader = PackageModules::Snapshot(self.sources.clone());
        let vm = Vm::new(loader.clone())?;
        for entry in self.entries.iter() {
            vm.register(entry, &loader.source(entry)?, &self.options)?;
        }
        Ok(vm)
    }
}

pub(super) fn options(
    config: Option<&Value>,
    environment: &BTreeMap<String, String>,
    diagnostics: &mut Vec<String>,
) -> Value {
    let mut options = Map::new();
    for (name, definition) in config.and_then(Value::as_object).into_iter().flatten() {
        if let Some(default) = definition.get("default")
            && definition["sensitive"] != true
        {
            options.insert(name.clone(), default.clone());
        }
        let Some(variable) = environment.get(name) else {
            continue;
        };
        let value = super::mcp::read_var(variable).ok().and_then(|value| {
            if definition["multiple"] == true {
                let values: Vec<String> = serde_json::from_str(&value).ok()?;
                return Some(json!(values));
            }
            match definition["type"].as_str() {
                Some("boolean") => value.parse::<bool>().ok().map(Value::Bool),
                Some("number") => value.parse::<serde_json::Number>().ok().map(Value::Number),
                _ => Some(Value::String(value)),
            }
        });
        if let Some(value) = value {
            options.insert(name.clone(), value);
        } else {
            diagnostics.push(format!(
                "Claude mod option {name:?} could not be read from {variable}"
            ));
        }
    }
    Value::Object(options)
}

struct SessionVm {
    vm: Mutex<Option<Vm>>,
    started: tokio::sync::Mutex<bool>,
    started_fast: std::sync::atomic::AtomicBool,
    tools: tokio::sync::Mutex<BTreeMap<String, Option<String>>>,
    frames: Semaphore,
    closed_frames: Mutex<Vec<String>>,
}

impl Default for SessionVm {
    fn default() -> Self {
        Self {
            vm: Mutex::default(),
            started: tokio::sync::Mutex::default(),
            started_fast: std::sync::atomic::AtomicBool::new(false),
            tools: tokio::sync::Mutex::default(),
            frames: Semaphore::new(MAX_IN_FLIGHT),
            closed_frames: Mutex::default(),
        }
    }
}

struct FrameLease<'a> {
    session: &'a SessionVm,
    id: &'a str,
}

impl Drop for FrameLease<'_> {
    fn drop(&mut self) {
        self.session
            .closed_frames
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(self.id.to_owned());
    }
}

pub(crate) struct Mods {
    bundle: Arc<Bundle>,
    key: PluginKey,
    name: String,
    root: PathBuf,
    environment: BTreeMap<String, String>,
    static_commands: BTreeSet<String>,
    host: OnceLock<Arc<dyn HostClient>>,
    sessions: Mutex<BTreeMap<i64, Arc<SessionVm>>>,
    store_lock: tokio::sync::Mutex<()>,
}

impl Mods {
    pub(super) fn new(
        bundle: Bundle,
        key: &PluginKey,
        name: &str,
        root: &Path,
        environment: &[String],
        static_commands: BTreeSet<String>,
    ) -> Self {
        Self {
            bundle: Arc::new(bundle),
            key: key.clone(),
            name: name.into(),
            root: root.into(),
            environment: environment
                .iter()
                .filter_map(|name| {
                    super::mcp::read_var(name)
                        .ok()
                        .map(|value| (name.clone(), value))
                })
                .collect(),
            static_commands,
            host: OnceLock::new(),
            sessions: Mutex::default(),
            store_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub(crate) fn matches(&self, event: &str) -> bool {
        self.bundle.matches(event)
    }

    pub(crate) fn matches_tool(&self, event: &str, tool: &str) -> bool {
        self.bundle.matches_tool(event, tool)
    }

    pub(super) fn bind_host(&self, host: Arc<dyn HostClient>) {
        let _ = self.host.set(host);
    }

    pub(crate) fn host(&self) -> Option<&dyn HostClient> {
        self.host.get().map(Arc::as_ref)
    }

    pub(super) fn close(&self, session: i64) {
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&session);
    }

    pub(crate) async fn start(&self, host: &dyn HostClient, input: &SessionStartInput) {
        if !self.matches("session.start")
            || input.session_id <= 0
            || input.workspace_root.is_empty()
            || STARTING_MODS.try_with(|id| *id == input.session_id).unwrap_or(false)
        {
            return;
        }
        let Some(slot) = self.session(input.session_id) else {
            return;
        };
        if slot.started_fast.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let mut started = if ImportedPromptScope::current().from_plugin() {
            // A plugin may submit while its own or another start callback is
            // still waiting on the submission. The module is already loaded.
            let Ok(started) = slot.started.try_lock() else {
                return;
            };
            started
        } else {
            slot.started.lock().await
        };
        if *started {
            return;
        }
        STARTING_MODS.scope(input.session_id, self.run(
            host,
            input.session_id,
            "session.start",
            json!({
                "cwd":input.workspace_root, "surface":null, "isInteractive":false,
                "model":input.model,
            }),
            |event| async move { json!({"cwd":event["cwd"]}) },
        ))
        .await;
        *started = true;
        slot.started_fast
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub(super) async fn end(&self, host: &dyn HostClient, session: i64, reason: &str) {
        let session_id = session.to_string();
        self.run(
            host,
            session,
            "session.end",
            json!({
                "reason":reason, "sessionId":session_id, "resume":{"id":session_id},
            }),
            |_| async move { json!({"sessionId":session_id}) },
        )
        .await;
    }

    fn session(&self, id: i64) -> Option<Arc<SessionVm>> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(session) = sessions.get(&id) {
            return Some(session.clone());
        }
        if sessions.len() >= MAX_SESSIONS {
            return None;
        }
        let session = Arc::new(SessionVm::default());
        sessions.insert(id, session.clone());
        Some(session)
    }

    pub(crate) async fn run<F, Fut>(
        &self,
        host: &dyn HostClient,
        session: i64,
        event: &str,
        input: Value,
        terminal: F,
    ) -> Value
    where
        F: FnOnce(Value) -> Fut + Send,
        Fut: Future<Output = Value> + Send,
    {
        Box::pin(self.run_inner(host, session, event, input, None, None, terminal)).await
    }

    pub(crate) async fn run_with_prompt_media<F, Fut>(
        &self,
        host: &dyn HostClient,
        session: i64,
        event: &str,
        input: Value,
        media: Option<&crate::host::ImportedPromptMediaReader<'_>>,
        terminal: F,
    ) -> Value
    where
        F: FnOnce(Value) -> Fut + Send,
        Fut: Future<Output = Value> + Send,
    {
        Box::pin(self.run_inner(host, session, event, input, media, None, terminal)).await
    }

    pub(super) async fn run_with_transcripts<F, Fut>(
        &self,
        host: &dyn HostClient,
        session: i64,
        event: &str,
        input: Value,
        transcripts: Option<&ImportedHookTranscripts>,
        terminal: F,
    ) -> Value
    where
        F: FnOnce(Value) -> Fut + Send,
        Fut: Future<Output = Value> + Send,
    {
        Box::pin(self.run_inner(host, session, event, input, None, transcripts, terminal)).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_inner<F, Fut>(
        &self,
        host: &dyn HostClient,
        session: i64,
        event: &str,
        input: Value,
        media: Option<&crate::host::ImportedPromptMediaReader<'_>>,
        transcripts: Option<&ImportedHookTranscripts>,
        terminal: F,
    ) -> Value
    where
        F: FnOnce(Value) -> Fut + Send,
        Fut: Future<Output = Value> + Send,
    {
        let context = current_host_callback_context().unwrap_or_default();
        if session <= 0 || !self.matches(event) {
            return terminal(input).await;
        }
        let Some(slot) = self.session(session) else {
            host.log(
                LogLevel::Warn,
                "Claude mod session limit reached".into(),
                json!({"event":event}),
            )
            .await;
            return terminal(input).await;
        };
        let Ok(_permit) = slot.frames.try_acquire() else {
            host.log(
                LogLevel::Warn,
                "Claude mod event limit reached".into(),
                json!({"event":event}),
            )
            .await;
            return terminal(input).await;
        };
        let cancel = agena_plugin_sdk::current_cancellation_token()
            .unwrap_or_default()
            .child_token();
        let _stop = cancel.clone().drop_guard();
        let frame = Frame::new(
            self,
            session,
            event,
            &input,
            context.workspace_root.as_deref(),
            event == "tool.run"
                || (event == "tool.call"
                    && context.tool_name.as_deref() == input["tool"].as_str()
                    && input["tool"].is_string()),
        );
        let id = frame.id.clone();
        let _frame = FrameLease {
            session: &slot,
            id: &id,
        };
        let mut update = Update::Begin {
            frame: Box::new(frame),
            input: input.clone(),
        };
        let mut remaining = JS_BUDGET;
        let mut terminal = Some(terminal);
        let mut answer = None;
        let result = loop {
            let worker_slot = slot.clone();
            let bundle = self.bundle.clone();
            let worker_id = id.clone();
            let worker_cancel = cancel.clone();
            let step = WORKERS
                .run(move || {
                    let mut vm = worker_slot.vm.lock().unwrap_or_else(|poison| poison.into_inner());
                    if worker_cancel.is_cancelled() {
                        return Err("Claude mod callback was cancelled".into());
                    }
                    let closed = std::mem::take(
                        &mut *worker_slot
                            .closed_frames
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner()),
                    );
                    if vm.is_none() {
                        *vm = Some(bundle.new_session()?);
                    }
                    let vm = vm.as_ref().expect("session context created");
                    vm.step(&worker_id, closed, update, remaining, worker_cancel)
                })
                .await;
            let (step, elapsed) = match step {
                Ok(Ok(step)) => step,
                Ok(Err(error)) => break Err(error),
                Err(error) => break Err(error.to_string()),
            };
            remaining = remaining.saturating_sub(elapsed);
            let mut replies = Vec::with_capacity(step.requests.len());
            for Request { id, call } in step.requests {
                let result = match call {
                    Call::Next { event: changed } => {
                        if let Some(next) = terminal.take() {
                            let next_value = next(changed).await;
                            answer = Some(next_value.clone());
                            Ok(next_value)
                        } else {
                            Ok(answer.clone().unwrap_or_else(|| json!({})))
                        }
                    }
                    Call::PromptMedia {
                        attachment_id,
                        offset,
                    } => Ok(match media {
                        Some(reader) if event == "prompt.submit" && terminal.is_some() => {
                            reader(attachment_id, offset).await
                        }
                        _ => json!({"deny":"This hook has no open prompt attachment"}),
                    }),
                    Call::SubmitPrompt { .. }
                        if matches!(event, "session.end" | "classic.SessionEnd") =>
                    {
                        Ok(json!({"drop":"The session is ending"}))
                    }
                    Call::SubmitPrompt { hook, request } => {
                        let scope = ImportedPromptScope::current().with_caller(
                            self.key.clone(),
                            session,
                            hook,
                        );
                        Ok(match scope {
                            Some(scope) => {
                                match scope.run(host.submit_imported_plugin_prompt(request)).await {
                                    Ok(answer) => answer,
                                    Err(error) => json!({"drop":error.failure.user.fallback}),
                                }
                            }
                            None => json!({"drop":"Too many nested plugin prompt submissions"}),
                        })
                    }
                    Call::RegisterTool { definition }
                        if context.session_id == Some(session)
                            && !matches!(event, "session.end" | "classic.SessionEnd") =>
                    {
                        self.register_tool(host, &slot, definition).await
                    }
                    Call::RegisterTool { .. } => Err("Claude mod tool registration requires an active project session".into()),
                    Call::ToolOperation { hook, request } => {
                        let scope = ImportedPromptScope::current().with_caller(
                            self.key.clone(), session, hook,
                        );
                        match scope {
                            Some(scope) => scope.run(host.imported_mod_tool(request)).await
                                .map_err(|error| error.failure.user.fallback),
                            None => Err("too many nested Claude mod operations".into()),
                        }
                    }
                    call => Box::pin(self.host_call(host, event, call, transcripts, &cancel)).await,
                };
                replies.push(match result {
                    Ok(value) => json!({"id":id,"value":value}),
                    Err(error) => json!({"id":id,"error":error}),
                });
            }
            if let Some(error) = step.error {
                break Err(error);
            }
            if let Some(answer) = step.answer {
                break Ok(answer);
            }
            if remaining.is_zero() {
                break Err("Claude mod timed out".into());
            }
            if replies.is_empty() {
                break Err("Claude mod is waiting for work outside its callback".into());
            }
            // A continuation may itself call function hooks from this
            // package. Release the VM while Rust performs those calls.
            update = Update::Resume(replies);
        };
        match result {
            Ok(result) => result,
            Err(error) => {
                host.log(
                    LogLevel::Warn,
                    bounded(&error, 1024),
                    json!({"event": event}),
                )
                .await;
                match terminal {
                    Some(next) => next(input).await,
                    None => answer.unwrap_or_else(|| json!({})),
                }
            }
        }
    }

    async fn host_call(
        &self,
        host: &dyn HostClient,
        event: &str,
        call: Call,
        transcripts: Option<&ImportedHookTranscripts>,
        cancel: &CancellationToken,
    ) -> Result<Value, String> {
        let error = |error: agena_plugin_sdk::PluginError| error.failure.user.fallback.clone();
        const STORAGE: &str = "claude-mod";
        match call {
            Call::Log { text, level } => {
                let level = match level.as_deref() {
                    Some("warn") => LogLevel::Warn,
                    Some("error") => LogLevel::Error,
                    Some("debug") => LogLevel::Debug,
                    _ => LogLevel::Info,
                };
                host.log(level, bounded(&text, 4096), json!({"event":event}))
                    .await;
                Ok(Value::Null)
            }
            Call::HookError { source, text } => {
                host.log(
                    LogLevel::Warn,
                    bounded(&text, 1024),
                    json!({"event":event,"source":bounded(&source, 512)}),
                )
                .await;
                Ok(Value::Null)
            }
            Call::RegisterCommand { definition } if event == "session.start" => {
                let id = definition.name;
                if !valid_command_name(&id)
                    || definition.aliases.len() > 8
                    || !definition
                        .aliases
                        .iter()
                        .all(|alias| valid_command_name(alias))
                {
                    return Err("invalid Claude mod command name".into());
                }
                if self.static_commands.contains(&id) {
                    return Err(format!("the plugin already declares command {id:?}"));
                }
                let mut command = super::markdown::plugin_command_definition(
                    &self.key,
                    &id,
                    Some(definition.description),
                    definition.argument_hint,
                    COMMAND_HANDLER,
                );
                command.aliases = definition.aliases;
                command
                    .aliases
                    .extend(command.slash.replace(format!("/{id}")));
                host.register_command(HostCommandRegisterRequest { command })
                    .await
                    .map_err(error)?;
                Ok(json!({"command":id}))
            }
            Call::RegisterCommand { .. } => {
                Err("register a Claude mod command during session.start".into())
            }
            Call::StoreGet { key, version } => {
                let result = host
                    .storage_get(HostStorageGetRequest {
                        scope: HostStorageScope::Global,
                        visibility: HostStorageVisibility::Private,
                        namespace: STORAGE.into(),
                        key,
                    })
                    .await
                    .map_err(error)?;
                match result.value {
                    Some(raw) => {
                        let (ver, value) = decode_store_record(&raw)?;
                        if version {
                            Ok(json!({"found": true, "value": value, "version": ver}))
                        } else {
                            Ok(json!({"found": true, "value": value}))
                        }
                    }
                    None => Ok(json!({"found": false})),
                }
            }
            Call::StoreSet {
                key,
                value,
                if_version,
            } => {
                let _guard = self.store_lock.lock().await;
                let existing = host
                    .storage_get(HostStorageGetRequest {
                        scope: HostStorageScope::Global,
                        visibility: HostStorageVisibility::Private,
                        namespace: STORAGE.into(),
                        key: key.clone(),
                    })
                    .await
                    .map_err(error)?;
                let current_version = match existing.value {
                    Some(raw) => Some(decode_store_record(&raw)?.0),
                    None => None,
                };
                if let Some(expected) = if_version {
                    let matched = match (expected, current_version) {
                        (0, None) => true,
                        (expected, Some(current)) if expected > 0 => current == expected,
                        _ => false,
                    };
                    if !matched {
                        return Ok(json!({
                            "ok": false,
                            "version": current_version.unwrap_or(0),
                        }));
                    }
                }
                let next_version = current_version.unwrap_or(0).saturating_add(1);
                let encoded = encode_store_record(next_version, &value)?;
                host.storage_set(HostStorageSetRequest {
                    scope: HostStorageScope::Global,
                    visibility: HostStorageVisibility::Private,
                    namespace: STORAGE.into(),
                    key,
                    value: encoded,
                })
                .await
                .map_err(error)?;
                Ok(json!({"ok": true, "version": next_version}))
            }
            Call::StoreDelete { key, if_version } => {
                let _guard = self.store_lock.lock().await;
                if let Some(expected) = if_version {
                    let existing = host
                        .storage_get(HostStorageGetRequest {
                            scope: HostStorageScope::Global,
                            visibility: HostStorageVisibility::Private,
                            namespace: STORAGE.into(),
                            key: key.clone(),
                        })
                        .await
                        .map_err(error)?;
                    let current_version = match existing.value {
                        Some(raw) => Some(decode_store_record(&raw)?.0),
                        None => None,
                    };
                    if current_version != Some(expected) {
                        return Ok(json!({
                            "ok": false,
                            "version": current_version.unwrap_or(0),
                        }));
                    }
                }
                host.storage_delete(HostStorageDeleteRequest {
                    scope: HostStorageScope::Global,
                    visibility: HostStorageVisibility::Private,
                    namespace: STORAGE.into(),
                    key,
                })
                .await
                .map_err(error)?;
                Ok(json!({"ok": true}))
            }
            Call::StoreKeys => {
                let list = host
                    .storage_list(HostStorageListRequest {
                        scope: HostStorageScope::Global,
                        visibility: HostStorageVisibility::Private,
                        namespace: Some(STORAGE.into()),
                        prefix: None,
                    })
                    .await
                    .map_err(error)?;
                Ok(json!(
                    list.records
                        .into_iter()
                        .map(|record| record.key)
                        .collect::<Vec<_>>()
                ))
            }
            Call::ProcessRun { argv, options } => {
                run_mod_process(host, argv, options, transcripts, cancel.clone()).await
            }
            Call::HttpFetch { url, init } => Box::pin(http::fetch(host, url, init, cancel.clone())).await,
            Call::Next { .. } | Call::PromptMedia { .. } | Call::SubmitPrompt { .. }
            | Call::RegisterTool { .. } | Call::ToolOperation { .. } => {
                Err("unexpected event callback".into())
            }
        }
    }

    pub(super) async fn command(
        &self,
        host: &dyn HostClient,
        input: &CommandInvokeInput,
    ) -> CommandResult {
        let Some(session) = input.session_id.filter(|session| *session > 0) else {
            return CommandResult::unavailable("This command requires a project session");
        };
        let args = input.input["args"].as_str().unwrap_or_default();
        let output = self
            .run(
                host,
                session,
                "command.run",
                json!({
                    "command":input.command_id, "args":args,
                    "origin":{"kind":"composer"},
                    "presentation":{"isFullscreen":false,"columns":80},
                }),
                |_| async { json!({}) },
            )
            .await;
        let text = bounded(output["text"].as_str().unwrap_or_default(), 8192);
        let notes = output["context"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let shown = (output["isTextDisplayOnly"] != true).then_some(text.as_str());
        if let Some(context) = joined_context(
            shown
                .into_iter()
                .chain(notes.iter().filter_map(Value::as_str)),
        ) && let Ok(binding) = host.bind_imported_plugin_background().await
        {
            let _ = binding.notices.deliver(context, false).await;
        }
        CommandResult::succeeded(text)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
    id: String,
    session_id: String,
    event: String,
    plugin: String,
    plugin_root: PathBuf,
    cwd: String,
    workspace: String,
    model: Option<String>,
    env: BTreeMap<String, String>,
    budget: u64,
    deadline: u64,
    max_requests: usize,
    skip_hook: Option<usize>,
    pin_input: bool,
}

impl Frame {
    fn new(
        mods: &Mods, session: i64, event: &str, input: &Value, workspace: Option<&str>,
        pin_input: bool,
    ) -> Self {
        let workspace = workspace
            .or_else(|| input["cwd"].as_str())
            .unwrap_or_default();
        let cwd = if matches!(event, "tool.call" | "classic.PreToolUse") {
            workspace
        } else {
            input["cwd"].as_str().unwrap_or(workspace)
        };
        let budget = JS_BUDGET.as_millis() as u64;
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session.to_string(),
            event: event.into(),
            plugin: mods.name.clone(),
            plugin_root: mods.root.clone(),
            cwd: cwd.into(),
            workspace: workspace.into(),
            model: input["model"].as_str().map(str::to_owned),
            env: mods.environment.clone(),
            budget,
            deadline: millis_now() + budget,
            max_requests: MAX_MESSAGES,
            skip_hook: ImportedPromptScope::current().skip_hook(&mods.key, session),
            pin_input,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Call {
    Next {
        event: Value,
    },
    PromptMedia {
        attachment_id: String,
        offset: u64,
    },
    SubmitPrompt {
        hook: usize,
        request: agena_plugin_sdk::host_api::ImportedPluginPromptRequest,
    },
    Log {
        text: String,
        level: Option<String>,
    },
    HookError {
        source: String,
        text: String,
    },
    RegisterCommand {
        definition: CommandSpec,
    },
    RegisterTool {
        definition: tools::ToolSpec,
    },
    ToolOperation {
        hook: usize,
        request: agena_plugin_sdk::host_api::ImportedModToolRequest,
    },
    StoreGet {
        key: String,
        #[serde(default)]
        version: bool,
    },
    StoreSet {
        key: String,
        value: Value,
        #[serde(default)]
        if_version: Option<u64>,
    },
    StoreDelete {
        key: String,
        #[serde(default)]
        if_version: Option<u64>,
    },
    StoreKeys,
    ProcessRun {
        argv: Vec<String>,
        #[serde(default)]
        options: ModProcessOptions,
    },
    HttpFetch {
        url: String,
        #[serde(default)]
        init: http::HttpInit,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandSpec {
    name: String,
    description: String,
    argument_hint: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
}

#[derive(Deserialize)]
struct Request {
    id: u32,
    #[serde(flatten)]
    call: Call,
}

#[derive(Deserialize)]
struct Step {
    answer: Option<Value>,
    error: Option<String>,
    requests: Vec<Request>,
}

enum Update {
    Begin { frame: Box<Frame>, input: Value },
    Resume(Vec<Value>),
}

struct Budget {
    deadline: Instant,
    cancel: CancellationToken,
}

struct Vm {
    context: Context,
    active: Arc<Mutex<Option<Budget>>>,
}

impl Vm {
    fn new(loader: PackageModules) -> Result<Self, String> {
        let active = Arc::new(Mutex::new(None::<Budget>));
        let interrupt = active.clone();
        let runtime = super::deepseek_js::create_runtime(move || {
            let active = interrupt
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            active.as_ref().is_some_and(|budget| {
                Instant::now() >= budget.deadline || budget.cancel.is_cancelled()
            })
        })?;
        runtime.set_loader(loader.clone(), loader);
        let context = Context::full(&runtime).map_err(|error| error.to_string())?;
        context.with(|ctx| {
            let api: Object = ctx
                .eval(BRIDGE)
                .catch(&ctx)
                .map_err(|error| error.to_string())?;
            ctx.globals()
                .set("__agenaClaudeMod", api)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(())
        })?;
        Ok(Self { context, active })
    }

    fn register(&self, url: &str, source: &str, options: &Value) -> Result<Vec<RegisteredHook>, String> {
        self.with_budget(JS_BUDGET, CancellationToken::new(), || {
            self.context.with(|ctx| {
                let error = |error: rquickjs::CaughtError<'_>| error.to_string();
                let module = modules::declare(&ctx, url, source)
                    .catch(&ctx)
                    .map_err(error)?;
                let (module, ready) = module.eval().catch(&ctx).map_err(error)?;
                ready.finish::<()>().catch(&ctx).map_err(error)?;
                let bridge: Object = ctx
                    .globals()
                    .get("__agenaClaudeMod")
                    .map_err(|error| error.to_string())?;
                let register: Function =
                    bridge.get("register").map_err(|error| error.to_string())?;
                let options = ctx
                    .json_parse(options.to_string())
                    .catch(&ctx)
                    .map_err(error)?;
                let output: rquickjs::Value = register
                    .call((module.namespace().catch(&ctx).map_err(error)?, url, options))
                    .catch(&ctx)
                    .map_err(error)?;
                let encoded = ctx
                    .json_stringify(output)
                    .catch(&ctx)
                    .map_err(error)?
                    .ok_or("Claude mod register returned non-JSON hooks")?
                    .to_string()
                    .map_err(|error| error.to_string())?;
                serde_json::from_str(&encoded).map_err(|error| error.to_string())
            })
        })
    }

    fn step(
        &self,
        id: &str,
        closed: Vec<String>,
        update: Update,
        remaining: Duration,
        cancel: CancellationToken,
    ) -> Result<(Step, Duration), String> {
        let started = Instant::now();
        let deadline = started + remaining;
        let step = self.with_budget(remaining, cancel, || {
            self.context.with(|ctx| {
                let error = |error: rquickjs::CaughtError<'_>| error.to_string();
                let bridge: Object = ctx
                    .globals()
                    .get("__agenaClaudeMod")
                    .map_err(|error| error.to_string())?;
                let close: Function = bridge.get("close").map_err(|error| error.to_string())?;
                for closed_id in closed {
                    close
                        .call::<_, ()>((closed_id,))
                        .catch(&ctx)
                        .map_err(error)?;
                }
                match update {
                    Update::Begin { mut frame, input } => {
                        let begin: Function =
                            bridge.get("begin").map_err(|error| error.to_string())?;
                        frame.deadline = millis_now() + remaining.as_millis() as u64;
                        let frame = ctx
                            .json_parse(encode_frame(&frame)?)
                            .catch(&ctx)
                            .map_err(error)?;
                        let event = ctx
                            .json_parse(encode_frame(&input)?)
                            .catch(&ctx)
                            .map_err(error)?;
                        begin
                            .call::<_, ()>((frame, event))
                            .catch(&ctx)
                            .map_err(error)?;
                    }
                    Update::Resume(replies) => {
                        let resume: Function =
                            bridge.get("resume").map_err(|error| error.to_string())?;
                        let replies = ctx
                            .json_parse(encode_frame(&replies)?)
                            .catch(&ctx)
                            .map_err(error)?;
                        let deadline = millis_now() + remaining.as_millis() as u64;
                        resume
                            .call::<_, ()>((id, replies, deadline))
                            .catch(&ctx)
                            .map_err(error)?;
                    }
                }
                while Instant::now() < deadline && ctx.execute_pending_job() {}
                if Instant::now() >= deadline {
                    return Err("Claude mod timed out".into());
                }
                let poll: Function = bridge.get("poll").map_err(|error| error.to_string())?;
                let output: rquickjs::Value = poll.call((id,)).catch(&ctx).map_err(error)?;
                let output = ctx
                    .json_stringify(output)
                    .catch(&ctx)
                    .map_err(error)?
                    .ok_or("Claude mod output is missing")?
                    .to_string()
                    .map_err(|error| error.to_string())?;
                if output.len() > MAX_FRAME_BYTES {
                    return Err("Claude mod output exceeds its byte limit".into());
                }
                serde_json::from_str(&output).map_err(|error| error.to_string())
            })
        })?;
        Ok((step, started.elapsed()))
    }

    fn with_budget<T>(
        &self,
        remaining: Duration,
        cancel: CancellationToken,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let state = || {
            self.active
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
        };
        *state() = Some(Budget {
            deadline: Instant::now() + remaining,
            cancel,
        });
        let result = action();
        *state() = None;
        result
    }
}

fn encode_frame(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let encoded = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err("Claude mod input exceeds its byte limit".into());
    }
    Ok(encoded)
}

fn millis_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn bounded(value: &str, max: usize) -> String {
    value[..value.floor_char_boundary(value.len().min(max))].to_owned()
}

fn valid_command_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

const STORE_VERSION_FIELD: &str = "__agena_mod_store_v";

fn decode_store_record(raw: &str) -> Result<(u64, Value), String> {
    let parsed: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    if let Value::Object(mut map) = parsed {
        if map.len() == 2
            && let Some(version) = map
                .get(STORE_VERSION_FIELD)
                .and_then(Value::as_u64)
                .filter(|version| *version >= 1)
            && map.contains_key("value")
        {
            let value = map.remove("value").unwrap_or(Value::Null);
            return Ok((version, value));
        }
        return Ok((1, Value::Object(map)));
    }
    Ok((1, parsed))
}

fn encode_store_record(version: u64, value: &Value) -> Result<String, String> {
    serde_json::to_string(&json!({
        STORE_VERSION_FIELD: version,
        "value": value,
    }))
    .map_err(|error| error.to_string())
}

fn joined_context<'a>(context: impl Iterator<Item = &'a str>) -> Option<String> {
    let context = context
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!context.is_empty()).then(|| bounded(&context, 16 * 1024))
}
