//! Transport abstraction for an LSP child process.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::LspResult;
use crate::protocol::InboundMessage;

#[async_trait]
/// Transport used to exchange messages with an LSP server.
pub trait LspTransport: Send + Sync {
    async fn send(&self, payload: Value) -> LspResult<()>;
    async fn recv(&self) -> LspResult<InboundMessage>;
    async fn close(&self) -> LspResult<()>;
}

mod stdio;

pub use stdio::StdioTransport;
