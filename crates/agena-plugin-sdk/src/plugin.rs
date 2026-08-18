//! Core plugin contract: `Plugin`, `PluginSettings`, lifecycle, and tool streaming.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::error::{PluginError, Result};
use crate::hooks::*;
use crate::host_api::HostClient;
use crate::identity::PluginKey;
use crate::manifest::PluginManifest;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Context passed to a plugin at initialization.
pub struct InitContext {
    pub agena_version: String,
    pub workspace_root: PathBuf,
    pub plugin_id: PluginKey,
    /// Optional callback URL for HTTP plugins (host's bidirectional endpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_callback_url: Option<String>,
    /// Bearer token the plugin must use when calling back into the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_callback_token: Option<String>,
    /// Plugin-owned configuration forwarded from `plugins.list.<id>.settings`.
    #[serde(default)]
    pub settings: serde_json::Value,
    /// Protocol version both sides agreed on (currently always `1`).
    pub protocol_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Outcome of plugin initialization.
pub struct InitOutcome {
    pub manifest: PluginManifest,
    pub protocol_version: u32,
}

impl InitOutcome {
    pub fn ack(manifest: PluginManifest) -> Self {
        Self {
            manifest,
            protocol_version: crate::rpc::PROTOCOL_VERSION,
        }
    }
}

#[derive(Debug)]
/// Typed plugin configuration with deferred access.
pub struct PluginSettings<T> {
    value: OnceLock<T>,
}

impl<T> Default for PluginSettings<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PluginSettings<T> {
    pub const fn new() -> Self {
        Self {
            value: OnceLock::new(),
        }
    }

    pub fn get(&self) -> Option<&T> {
        self.value.get()
    }

    pub fn expect(&self, message: &str) -> &T {
        self.value.get().expect(message)
    }

    pub fn set(&self, value: T, already: impl Into<String>) -> Result<()> {
        self.value
            .set(value)
            .map_err(|_| PluginError::internal(already.into()))
    }
}

impl<T> PluginSettings<T>
where
    T: Default + DeserializeOwned,
{
    pub fn set_from_json(
        &self,
        input: serde_json::Value,
        invalid: impl AsRef<str>,
        already: impl Into<String>,
    ) -> Result<()> {
        let value = crate::macro_support::parse_defaulted_settings(input, invalid.as_ref())?;
        self.set(value, already)
    }
}

#[doc(hidden)]
/// Access to the plugin config store.
pub trait PluginSettingsStoreAccess {
    fn plugin_settings_schema() -> serde_json::Value;

    fn set_plugin_settings_from_json(
        &self,
        input: serde_json::Value,
        invalid: &str,
        already: String,
    ) -> Result<()>;
}

/// The trait every plugin implements. Every method has a default no-op body so
/// you only fill in the hooks you care about. Add `#[async_trait::async_trait]`.
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn manifest(&self) -> PluginManifest;

    async fn init(&self, _ctx: InitContext, _host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    // -------- tool execution --------
    async fn tool_execute_before(
        &self,
        _input: ToolBeforeInput,
    ) -> Result<Option<ToolBeforePatch>> {
        Ok(None)
    }

    async fn tool_execute_after(&self, _input: ToolAfterInput) -> Result<Option<ToolAfterPatch>> {
        Ok(None)
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        Err(crate::error::PluginError::not_implemented(format!(
            "tool_invoke({})",
            input.tool_name
        )))
    }

    /// Dynamic per-invocation path-permission requests. Returned alongside
    /// any declarative `InputPathSpec`s in the manifest, the host audits all
    /// of them via the permission system before the tool body executes.
    /// Default: no extra paths.
    async fn permission_paths(
        &self,
        _tool: &str,
        _input: &serde_json::Value,
    ) -> Result<Vec<crate::hooks::PathRequest>> {
        Ok(Vec::new())
    }

    /// Dynamic per-invocation network-permission requests. Returned alongside
    /// any declarative `InputNetworkSpec`s in the manifest, the host audits all
    /// of them via the permission system before the tool body executes.
    /// Default: no extra network targets.
    async fn permission_networks(
        &self,
        _tool: &str,
        _input: &serde_json::Value,
    ) -> Result<Vec<crate::hooks::NetworkRequest>> {
        Ok(Vec::new())
    }

    /// Streaming variant of [`Plugin::tool_invoke`]. Default implementation
    /// falls back to `tool_invoke` and wraps the result as a single-chunk
    /// stream so plugins that don't care about streaming still work.
    ///
    /// `sink` lets the plugin push [`ToolStreamChunk`] frames as they are
    /// produced; the host's stream consumer sees them as they arrive. The
    /// returned future MUST eventually resolve, ideally with the same
    /// terminal output as `tool_invoke`.
    async fn tool_invoke_stream(
        &self,
        input: ToolInvokeInput,
        sink: ToolStreamSink,
    ) -> Result<ToolStreamEnd> {
        let stream_id = sink.stream_id().to_string();
        let result = self.tool_invoke(input).await?;
        // Single chunk + end: keeps the default streaming path equivalent to
        // a normal tool invocation.
        sink.chunk(ToolStreamChunk {
            stream_id: stream_id.clone(),
            text_delta: Some(result.output_text.clone()),
            metadata: result.metadata.clone(),
        })
        .await;
        Ok(ToolStreamEnd::from_output(stream_id, result))
    }

    async fn command_invoke(&self, input: PluginCommandInvokeInput) -> Result<PluginCommandOutput> {
        Err(crate::error::PluginError::not_implemented(format!(
            "command_invoke({})",
            input.command_id
        )))
    }

    // -------- chat --------
    async fn chat_message(&self, _input: ChatMessageInput) -> Result<Option<ChatMessagePatch>> {
        Ok(None)
    }

    async fn chat_params(&self, _input: ChatParamsInput) -> Result<Option<ChatParamsPatch>> {
        Ok(None)
    }

    async fn chat_headers(&self, _input: ChatHeadersInput) -> Result<Option<ChatHeadersPatch>> {
        Ok(None)
    }

    async fn chat_system_transform(
        &self,
        _input: ChatSystemTransformInput,
    ) -> Result<Option<ChatSystemTransformPatch>> {
        Ok(None)
    }

    // -------- events --------
    async fn event(&self, _ev: EventEnvelope) -> Result<()> {
        Ok(())
    }

    // -------- auth & providers --------
    async fn auth(&self, _input: AuthInput) -> Result<Option<AuthOutput>> {
        Ok(None)
    }

    async fn provider_list(&self, _input: ProviderListInput) -> Result<Option<ProviderListPatch>> {
        Ok(None)
    }

    // -------- notification --------
    async fn notification(&self, _input: NotificationInput) -> Result<()> {
        Ok(())
    }

    // -------- command & shell --------
    async fn command_execute_before(
        &self,
        _input: CommandBeforeInput,
    ) -> Result<Option<CommandBeforeResponse>> {
        Ok(None)
    }

    async fn shell_env(&self, _input: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        Ok(None)
    }

    // -------- session lifecycle --------
    async fn pre_run(&self, _input: PreRunInput) -> Result<()> {
        Ok(())
    }

    async fn post_run(&self, _input: PostRunInput) -> Result<()> {
        Ok(())
    }

    async fn session_start(&self, _input: SessionStartInput) -> Result<Option<SessionStartPatch>> {
        Ok(None)
    }

    async fn session_end(&self, _input: SessionEndInput) -> Result<()> {
        Ok(())
    }

    // -------- user prompt --------
    async fn user_prompt_submit(
        &self,
        _input: UserPromptSubmitInput,
    ) -> Result<Option<UserPromptSubmitPatch>> {
        Ok(None)
    }

    // -------- agent stop --------
    async fn agent_stop(&self, _input: AgentStopInput) -> Result<Option<AgentStopPatch>> {
        Ok(None)
    }

    // -------- agent cancellation --------
    async fn agent_cancel(&self, _input: AgentCancelInput) -> Result<()> {
        Ok(())
    }

    // -------- tool definition --------
    async fn tool_definition(
        &self,
        _input: ToolDefinitionInput,
    ) -> Result<Option<ToolDefinitionPatch>> {
        Ok(None)
    }

    // -------- tool failure notification --------
    async fn tool_execute_failure(&self, _input: ToolFailureInput) -> Result<()> {
        Ok(())
    }

    // -------- command after --------
    async fn command_execute_after(
        &self,
        _input: CommandAfterInput,
    ) -> Result<Option<CommandAfterPatch>> {
        Ok(None)
    }

    // -------- chat messages transform --------
    async fn chat_messages_transform(
        &self,
        _input: ChatMessagesTransformInput,
    ) -> Result<Option<ChatMessagesTransformPatch>> {
        Ok(None)
    }

    // -------- config & session --------
    async fn config_resolved(&self, _input: ConfigInput) -> Result<Option<ConfigPatch>> {
        Ok(None)
    }
}

/// Sink passed to [`Plugin::tool_invoke_stream`]. The plugin pushes
/// [`ToolStreamChunk`]s as they are produced; the host fans them out.
///
/// Implementation: thin handle around an `mpsc::Sender<ToolStreamChunk>`.
#[derive(Clone)]
pub struct ToolStreamSink {
    stream_id: String,
    tx: tokio::sync::mpsc::Sender<ToolStreamChunk>,
}

impl ToolStreamSink {
    #[doc(hidden)]
    pub fn new(stream_id: String, tx: tokio::sync::mpsc::Sender<ToolStreamChunk>) -> Self {
        Self { stream_id, tx }
    }

    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    pub async fn chunk(&self, chunk: ToolStreamChunk) {
        let _ = self.tx.send(chunk).await;
    }

    /// Convenience: push a text delta without a payload patch.
    pub async fn text(&self, delta: impl Into<String>) {
        let _ = self
            .tx
            .send(ToolStreamChunk {
                stream_id: self.stream_id.clone(),
                text_delta: Some(delta.into()),
                metadata: Default::default(),
            })
            .await;
    }
}
