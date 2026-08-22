//! Transport abstraction. Each transport implements one async method:
//! `dispatch(method, params) -> Value`.

pub mod cdylib;
pub mod http;
pub mod inproc;
pub mod quiescent;
pub mod stdio;

#[cfg(feature = "wasm")]
pub mod wasm;

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::TransportError;
use crate::sdk::host_api::HostClient;
use crate::sdk::rpc::ErrorObject;
use crate::sdk::{PluginError, ToolInvokeInput, ToolStreamChunk, ToolStreamEnd};

pub(super) fn plugin_error_from_rpc(error: ErrorObject, context: &str) -> PluginError {
    if let Some(data) = error.data.as_ref() {
        match serde_json::from_value::<PluginError>(data.clone()) {
            Ok(plugin_error) => return plugin_error,
            Err(decode_error) => {
                tracing::warn!(
                    operation = context,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        context,
                        &decode_error,
                    ),
                    "plugin JSON-RPC diagnostic data did not decode; preserving the public message and raw data"
                );
            }
        }
    }
    let mut plugin_error = PluginError::internal(error.message);
    plugin_error.diagnostic.data = error.data;
    plugin_error
}

/// Handle to a streaming tool execution.
pub struct ToolStreamHandle {
    pub stream_id: String,
    pub chunks: tokio::sync::mpsc::Receiver<ToolStreamChunk>,
    pub end: tokio::sync::oneshot::Receiver<Result<ToolStreamEnd, PluginError>>,
}

#[async_trait]
/// Transport used by the plugin host to talk to a plugin.
pub trait PluginTransport: Send + Sync + 'static {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError>;

    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), TransportError> {
        let _ = self.dispatch(method, params).await?;
        Ok(())
    }

    async fn attach_host(&self, _host: Arc<dyn HostClient>) -> Result<(), TransportError> {
        Ok(())
    }

    async fn invoke_stream(
        &self,
        _input: ToolInvokeInput,
    ) -> Result<Option<ToolStreamHandle>, TransportError> {
        Ok(None)
    }

    async fn ingest_stream_event(
        &self,
        _method: &str,
        _params: serde_json::Value,
    ) -> Result<bool, TransportError> {
        Ok(false)
    }

    /// Graceful terminal lifecycle boundary. Implementations that need to
    /// stop admission and await in-flight work atomically may override this;
    /// the default preserves the historical shutdown-hook then close order.
    async fn shutdown(&self) -> Result<(), TransportError> {
        let dispatch_error = self
            .dispatch(
                crate::sdk::rpc::method::META_SHUTDOWN,
                serde_json::Value::Object(Default::default()),
            )
            .await
            .err();
        let close_result = self.close().await;
        match (dispatch_error, close_result) {
            (None, result) => result,
            (Some(dispatch_error), Ok(())) => Err(dispatch_error),
            (Some(dispatch_error), Err(close_error)) => {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "close plugin transport after shutdown dispatch failure",
                        &close_error,
                    ),
                    "plugin transport close also failed; returning the primary shutdown dispatch error"
                );
                Err(dispatch_error)
            }
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}
