use super::*;
use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::{KernelWatch, RequestId, ToolStart};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn subscriber(endpoint: String) -> Subscriber {
    Subscriber {
        subscription: SubscriptionInfo::new(endpoint,
            "BLMbF9ffKBiWQLCKvTHb6LO8Nb6dcUh6TItC455vu2kElga6PQvUmaFyCdykxY2nOSSL3yKgfbmFLRTUaGv4yV8".into(),
            "xS03Fi5ErfTNH_l9WHE9Ig".into()),
        messages: [EventKind::Completed, EventKind::Failed, EventKind::Cancelled, EventKind::Permission, EventKind::Question, EventKind::Test]
            .into_iter().map(|kind| (kind, format!("{kind:?}"))).collect(),
    }
}

async fn fixture() -> (
    agena_storage_sqlite::SqliteKernel,
    WebPushService,
    tempfile::TempDir,
) {
    let kernel = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let db = Arc::new(
        ServerStateDb::from_store(kernel.instance_state.clone())
            .await
            .unwrap(),
    );
    let service = WebPushService::load_with_secrets(
        db,
        FileSecretStore::at(directory.path(), "test.web-push"),
    )
    .await
    .unwrap();
    let mut state = service.state.lock().await;
    state.subscribers.insert(
        "browser".into(),
        subscriber("https://fcm.googleapis.com/fcm/send/test".into()),
    );
    service.persist(&state).await.unwrap();
    drop(state);
    (kernel, service, directory)
}

async fn observe_pending(service: &WebPushService, reader: &KernelReader, watch: &mut KernelWatch) {
    while let Some(change) = watch.try_recv().unwrap() {
        service.observe(reader, &change).await.unwrap();
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
async fn real_kernel_completions_and_failures_enqueue_without_any_browser_or_sse_client() {
    let (kernel, service, _directory) = fixture().await;
    let sid = session(&kernel).await;
    let mut watch = kernel.ports.reader.watch_all(None).unwrap();
    for phase in [
        ExecutionPhase::Succeeded,
        ExecutionPhase::Failed,
        ExecutionPhase::Cancelled,
    ] {
        let attempt = attempt(&kernel, sid).await;
        observe_pending(&service, &kernel.ports.reader, &mut watch).await;
        assert!(
            service
                .state
                .lock()
                .await
                .outbox
                .iter()
                .all(|item| item.event.kind != EventKind::Question)
        );
        attempt.finish(RequestId::new(), phase).await.unwrap();
        observe_pending(&service, &kernel.ports.reader, &mut watch).await;
    }
    let state = service.state.lock().await;
    assert_eq!(
        state
            .outbox
            .iter()
            .map(|item| item.event.kind)
            .collect::<Vec<_>>(),
        vec![
            EventKind::Completed,
            EventKind::Failed,
            EventKind::Cancelled
        ]
    );
    assert!(state.active.is_empty());
}

#[tokio::test]
async fn canonical_questions_and_permissions_enqueue_once_without_tool_display_events() {
    let (kernel, service, _directory) = fixture().await;
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
    observe_pending(&service, &kernel.ports.reader, &mut watch).await;
    let request = serde_json::from_value(serde_json::json!({
        "request_id":"question", "session_id":sid.get(), "kind":"ask_user", "source":"host", "questions":[], "created_at":chrono::Utc::now(),
    })).unwrap();
    tool.request_interaction(RequestId::new(), InteractionRequest::Input { request })
        .await
        .unwrap();
    observe_pending(&service, &kernel.ports.reader, &mut watch).await;
    assert_eq!(
        service
            .state
            .lock()
            .await
            .outbox
            .iter()
            .map(|item| item.event.kind)
            .collect::<Vec<_>>(),
        vec![EventKind::Permission, EventKind::Question]
    );
}

#[tokio::test]
async fn enabling_push_during_an_existing_question_recovers_it_without_replaying_old_replies() {
    let (kernel, service, _directory) = fixture().await;
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
    assert!(service.state.lock().await.active.is_empty());
    service.recover(&kernel.ports.reader).await.unwrap();
    service.recover(&kernel.ports.reader).await.unwrap();
    let state = service.state.lock().await;
    assert_eq!(state.outbox.len(), 1);
    assert_eq!(state.outbox[0].event.kind, EventKind::Permission);
    assert!(state.active.contains_key(&attempt.execution_id()));
}

#[tokio::test]
async fn long_executions_and_lost_finish_events_recover_after_restart_with_stable_identity() {
    let (kernel, service, directory) = fixture().await;
    let sid = session(&kernel).await;
    let mut watch = kernel.ports.reader.watch_all(None).unwrap();
    let attempt = attempt(&kernel, sid).await;
    observe_pending(&service, &kernel.ports.reader, &mut watch).await;
    for _ in 0..130 {
        attempt
            .notice(RequestId::new(), "progress".into())
            .await
            .unwrap();
        observe_pending(&service, &kernel.ports.reader, &mut watch).await;
    }
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let finish = watch.recv().await.unwrap();
    assert!(finish.requires_snapshot);
    // Simulate a crash before the observer handles the terminal change.
    let restored = WebPushService::load_with_secrets(
        service.db.clone(),
        FileSecretStore::at(directory.path(), "test.web-push"),
    )
    .await
    .unwrap();
    assert_eq!(restored.public_key, service.public_key);
    restored.recover(&kernel.ports.reader).await.unwrap();
    restored
        .observe(&kernel.ports.reader, &finish)
        .await
        .unwrap();
    let state = restored.state.lock().await;
    assert_eq!(state.subscribers.len(), 1);
    assert_eq!(state.outbox.len(), 1);
    assert_eq!(state.outbox[0].event.kind, EventKind::Completed);
}

async fn push_server(status: u16) -> (String, JoinHandle<Vec<u8>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/push", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut bytes = [0; 4096];
            let n = socket.read(&mut bytes).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&bytes[..n]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        socket
            .write_all(
                format!("HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        request
    });
    (endpoint, task)
}

#[tokio::test]
async fn encrypted_vapid_delivery_retries_failures_and_removes_expired_subscriptions() {
    let (_kernel, service, _directory) = fixture().await;
    for status in [503, 201, 410] {
        let (endpoint, response) = push_server(status).await;
        {
            let mut state = service.state.lock().await;
            state
                .subscribers
                .insert("browser".into(), subscriber(endpoint));
            if state.outbox.is_empty() {
                enqueue(
                    &mut state,
                    PushEvent {
                        id: format!("delivery-{status}"),
                        session_id: Some(7),
                        title: "Private title".into(),
                        kind: EventKind::Question,
                        created_at_ms: now_ms(),
                    },
                );
            }
            state.outbox[0].next_at_ms = 0;
        }
        service.send_due().await.unwrap();
        let request = response.await.unwrap();
        let text = String::from_utf8_lossy(&request).to_ascii_lowercase();
        assert!(text.contains("content-encoding: aes128gcm"));
        assert!(text.contains("authorization: vapid t="));
        assert!(text.contains("urgency: high"));
        assert!(!text.contains("private title"));
        let state = service.state.lock().await;
        if status == 503 {
            assert_eq!(state.outbox.len(), 1);
            assert_eq!(state.outbox[0].attempts, 1);
            assert!(state.outbox[0].next_at_ms > now_ms());
        } else {
            assert!(state.outbox.is_empty());
        }
        assert_eq!(state.subscribers.is_empty(), status == 410);
    }
}

#[test]
fn rejects_non_browser_push_endpoints_and_invalid_keys() {
    for endpoint in [
        "http://fcm.googleapis.com/push",
        "https://127.0.0.1/push",
        "https://example.com/push",
        "https://fcm.googleapis.com:444/push",
        "https://fcm.googleapis.com.evil.test/push",
    ] {
        assert!(validate_subscription(&subscriber(endpoint.into())).is_err());
    }
    for endpoint in [
        "https://fcm.googleapis.com/fcm/send/test",
        "https://updates.push.services.mozilla.com/wpush/v2/test",
        "https://web.push.apple.com/test",
        "https://wns2-db5p.notify.windows.com/w/?token=test",
    ] {
        assert!(validate_subscription(&subscriber(endpoint.into())).is_ok());
    }
    let mut invalid = subscriber("https://fcm.googleapis.com/fcm/send/test".into());
    invalid.subscription.keys.auth = "bad".into();
    assert!(validate_subscription(&invalid).is_err());
}
