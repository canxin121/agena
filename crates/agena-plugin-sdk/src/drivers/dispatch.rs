//! Generic plugin → JSON dispatch trampoline. Used by every driver.

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::error::{PluginError, Result};
use crate::hooks::*;
use crate::host_api::HostClient;
use crate::plugin::{InitContext, Plugin, ToolStreamSink};
use crate::rpc::method;

/// Stateful dispatcher held by a driver. Wraps the `Plugin` impl and the
/// `HostClient` once `meta/init` has been called.
pub struct PluginDispatcher<P: Plugin> {
    plugin: Arc<P>,
    host: tokio::sync::RwLock<Option<Arc<dyn HostClient>>>,
}

impl<P: Plugin> PluginDispatcher<P> {
    pub fn new(plugin: P) -> Self {
        Self {
            plugin: Arc::new(plugin),
            host: tokio::sync::RwLock::new(None),
        }
    }

    pub fn from_arc(plugin: Arc<P>) -> Self {
        Self {
            plugin,
            host: tokio::sync::RwLock::new(None),
        }
    }

    pub async fn set_host(&self, host: Arc<dyn HostClient>) {
        *self.host.write().await = Some(host);
    }

    /// Single entry-point: routes a method/params pair to the right `Plugin`
    /// trait method and returns the JSON-encoded result.
    pub async fn dispatch(&self, method_name: &str, params: Value) -> Result<Value> {
        let plugin = Arc::clone(&self.plugin);
        match method_name {
            method::META_INIT => {
                let ctx: InitContext = serde_json::from_value(params)?;
                let host = self
                    .host
                    .read()
                    .await
                    .clone()
                    .unwrap_or_else(|| Arc::new(crate::host_api::NoopHostClient));
                let callback_context = crate::host_api::HostCallbackContext {
                    plugin_id: Some(ctx.plugin_id.clone()),
                    workspace_root: Some(ctx.workspace_root.to_string_lossy().to_string()),
                    ..crate::host_api::HostCallbackContext::default()
                };
                let outcome = crate::host_api::with_host_callback_context(
                    callback_context,
                    plugin.init(ctx, host),
                )
                .await?;
                ok_json(&outcome)
            }
            method::META_SHUTDOWN => {
                plugin.shutdown().await?;
                Ok(Value::Object(Default::default()))
            }
            method::META_MANIFEST => ok_json(&plugin.manifest()),
            method::META_PING => Ok(serde_json::json!({"ok": true})),

            method::HOOK_EVENT => {
                let env: EventEnvelope = serde_json::from_value(params)?;
                plugin.event(env).await?;
                Ok(Value::Object(Default::default()))
            }
            method::HOOK_TOOL_BEFORE => {
                let i: ToolBeforeInput = serde_json::from_value(params)?;
                let ctx = crate::host_api::HostCallbackContext {
                    session_id: Some(i.session_id),
                    call_id: Some(i.call_id),
                    workspace_root: Some(i.workspace_root.clone()),
                    entry_name: Some(i.tool_name.clone()),
                    ..crate::host_api::HostCallbackContext::default()
                };
                ok_json(
                    &crate::host_api::with_host_callback_context(
                        ctx,
                        plugin.tool_execute_before(i),
                    )
                    .await?,
                )
            }
            method::HOOK_TOOL_AFTER => {
                let i: ToolAfterInput = serde_json::from_value(params)?;
                let ctx = crate::host_api::HostCallbackContext {
                    session_id: Some(i.session_id),
                    call_id: Some(i.call_id),
                    workspace_root: Some(i.workspace_root.clone()),
                    entry_name: Some(i.tool_name.clone()),
                    ..crate::host_api::HostCallbackContext::default()
                };
                ok_json(
                    &crate::host_api::with_host_callback_context(ctx, plugin.tool_execute_after(i))
                        .await?,
                )
            }
            method::HOOK_TOOL_INVOKE => {
                let i: ToolInvokeInput = serde_json::from_value(params)?;
                let ctx = crate::host_api::HostCallbackContext {
                    session_id: Some(i.session_id),
                    call_id: Some(i.call_id),
                    workspace_root: Some(i.workspace_root.clone()),
                    entry_name: Some(i.tool_name.clone()),
                    ..crate::host_api::HostCallbackContext::default()
                };
                let output =
                    crate::host_api::with_host_callback_context(ctx, plugin.tool_invoke(i)).await?;
                ok_json(&output)
            }
            method::HOOK_TOOL_PERMISSION_PATHS => {
                let i: ToolPermissionPathsInput = serde_json::from_value(params)?;
                let ctx = crate::host_api::HostCallbackContext {
                    entry_name: Some(i.tool_name.clone()),
                    ..crate::host_api::HostCallbackContext::default()
                };
                ok_json(
                    &crate::host_api::with_host_callback_context(
                        ctx,
                        plugin.permission_paths(&i.tool_name, &i.input),
                    )
                    .await?,
                )
            }
            method::HOOK_TOOL_INVOKE_STREAM => Err(PluginError::new(
                "tool.invoke.stream cannot be dispatched without a stream sink; \
                     transports should call PluginDispatcher::dispatch_stream",
            )),
            method::HOOK_CHAT_MESSAGE => {
                let i: ChatMessageInput = serde_json::from_value(params)?;
                ok_json(&plugin.chat_message(i).await?)
            }
            method::HOOK_CHAT_PARAMS => {
                let i: ChatParamsInput = serde_json::from_value(params)?;
                ok_json(&plugin.chat_params(i).await?)
            }
            method::HOOK_CHAT_HEADERS => {
                let i: ChatHeadersInput = serde_json::from_value(params)?;
                ok_json(&plugin.chat_headers(i).await?)
            }
            method::HOOK_CHAT_SYSTEM_TRANSFORM => {
                let i: ChatSystemTransformInput = serde_json::from_value(params)?;
                ok_json(&plugin.chat_system_transform(i).await?)
            }
            method::HOOK_AUTH => {
                let i: AuthInput = serde_json::from_value(params)?;
                ok_json(&plugin.auth(i).await?)
            }
            method::HOOK_PROVIDER_LIST => {
                let i: ProviderListInput = serde_json::from_value(params)?;
                ok_json(&plugin.provider_list(i).await?)
            }
            method::HOOK_PERMISSION_ASK => {
                let i: PermissionAskInput = serde_json::from_value(params)?;
                ok_json(&plugin.permission_ask(i).await?)
            }
            method::HOOK_NOTIFICATION => {
                let i: NotificationInput = serde_json::from_value(params)?;
                plugin.notification(i).await?;
                Ok(Value::Object(Default::default()))
            }
            method::HOOK_COMMAND_BEFORE => {
                let i: CommandBeforeInput = serde_json::from_value(params)?;
                ok_json(&plugin.command_execute_before(i).await?)
            }
            method::HOOK_SHELL_ENV => {
                let i: ShellEnvInput = serde_json::from_value(params)?;
                ok_json(&plugin.shell_env(i).await?)
            }
            method::HOOK_CONFIG => {
                let i: ConfigInput = serde_json::from_value(params)?;
                ok_json(&plugin.config_resolved(i).await?)
            }
            method::HOOK_SESSION_COMPACTING => {
                let i: SessionCompactingInput = serde_json::from_value(params)?;
                ok_json(&plugin.session_compacting(i).await?)
            }
            method::HOOK_PRE_TURN => {
                let i: PreTurnInput = serde_json::from_value(params)?;
                plugin.pre_turn(i).await?;
                Ok(Value::Object(Default::default()))
            }
            method::HOOK_POST_TURN => {
                let i: PostTurnInput = serde_json::from_value(params)?;
                plugin.post_turn(i).await?;
                Ok(Value::Object(Default::default()))
            }
            method::HOOK_SESSION_START => {
                let i: SessionStartInput = serde_json::from_value(params)?;
                ok_json(&plugin.session_start(i).await?)
            }
            method::HOOK_SESSION_END => {
                let i: SessionEndInput = serde_json::from_value(params)?;
                plugin.session_end(i).await?;
                Ok(Value::Object(Default::default()))
            }
            method::HOOK_SESSION_COMPACTED => {
                let i: SessionCompactedInput = serde_json::from_value(params)?;
                plugin.session_compacted(i).await?;
                Ok(Value::Object(Default::default()))
            }
            method::HOOK_USER_PROMPT_SUBMIT => {
                let i: UserPromptSubmitInput = serde_json::from_value(params)?;
                ok_json(&plugin.user_prompt_submit(i).await?)
            }
            method::HOOK_TOOL_FAILURE => {
                let i: ToolFailureInput = serde_json::from_value(params)?;
                let ctx = crate::host_api::HostCallbackContext {
                    session_id: Some(i.session_id),
                    call_id: Some(i.call_id),
                    workspace_root: Some(i.workspace_root.clone()),
                    entry_name: Some(i.tool_name.clone()),
                    ..crate::host_api::HostCallbackContext::default()
                };
                crate::host_api::with_host_callback_context(ctx, plugin.tool_execute_failure(i))
                    .await?;
                Ok(Value::Object(Default::default()))
            }
            method::HOOK_TOOL_DEFINITION => {
                let i: EntryDefinitionInput = serde_json::from_value(params)?;
                let ctx = crate::host_api::HostCallbackContext {
                    entry_name: Some(i.tool_name.clone()),
                    ..crate::host_api::HostCallbackContext::default()
                };
                ok_json(
                    &crate::host_api::with_host_callback_context(ctx, plugin.tool_definition(i))
                        .await?,
                )
            }
            method::HOOK_AGENT_STOP => {
                let i: AgentStopInput = serde_json::from_value(params)?;
                ok_json(&plugin.agent_stop(i).await?)
            }
            method::HOOK_COMMAND_AFTER => {
                let i: CommandAfterInput = serde_json::from_value(params)?;
                ok_json(&plugin.command_execute_after(i).await?)
            }
            method::HOOK_CHAT_MESSAGES_TRANSFORM => {
                let i: ChatMessagesTransformInput = serde_json::from_value(params)?;
                ok_json(&plugin.chat_messages_transform(i).await?)
            }
            other => Err(PluginError::not_implemented(other)),
        }
    }
}

fn ok_json<T: Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| PluginError::invalid_params(e.to_string()))
}

impl<P: Plugin> PluginDispatcher<P> {
    /// Run a streaming tool invocation. Returns a stream id immediately
    /// (the plugin's `tool_invoke_stream` runs in a background task) plus a
    /// receiver of [`ToolStreamChunk`]s and a oneshot for the terminal
    /// [`ToolStreamEnd`] (or error). Transports translate these into the
    /// `tool.stream.chunk` / `tool.stream.end` notifications.
    pub fn dispatch_stream(self: &std::sync::Arc<Self>, input: ToolInvokeInput) -> StreamHandle {
        let stream_id = format!("stream-{}", _random_id());
        let (tx, rx) = tokio::sync::mpsc::channel::<ToolStreamChunk>(64);
        let (end_tx, end_rx) = tokio::sync::oneshot::channel::<Result<ToolStreamEnd>>();
        let sink = ToolStreamSink::new(stream_id.clone(), tx);
        let plugin = std::sync::Arc::clone(&self.plugin);
        let inherited_context = crate::host_api::current_host_callback_context();
        tokio::spawn(async move {
            let ctx = crate::host_api::HostCallbackContext {
                session_id: Some(input.session_id),
                call_id: Some(input.call_id),
                workspace_root: Some(input.workspace_root.clone()),
                entry_name: Some(input.tool_name.clone()),
                ..inherited_context.unwrap_or_default()
            };
            let result = crate::host_api::with_host_callback_context(
                ctx,
                plugin.tool_invoke_stream(input, sink),
            )
            .await;
            let _ = end_tx.send(result);
        });
        StreamHandle {
            stream_id,
            chunks: rx,
            end: end_rx,
        }
    }
}

pub struct StreamHandle {
    pub stream_id: String,
    pub chunks: tokio::sync::mpsc::Receiver<ToolStreamChunk>,
    pub end: tokio::sync::oneshot::Receiver<Result<ToolStreamEnd>>,
}

fn _random_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{nanos:x}")
}
