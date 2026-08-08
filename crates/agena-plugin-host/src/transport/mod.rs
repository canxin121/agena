//! Transport abstraction. Each transport implements one async method:
//! `dispatch(method, params) -> Value`.

pub mod cdylib;
pub mod http;
pub mod inproc;
pub mod stdio;

#[cfg(feature = "wasm")]
pub mod wasm;

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::TransportError;
use crate::sdk::host_api::HostClient;
use crate::sdk::{PluginError, ToolInvokeInput, ToolStreamChunk, ToolStreamEnd};

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

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}
