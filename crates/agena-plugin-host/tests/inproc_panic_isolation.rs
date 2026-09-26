#[path = "support/panic_isolation.rs"]
mod panic_isolation;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agena_plugin_host::sdk::host_api::{
    HostCallbackContext, current_host_callback_context, run_in_host_callback_context,
};
use agena_plugin_host::sdk::{Plugin, PluginManifest, rpc::method};
use agena_plugin_host::transport::{PluginTransport, inproc::InProcessTransport};
use async_trait::async_trait;
use tokio::sync::{Notify, oneshot};

#[tokio::test]
async fn plugin_panics_are_isolated_and_transport_remains_usable() {
    panic_isolation::assert_panic_isolation().await;
}

struct BlockingPlugin {
    entered: Mutex<Option<oneshot::Sender<Option<HostCallbackContext>>>>,
    stopped: Arc<Notify>,
}

struct NotifyOnDrop(Arc<Notify>);

impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

#[async_trait]
impl Plugin for BlockingPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("test", "blocking", "0.1.0")
    }

    async fn shutdown(&self) -> agena_plugin_host::sdk::Result<()> {
        let _on_drop = NotifyOnDrop(Arc::clone(&self.stopped));
        if let Some(sender) = self.entered.lock().expect("entered lock").take() {
            let _ = sender.send(current_host_callback_context());
        }
        std::future::pending::<()>().await;
        Ok(())
    }
}

#[tokio::test]
async fn cancelled_dispatch_aborts_plugin_and_preserves_callback_context() {
    let (entered_sender, entered_receiver) = oneshot::channel();
    let stopped = Arc::new(Notify::new());
    let transport = InProcessTransport::new(BlockingPlugin {
        entered: Mutex::new(Some(entered_sender)),
        stopped: Arc::clone(&stopped),
    });
    let expected = HostCallbackContext {
        plugin_id: Some("test.blocking".to_owned()),
        session_id: Some(26),
        call_id: Some(25),
        ..Default::default()
    };
    let mut dispatch = Box::pin(run_in_host_callback_context(
        expected.clone(),
        transport.dispatch(method::META_SHUTDOWN, serde_json::json!({})),
    ));
    let observed = tokio::select! {
        observed = entered_receiver => observed.expect("plugin entered"),
        result = &mut dispatch => panic!("dispatch returned unexpectedly: {result:?}"),
    };
    assert_eq!(observed, Some(expected));
    drop(dispatch);
    tokio::time::timeout(Duration::from_secs(2), stopped.notified())
        .await
        .expect("cancelled plugin task was stopped");
}
