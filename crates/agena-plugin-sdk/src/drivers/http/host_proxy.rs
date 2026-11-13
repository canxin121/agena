//! Stable HostClient identity for HTTP plugins across callback destination changes.

use std::sync::{Arc, RwLock};

use crate::activity::ActivitySourceAdapter;
use crate::host_api::*;
use crate::*;

pub(super) struct HostClientProxy {
    current: RwLock<Arc<dyn HostClient>>,
    stop: tokio_util::sync::CancellationToken,
}

impl HostClientProxy {
    pub(super) fn with_cancellation(
        host: Arc<dyn HostClient>,
        stop: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            current: RwLock::new(host),
            stop,
        }
    }

    async fn run<T>(&self, future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        if super::bindings::in_shutdown_scope() {
            return future.await;
        }
        tokio::select! {
            biased;
            _ = self.stop.cancelled() => Err(PluginError::from_kind(PluginErrorKind::HostUnavailable, "HTTP plugin callback owner has stopped")),
            result = future => result,
        }
    }

    fn current(&self) -> Arc<dyn HostClient> {
        self.current
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(super) fn replace(&self, host: Arc<dyn HostClient>) {
        let previous = std::mem::replace(
            &mut *self
                .current
                .write()
                .unwrap_or_else(|error| error.into_inner()),
            host,
        );
        drop(previous);
    }
}

crate::drivers::impl_callback_host_client!(@proxy HostClientProxy, {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        let _ = self
            .run(async {
                self.current().log(level, message, fields).await;
                Ok(())
            })
            .await;
    }

    async fn publish_event(&self, env: EventEnvelope) -> Result<()> {
        self.run(self.current().publish_event(env)).await
    }

    async fn publish_activity(&self, activity: BackgroundActivity) -> Result<()> {
        self.run(self.current().publish_activity(activity)).await
    }

    async fn register_activity_source(
        &self,
        kind: BackgroundActivityKind,
        adapter: Arc<dyn ActivitySourceAdapter>,
    ) -> Result<()> {
        self.run(self.current().register_activity_source(kind, adapter))
            .await
    }

    async fn subscribe_events(&self, filter: EventFilter) -> Result<EventSubscription> {
        self.run(self.current().subscribe_events(filter)).await
    }

    async fn unsubscribe_events(&self, subscription_id: String) -> Result<()> {
        self.run(self.current().unsubscribe_events(subscription_id))
            .await
    }

    async fn read_config(&self, path: Option<String>) -> Result<serde_json::Value> {
        self.run(self.current().read_config(path)).await
    }

    async fn require_path_permission(
        &self,
        request: crate::host_api::PathPermissionQuery,
    ) -> Result<()> {
        self.run(self.current().require_path_permission(request))
            .await
    }

    async fn require_network_permission(&self, target: String) -> Result<()> {
        self.run(self.current().require_network_permission(target))
            .await
    }

    async fn get_session(&self, _req: HostGetSessionRequest) -> Result<HostGetSessionResponse> {
        self.run(self.current().get_session(_req)).await
    }

    async fn rename_session(
        &self,
        _req: HostRenameSessionRequest,
    ) -> Result<HostRenameSessionResponse> {
        self.run(self.current().rename_session(_req)).await
    }
});

#[cfg(test)]
mod generated_callback_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct BlockingHost {
        calls: Arc<AtomicUsize>,
        started: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl HostClient for BlockingHost {
        async fn log(&self, _: LogLevel, _: String, _: serde_json::Value) {}
        async fn publish_event(&self, env: EventEnvelope) -> Result<()> {
            NoopHostClient.publish_event(env).await
        }
        async fn subscribe_events(&self, filter: EventFilter) -> Result<EventSubscription> {
            NoopHostClient.subscribe_events(filter).await
        }
        async fn read_config(&self, path: Option<String>) -> Result<serde_json::Value> {
            NoopHostClient.read_config(path).await
        }
        async fn invoke_tool(
            &self,
            tool: String,
            input: serde_json::Value,
        ) -> Result<ToolInvokeOutput> {
            NoopHostClient.invoke_tool(tool, input).await
        }
        async fn storage_get(
            &self,
            request: HostStorageGetRequest,
        ) -> Result<HostStorageGetResponse> {
            self.check_network_permission(request.key).await?;
            Ok(HostStorageGetResponse {
                value: Some(request.namespace),
            })
        }
        async fn check_network_permission(&self, target: String) -> Result<PermissionQuery> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if target == "pending" {
                self.started.notify_one();
                std::future::pending::<()>().await;
            }
            Ok(PermissionQuery::Allow {})
        }
    }

    #[tokio::test]
    async fn generated_callbacks_follow_replacement_and_owner_cancellation() {
        let stop = tokio_util::sync::CancellationToken::new();
        let proxy = Arc::new(HostClientProxy::with_cancellation(
            Arc::new(NoopHostClient),
            stop.clone(),
        ));
        assert_eq!(
            proxy
                .check_network_permission("ready".into())
                .await
                .unwrap_err()
                .kind,
            PluginErrorKind::HostUnavailable,
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(tokio::sync::Notify::new());
        proxy.replace(Arc::new(BlockingHost {
            calls: calls.clone(),
            started: started.clone(),
        }));
        assert!(matches!(
            proxy
                .check_network_permission("ready".into())
                .await
                .unwrap(),
            PermissionQuery::Allow {}
        ));
        let stored = proxy
            .storage_get(HostStorageGetRequest {
                scope: Default::default(),
                visibility: Default::default(),
                namespace: "replacement-host".into(),
                key: "ready".into(),
            })
            .await
            .unwrap();
        assert_eq!(stored.value.as_deref(), Some("replacement-host"));
        let pending = tokio::spawn({
            let proxy = proxy.clone();
            async move { proxy.check_network_permission("pending".into()).await }
        });
        started.notified().await;
        stop.cancel();
        assert_eq!(
            pending.await.unwrap().unwrap_err().kind,
            PluginErrorKind::HostUnavailable
        );
        assert_eq!(
            proxy
                .check_network_permission("ready".into())
                .await
                .unwrap_err()
                .kind,
            PluginErrorKind::HostUnavailable
        );
        assert_eq!(
            proxy
                .storage_get(HostStorageGetRequest {
                    scope: Default::default(),
                    visibility: Default::default(),
                    namespace: "replacement-host".into(),
                    key: "ready".into(),
                })
                .await
                .unwrap_err()
                .kind,
            PluginErrorKind::HostUnavailable,
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "stopped callbacks must not reach the replacement host"
        );
    }
}
