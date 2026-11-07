//! A deterministic fault regression: deliberately discard a committed finish watch event.
//! Public notification behavior is covered by TestAgena in
//! apps/agena/tests/bundled/notifications.rs and notifications/recovery.rs.
//! Ordinary process restart cannot reliably reproduce this exact lost-event window.

use super::*;
use agena_domain::{ComposerDocument, ComposerNode};
use agena_plugin_host::sdk::{
    EventEnvelope, EventFilter, ToolInvokeOutput, host_api::EventSubscription,
};
use agena_storage::kernel::{KernelWatch, RequestId};

#[derive(Default)]
struct RecordingHost {
    notifications: std::sync::Mutex<Vec<PluginNotifyRequest>>,
}
#[async_trait::async_trait]
impl HostClient for RecordingHost {
    async fn log(
        &self,
        _: agena_plugin_host::sdk::host_api::LogLevel,
        _: String,
        _: serde_json::Value,
    ) {
    }
    async fn publish_event(&self, _: EventEnvelope) -> agena_plugin_host::sdk::Result<()> {
        Ok(())
    }
    async fn subscribe_events(
        &self,
        _: EventFilter,
    ) -> agena_plugin_host::sdk::Result<EventSubscription> {
        Ok(EventSubscription { id: "test".into() })
    }
    async fn read_config(
        &self,
        _: Option<String>,
    ) -> agena_plugin_host::sdk::Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }
    async fn invoke_tool(
        &self,
        _: String,
        _: serde_json::Value,
    ) -> agena_plugin_host::sdk::Result<ToolInvokeOutput> {
        unreachable!()
    }
    async fn notify(&self, request: PluginNotifyRequest) -> agena_plugin_host::sdk::Result<()> {
        self.notifications.lock().unwrap().push(request);
        Ok(())
    }
}

async fn fixture() -> (
    agena_storage_sqlite::SqliteKernel,
    Observer,
    Arc<RecordingHost>,
) {
    let kernel = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    let host = Arc::new(RecordingHost::default());
    let observer = Observer::load(
        NotificationSource {
            reader: kernel.ports.reader.clone(),
            state: kernel.instance_state.clone(),
        },
        host.clone(),
    )
    .await
    .unwrap();
    (kernel, observer, host)
}

async fn observe_pending(service: &mut Observer, _reader: &KernelReader, watch: &mut KernelWatch) {
    while let Some(change) = watch.try_recv().unwrap() {
        service.observe(&change).await.unwrap();
    }
}

async fn session(kernel: &agena_storage_sqlite::SqliteKernel) -> SessionId {
    let workspace = kernel
        .workspaces
        .ensure_id("/tmp/agena-web-push-test")
        .await
        .unwrap();
    kernel
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Push test".into())
        .await
        .unwrap()
        .value
        .id
}

async fn attempt(
    kernel: &agena_storage_sqlite::SqliteKernel,
    sid: SessionId,
) -> agena_storage::kernel::AttemptHandle {
    let execution = kernel
        .ports
        .sessions
        .send(
            RequestId::new(),
            sid,
            ComposerDocument(vec![ComposerNode::Text {
                text: "test".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    kernel
        .ports
        .executor
        .claim(RequestId::new(), sid, execution.id)
        .await
        .unwrap()
        .value
}

#[tokio::test]
async fn long_executions_and_lost_finish_events_recover_after_restart_with_stable_identity() {
    let (kernel, mut service, _host) = fixture().await;
    let sid = session(&kernel).await;
    let mut watch = kernel.ports.reader.watch_all(None).unwrap();
    let attempt = attempt(&kernel, sid).await;
    observe_pending(&mut service, &kernel.ports.reader, &mut watch).await;
    for _ in 0..130 {
        attempt
            .notice(RequestId::new(), "progress".into())
            .await
            .unwrap();
        observe_pending(&mut service, &kernel.ports.reader, &mut watch).await;
    }
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let finish = watch.recv().await.unwrap();
    assert!(finish.requires_snapshot);
    // Simulate a crash before the observer handles the terminal change.
    let mut restored = Observer::load(service.source.clone(), service.host.clone())
        .await
        .unwrap();
    restored.recover().await.unwrap();
    restored.observe(&finish).await.unwrap();
    let state = restored.state;
    assert_eq!(state.recent.len(), 1);
    assert_eq!(
        state.recent[0].attention.as_ref().unwrap().kind,
        AttentionKind::Completed
    );
}
