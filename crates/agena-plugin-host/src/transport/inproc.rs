//! In-process transport. Wraps a concrete `Plugin` impl through the SDK's
//! `PluginDispatcher`, no serialization across process boundaries needed.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::FutureExt;

use crate::error::TransportError;
use crate::sdk::drivers::dispatch::PluginDispatcher;
use crate::sdk::host_api::{current_host_callback_context, run_in_host_callback_context};
use crate::sdk::{HostClient, Plugin, ToolInvokeInput};
use crate::transport::{PluginTransport, ToolStreamHandle};

/// In-process plugin transport.
pub struct InProcessTransport<P: Plugin> {
    dispatcher: Arc<PluginDispatcher<P>>,
}

/// Keep cancellation tied to the caller when a plugin is dispatched on a
/// fresh Tokio task. Dropping a Tool API call must not leave its body running.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
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
        let context = current_host_callback_context();
        // A Tool API call may arrive through several nested async layers.
        // Dispatch it on a fresh worker stack while preserving the callback
        // context and aborting the task if the caller cancels. This also keeps
        // a large plugin callback from overflowing the initiating worker.
        let mut task = AbortOnDrop(tokio::spawn(async move {
            AssertUnwindSafe(async move {
                let fut = Box::pin(dispatcher.dispatch(&method, params));
                if let Some(context) = context {
                    run_in_host_callback_context(context, fut).await
                } else {
                    fut.await
                }
            })
            .catch_unwind()
            .await
        }));
        let dispatch = (&mut task.0).await.map_err(|error| {
            TransportError::disconnected_error("in-process plugin task stopped", &error)
        })?;
        match dispatch {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(TransportError::Plugin(error)),
            Err(payload) => Err(TransportError::panicked(payload)),
        }
    }

    async fn attach_host(&self, host: Arc<dyn HostClient>) -> Result<(), TransportError> {
        self.dispatcher.set_host(host).await;
        Ok(())
    }

    async fn invoke_stream(
        &self,
        input: ToolInvokeInput,
    ) -> Result<Option<ToolStreamHandle>, TransportError> {
        let handle = self.dispatcher.dispatch_stream(input);
        Ok(Some(ToolStreamHandle {
            stream_id: handle.stream_id,
            chunks: handle.chunks,
            end: handle.end,
        }))
    }
}
