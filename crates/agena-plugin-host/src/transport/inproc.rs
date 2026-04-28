//! In-process transport. Wraps a concrete `Plugin` impl through the SDK's
//! `PluginDispatcher`, no serialization across process boundaries needed.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::TransportError;
use crate::sdk::HostClient;
use crate::sdk::Plugin;
use crate::sdk::drivers::dispatch::PluginDispatcher;
use crate::transport::PluginTransport;

pub struct InProcessTransport<P: Plugin> {
    dispatcher: Arc<PluginDispatcher<P>>,
}

impl<P: Plugin> InProcessTransport<P> {
    pub fn new(plugin: P) -> Self {
        Self {
            dispatcher: Arc::new(PluginDispatcher::new(plugin)),
        }
    }

    pub async fn set_host(&self, host: Arc<dyn HostClient>) {
        self.dispatcher.set_host(host).await;
    }
}

#[async_trait]
impl<P: Plugin> PluginTransport for InProcessTransport<P> {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let dispatcher = Arc::clone(&self.dispatcher);
        let method = method.to_string();
        let join = tokio::spawn(async move { dispatcher.dispatch(&method, params).await });
        match join.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(TransportError::Plugin(e)),
            Err(join_err) => {
                if join_err.is_panic() {
                    Err(TransportError::Panicked)
                } else {
                    Err(TransportError::Disconnected)
                }
            }
        }
    }
}
