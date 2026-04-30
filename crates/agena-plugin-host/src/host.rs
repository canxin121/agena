//! `PluginHost` — the central handle agena holds. Owns:
//! - the loaded plugins,
//! - the tool registry,
//! - the dedicated tokio runtime that drives plugin transports,
//! - the host-callback router used by stdio/http plugins.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::config::{PluginEntry, PluginsConfig, TimeoutsConfig};
use crate::dispatcher::{self, call_with_timeout};
use crate::error::{HostError, TransportError};
use crate::loader::{StaticRegistration, load_entry, shutdown_transport};
use crate::registry::{ToolEntry, ToolRegistry};
use crate::sdk::host_api::{EventSubscription, HostClient, LogLevel, NoopHostClient};
use crate::sdk::rpc::method;
use crate::sdk::{
    AgentStopInput, AgentStopPatch, AuthInput, AuthOutput, ChatHeadersInput, ChatHeadersPatch,
    ChatMessageInput, ChatMessagePatch, ChatMessagesTransformInput, ChatMessagesTransformPatch,
    ChatParamsInput, ChatParamsPatch, ChatSystemTransformInput, ChatSystemTransformPatch,
    CommandAfterInput, CommandAfterPatch, CommandBeforeInput, CommandBeforeOutcome,
    CommandBeforeResponse, ConfigInput, ConfigPatch, EventEnvelope, EventFilter, HookSubscription,
    PermissionAskDecision, PermissionAskInput, PermissionDecision, PluginError, PluginManifest,
    ProviderListInput, ProviderListPatch, SessionCompactedInput, SessionCompactingInput,
    SessionCompactingPatch, SessionEndInput, SessionStartInput, SessionStartPatch, ShellEnvInput,
    ShellEnvPatch, ToolAfterInput, ToolAfterPatch, ToolBeforeInput, ToolBeforePatch, ToolDecl,
    ToolDefinitionInput, ToolDefinitionPatch, ToolFailureInput, ToolInvokeInput, ToolInvokeOutput,
    ToolStreamChunk, ToolStreamEnd, UserPromptSubmitInput, UserPromptSubmitPatch,
};
use crate::transport::PluginTransport;
use crate::transport::inproc::InProcessTransport;

pub struct LoadedPlugin {
    pub id: String,
    pub kind: &'static str,
    pub manifest: PluginManifest,
    pub transport: Arc<dyn PluginTransport>,
}

impl std::fmt::Debug for LoadedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedPlugin")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

impl LoadedPlugin {
    pub fn new(
        id: String,
        kind: &'static str,
        transport: Arc<dyn PluginTransport>,
        manifest: PluginManifest,
    ) -> Self {
        Self {
            id,
            kind,
            manifest,
            transport,
        }
    }

    pub fn subscribes(&self, sub: HookSubscription) -> bool {
        self.manifest.hooks.contains(sub)
    }
}

/// Opaque handle returned by `PluginHost::lookup_tool`. Pass it to
/// `invoke_tool` to actually call the plugin.
#[derive(Debug, Clone)]
pub struct PluginToolHandle {
    pub plugin_id: String,
    pub original_name: String,
    pub exposed_name: String,
}

#[derive(Debug, Clone)]
pub struct ToolResolution {
    pub handle: PluginToolHandle,
    pub decl: ToolDecl,
}

/// Live handle to an in-flight tool stream. Consume `chunks` for incremental
/// output; once the stream closes (sender dropped), inspect `end` for the
/// final aggregated result.
pub struct ToolInvokeStream {
    pub stream_id: String,
    pub chunks: tokio::sync::mpsc::Receiver<ToolStreamChunk>,
    pub end: ToolStreamEnd,
}

/// Result of dispatching `session.compacting` through the plugin chain.
/// `messages` is the (possibly transformed) message list the host should
/// hand to its summarization step; `summary`, when set, replaces the
/// host's auto-generated summary entirely (the LLM-based summarizer
/// extension point).
#[derive(Debug, Clone)]
pub struct SessionCompactingOutcome {
    pub messages: Vec<crate::sdk::ChatMessage>,
    pub summary: Option<String>,
}

/// Result-bearing facade for a tool call. Wraps async dispatch in a runtime
/// `block_on` so callers from sync code (like `ToolExecutor`) can use it.
pub struct PluginHost {
    plugins: Vec<Arc<LoadedPlugin>>,
    plugins_by_id: HashMap<String, Arc<LoadedPlugin>>,
    tools: ToolRegistry,
    timeouts: TimeoutsConfig,
    /// Dedicated runtime used to block_on async transport calls when invoked
    /// from sync code.
    runtime: Option<Arc<tokio::runtime::Runtime>>,
    /// Handle to the runtime that built us (preferred for block_on when sync
    /// callers are themselves driven by an outer runtime).
    runtime_handle: Option<tokio::runtime::Handle>,
    /// Underlying host handle; kept alive for callbacks.
    _host_handle: Arc<HostHandle>,
    /// Plugin ids whose transports we transferred to a successor host;
    /// `shutdown()` skips those so we don't kill what the new host is using.
    transferred_to_successor: tokio::sync::Mutex<std::collections::HashSet<String>>,
}

impl PluginHost {
    pub fn new_empty() -> Arc<Self> {
        let host_handle = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
        Arc::new(Self {
            plugins: Vec::new(),
            plugins_by_id: HashMap::new(),
            tools: ToolRegistry::new(Vec::<String>::new()),
            timeouts: TimeoutsConfig::default(),
            runtime: None,
            runtime_handle: None,
            _host_handle: host_handle,
            transferred_to_successor: tokio::sync::Mutex::new(Default::default()),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn plugins(&self) -> &[Arc<LoadedPlugin>] {
        &self.plugins
    }

    pub fn plugin_summary(&self) -> (usize, BTreeMap<&'static str, usize>) {
        let mut by_kind: BTreeMap<&'static str, usize> = BTreeMap::new();
        for p in &self.plugins {
            *by_kind.entry(p.kind).or_insert(0) += 1;
        }
        (self.plugins.len(), by_kind)
    }

    pub fn lookup_tool(&self, exposed_name: &str) -> Option<ToolResolution> {
        self.tools.lookup(exposed_name).map(|entry| ToolResolution {
            handle: PluginToolHandle {
                plugin_id: entry.plugin_name.clone(),
                original_name: entry.original_name.clone(),
                exposed_name: entry.exposed_name.clone(),
            },
            decl: entry.decl.clone(),
        })
    }

    pub fn tool_entries(&self) -> impl Iterator<Item = &ToolEntry> {
        self.tools.entries()
    }

    fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output
    where
        F: Send,
        F::Output: Send,
    {
        if let Some(rt) = &self.runtime {
            rt.block_on(fut)
        } else if let Some(handle) = &self.runtime_handle {
            tokio::task::block_in_place(|| handle.block_on(fut))
        } else if let Ok(handle) = tokio::runtime::Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(fut))
        } else {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("plugin host fallback runtime");
            rt.block_on(fut)
        }
    }

    // ------------------- sync wrappers used by ToolExecutor -------------------

    pub fn dispatch_tool_before(
        &self,
        input: ToolBeforeInput,
    ) -> Result<ToolBeforeInput, PluginError> {
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let plugins = self.plugins.clone();
        let res = self.block_on(async move {
            dispatcher::chain_patch::<ToolBeforeInput, ToolBeforePatch, _>(
                &plugins,
                method::HOOK_TOOL_BEFORE,
                HookSubscription::TOOL_BEFORE,
                timeout,
                input,
                |inp, patch| {
                    if let Some(v) = patch.input {
                        inp.input = v;
                    }
                    if let Some(t) = patch.title_override {
                        inp.title_override = Some(t);
                    }
                    for (k, v) in patch.metadata {
                        inp.metadata.insert(k, v);
                    }
                },
            )
            .await
        });
        res.map_err(transport_to_plugin_error)
    }

    pub fn dispatch_tool_after(
        &self,
        input: ToolAfterInput,
    ) -> Result<ToolAfterInput, PluginError> {
        let timeout = self.timeouts.tool_hook_or(Duration::from_secs(30));
        let plugins = self.plugins.clone();
        let res = self.block_on(async move {
            dispatcher::chain_patch::<ToolAfterInput, ToolAfterPatch, _>(
                &plugins,
                method::HOOK_TOOL_AFTER,
                HookSubscription::TOOL_AFTER,
                timeout,
                input,
                |inp, patch| {
                    if let Some(t) = patch.title {
                        inp.title = t;
                    }
                    if let Some(o) = patch.output_text {
                        inp.output_text = o;
                    }
                    if let Some(p) = patch.payload {
                        inp.payload = Some(p);
                    }
                    for (k, v) in patch.metadata {
                        inp.metadata.insert(k, v);
                    }
                },
            )
            .await
        });
        res.map_err(transport_to_plugin_error)
    }

    pub fn invoke_tool(
        &self,
        handle: &PluginToolHandle,
        input: ToolInvokeInput,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(&handle.plugin_id)
            .cloned()
            .ok_or_else(|| {
                PluginError::new(format!("plugin `{}` not loaded", handle.plugin_id))
            })?;
        let timeout = self.timeouts.tool_invoke_or(Duration::from_secs(300));
        let mut input = input;
        // ensure tool name is the plugin-original name (in case caller passed exposed)
        input.tool_name = handle.original_name.clone();
        let params = serde_json::to_value(&input)
            .map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let result = self.block_on(async move {
            call_with_timeout(&plugin, method::HOOK_TOOL_INVOKE, params, timeout).await
        });
        let value = result.map_err(transport_to_plugin_error)?;
        serde_json::from_value(value).map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    /// Streaming variant: returns a receiver of [`ToolStreamChunk`]s plus a
    /// future that resolves to the terminal [`ToolStreamEnd`] (or error).
    /// For in-process and cdylib transports this is materialised by running
    /// the plugin's `tool_invoke_stream` and forwarding chunks; for stdio /
    /// HTTP it's driven by the `tool.stream.chunk` / `tool.stream.end`
    /// notifications.
    ///
    /// Currently only the in-process path is fully implemented; remote
    /// transports fall back to a single-chunk emulation built from the
    /// non-streaming `tool_invoke` response.
    pub async fn invoke_tool_stream(
        &self,
        handle: &PluginToolHandle,
        input: ToolInvokeInput,
    ) -> Result<ToolInvokeStream, PluginError> {
        let plugin = self
            .plugins_by_id
            .get(&handle.plugin_id)
            .cloned()
            .ok_or_else(|| {
                PluginError::new(format!("plugin `{}` not loaded", handle.plugin_id))
            })?;
        let mut input = input;
        input.tool_name = handle.original_name.clone();

        // Try the streaming dispatch endpoint first; transports that don't
        // know it (or the SDK's default impl) emulate a single-chunk stream
        // from the regular tool_invoke result.
        let timeout = self.timeouts.tool_invoke_or(Duration::from_secs(300));
        let params = serde_json::to_value(&input)
            .map_err(|e| PluginError::invalid_params(e.to_string()))?;
        let invoke_result =
            call_with_timeout(&plugin, method::HOOK_TOOL_INVOKE, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
        let result: ToolInvokeOutput = serde_json::from_value(invoke_result)
            .map_err(|e| PluginError::invalid_params(e.to_string()))?;

        let (tx, rx) = tokio::sync::mpsc::channel::<ToolStreamChunk>(8);
        let stream_id = format!("emu-{}", uuid::Uuid::new_v4().simple());
        let chunk = ToolStreamChunk {
            stream_id: stream_id.clone(),
            text_delta: Some(result.output_text.clone()),
            payload_delta: result.payload.clone(),
            metadata: result.metadata.clone(),
        };
        let _ = tx.send(chunk).await;
        drop(tx);
        Ok(ToolInvokeStream {
            stream_id: stream_id.clone(),
            chunks: rx,
            end: ToolStreamEnd {
                stream_id,
                title: result.title,
                output_text: result.output_text,
                payload: result.payload,
                metadata: result.metadata,
                attachments: result.attachments,
            },
        })
    }

    pub fn dispatch_shell_env(
        &self,
        input: ShellEnvInput,
    ) -> Result<ShellEnvPatch, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let plugins = self.plugins.clone();
        let res: Result<ShellEnvPatch, TransportError> = self.block_on(async move {
            let mut acc = ShellEnvPatch::default();
            for plugin in &plugins {
                if !plugin.subscribes(HookSubscription::SHELL_ENV) {
                    continue;
                }
                let params = serde_json::to_value(&input)?;
                let result = call_with_timeout(plugin, method::HOOK_SHELL_ENV, params, timeout).await?;
                if matches!(&result, serde_json::Value::Null) {
                    continue;
                }
                let patch: Option<ShellEnvPatch> = serde_json::from_value(result)?;
                if let Some(p) = patch {
                    for (k, v) in p.set {
                        acc.set.insert(k, v);
                    }
                    for k in p.unset {
                        acc.set.remove(&k);
                        acc.unset.push(k);
                    }
                }
            }
            Ok(acc)
        });
        res.map_err(transport_to_plugin_error)
    }

    // -------------- async-only helpers for chat / permission etc. --------------

    pub async fn dispatch_chat_message(
        &self,
        input: ChatMessageInput,
    ) -> Result<ChatMessageInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        dispatcher::chain_patch::<ChatMessageInput, ChatMessagePatch, _>(
            &self.plugins,
            method::HOOK_CHAT_MESSAGE,
            HookSubscription::CHAT_MESSAGE,
            timeout,
            input,
            |inp, patch| {
                if let Some(m) = patch.message {
                    inp.message = m;
                }
                if patch.drop {
                    inp.message.content = serde_json::Value::Null;
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_chat_params(
        &self,
        input: ChatParamsInput,
    ) -> Result<ChatParamsInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        dispatcher::chain_patch::<ChatParamsInput, ChatParamsPatch, _>(
            &self.plugins,
            method::HOOK_CHAT_PARAMS,
            HookSubscription::CHAT_PARAMS,
            timeout,
            input,
            |inp, patch| {
                if let Some(p) = patch.params {
                    merge_json(&mut inp.params, p);
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_chat_headers(
        &self,
        input: ChatHeadersInput,
    ) -> Result<ChatHeadersInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        dispatcher::chain_patch::<ChatHeadersInput, ChatHeadersPatch, _>(
            &self.plugins,
            method::HOOK_CHAT_HEADERS,
            HookSubscription::CHAT_HEADERS,
            timeout,
            input,
            |inp, patch| {
                for (k, v) in patch.set {
                    inp.headers.insert(k, v);
                }
                for k in patch.remove {
                    inp.headers.remove(&k);
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    /// Sync variant for code paths driven from a non-async context (the
    /// provider request building path runs `block_on` from sync helpers).
    pub fn dispatch_chat_headers_blocking(
        &self,
        input: ChatHeadersInput,
    ) -> Result<ChatHeadersInput, PluginError> {
        self.block_on(self.dispatch_chat_headers(input))
    }

    pub async fn dispatch_chat_system_transform(
        &self,
        input: ChatSystemTransformInput,
    ) -> Result<ChatSystemTransformInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        dispatcher::chain_patch::<ChatSystemTransformInput, ChatSystemTransformPatch, _>(
            &self.plugins,
            method::HOOK_CHAT_SYSTEM_TRANSFORM,
            HookSubscription::CHAT_SYSTEM_TRANSFORM,
            timeout,
            input,
            |inp, patch| {
                if let Some(p) = patch.prepend {
                    inp.current_system = format!("{p}\n{}", inp.current_system);
                }
                if let Some(a) = patch.append {
                    inp.current_system = format!("{}\n{a}", inp.current_system);
                }
                if let Some(s) = patch.system {
                    inp.current_system = s;
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_permission_ask(
        &self,
        input: PermissionAskInput,
    ) -> Result<Option<PermissionDecision>, PluginError> {
        let timeout = self
            .timeouts
            .permission_ask_or(Duration::from_secs(60));
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::PERMISSION_ASK) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_PERMISSION_ASK, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let decision: Option<PermissionAskDecision> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(PermissionAskDecision::Decide(d)) = decision {
                return Ok(Some(d));
            }
        }
        Ok(None)
    }

    /// Sync wrapper for callers running in a non-async context (e.g.
    /// `PermissionRuntime`).
    pub fn dispatch_permission_ask_blocking(
        &self,
        input: PermissionAskInput,
    ) -> Result<Option<PermissionDecision>, PluginError> {
        self.block_on(self.dispatch_permission_ask(input))
    }

    pub async fn dispatch_command_before(
        &self,
        input: CommandBeforeInput,
    ) -> Result<CommandBeforeOutcome, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let mut current = input;
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::COMMAND_BEFORE) {
                continue;
            }
            let params = serde_json::to_value(&current)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_COMMAND_BEFORE, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let resp: Option<CommandBeforeResponse> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            match resp {
                Some(CommandBeforeResponse::Abort { reason }) => {
                    return Ok(CommandBeforeOutcome::Abort(reason));
                }
                Some(CommandBeforeResponse::Patch(p)) => {
                    if let Some(c) = p.command { current.command = c; }
                    if let Some(a) = p.args { current.args = a; }
                    if let Some(c) = p.cwd { current.cwd = c; }
                    if let Some(env) = p.env {
                        for (k, v) in env { current.env.insert(k, v); }
                    }
                }
                None => {}
            }
        }
        Ok(CommandBeforeOutcome::Continue(current))
    }

    pub fn dispatch_command_before_blocking(
        &self,
        input: CommandBeforeInput,
    ) -> Result<CommandBeforeOutcome, PluginError> {
        self.block_on(self.dispatch_command_before(input))
    }

    pub async fn dispatch_auth(
        &self,
        input: AuthInput,
    ) -> Result<Option<AuthOutput>, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::AUTH) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_AUTH, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let out: Option<AuthOutput> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if out.is_some() {
                return Ok(out);
            }
        }
        Ok(None)
    }

    pub async fn dispatch_provider_list(
        &self,
        input: ProviderListInput,
    ) -> Result<ProviderListPatch, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        let mut acc = ProviderListPatch::default();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::PROVIDER_LIST) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_PROVIDER_LIST, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<ProviderListPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                acc.add.extend(p.add);
                acc.remove.extend(p.remove);
            }
        }
        Ok(acc)
    }

    pub async fn dispatch_config(
        &self,
        input: ConfigInput,
    ) -> Result<ConfigInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        dispatcher::chain_patch::<ConfigInput, ConfigPatch, _>(
            &self.plugins,
            method::HOOK_CONFIG,
            HookSubscription::CONFIG,
            timeout,
            input,
            |inp, patch| {
                if let Some(m) = patch.merge {
                    merge_json(&mut inp.current, m);
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub async fn dispatch_session_compacting(
        &self,
        input: SessionCompactingInput,
    ) -> Result<SessionCompactingOutcome, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(5));
        let mut summary: Option<String> = None;
        let folded = dispatcher::chain_patch::<SessionCompactingInput, SessionCompactingPatch, _>(
            &self.plugins,
            method::HOOK_SESSION_COMPACTING,
            HookSubscription::SESSION_COMPACTING,
            timeout,
            input,
            |inp, patch| {
                if let Some(m) = patch.messages {
                    inp.messages = m;
                }
                if let Some(s) = patch.summary {
                    summary = Some(s);
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)?;
        Ok(SessionCompactingOutcome {
            messages: folded.messages,
            summary,
        })
    }

    // ── session.start ──────────────────────────────────────────────────────

    pub async fn dispatch_session_start(
        &self,
        input: SessionStartInput,
    ) -> Result<SessionStartPatch, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        let mut acc = SessionStartPatch::default();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::SESSION_START) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_SESSION_START, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<SessionStartPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                if let Some(ctx) = p.additional_context {
                    let existing = acc.additional_context.get_or_insert_with(String::new);
                    if !existing.is_empty() {
                        existing.push('\n');
                    }
                    existing.push_str(&ctx);
                }
                if p.initial_user_message.is_some() {
                    acc.initial_user_message = p.initial_user_message;
                }
            }
        }
        Ok(acc)
    }

    // ── session.end ────────────────────────────────────────────────────────

    pub async fn broadcast_session_end(&self, input: SessionEndInput) {
        let timeout = Duration::from_secs(5);
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::SESSION_END) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_SESSION_END, params),
                )
                .await;
            });
        }
    }

    // ── session.compacted ──────────────────────────────────────────────────

    pub async fn broadcast_session_compacted(&self, input: SessionCompactedInput) {
        let timeout = Duration::from_secs(5);
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::SESSION_COMPACTED) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_SESSION_COMPACTED, params),
                )
                .await;
            });
        }
    }

    // ── user.prompt.submit ─────────────────────────────────────────────────

    pub async fn dispatch_user_prompt_submit(
        &self,
        input: UserPromptSubmitInput,
    ) -> Result<UserPromptSubmitInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        let mut current = input;
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::USER_PROMPT_SUBMIT) {
                continue;
            }
            let params = serde_json::to_value(&current)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_USER_PROMPT_SUBMIT, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<UserPromptSubmitPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                if let Some(r) = p.block_reason {
                    return Err(PluginError::new(format!("prompt blocked: {r}")));
                }
                if let Some(text) = p.prompt {
                    current.prompt = text;
                }
                if let Some(ctx) = p.additional_context {
                    current.prompt.push('\n');
                    current.prompt.push_str(&ctx);
                }
            }
        }
        Ok(current)
    }

    /// Blocking variant for callers in sync context.
    pub fn dispatch_user_prompt_submit_blocking(
        &self,
        input: UserPromptSubmitInput,
    ) -> Result<UserPromptSubmitInput, PluginError> {
        self.block_on(self.dispatch_user_prompt_submit(input))
    }

    // ── tool.execute.failure ───────────────────────────────────────────────

    pub async fn broadcast_tool_failure(&self, input: ToolFailureInput) {
        let timeout = Duration::from_secs(5);
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::TOOL_FAILURE) {
                continue;
            }
            let input = input.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move {
                let params = match serde_json::to_value(&input) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_TOOL_FAILURE, params),
                )
                .await;
            });
        }
    }

    // ── tool.definition ────────────────────────────────────────────────────

    pub async fn dispatch_tool_definition(
        &self,
        input: ToolDefinitionInput,
    ) -> Result<ToolDefinitionInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(2));
        dispatcher::chain_patch::<ToolDefinitionInput, ToolDefinitionPatch, _>(
            &self.plugins,
            method::HOOK_TOOL_DEFINITION,
            HookSubscription::TOOL_DEFINITION,
            timeout,
            input,
            |inp, patch| {
                if let Some(d) = patch.description {
                    inp.description = d;
                }
                if let Some(s) = patch.input_schema {
                    inp.input_schema = s;
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub fn dispatch_tool_definition_blocking(
        &self,
        input: ToolDefinitionInput,
    ) -> Result<ToolDefinitionInput, PluginError> {
        self.block_on(self.dispatch_tool_definition(input))
    }

    // ── agent.stop ─────────────────────────────────────────────────────────

    pub async fn dispatch_agent_stop(
        &self,
        input: AgentStopInput,
    ) -> Result<AgentStopPatch, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(30));
        let mut acc = AgentStopPatch::default();
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::AGENT_STOP) {
                continue;
            }
            let params = serde_json::to_value(&input)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            let v = call_with_timeout(plugin, method::HOOK_AGENT_STOP, params, timeout)
                .await
                .map_err(transport_to_plugin_error)?;
            if matches!(&v, serde_json::Value::Null) {
                continue;
            }
            let patch: Option<AgentStopPatch> = serde_json::from_value(v)
                .map_err(|e| PluginError::invalid_params(e.to_string()))?;
            if let Some(p) = patch {
                if p.continue_with_message.is_some() {
                    acc.continue_with_message = p.continue_with_message;
                    acc.reason = p.reason;
                    // First plugin that wants to block stop wins.
                    break;
                }
            }
        }
        Ok(acc)
    }

    // ── command.execute.after ──────────────────────────────────────────────

    pub async fn dispatch_command_after(
        &self,
        input: CommandAfterInput,
    ) -> Result<CommandAfterInput, PluginError> {
        let timeout = self.timeouts.fast_or(Duration::from_secs(5));
        dispatcher::chain_patch::<CommandAfterInput, CommandAfterPatch, _>(
            &self.plugins,
            method::HOOK_COMMAND_AFTER,
            HookSubscription::COMMAND_AFTER,
            timeout,
            input,
            |inp, patch| {
                if let Some(s) = patch.stdout {
                    inp.stdout = s;
                }
                if let Some(s) = patch.stderr {
                    inp.stderr = s;
                }
                if patch.exit_code.is_some() {
                    inp.exit_code = patch.exit_code;
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    pub fn dispatch_command_after_blocking(
        &self,
        input: CommandAfterInput,
    ) -> Result<CommandAfterInput, PluginError> {
        self.block_on(self.dispatch_command_after(input))
    }

    // ── chat.messages.transform ────────────────────────────────────────────

    pub async fn dispatch_chat_messages_transform(
        &self,
        input: ChatMessagesTransformInput,
    ) -> Result<ChatMessagesTransformInput, PluginError> {
        let timeout = self.timeouts.chat_or(Duration::from_secs(10));
        dispatcher::chain_patch::<ChatMessagesTransformInput, ChatMessagesTransformPatch, _>(
            &self.plugins,
            method::HOOK_CHAT_MESSAGES_TRANSFORM,
            HookSubscription::CHAT_MESSAGES_TRANSFORM,
            timeout,
            input,
            |inp, patch| {
                if let Some(msgs) = patch.messages {
                    inp.messages = msgs;
                }
            },
        )
        .await
        .map_err(transport_to_plugin_error)
    }

    /// Push an `EventEnvelope` to every subscribed plugin (best-effort, no
    /// error propagation — events are notifications).
    pub async fn broadcast_event(&self, env: EventEnvelope) {
        let timeout = Duration::from_secs(2);
        for plugin in &self.plugins {
            if !plugin.subscribes(HookSubscription::EVENT) {
                continue;
            }
            let env = env.clone();
            let plugin = plugin.clone();
            tokio::spawn(async move {
                let params = match serde_json::to_value(&env) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let _ = tokio::time::timeout(
                    timeout,
                    plugin.transport.notify(method::HOOK_EVENT, params),
                )
                .await;
            });
        }
    }

    /// Async shutdown — sends `meta/shutdown` and closes every transport.
    /// Plugins whose transport has been transferred to a successor host
    /// (via [`PluginHostBuilder::with_previous`] hot-reload) are skipped.
    pub async fn shutdown(&self) {
        let transferred = self.transferred_to_successor.lock().await.clone();
        for plugin in &self.plugins {
            if transferred.contains(&plugin.id) {
                continue;
            }
            let _ = shutdown_transport(Arc::clone(&plugin.transport)).await;
        }
    }

    /// Direct access to the host-side bidirectional router. Used by HTTP
    /// callback routes and bidirectional stdio transports.
    pub fn host_handle(&self) -> Arc<HostHandle> {
        Arc::clone(&self._host_handle)
    }
}

fn transport_to_plugin_error(e: TransportError) -> PluginError {
    match e {
        TransportError::Plugin(pe) => pe,
        other => PluginError::new(other.to_string()),
    }
}

fn merge_json(into: &mut serde_json::Value, from: serde_json::Value) {
    match (into, from) {
        (serde_json::Value::Object(map_into), serde_json::Value::Object(map_from)) => {
            for (k, v) in map_from {
                merge_json(map_into.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (slot, value) => {
            *slot = value;
        }
    }
}

// ---------- builder ----------

pub struct PluginHostBuilder {
    static_plugins: HashMap<String, StaticRegistration>,
    config: PluginsConfig,
    workspace_root: PathBuf,
    agena_version: String,
    builtin_tool_names: Vec<String>,
    callback_base_url: Option<String>,
    host_client: Option<Arc<dyn HostClient>>,
    /// Optional previous host: for any entry whose config is byte-identical
    /// to the previous run, the old transport is reused (hot-reload).
    previous: Option<Arc<PluginHost>>,
    previous_entries: HashMap<String, PluginEntry>,
}

impl PluginHostBuilder {
    pub fn new(workspace_root: impl Into<PathBuf>, agena_version: impl Into<String>) -> Self {
        Self {
            static_plugins: HashMap::new(),
            config: PluginsConfig::default(),
            workspace_root: workspace_root.into(),
            agena_version: agena_version.into(),
            builtin_tool_names: Vec::new(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_entries: HashMap::new(),
        }
    }

    pub fn with_config(mut self, config: PluginsConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_builtin_tools(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.builtin_tool_names = names.into_iter().collect();
        self
    }

    pub fn with_callback_base_url(mut self, url: impl Into<String>) -> Self {
        self.callback_base_url = Some(url.into());
        self
    }

    pub fn with_host_client(mut self, client: Arc<dyn HostClient>) -> Self {
        self.host_client = Some(client);
        self
    }

    /// Reuse transports from a previous build for entries whose config is
    /// byte-identical. Used for hot-reload across snapshot rebuilds.
    pub fn with_previous(
        mut self,
        previous: Arc<PluginHost>,
        previous_config: &PluginsConfig,
    ) -> Self {
        self.previous_entries = previous_config
            .list
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.previous = Some(previous);
        self
    }

    /// Register a compiled-in plugin under a stable id. Matches the config
    /// entry `kind = "static"` with the same key. If no such entry exists in
    /// the current config, one is added automatically with default options
    /// and timeouts so the plugin participates in the load loop.
    pub fn register_static<P: crate::sdk::Plugin>(mut self, id: impl Into<String>, plugin: P) -> Self {
        let id = id.into();
        let inproc = InProcessTransport::new(plugin);
        self.static_plugins.insert(
            id.clone(),
            StaticRegistration {
                builder: Box::new(move || Arc::new(inproc) as Arc<dyn PluginTransport>),
            },
        );
        self.config.list.entry(id).or_insert_with(|| PluginEntry::Static {
            options: serde_json::Value::Null,
            timeouts: Default::default(),
        });
        self
    }

    pub async fn build(self) -> Result<Arc<PluginHost>, HostError> {
        if !self.config.enabled {
            tracing::info!(target: "agena_plugin_host", "plugins disabled in config");
            return Ok(PluginHost::new_empty());
        }

        let host_inner = self.host_client.unwrap_or_else(|| Arc::new(NoopHostClient));
        let mut handle = HostHandle::new(host_inner);
        if let Some(url) = self.callback_base_url.clone() {
            handle = handle.with_callback_base_url(url);
        }
        let host_handle = Arc::new(handle);
        let env_lookup: Box<dyn Fn(&str) -> Option<String> + Send + Sync> =
            Box::new(|k: &str| std::env::var(k).ok());

        let mut static_registry = self.static_plugins;
        let mut loaded: Vec<Arc<LoadedPlugin>> = Vec::new();
        let mut by_id: HashMap<String, Arc<LoadedPlugin>> = HashMap::new();
        let mut tools = ToolRegistry::new(self.builtin_tool_names);

        // Sort entries by id for deterministic load order.
        let mut entries: Vec<(String, PluginEntry)> = self.config.list.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Build a quick lookup of previous LoadedPlugin by id for reuse.
        let previous_loaded: HashMap<String, Arc<LoadedPlugin>> = self
            .previous
            .as_ref()
            .map(|p| {
                p.plugins
                    .iter()
                    .map(|lp| (lp.id.clone(), Arc::clone(lp)))
                    .collect()
            })
            .unwrap_or_default();

        for (idx, (id, entry)) in entries.into_iter().enumerate() {
            // Hot-reload: if a previous host had this id with a byte-identical
            // entry, reuse the transport (no respawn).
            if let Some(prev_entry) = self.previous_entries.get(&id)
                && prev_entry == &entry
                && let Some(reused) = previous_loaded.get(&id).cloned()
            {
                tracing::info!(
                    target: "agena_plugin_host",
                    plugin = %id,
                    "reusing existing plugin transport (config unchanged)"
                );
                if let Some(prev_host) = &self.previous {
                    prev_host
                        .transferred_to_successor
                        .lock()
                        .await
                        .insert(id.clone());
                }
                tools.extend_from_plugin(idx, &reused.id, &reused.manifest.tools);
                by_id.insert(reused.id.clone(), Arc::clone(&reused));
                loaded.push(reused);
                continue;
            }
            match load_entry(
                &id,
                &entry,
                &mut static_registry,
                Arc::clone(&host_handle),
                &self.agena_version,
                &self.workspace_root,
                &env_lookup,
                &self.config.trusted_keys,
            )
            .await
            {
                Ok(plugin) => {
                    let plugin = Arc::new(plugin);
                    tools.extend_from_plugin(idx, &plugin.id, &plugin.manifest.tools);
                    by_id.insert(plugin.id.clone(), plugin.clone());
                    loaded.push(plugin);
                }
                Err(err) => {
                    tracing::warn!(
                        target: "agena_plugin_host",
                        plugin = %id,
                        "failed to load plugin: {err}"
                    );
                }
            }
        }

        Ok(Arc::new(PluginHost {
            plugins: loaded,
            plugins_by_id: by_id,
            tools,
            timeouts: self.config.timeouts,
            runtime: None,
            runtime_handle: tokio::runtime::Handle::try_current().ok(),
            _host_handle: host_handle,
            transferred_to_successor: tokio::sync::Mutex::new(Default::default()),
        }))
    }
}

// ---------- HostHandle: routes plugin -> host requests ----------

/// `HostHandle` is the shared object that knows how to answer plugin
/// callbacks. Stdio plugins receive a closure that calls into it; HTTP
/// plugins receive a callback URL + bearer token; cdylib plugins currently
/// don't get callbacks (would require shared FFI surface).
pub struct HostHandle {
    inner: tokio::sync::RwLock<Arc<dyn HostClient>>,
    /// Per-plugin bearer tokens for HTTP callbacks.
    #[allow(dead_code)]
    tokens: tokio::sync::Mutex<HashMap<String, String>>,
    callback_base_url: Option<String>,
}

impl HostHandle {
    pub fn new(inner: Arc<dyn HostClient>) -> Self {
        Self {
            inner: tokio::sync::RwLock::new(inner),
            tokens: tokio::sync::Mutex::new(HashMap::new()),
            callback_base_url: None,
        }
    }

    pub fn with_callback_base_url(mut self, url: String) -> Self {
        self.callback_base_url = Some(url);
        self
    }

    /// Replace the underlying [`HostClient`] live (used after the runtime is
    /// constructed and we can install the real implementation).
    pub async fn install_client(&self, client: Arc<dyn HostClient>) {
        *self.inner.write().await = client;
    }

    pub fn callback_url(&self, plugin_id: &str) -> Option<String> {
        self.callback_base_url
            .as_ref()
            .map(|base| format!("{}/plugin-rpc/{}", base.trim_end_matches('/'), plugin_id))
    }

    pub fn callback_token(&self, _plugin_id: &str) -> Option<String> {
        // Tokens are only generated lazily when an HTTP plugin is registered.
        None
    }

    pub fn host_handler(self: &Arc<Self>) -> crate::transport::stdio::HostHandler {
        let this = Arc::clone(self);
        Arc::new(move |method: String, params: serde_json::Value| {
            let this = Arc::clone(&this);
            Box::pin(async move { this.handle_call(&method, params).await })
        })
    }

    pub async fn handle_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        let inner = self.inner.read().await.clone();
        match method {
            method::HOST_LOG => {
                let p: HostLogParams = parse(params)?;
                inner.log(p.level, p.message, p.fields).await;
                Ok(serde_json::Value::Object(Default::default()))
            }
            method::HOST_EVENT_PUBLISH => {
                let env: EventEnvelope = parse(params)?;
                inner.publish_event(env).await?;
                Ok(serde_json::Value::Object(Default::default()))
            }
            method::HOST_EVENT_SUBSCRIBE => {
                let p: HostSubscribeParams = parse(params)?;
                let sub: EventSubscription = inner.subscribe_events(p.filter).await?;
                Ok(serde_json::json!({ "subscription_id": sub.id }))
            }
            method::HOST_EVENT_UNSUBSCRIBE => {
                let p: HostUnsubscribeParams = parse(params)?;
                inner.unsubscribe_events(p.subscription_id).await?;
                Ok(serde_json::Value::Object(Default::default()))
            }
            method::HOST_PERMISSION_ASK => {
                let req: PermissionAskInput = parse(params)?;
                let d = inner.ask_permission(req).await?;
                serde_json::to_value(&d).map_err(|e| PluginError::invalid_params(e.to_string()))
            }
            method::HOST_CONFIG_READ => {
                let p: HostConfigReadParams = parse(params)?;
                inner.read_config(p.path).await
            }
            method::HOST_TOOL_INVOKE => {
                let p: HostInvokeToolParams = parse(params)?;
                let out = inner.invoke_tool(p.tool, p.input).await?;
                serde_json::to_value(&out).map_err(|e| PluginError::invalid_params(e.to_string()))
            }
            other => Err(PluginError::not_implemented(other)),
        }
    }
}

#[derive(serde::Deserialize)]
struct HostLogParams {
    level: LogLevel,
    message: String,
    #[serde(default)]
    fields: serde_json::Value,
}

#[derive(serde::Deserialize)]
struct HostSubscribeParams {
    filter: EventFilter,
}

#[derive(serde::Deserialize)]
struct HostUnsubscribeParams {
    subscription_id: String,
}

#[derive(serde::Deserialize)]
struct HostConfigReadParams {
    #[serde(default)]
    path: Option<String>,
}

#[derive(serde::Deserialize)]
struct HostInvokeToolParams {
    tool: String,
    #[serde(default)]
    input: serde_json::Value,
}

fn parse<T: DeserializeOwned>(v: serde_json::Value) -> Result<T, PluginError> {
    serde_json::from_value(v).map_err(|e| PluginError::invalid_params(e.to_string()))
}

/// Convenience: a `HostClient` impl that always errors. Used as the default
/// inside `HostHandle` until agena wires its own.
#[allow(dead_code)]
pub struct HostHandleClient;
