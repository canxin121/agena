//! Transport abstraction. Each transport implements one async method:
//! `dispatch(method, params) -> Value`.

pub mod cdylib;
pub mod http;
pub mod initialization;
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
    PluginError::from_rpc_error(error, context)
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
    async fn initialize(
        &self,
        initialization: initialization::PluginInitialization,
    ) -> Result<crate::sdk::InitOutcome, TransportError> {
        initialization.initialize(self).await
    }

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

    /// Attach the logical host that owns this transport. Process transports
    /// also rebind their callback router and lifecycle status projection.
    async fn bind_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
    ) -> Result<(), TransportError> {
        self.attach_host(host.scoped_host_client_for_scope(scope.plugin_id().to_string(), scope))
            .await
    }

    /// A planning hint only. The actual handoff must recheck this condition.
    async fn can_reuse(&self, owner: &Arc<crate::effect_scope::PluginEffectScope>) -> bool {
        owner.is_accepting()
    }

    /// Transfer a running transport and its future restart inputs together.
    /// False means that no handoff started and the caller must prepare a new
    /// transport. This does not invoke init again in the current process.
    async fn try_rebind_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
        previous_scope: Arc<crate::effect_scope::PluginEffectScope>,
        _initialization: initialization::PluginInitialization,
    ) -> Result<bool, TransportError> {
        if !self.can_reuse(&previous_scope).await {
            return Ok(false);
        }
        self.bind_host(host, scope).await?;
        Ok(true)
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
