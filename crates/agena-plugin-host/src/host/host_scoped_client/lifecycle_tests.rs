use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::Poll;

use futures_util::poll;
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::effect_scope::PluginEffectScope;
use crate::host::HostHandle;
use crate::sdk::{PluginErrorKind, PluginKey};

fn key() -> PluginKey {
    "test.scoped-callback".parse().unwrap()
}

#[derive(Default)]
struct BlockedCall {
    release: Notify,
    dropped: CancellationToken,
}

enum Action {
    ReplaceAndRegister(Weak<HostHandle>),
    RegisterOnOtherHost(Weak<HostHandle>),
    Dispose(Arc<PluginEffectScope>),
}

#[derive(Default)]
struct TestHost {
    calls: AtomicUsize,
    logs: AtomicUsize,
    blocked: Option<Arc<BlockedCall>>,
    action: Mutex<Option<Action>>,
    contexts: Mutex<Vec<HostCallbackContext>>,
}

#[async_trait::async_trait]
impl HostClient for TestHost {
    async fn log(&self, _: LogLevel, _: String, _: Value) {
        self.logs.fetch_add(1, Ordering::SeqCst);
    }

    async fn publish_event(&self, _: EventEnvelope) -> crate::sdk::Result<()> {
        Ok(())
    }

    async fn subscribe_events(&self, _: EventFilter) -> crate::sdk::Result<EventSubscription> {
        Ok(EventSubscription { id: "test".into() })
    }

    async fn read_config(&self, _: Option<String>) -> crate::sdk::Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.contexts
            .lock()
            .unwrap()
            .push(host_api::current_host_callback_context().unwrap_or_default());
        let action = self.action.lock().unwrap().take();
        match action {
            Some(Action::ReplaceAndRegister(weak)) => {
                let host = weak.upgrade().unwrap();
                host.begin_plugin_instance(key());
                host.theme_register(
                    &key().to_string(),
                    serde_json::from_value(json!({
                        "id": "during-callback", "display_name": "Original owner", "colors": {}
                    }))
                    .unwrap(),
                )?;
            }
            Some(Action::RegisterOnOtherHost(weak)) => {
                weak.upgrade().unwrap().theme_register(
                    &key().to_string(),
                    serde_json::from_value(json!({
                        "id": "other-host", "display_name": "Other host", "colors": {}
                    }))
                    .unwrap(),
                )?;
            }
            Some(Action::Dispose(scope)) => {
                assert!(scope.dispose().await.errors.is_empty());
            }
            None => {}
        }
        if let Some(blocked) = &self.blocked {
            let _drop = blocked.dropped.clone().drop_guard();
            blocked.release.notified().await;
        }
        Ok(json!({"config": "test"}))
    }

    async fn invoke_tool(&self, _: String, _: Value) -> crate::sdk::Result<ToolInvokeOutput> {
        Err(PluginError::internal("not needed by the callback fixture"))
    }
}

fn setup(inner: Arc<TestHost>) -> (Arc<HostHandle>, Arc<PluginEffectScope>, Arc<dyn HostClient>) {
    let host = Arc::new(HostHandle::new(inner));
    let scope = host.begin_plugin_instance(key());
    let client = host.scoped_host_client_for_scope(key().to_string(), scope.clone());
    (host, scope, client)
}

fn assert_stopped<T>(result: crate::sdk::Result<T>) {
    let error = match result {
        Ok(_) => panic!("a stopped generation must reject the callback"),
        Err(error) => error,
    };
    assert_eq!(error.kind, PluginErrorKind::HostUnavailable);
}

#[tokio::test]
async fn quiesced_client_rejects_runtime_calls() {
    let inner = Arc::new(TestHost::default());
    let (_host, scope, client) = setup(inner.clone());
    scope.quiesce().await;
    assert_stopped(client.read_config(None).await);
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn quiesced_client_suppresses_logs() {
    let inner = Arc::new(TestHost::default());
    let (_host, scope, client) = setup(inner.clone());
    scope.quiesce().await;
    client
        .log(LogLevel::Info, "late log".into(), json!({}))
        .await;
    assert_eq!(inner.logs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn quiesced_client_rejects_catalog_access_and_notifications() {
    let (host, scope, client) = setup(Arc::new(TestHost::default()));
    scope.quiesce().await;
    let catalog = client.list_plugins().await;
    let notification = client
        .notify(serde_json::from_value(json!({"body": "late notification"})).unwrap())
        .await;
    assert_stopped(catalog);
    assert_stopped(notification);
    assert!(host.host_notifications().is_empty());
}

#[tokio::test]
async fn inflight_callback_is_accounted_until_completion() {
    let blocked = Arc::new(BlockedCall::default());
    let (_host, scope, client) = setup(Arc::new(TestHost {
        blocked: Some(blocked.clone()),
        ..Default::default()
    }));
    let mut callback = Box::pin(client.read_config(None));
    assert!(poll!(&mut callback).is_pending());
    assert_eq!(scope.active_leases(), 1);
    blocked.release.notify_one();
    assert_eq!(callback.await.unwrap(), json!({"config": "test"}));
    assert_eq!(scope.active_leases(), 0);
    assert!(scope.dispose().await.errors.is_empty());
}

#[tokio::test]
async fn disposal_cancels_inflight_callback_before_running_disposers() {
    let blocked = Arc::new(BlockedCall::default());
    let (_host, scope, client) = setup(Arc::new(TestHost {
        blocked: Some(blocked.clone()),
        ..Default::default()
    }));
    let callback_dropped = blocked.dropped.clone();
    scope
        .own_sync("test", "check callback settled", move || {
            if callback_dropped.is_cancelled() {
                Ok(())
            } else {
                Err("cleanup ran while a callback was still alive".into())
            }
        })
        .unwrap();
    let mut callback = Box::pin(client.read_config(None));
    assert!(poll!(&mut callback).is_pending());
    let mut cleanup = Box::pin(scope.dispose());
    assert!(poll!(&mut cleanup).is_pending());
    let Poll::Ready(result) = poll!(&mut callback) else {
        panic!("scope disposal must cancel the in-flight callback");
    };
    assert_stopped(result);
    assert!(blocked.dropped.is_cancelled());
    assert!(cleanup.await.errors.is_empty());
    assert_eq!(scope.active_leases(), 0);
}

#[tokio::test]
async fn quiesce_cancels_callback_waiting_for_client_lock() {
    let inner = Arc::new(TestHost::default());
    let (host, scope, client) = setup(inner.clone());
    let installation = host.inner.write().await;
    let mut callback = Box::pin(client.read_config(None));
    assert!(poll!(&mut callback).is_pending());
    let mut stop = Box::pin(scope.quiesce());
    let _ = poll!(&mut stop);
    let Poll::Ready(result) = poll!(&mut callback) else {
        panic!("quiesce must cancel a callback waiting to read the host client");
    };
    assert_stopped(result);
    stop.await;
    drop(installation);
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
    assert_eq!(scope.active_leases(), 0);
}

#[tokio::test]
async fn admitted_callback_registration_keeps_its_original_owner() {
    let inner = Arc::new(TestHost::default());
    let (host, original, client) = setup(inner.clone());
    // Simulate a generation switch inside the host operation. The callback
    // was accepted by the original scope and must not borrow its successor.
    *inner.action.lock().unwrap() = Some(Action::ReplaceAndRegister(Arc::downgrade(&host)));
    client.read_config(None).await.unwrap();
    let successor = host.effect_scope(&key()).unwrap();
    assert!(!Arc::ptr_eq(&original, &successor));
    assert_eq!(host.themes.owned_values(&original).len(), 1);
    assert!(host.themes.owned_values(&successor).is_empty());
    host.theme_register(
        &key().to_string(),
        serde_json::from_value(json!({
            "id": "after-callback", "display_name": "Successor owner", "colors": {}
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(host.themes.owned_values(&successor).len(), 1);
    assert!(original.dispose().await.errors.is_empty());
    assert_eq!(host.theme_list_response().themes[0].id, "after-callback");
    assert!(successor.dispose().await.errors.is_empty());
    assert!(host.theme_list_response().themes.is_empty());
}

#[tokio::test]
async fn caller_cancellation_drops_the_callback_and_releases_its_lease() {
    let blocked = Arc::new(BlockedCall::default());
    let (_host, scope, client) = setup(Arc::new(TestHost {
        blocked: Some(blocked.clone()),
        ..Default::default()
    }));
    let mut callback = Box::pin(client.read_config(None));
    assert!(poll!(&mut callback).is_pending());
    assert_eq!(scope.active_leases(), 1);
    drop(callback);
    assert!(blocked.dropped.is_cancelled());
    assert_eq!(scope.active_leases(), 0);
    assert!(scope.dispose().await.errors.is_empty());
}

#[tokio::test]
async fn callback_can_request_its_own_disposal_without_deadlocking() {
    let inner = Arc::new(TestHost::default());
    let (_host, scope, client) = setup(inner.clone());
    *inner.action.lock().unwrap() = Some(Action::Dispose(scope.clone()));
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), client.read_config(None))
        .await
        .expect("a callback must not wait indefinitely for its own lease");
    assert_stopped(result);
    let report = tokio::time::timeout(std::time::Duration::from_secs(1), scope.dispose())
        .await
        .expect("scope cleanup must continue when the callback stops");
    assert!(report.errors.is_empty());
    assert_eq!(scope.active_leases(), 0);
}

#[tokio::test]
async fn stale_client_is_rejected_while_the_successor_remains_usable() {
    let inner = Arc::new(TestHost::default());
    let (host, original, stale) = setup(inner.clone());
    let successor = host.begin_plugin_instance(key());
    let current = host.scoped_host_client_for_scope(key().to_string(), successor.clone());
    assert_stopped(stale.read_config(None).await);
    stale.log(LogLevel::Info, "stale".into(), json!({})).await;
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
    assert_eq!(inner.logs.load(Ordering::SeqCst), 0);
    original.dispose().await;
    assert_eq!(
        current.read_config(None).await.unwrap(),
        json!({"config": "test"})
    );
    assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
    assert_eq!(successor.active_leases(), 0);
}

#[tokio::test]
async fn callbacks_preserve_host_issued_authority_and_reject_expired_context() {
    let inner = Arc::new(TestHost::default());
    let (host, scope, client) = setup(inner.clone());
    let (context, authority) = host.issue_callback_authority(
        &key(),
        HostCallbackContext {
            session_id: Some(17),
            workspace_root: Some("/test/workspace".into()),
            ..Default::default()
        },
    );
    host_api::run_in_host_callback_context(context.clone(), client.read_config(None))
        .await
        .unwrap();
    {
        let observed = inner.contexts.lock().unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(
            observed[0].plugin_id.as_deref(),
            Some(key().to_string().as_str())
        );
        assert_eq!(observed[0].session_id, context.session_id);
        assert_eq!(observed[0].workspace_root, context.workspace_root);
        assert_eq!(observed[0].authority_token, context.authority_token);
    }
    drop(authority);
    let error = host_api::run_in_host_callback_context(context, client.read_config(None))
        .await
        .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
    assert_eq!(scope.active_leases(), 0);
}

#[tokio::test]
async fn nested_work_on_another_host_does_not_borrow_the_callback_owner() {
    let inner = Arc::new(TestHost::default());
    let (_host, owner, client) = setup(inner.clone());
    let (other_host, other_owner, _) = setup(Arc::new(TestHost::default()));
    *inner.action.lock().unwrap() = Some(Action::RegisterOnOtherHost(Arc::downgrade(&other_host)));
    client.read_config(None).await.unwrap();
    assert!(other_host.themes.owned_values(&owner).is_empty());
    assert_eq!(other_host.themes.owned_values(&other_owner).len(), 1);
    assert!(owner.dispose().await.errors.is_empty());
    assert_eq!(other_host.theme_list_response().themes.len(), 1);
    assert!(other_owner.dispose().await.errors.is_empty());
    assert!(other_host.theme_list_response().themes.is_empty());
}
