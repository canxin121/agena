//! Transport abstraction. Each transport implements one async method:
//! `dispatch(method, params) -> Value`.

pub mod cdylib;
pub mod http;
pub mod inproc;
pub mod stdio;

#[cfg(feature = "wasm")]
pub mod wasm;

use async_trait::async_trait;

use crate::error::TransportError;

#[async_trait]
pub trait PluginTransport: Send + Sync + 'static {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError>;

    async fn notify(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), TransportError> {
        let _ = self.dispatch(method, params).await?;
        Ok(())
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}
