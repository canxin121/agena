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

    async fn ask_user(&self, _req: AskUserRequest) -> Result<AskUserResponse> {
        self.run(self.current().ask_user(_req)).await
    }

    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>> {
        self.run(self.current().list_tools()).await
    }

    async fn get_session(&self, _req: HostGetSessionRequest) -> Result<HostGetSessionResponse> {
        self.run(self.current().get_session(_req)).await
    }

    async fn get_context_status(
        &self,
        _req: HostContextStatusRequest,
    ) -> Result<HostContextStatusResponse> {
        self.run(self.current().get_context_status(_req)).await
    }

    async fn rename_session(
        &self,
        _req: HostRenameSessionRequest,
    ) -> Result<HostRenameSessionResponse> {
        self.run(self.current().rename_session(_req)).await
    }

    async fn set_session_model(
        &self,
        _req: HostSetSessionModelRequest,
    ) -> Result<HostSetSessionModelResponse> {
        self.run(self.current().set_session_model(_req)).await
    }

    async fn monitor_start(&self, _req: MonitorStartRequest) -> Result<MonitorHandle> {
        self.run(self.current().monitor_start(_req)).await
    }

    async fn monitor_list(&self) -> Result<Vec<MonitorHandle>> {
        self.run(self.current().monitor_list()).await
    }

    async fn monitor_read(&self, _req: MonitorReadRequest) -> Result<MonitorReadResponse> {
        self.run(self.current().monitor_read(_req)).await
    }

    async fn monitor_stop(&self, _req: MonitorStopRequest) -> Result<MonitorHandle> {
        self.run(self.current().monitor_stop(_req)).await
    }

    async fn register_tool(
        &self,
        _req: HostToolRegisterRequest,
    ) -> Result<HostToolMutationResponse> {
        self.run(self.current().register_tool(_req)).await
    }

    async fn update_tool(&self, _req: HostToolUpdateRequest) -> Result<HostToolMutationResponse> {
        self.run(self.current().update_tool(_req)).await
    }

    async fn remove_tool(&self, _req: HostToolRemoveRequest) -> Result<HostToolMutationResponse> {
        self.run(self.current().remove_tool(_req)).await
    }

    async fn list_registered_tools(&self) -> Result<HostRegisteredToolListResponse> {
        self.run(self.current().list_registered_tools()).await
    }

    async fn list_plugins(&self) -> Result<HostPluginListResponse> {
        self.run(self.current().list_plugins()).await
    }

    async fn storage_get(&self, _req: HostStorageGetRequest) -> Result<HostStorageGetResponse> {
        self.run(self.current().storage_get(_req)).await
    }

    async fn storage_set(&self, _req: HostStorageSetRequest) -> Result<()> {
        self.run(self.current().storage_set(_req)).await
    }

    async fn storage_delete(&self, _req: HostStorageDeleteRequest) -> Result<()> {
        self.run(self.current().storage_delete(_req)).await
    }

    async fn storage_list(&self, _req: HostStorageListRequest) -> Result<HostStorageListResponse> {
        self.run(self.current().storage_list(_req)).await
    }

    async fn secret_get(&self, _req: HostSecretGetRequest) -> Result<HostSecretGetResponse> {
        self.run(self.current().secret_get(_req)).await
    }

    async fn secret_set(&self, _req: HostSecretSetRequest) -> Result<()> {
        self.run(self.current().secret_set(_req)).await
    }

    async fn secret_delete(&self, _req: HostSecretDeleteRequest) -> Result<()> {
        self.run(self.current().secret_delete(_req)).await
    }

    async fn secret_list(&self) -> Result<HostSecretListResponse> {
        self.run(self.current().secret_list()).await
    }

    async fn plugin_status_list(&self) -> Result<HostPluginStatusListResponse> {
        self.run(self.current().plugin_status_list()).await
    }

    async fn plugin_status_get(
        &self,
        _req: HostPluginStatusGetRequest,
    ) -> Result<HostPluginStatusGetResponse> {
        self.run(self.current().plugin_status_get(_req)).await
    }

    async fn lsp_list_servers(&self) -> Result<HostLspListServersResponse> {
        self.run(self.current().lsp_list_servers()).await
    }

    async fn lsp_list_diagnostics(
        &self,
        _req: HostLspListDiagnosticsRequest,
    ) -> Result<HostLspListDiagnosticsResponse> {
        self.run(self.current().lsp_list_diagnostics(_req)).await
    }

    async fn scheduler_list(&self) -> Result<HostSchedulerListResponse> {
        self.run(self.current().scheduler_list()).await
    }

    async fn scheduler_create(
        &self,
        _req: HostSchedulerCreateRequest,
    ) -> Result<HostSchedulerCreateResponse> {
        self.run(self.current().scheduler_create(_req)).await
    }

    async fn scheduler_delete(
        &self,
        _req: HostSchedulerDeleteRequest,
    ) -> Result<HostSchedulerDeleteResponse> {
        self.run(self.current().scheduler_delete(_req)).await
    }

    async fn hook_list(&self) -> Result<HostHookListResponse> {
        self.run(self.current().hook_list()).await
    }

    async fn mcp_list_servers(&self) -> Result<HostMcpListServersResponse> {
        self.run(self.current().mcp_list_servers()).await
    }

    async fn mcp_add_server(&self, _req: HostMcpAddServerRequest) -> Result<()> {
        self.run(self.current().mcp_add_server(_req)).await
    }

    async fn mcp_remove_server(
        &self,
        _req: HostMcpRemoveServerRequest,
    ) -> Result<HostMcpRemoveServerResponse> {
        self.run(self.current().mcp_remove_server(_req)).await
    }

    async fn display_contribute(&self, _req: HostDisplayContributeRequest) -> Result<()> {
        self.run(self.current().display_contribute(_req)).await
    }

    async fn display_remove(
        &self,
        _req: HostDisplayRemoveRequest,
    ) -> Result<HostDisplayRemoveResponse> {
        self.run(self.current().display_remove(_req)).await
    }

    async fn notify(&self, _req: PluginNotifyRequest) -> Result<()> {
        self.run(self.current().notify(_req)).await
    }

    async fn ui_theme_register(&self, _req: HostThemeRegisterRequest) -> Result<()> {
        self.run(self.current().ui_theme_register(_req)).await
    }

    async fn ui_theme_list(&self) -> Result<HostThemeListResponse> {
        self.run(self.current().ui_theme_list()).await
    }

    async fn ui_theme_remove(
        &self,
        _req: HostThemeRemoveRequest,
    ) -> Result<HostThemeRemoveResponse> {
        self.run(self.current().ui_theme_remove(_req)).await
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
            calls.load(Ordering::SeqCst),
            2,
            "stopped callbacks must not reach the replacement host"
        );
    }
}
