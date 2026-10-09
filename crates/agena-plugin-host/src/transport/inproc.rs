//! In-process transport. Wraps a concrete `Plugin` impl through the SDK's
//! `PluginDispatcher`, no serialization across process boundaries needed.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::FutureExt;

use crate::error::TransportError;
use crate::sdk::drivers::dispatch::PluginDispatcher;
use crate::sdk::host_api::{current_host_callback_context, run_in_host_callback_context};
use crate::sdk::{HostClient, InitOutcome, Plugin, PluginManifest, ToolInvokeInput};
use crate::transport::{PluginTransport, ToolStreamHandle};

type ManifestTransform = Arc<dyn Fn(&mut PluginManifest) + Send + Sync>;

/// In-process plugin transport.
pub struct InProcessTransport<P: Plugin> {
    dispatcher: Arc<PluginDispatcher<P>>,
    manifest_transform: Option<ManifestTransform>,
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
            manifest_transform: None,
        }
    }

    pub fn new_with_manifest_transform(
        plugin: P,
        transform: impl Fn(&mut PluginManifest) + Send + Sync + 'static,
    ) -> Self {
        Self {
            dispatcher: Arc::new(PluginDispatcher::new(plugin)),
            manifest_transform: Some(Arc::new(transform)),
        }
    }

    fn transform_manifest_reply(
        &self,
        method: &str,
        value: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let Some(transform) = &self.manifest_transform else {
            return Ok(value);
        };
        match method {
            crate::sdk::rpc::method::META_MANIFEST => {
                let mut manifest: PluginManifest = serde_json::from_value(value)
                    .map_err(|error| TransportError::Io(error.to_string()))?;
                transform(&mut manifest);
                serde_json::to_value(manifest)
                    .map_err(|error| TransportError::Io(error.to_string()))
            }
            crate::sdk::rpc::method::META_INIT => {
                let mut outcome: InitOutcome = serde_json::from_value(value)
                    .map_err(|error| TransportError::Io(error.to_string()))?;
                transform(&mut outcome.manifest);
                serde_json::to_value(outcome).map_err(|error| TransportError::Io(error.to_string()))
            }
            _ => Ok(value),
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
        let method_name = method.to_string();
        let method = method_name.clone();
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
            Ok(Ok(value)) => self.transform_manifest_reply(&method_name, value),
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

#[cfg(test)]
mod tests {
    use super::InProcessTransport;
    use crate::sdk::{Plugin, PluginManifest};
    use crate::transport::PluginTransport;

    struct DocumentationPlugin;

    #[async_trait::async_trait]
    impl Plugin for DocumentationPlugin {
        fn manifest(&self) -> PluginManifest {
            let mut manifest = PluginManifest::new("example", "docs", "1.0.0");
            manifest.summary = Some("English summary".to_owned());
            manifest
        }
    }

    #[tokio::test]
    async fn manifest_transform_is_consistent_for_discovery_and_initialization() {
        let transport =
            InProcessTransport::new_with_manifest_transform(DocumentationPlugin, |manifest| {
                manifest.summary = Some("Localized summary".to_owned());
            });

        let manifest = transport
            .dispatch(
                crate::sdk::rpc::method::META_MANIFEST,
                serde_json::json!({}),
            )
            .await
            .expect("localized manifest discovery");
        assert_eq!(manifest["summary"], "Localized summary");

        let outcome = transport
            .dispatch(
                crate::sdk::rpc::method::META_INIT,
                serde_json::json!({
                    "agena_version": "test",
                    "workspace_root": "/workspace",
                    "plugin_id": "example.docs",
                    "settings": {},
                    "protocol_version": crate::sdk::rpc::PROTOCOL_VERSION,
                }),
            )
            .await
            .expect("localized initialization response");
        assert_eq!(outcome["manifest"]["summary"], "Localized summary");
    }
}
