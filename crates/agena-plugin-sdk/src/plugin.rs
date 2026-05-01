use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::hooks::*;
use crate::host_api::HostClient;
use crate::manifest::PluginManifest;
use crate::rpc::method;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitContext {
    pub agena_version: String,
    pub workspace_root: PathBuf,
    pub plugin_id: String,
    /// Optional callback URL for HTTP plugins (host's bidirectional endpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_callback_url: Option<String>,
    /// Bearer token the plugin must use when calling back into the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_callback_token: Option<String>,
    /// Plugin-scoped options forwarded from `[plugins.list.<id>.options]`.
    #[serde(default)]
    pub options: serde_json::Value,
    /// Protocol version both sides agreed on (currently always `1`).
    pub protocol_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        // Single chunk + end: makes the streaming path trivially correct
        // for plugins that haven't migrated yet.
        sink.chunk(ToolStreamChunk {
            stream_id: stream_id.clone(),
            text_delta: Some(result.output_text.clone()),
            payload_delta: result.payload.clone(),
            metadata: result.metadata.clone(),
        })
        .await;
        Ok(ToolStreamEnd {
            stream_id,
            title: result.title,
            output_text: result.output_text,
            payload: result.payload,
            metadata: result.metadata,
            attachments: result.attachments,
        })
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

    // -------- permission --------
    async fn permission_ask(
        &self,
        _input: PermissionAskInput,
    ) -> Result<Option<PermissionAskDecision>> {
        Ok(None)
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
    async fn pre_turn(&self, _input: PreTurnInput) -> Result<()> {
        Ok(())
    }

    async fn post_turn(&self, _input: PostTurnInput) -> Result<()> {
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

    // -------- session compacted notification --------
    async fn session_compacted(&self, _input: SessionCompactedInput) -> Result<()> {
        Ok(())
    }

    // -------- config & session --------
    async fn config_resolved(&self, _input: ConfigInput) -> Result<Option<ConfigPatch>> {
        Ok(None)
    }

    async fn session_compacting(
        &self,
        _input: SessionCompactingInput,
    ) -> Result<Option<SessionCompactingPatch>> {
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
                payload_delta: None,
                metadata: Default::default(),
            })
            .await;
    }
}

#[allow(dead_code)]
fn _ensure_method_used() {
    let _ = method::HOOK_TOOL_INVOKE_STREAM;
}
