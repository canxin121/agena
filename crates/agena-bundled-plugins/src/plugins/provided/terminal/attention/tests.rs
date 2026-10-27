use super::*;
use agena_domain::{ComposerDocument, ComposerNode};
use agena_plugin_host::sdk::{
    EventEnvelope, EventFilter, ToolInvokeOutput, host_api::EventSubscription,
};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_storage::kernel::{KernelWatch, RequestId, ToolStart};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct RecordingHost {
    notifications: std::sync::Mutex<Vec<PluginNotifyRequest>>,
    invalidations: AtomicUsize,
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
    async fn publish_event(&self, event: EventEnvelope) -> agena_plugin_host::sdk::Result<()> {
        if event.kind == "attention.changed" {
            self.invalidations.fetch_add(1, Ordering::SeqCst);
        }
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
async fn canonical_completions_and_failures_publish_without_a_connected_client() {
    let (kernel, mut service, _directory) = fixture().await;
    let sid = session(&kernel).await;
    let mut watch = kernel.ports.reader.watch_all(None).unwrap();
    for phase in [
        ExecutionPhase::Succeeded,
        ExecutionPhase::Failed,
        ExecutionPhase::Cancelled,
    ] {
        let attempt = attempt(&kernel, sid).await;
        observe_pending(&mut service, &kernel.ports.reader, &mut watch).await;
        assert!(
            service
                .state
                .recent
                .iter()
                .all(|item| item.attention.as_ref().unwrap().kind != AttentionKind::Question)
        );
        attempt.finish(RequestId::new(), phase).await.unwrap();
        observe_pending(&mut service, &kernel.ports.reader, &mut watch).await;
    }
    let state = service.state;
    assert_eq!(
        state
            .recent
            .iter()
            .map(|item| item.attention.as_ref().unwrap().kind)
            .collect::<Vec<_>>(),
        vec![
            AttentionKind::Completed,
            AttentionKind::Failed,
            AttentionKind::Cancelled
        ]
    );
    assert!(state.active.is_empty());
}

#[tokio::test]
async fn canonical_questions_and_permissions_enqueue_once_without_tool_display_events() {
    let (kernel, mut service, _directory) = fixture().await;
    let sid = session(&kernel).await;
    let attempt = attempt(&kernel, sid).await;
    let mut watch = kernel.ports.reader.watch_all(None).unwrap();
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "call-test".into(),
                name: "test".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.request_interaction(
        RequestId::new(),
        InteractionRequest::Permission {
            action_key: "{}".into(),
            description: "Approve test".into(),
        },
    )
    .await
    .unwrap();
    observe_pending(&mut service, &kernel.ports.reader, &mut watch).await;
    let request = serde_json::from_value(serde_json::json!({
        "request_id":"question", "session_id":sid.get(), "kind":"ask_user", "source":"host", "questions":[], "created_at":chrono::Utc::now(),
    })).unwrap();
    tool.request_interaction(RequestId::new(), InteractionRequest::Input { request })
        .await
        .unwrap();
    observe_pending(&mut service, &kernel.ports.reader, &mut watch).await;
    assert_eq!(
        service
            .state
            .recent
            .iter()
            .map(|item| item.attention.as_ref().unwrap().kind)
            .collect::<Vec<_>>(),
        vec![AttentionKind::Permission, AttentionKind::Question]
    );
}

#[tokio::test]
async fn startup_recovers_pending_requests_without_replaying_unobserved_old_replies() {
    let (kernel, mut service, _directory) = fixture().await;
    let sid = session(&kernel).await;
    let attempt = attempt(&kernel, sid).await;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "pending-test".into(),
                name: "test".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.request_interaction(
        RequestId::new(),
        InteractionRequest::Permission {
            action_key: "{}".into(),
            description: "Pending approval".into(),
        },
    )
    .await
    .unwrap();
    // No observer has handled any of the earlier events.
    assert!(service.state.active.is_empty());
    service.recover().await.unwrap();
    service.recover().await.unwrap();
    let state = service.state;
    assert_eq!(state.recent.len(), 1);
    assert_eq!(
        state.recent[0].attention.as_ref().unwrap().kind,
        AttentionKind::Permission
    );
    assert!(state.active.contains_key(&attempt.execution_id()));
}

async fn plugin_host(
    source: NotificationSource,
    enabled: bool,
    previous: Option<Arc<PluginHost>>,
    recording: Arc<RecordingHost>,
) -> Arc<PluginHost> {
    let configured = ConfiguredPlugin {
        enabled,
        ..Default::default()
    };
    let config = PluginsConfig {
        list: [("agena.terminal".into(), configured)].into(),
        ..Default::default()
    };
    PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "agena.terminal".parse().unwrap(),
            super::super::TerminalPlugin::new().with_notifications(Some(source)),
        )],
        config,
        workspace_root: std::env::temp_dir(),
        agena_version: env!("CARGO_PKG_VERSION").into(),
        callback_base_url: None,
        host_client: Some(recording),
        previous_plugins: if previous.is_some() {
            [("agena.terminal".into(), ConfiguredPlugin::default())].into()
        } else {
            Default::default()
        },
        previous,
    })
    .await
    .unwrap()
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("plugin observer should publish through the real host");
}

#[tokio::test]
async fn real_plugin_host_owns_attention_across_reload_disable_and_shutdown() {
    let (kernel, observer, recording) = fixture().await;
    let source = observer.source.clone();
    let sid = session(&kernel).await;
    let host = plugin_host(source.clone(), true, None, recording.clone()).await;
    let first = attempt(&kernel, sid).await;
    first
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    wait_for(|| recording.invalidations.load(Ordering::SeqCst) > 0).await;
    let notifications = host.host_notifications();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].plugin_id, "agena.terminal");
    assert_eq!(notifications[0].session_id, Some(sid.get()));
    let first_id = notifications[0].attention.as_ref().unwrap().id.clone();

    let successor = plugin_host(source.clone(), true, Some(host.clone()), recording.clone()).await;
    let during_handoff = attempt(&kernel, sid).await;
    during_handoff
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    wait_for(|| host.host_notifications().len() == 2).await;
    host.shutdown().await;
    wait_for(|| successor.host_notifications().len() == 2).await;
    assert_eq!(
        successor.host_notifications()[0]
            .attention
            .as_ref()
            .unwrap()
            .id,
        first_id
    );
    let second = attempt(&kernel, sid).await;
    second
        .finish(RequestId::new(), ExecutionPhase::Failed)
        .await
        .unwrap();
    wait_for(|| successor.host_notifications().len() == 3).await;
    assert_ne!(
        successor.host_notifications()[2]
            .attention
            .as_ref()
            .unwrap()
            .id,
        first_id
    );

    let disabled = plugin_host(source.clone(), false, Some(successor.clone()), recording).await;
    successor.shutdown().await;
    let persisted = source.state.get_payload(STATE_KEY).await.unwrap();
    let while_disabled = attempt(&kernel, sid).await;
    while_disabled
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(disabled.host_notifications().is_empty());
    assert_eq!(
        source.state.get_payload(STATE_KEY).await.unwrap(),
        persisted
    );
    assert!(successor.host_notifications().is_empty());
    disabled.shutdown().await;
}

#[tokio::test]
async fn answered_requests_replace_the_pending_intent_and_persist_resolution() {
    let (kernel, mut observer, host) = fixture().await;
    let sid = session(&kernel).await;
    let mut watch = kernel.ports.reader.watch_all(None).unwrap();
    let attempt = attempt(&kernel, sid).await;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "approval".into(),
                name: "test".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let interaction = tool
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: "{}".into(),
                description: "Approve".into(),
            },
        )
        .await
        .unwrap()
        .value;
    observe_pending(&mut observer, &kernel.ports.reader, &mut watch).await;
    kernel
        .ports
        .sessions
        .reply(
            RequestId::new(),
            sid,
            interaction,
            agena_storage::kernel::InteractionDecision::Cancel,
            None,
        )
        .await
        .unwrap();
    observe_pending(&mut observer, &kernel.ports.reader, &mut watch).await;
    assert_eq!(observer.state.recent.len(), 1);
    let resolved = observer.state.recent[0].attention.as_ref().unwrap();
    assert!(resolved.resolved);
    let published = host.notifications.lock().unwrap().clone();
    assert_eq!(published.len(), 2);
    assert_eq!(published[0].attention.as_ref().unwrap().id, resolved.id);
    assert!(published[1].attention.as_ref().unwrap().resolved);
    let mut restored = Observer::load(observer.source.clone(), host.clone())
        .await
        .unwrap();
    restored.recover().await.unwrap();
    assert!(
        host.notifications
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .attention
            .as_ref()
            .unwrap()
            .resolved
    );
}

#[tokio::test]
async fn host_queue_does_not_resurrect_a_resolved_request_during_replay() {
    let (_kernel, observer, recording) = fixture().await;
    let host = plugin_host(observer.source, true, None, recording).await;
    let client = host.host_handle().scoped_host_client("agena.terminal");
    let mut request = notification(
        "request:7:test".into(),
        AttentionKind::Question,
        SessionId::try_from(7).unwrap(),
        "Test",
        now_ms(),
    );
    client.notify(request.clone()).await.unwrap();
    request.attention.as_mut().unwrap().resolved = true;
    client.notify(request.clone()).await.unwrap();
    request.attention.as_mut().unwrap().resolved = false;
    client.notify(request).await.unwrap();
    let queue = host.host_notifications();
    assert_eq!(queue.len(), 1);
    assert!(queue[0].attention.as_ref().unwrap().resolved);
    host.shutdown().await;
}

#[tokio::test]
async fn legacy_browser_tracking_migrates_without_changing_subscription_state() {
    let (kernel, observer, host) = fixture().await;
    let sid = session(&kernel).await;
    let attempt = attempt(&kernel, sid).await;
    let legacy = serde_json::json!({
        "active": BTreeMap::from([(attempt.execution_id(), sid)]),
        "subscribers": {"preserved": {"endpoint": "preserved"}},
    })
    .to_string();
    observer
        .source
        .state
        .set_payload("notifications.webPush", legacy.clone())
        .await
        .unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let mut migrated = Observer::load(observer.source, host).await.unwrap();
    migrated.recover().await.unwrap();
    assert_eq!(migrated.state.recent.len(), 1);
    assert_eq!(
        migrated.state.recent[0].attention.as_ref().unwrap().kind,
        AttentionKind::Completed
    );
    assert_eq!(
        migrated
            .source
            .state
            .get_payload("notifications.webPush")
            .await
            .unwrap(),
        Some(legacy)
    );
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
