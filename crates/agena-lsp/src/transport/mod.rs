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

#[cfg(any(test, feature = "test-support"))]
pub use in_memory::InMemoryTransport;

#[cfg(any(test, feature = "test-support"))]
mod in_memory {
    //! Test-only transport that exposes a mpsc inbox the test harness can
    //! drive directly.

    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::Value;
    use tokio::sync::{Mutex, mpsc};

    use crate::error::{LspError, LspResult};
    use crate::protocol::InboundMessage;

    use super::LspTransport;

    /// In-memory LSP transport for tests.
    pub struct InMemoryTransport {
        outbox: mpsc::UnboundedSender<Value>,
        inbox: Mutex<mpsc::UnboundedReceiver<InboundMessage>>,
    }

    impl InMemoryTransport {
        pub fn new(
            outbox: mpsc::UnboundedSender<Value>,
            inbox: mpsc::UnboundedReceiver<InboundMessage>,
        ) -> Self {
            Self {
                outbox,
                inbox: Mutex::new(inbox),
            }
        }

        pub fn pair() -> (
            Arc<Self>,
            mpsc::UnboundedReceiver<Value>,
            mpsc::UnboundedSender<InboundMessage>,
        ) {
            let (out_tx, out_rx) = mpsc::unbounded_channel();
            let (in_tx, in_rx) = mpsc::unbounded_channel();
            (Arc::new(Self::new(out_tx, in_rx)), out_rx, in_tx)
        }
    }

    #[async_trait]
    impl LspTransport for InMemoryTransport {
        async fn send(&self, payload: Value) -> LspResult<()> {
            self.outbox
                .send(payload)
                .map_err(|e| LspError::Transport(e.to_string()))
        }

        async fn recv(&self) -> LspResult<InboundMessage> {
            let mut g = self.inbox.lock().await;
            g.recv().await.ok_or(LspError::TransportClosed)
        }

        async fn close(&self) -> LspResult<()> {
            Ok(())
        }
    }
}
