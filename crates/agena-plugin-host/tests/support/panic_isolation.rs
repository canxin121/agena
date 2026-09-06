use agena_plugin_host::TransportError;
use agena_plugin_host::sdk::{Plugin, PluginManifest, rpc::method};
use agena_plugin_host::transport::{PluginTransport, inproc::InProcessTransport};
use async_trait::async_trait;

struct PanickingPlugin;

#[async_trait]
impl Plugin for PanickingPlugin {
    fn manifest(&self) -> PluginManifest {
        panic!("synchronous plugin panic");
    }

    async fn shutdown(&self) -> agena_plugin_host::sdk::Result<()> {
        tokio::task::yield_now().await;
        panic!("asynchronous plugin panic");
    }
}

pub async fn assert_panic_isolation() {
    let transport = InProcessTransport::new(PanickingPlugin);
    for (method, expected) in [
        (method::META_MANIFEST, "synchronous plugin panic"),
        (method::META_SHUTDOWN, "asynchronous plugin panic"),
    ] {
        let error = transport
            .dispatch(method, serde_json::json!({}))
            .await
            .expect_err("plugin panic must become a transport error");
        assert!(matches!(error, TransportError::Panicked(message) if message == expected));
        assert_eq!(
            transport
                .dispatch(method::META_PING, serde_json::json!({}))
                .await
                .expect("transport survives a plugin panic"),
            serde_json::json!({"ok": true})
        );
    }
}
