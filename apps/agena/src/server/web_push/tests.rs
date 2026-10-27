use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn subscriber(endpoint: String) -> Subscriber {
    Subscriber {
        since_ms: 0,
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

fn plugin_event(id: &str, kind: &str, created_at_ms: i64, resolved: bool) -> HostNotification {
    serde_json::from_value(serde_json::json!({
        "plugin_id": "agena.notifications", "title": "Session title", "session_id": 7,
        "attention": { "id": id, "kind": kind, "created_at_ms": created_at_ms, "resolved": resolved },
    })).unwrap()
}

#[tokio::test]
async fn plugin_events_share_terminal_identities_and_resolved_requests_leave_the_outbox() {
    let (_kernel, service, _directory) = fixture().await;
    let notifications = vec![
        plugin_event("execution:one:1", "completed", now_ms(), false),
        plugin_event("request:7:one", "question", now_ms(), false),
    ];
    service
        .sync_notifications(notifications.clone(), None)
        .await
        .unwrap();
    service
        .sync_notifications(notifications, None)
        .await
        .unwrap();
    assert_eq!(service.state.lock().await.outbox.len(), 2);
    service
        .sync_notifications(
            vec![
                plugin_event("request:7:one", "question", now_ms(), true),
                plugin_event("execution:two:1", "completed", now_ms(), false),
            ],
            None,
        )
        .await
        .unwrap();
    let state = service.state.lock().await;
    assert_eq!(
        state
            .outbox
            .iter()
            .map(|delivery| delivery.event.id.as_str())
            .collect::<Vec<_>>(),
        vec!["execution:one:1", "execution:two:1"]
    );
}

#[tokio::test]
async fn newly_subscribed_browsers_receive_pending_requests_without_replaying_old_replies() {
    let (_kernel, service, _directory) = fixture().await;
    service
        .state
        .lock()
        .await
        .subscribers
        .get_mut("browser")
        .unwrap()
        .since_ms = now_ms();
    let old = now_ms() - 60_000;
    let notifications = vec![
        plugin_event("old-completion", "completed", old, false),
        plugin_event("pending", "permission", old, false),
    ];
    service
        .sync_notifications(notifications.clone(), Some("browser"))
        .await
        .unwrap();
    let mut second = subscriber("https://fcm.googleapis.com/fcm/send/second".into());
    second.since_ms = now_ms();
    service
        .state
        .lock()
        .await
        .subscribers
        .insert("second".into(), second);
    service
        .sync_notifications(notifications, Some("second"))
        .await
        .unwrap();
    let state = service.state.lock().await;
    assert_eq!(state.outbox.len(), 2);
    assert!(
        state
            .outbox
            .iter()
            .all(|delivery| delivery.event.id == "pending")
    );
}

#[tokio::test]
async fn transport_restart_preserves_identity_and_plugin_delivery_deduplication() {
    let (_kernel, service, directory) = fixture().await;
    let notifications = vec![plugin_event("persisted", "completed", now_ms(), false)];
    service
        .sync_notifications(notifications.clone(), None)
        .await
        .unwrap();
    let restored = WebPushService::load_with_secrets(
        service.db.clone(),
        FileSecretStore::at(directory.path(), "test.web-push"),
    )
    .await
    .unwrap();
    assert_eq!(restored.public_key, service.public_key);
    restored
        .sync_notifications(notifications, None)
        .await
        .unwrap();
    let state = restored.state.lock().await;
    assert_eq!(state.subscribers.len(), 1);
    assert_eq!(state.outbox.len(), 1);
    assert_eq!(state.outbox[0].event.id, "persisted");
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

#[tokio::test]
async fn a_still_pending_old_question_is_delivered_to_a_new_browser() {
    let (_kernel, service, _directory) = fixture().await;
    let (endpoint, response) = push_server(201).await;
    service
        .state
        .lock()
        .await
        .subscribers
        .insert("browser".into(), subscriber(endpoint));
    service
        .sync_notifications(
            vec![plugin_event(
                "old-pending",
                "question",
                now_ms() - TTL_MS - 1,
                false,
            )],
            Some("browser"),
        )
        .await
        .unwrap();
    service.send_due().await.unwrap();
    assert!(!response.await.unwrap().is_empty());
    let state = service.state.lock().await;
    assert!(state.outbox.is_empty());
    assert_eq!(state.subscribers.len(), 1);
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
