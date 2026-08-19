//! Stable read-only plugin-host operations exposed by a composed runtime.
//!
//! HTTP and other presentation layers can inspect plugin state through this
//! port without retaining or traversing a concrete runtime snapshot.

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
/// Error from the plugin runtime RPC service.
pub enum PluginRuntimeRpcError {
    #[error("invalid or missing plugin callback bearer token")]
    InvalidCallbackToken,
    #[error("plugin callback request is missing callback context")]
    MissingCallbackContext,
}

#[derive(Debug, Clone)]
/// Descriptor of a plugin tool.
pub struct PluginToolDescriptor {
    pub canonical_name: String,
    pub plugin_full_name: String,
    pub plugin_id: agena_plugin_host::sdk::PluginKey,
}

/// Presentation-neutral description of a registered plugin tool used by
/// permission UIs. It avoids exposing a concrete plugin-host registry entry.
#[derive(Debug, Clone)]
pub struct RuntimePluginToolCatalogItem {
    pub name: String,
    pub summary: String,
    pub tags: Vec<String>,
}

#[async_trait]
/// Service exposing plugin runtime information.
pub trait PluginRuntimeService: Send + Sync {
    fn plugin_statuses(&self) -> Vec<agena_plugin_host::status::PluginStatus>;

    fn plugin_status(&self, plugin_id: &str) -> Option<agena_plugin_host::status::PluginStatus>;

    fn plugin_surface_catalog(&self) -> agena_plugin_host::PluginSurfaceCatalog;
    fn plugin_architecture_catalog(&self) -> agena_plugin_host::PluginArchitectureCatalog {
        agena_plugin_host::PluginArchitectureCatalog::default()
    }

    fn permission_tool_catalog(&self) -> Vec<RuntimePluginToolCatalogItem>;

    fn display_contributions(&self) -> Vec<agena_plugin_host::HostDisplayContribution>;

    fn host_notifications(&self) -> Vec<agena_plugin_host::HostNotification>;

    fn theme_palettes(&self) -> Vec<agena_plugin_host::HostThemePalette>;

    fn operation_catalog(&self) -> Vec<agena_plugin_host::PluginOperationCatalogItem>;

    fn tool_registry_generation(&self) -> u64;

    fn tool_registry_events_since(
        &self,
        after_generation: Option<u64>,
        limit: usize,
    ) -> Vec<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent>;

    fn plugin_inspect(&self, plugin_id: &str) -> Option<agena_plugin_host::PluginInspect>;

    fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena_plugin_host::PluginLogRecord>;

    fn resolve_plugin_tool(
        &self,
        plugin_id: Option<&str>,
        tool_name: &str,
    ) -> Option<PluginToolDescriptor>;

    async fn invoke_plugin_operation(
        &self,
        plugin_id: &str,
        input: agena_plugin_host::sdk::PluginOperationInvokeInput,
    ) -> Result<agena_plugin_host::sdk::PluginOperationResult, String>;

    async fn plugin_rpc(
        &self,
        plugin_id: &str,
        callback_token: Option<String>,
        request: agena_plugin_host::sdk::rpc::Request,
    ) -> Result<agena_plugin_host::sdk::rpc::Response, PluginRuntimeRpcError>;
}

/// Concrete authenticated plugin-host bridge used by the Runtime service
/// implementation. Transport layers call `PluginRuntimeService::plugin_rpc`
/// and never receive the host.
pub async fn dispatch_plugin_rpc(
    host: std::sync::Arc<agena_plugin_host::PluginHost>,
    plugin_id: &str,
    callback_token: Option<String>,
    request: agena_plugin_host::sdk::rpc::Request,
) -> Result<agena_plugin_host::sdk::rpc::Response, PluginRuntimeRpcError> {
    use agena_plugin_host::sdk::rpc::{
        ErrorObject, JsonRpcVersion, Response, ResponsePayload, codes,
    };

    if !host
        .host_handle()
        .validate_callback_token(plugin_id, callback_token.as_deref())
        .await
    {
        return Err(PluginRuntimeRpcError::InvalidCallbackToken);
    }
    let id = request.id.clone();
    if host
        .plugins()
        .iter()
        .all(|plugin| plugin.key().to_string() != plugin_id)
    {
        return Ok(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::HOST_UNAVAILABLE,
                    message: format!("unknown plugin id: {plugin_id}"),
                    data: None,
                },
            },
        });
    }
    let params = request.params.unwrap_or(serde_json::Value::Null);
    let callback_context_present = params
        .as_object()
        .and_then(|object| object.get("context"))
        .and_then(|value| {
            serde_json::from_value::<agena_plugin_host::sdk::host_api::HostCallbackContext>(
                value.clone(),
            )
            .ok()
        })
        .is_some();
    if !callback_context_present {
        return Err(PluginRuntimeRpcError::MissingCallbackContext);
    }
    let handle = host.host_handle();
    match handle
        .ingest_stream_event_for_plugin(plugin_id, &request.method, params.clone())
        .await
    {
        Ok(true) => {
            return Ok(Response {
                jsonrpc: JsonRpcVersion,
                id,
                payload: ResponsePayload::Ok {
                    result: serde_json::Value::Object(Default::default()),
                },
            });
        }
        Ok(false) => {}
        Err(error) => {
            return Ok(Response {
                jsonrpc: JsonRpcVersion,
                id,
                payload: ResponsePayload::Err {
                    error: ErrorObject {
                        code: codes::PLUGIN_GENERIC,
                        message: error.to_string(),
                        data: serde_json::to_value(&error).ok(),
                    },
                },
            });
        }
    }
    match handle
        .handle_call_for_plugin(plugin_id, &request.method, params)
        .await
    {
        Ok(result) => Ok(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok { result },
        }),
        Err(error) => Ok(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::PLUGIN_GENERIC,
                    message: error.to_string(),
                    data: serde_json::to_value(&error).ok(),
                },
            },
        }),
    }
}
