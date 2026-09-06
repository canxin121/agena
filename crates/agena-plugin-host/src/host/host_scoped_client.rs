#[cfg(test)]
mod authority_tests;
#[cfg(test)]
mod lifecycle_tests;

impl ScopedHostClient {
    fn stopped_error(&self) -> PluginError {
        PluginError::from_kind(
            crate::sdk::PluginErrorKind::HostUnavailable,
            format!(
                "plugin callback generation {} for `{}` is stopped or stale",
                self.effect_scope.generation(),
                self.plugin_id
            ),
        )
        .with_plugin(self.plugin_id.clone())
    }

    async fn run_callback<T>(
        &self,
        callback: impl std::future::Future<Output = crate::sdk::Result<T>>,
    ) -> crate::sdk::Result<T> {
        // Admit against the exact generation while publication of a successor
        // is excluded. The lease covers lock waits as well as the host call.
        let _lease = {
            let scopes = super::host_handle::recover_read(
                &self.handle.effect_scopes,
                "plugin effect scope registry",
            );
            if !scopes
                .get(&self.plugin_key)
                .is_some_and(|scope| std::sync::Arc::ptr_eq(scope, &self.effect_scope))
            {
                return Err(self.stopped_error());
            }
            self.effect_scope
                .lease()
                .map_err(|_| self.stopped_error())?
        };
        // Local registry methods read the task-local session just like runtime
        // calls do. Validate and attribute that context before either path can
        // observe it, including calls that never await the inner HostClient.
        let context = self.context()?;
        let stop = self.effect_scope.cancellation_token();
        self.handle
            .run_in_callback_effect_scope(
                self.effect_scope.clone(),
                host_api::run_in_isolated_host_callback_context(context, async {
                    // Dropping the host future on shutdown also releases callbacks
                    // that themselves requested a reload or scope disposal.
                    self.handle
                        .run_in_callback_completion(&self.context()?, async {
                            tokio::select! {
                                biased;
                                _ = stop.cancelled() => Err(self.stopped_error()),
                                result = callback => result,
                            }
                        })
                        .await
                }),
            )
            .await
    }

    fn context(&self) -> crate::sdk::Result<HostCallbackContext> {
        let mut context = host_api::current_host_callback_context().unwrap_or_default();
        context.plugin_id = Some(self.plugin_id.clone());
        self.handle
            .validated_callback_context(Some(self.plugin_id.clone()), Some(context))
    }
}

#[async_trait::async_trait]
impl HostClient for ScopedHostClient {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        let _ = self
            .run_callback(async {
                let context = self.context()?;
                let inner = self.handle.inner.read().await.clone();
                host_api::run_in_host_callback_context(context, inner.log(level, message, fields))
                    .await;
                Ok(())
            })
            .await;
    }

    async fn publish_event(&self, env: EventEnvelope) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.publish_event(env)).await
        })
        .await
    }

    async fn publish_activity(&self, activity: BackgroundActivity) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.publish_activity(activity),
            )
            .await
        })
        .await
    }

    async fn register_activity_source(
        &self,
        kind: BackgroundActivityKind,
        adapter: std::sync::Arc<dyn crate::sdk::activity::ActivitySourceAdapter>,
    ) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.register_activity_source(kind, adapter),
            )
            .await
        })
        .await
    }

    async fn subscribe_events(&self, filter: EventFilter) -> crate::sdk::Result<EventSubscription> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.subscribe_events(filter))
                .await
        })
        .await
    }

    async fn unsubscribe_events(&self, subscription_id: String) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.unsubscribe_events(subscription_id),
            )
            .await
        })
        .await
    }

    async fn read_config(&self, path: Option<String>) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.read_config(path)).await
        })
        .await
    }

    async fn reload_config(&self) -> crate::sdk::Result<HostConfigReloadResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.reload_config()).await
        })
        .await
    }

    async fn request_config_reload(&self) -> crate::sdk::Result<HostConfigReloadRequestResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.request_config_reload())
                .await
        })
        .await
    }

    async fn config_reload_status(
        &self,
        request: HostConfigReloadStatusRequest,
    ) -> crate::sdk::Result<HostConfigReloadStatusResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.config_reload_status(request),
            )
            .await
        })
        .await
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.invoke_tool(tool, input))
                .await
        })
        .await
    }

    async fn invoke_service(
        &self,
        req: crate::sdk::PluginServiceInvokeInput,
    ) -> crate::sdk::Result<crate::sdk::PluginServiceInvokeOutput> {
        self.run_callback(async {
            self.handle
                .invoke_service_for_plugin(&self.plugin_id, req, Some(self.context()?))
                .await
        })
        .await
    }

    async fn ask_user(&self, req: AskUserRequest) -> crate::sdk::Result<AskUserResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.ask_user(req)).await
        })
        .await
    }

    async fn run_subtask(&self, req: RunSubtaskRequest) -> crate::sdk::Result<RunSubtaskResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.run_subtask(req)).await
        })
        .await
    }

    async fn cancel_subtask(
        &self,
        req: CancelSubtaskRequest,
    ) -> crate::sdk::Result<SubtaskControlResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.cancel_subtask(req)).await
        })
        .await
    }

    async fn message_subtask(
        &self,
        req: MessageSubtaskRequest,
    ) -> crate::sdk::Result<SubtaskControlResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.message_subtask(req))
                .await
        })
        .await
    }

    async fn read_subtask_output(
        &self,
        req: ReadSubtaskOutputRequest,
    ) -> crate::sdk::Result<ReadSubtaskOutputResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.read_subtask_output(req))
                .await
        })
        .await
    }

    async fn list_tools(&self) -> crate::sdk::Result<Vec<ToolDescriptor>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.list_tools()).await
        })
        .await
    }

    async fn get_context_status(
        &self,
        req: HostContextStatusRequest,
    ) -> crate::sdk::Result<HostContextStatusResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.get_context_status(req))
                .await
        })
        .await
    }

    async fn get_session(
        &self,
        req: crate::sdk::host_api::HostGetSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostGetSessionResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.get_session(req)).await
        })
        .await
    }

    async fn rename_session(
        &self,
        req: crate::sdk::host_api::HostRenameSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostRenameSessionResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.rename_session(req)).await
        })
        .await
    }

    async fn set_session_model(
        &self,
        req: crate::sdk::host_api::HostSetSessionModelRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostSetSessionModelResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.set_session_model(req))
                .await
        })
        .await
    }

    async fn image_execute(
        &self,
        req: HostImageExecuteRequest,
    ) -> crate::sdk::Result<HostImageExecuteResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.image_execute(req)).await
        })
        .await
    }

    async fn enter_snapshot(
        &self,
        req: HostEnterSnapshotRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.enter_snapshot(req)).await
        })
        .await
    }

    async fn exit_snapshot(
        &self,
        req: HostExitSnapshotRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.exit_snapshot(req)).await
        })
        .await
    }

    async fn monitor_start(&self, req: MonitorStartRequest) -> crate::sdk::Result<MonitorHandle> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.monitor_start(req)).await
        })
        .await
    }

    async fn monitor_list(&self) -> crate::sdk::Result<Vec<MonitorHandle>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.monitor_list()).await
        })
        .await
    }

    async fn monitor_read(
        &self,
        req: MonitorReadRequest,
    ) -> crate::sdk::Result<MonitorReadResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.monitor_read(req)).await
        })
        .await
    }

    async fn monitor_stop(&self, req: MonitorStopRequest) -> crate::sdk::Result<MonitorHandle> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.monitor_stop(req)).await
        })
        .await
    }

    async fn register_tool(
        &self,
        req: HostToolRegisterRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.run_callback(async {
            self.handle
                .tool_upsert_for_plugin(&self.plugin_id, req.tool)
        })
        .await
    }

    async fn update_tool(
        &self,
        req: HostToolUpdateRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.run_callback(async {
            self.handle
                .tool_upsert_for_plugin(&self.plugin_id, req.tool)
        })
        .await
    }

    async fn remove_tool(
        &self,
        req: HostToolRemoveRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.run_callback(async {
            self.handle
                .tool_remove_for_plugin(&self.plugin_id, &req.name, req.by_model_name)
        })
        .await
    }

    async fn list_registered_tools(&self) -> crate::sdk::Result<HostRegisteredToolListResponse> {
        self.run_callback(async { self.handle.registered_tool_list_response() })
            .await
    }

    async fn list_plugins(&self) -> crate::sdk::Result<crate::sdk::HostPluginListResponse> {
        self.run_callback(async { Ok(self.handle.plugin_list_response()) })
            .await
    }

    async fn storage_get(
        &self,
        req: HostStorageGetRequest,
    ) -> crate::sdk::Result<HostStorageGetResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.storage_get(req)).await
        })
        .await
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.storage_set(req)).await
        })
        .await
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.storage_delete(req)).await
        })
        .await
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> crate::sdk::Result<HostStorageListResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.storage_list(req)).await
        })
        .await
    }

    async fn secret_get(
        &self,
        req: HostSecretGetRequest,
    ) -> crate::sdk::Result<HostSecretGetResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.secret_get(req)).await
        })
        .await
    }

    async fn secret_set(&self, req: HostSecretSetRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.secret_set(req)).await
        })
        .await
    }

    async fn secret_delete(&self, req: HostSecretDeleteRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.secret_delete(req)).await
        })
        .await
    }

    async fn secret_list(&self) -> crate::sdk::Result<HostSecretListResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.secret_list()).await
        })
        .await
    }

    async fn plugin_status_list(&self) -> crate::sdk::Result<HostPluginStatusListResponse> {
        self.run_callback(async { Ok(self.handle.plugin_status_list_response()) })
            .await
    }

    async fn plugin_status_get(
        &self,
        req: HostPluginStatusGetRequest,
    ) -> crate::sdk::Result<HostPluginStatusGetResponse> {
        self.run_callback(async { Ok(self.handle.plugin_status_get_response(&req.plugin_id)) })
            .await
    }

    async fn lsp_list_servers(&self) -> crate::sdk::Result<HostLspListServersResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.lsp_list_servers()).await
        })
        .await
    }

    async fn lsp_list_diagnostics(
        &self,
        req: HostLspListDiagnosticsRequest,
    ) -> crate::sdk::Result<HostLspListDiagnosticsResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.lsp_list_diagnostics(req))
                .await
        })
        .await
    }

    async fn snapshot_list(&self) -> crate::sdk::Result<HostSnapshotListResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.snapshot_list()).await
        })
        .await
    }

    async fn scheduler_list(&self) -> crate::sdk::Result<HostSchedulerListResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.scheduler_list()).await
        })
        .await
    }

    async fn scheduler_create(
        &self,
        req: HostSchedulerCreateRequest,
    ) -> crate::sdk::Result<HostSchedulerCreateResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.scheduler_create(req))
                .await
        })
        .await
    }

    async fn scheduler_delete(
        &self,
        req: HostSchedulerDeleteRequest,
    ) -> crate::sdk::Result<HostSchedulerDeleteResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.scheduler_delete(req))
                .await
        })
        .await
    }

    async fn hook_list(&self) -> crate::sdk::Result<HostHookListResponse> {
        self.run_callback(async { Ok(self.handle.hook_list_response().await) })
            .await
    }

    async fn mcp_list_servers(&self) -> crate::sdk::Result<HostMcpListServersResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.mcp_list_servers()).await
        })
        .await
    }

    async fn mcp_add_server(&self, req: HostMcpAddServerRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.mcp_add_server(req)).await
        })
        .await
    }

    async fn mcp_remove_server(
        &self,
        req: HostMcpRemoveServerRequest,
    ) -> crate::sdk::Result<HostMcpRemoveServerResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.mcp_remove_server(req))
                .await
        })
        .await
    }

    async fn display_contribute(
        &self,
        req: HostDisplayContributeRequest,
    ) -> crate::sdk::Result<()> {
        self.run_callback(async { self.handle.display_contribute(&self.plugin_id, req) })
            .await
    }

    async fn display_remove(
        &self,
        req: HostDisplayRemoveRequest,
    ) -> crate::sdk::Result<HostDisplayRemoveResponse> {
        self.run_callback(async {
            let removed = self
                .handle
                .display_remove(&self.plugin_id, &req.contribution_id);
            Ok(HostDisplayRemoveResponse { removed })
        })
        .await
    }

    async fn notify(&self, req: PluginNotifyRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            self.handle.push_host_notification(&self.plugin_id, req);
            Ok(())
        })
        .await
    }

    async fn ui_theme_register(&self, req: HostThemeRegisterRequest) -> crate::sdk::Result<()> {
        self.run_callback(async { self.handle.theme_register(&self.plugin_id, req) })
            .await
    }

    async fn ui_theme_list(&self) -> crate::sdk::Result<HostThemeListResponse> {
        self.run_callback(async { Ok(self.handle.theme_list_response()) })
            .await
    }

    async fn ui_theme_remove(
        &self,
        req: HostThemeRemoveRequest,
    ) -> crate::sdk::Result<HostThemeRemoveResponse> {
        self.run_callback(async {
            let removed = self.handle.theme_remove(&self.plugin_id, &req.id);
            Ok(HostThemeRemoveResponse { removed })
        })
        .await
    }
}
use super::{
    AskUserRequest, AskUserResponse, CancelSubtaskRequest, EventEnvelope, EventFilter,
    EventSubscription, HostCallbackContext, HostClient, HostConfigReloadRequestResponse,
    HostConfigReloadResponse, HostConfigReloadStatusRequest, HostConfigReloadStatusResponse,
    HostContextStatusRequest, HostContextStatusResponse, HostDisplayContributeRequest,
    HostDisplayRemoveRequest, HostDisplayRemoveResponse, HostEnterSnapshotRequest,
    HostExitSnapshotRequest, HostHookListResponse, HostImageExecuteRequest,
    HostImageExecuteResponse, HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse,
    HostLspListServersResponse, HostMcpAddServerRequest, HostMcpListServersResponse,
    HostMcpRemoveServerRequest, HostMcpRemoveServerResponse, HostPluginStatusGetRequest,
    HostPluginStatusGetResponse, HostPluginStatusListResponse, HostRegisteredToolListResponse,
    HostSchedulerCreateRequest, HostSchedulerCreateResponse, HostSchedulerDeleteRequest,
    HostSchedulerDeleteResponse, HostSchedulerListResponse, HostSecretDeleteRequest,
    HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest,
    HostSnapshotListResponse, HostStorageDeleteRequest, HostStorageGetRequest,
    HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse, HostStorageSetRequest,
    HostThemeListResponse, HostThemeRegisterRequest, HostThemeRemoveRequest,
    HostThemeRemoveResponse, HostToolMutationResponse, HostToolRegisterRequest,
    HostToolRemoveRequest, HostToolUpdateRequest, LogLevel, MessageSubtaskRequest, MonitorHandle,
    MonitorReadRequest, MonitorReadResponse, MonitorStartRequest, MonitorStopRequest, PluginError,
    PluginNotifyRequest, ReadSubtaskOutputRequest, ReadSubtaskOutputResponse, RunSubtaskRequest,
    RunSubtaskResponse, ScopedHostClient, SubtaskControlResponse, ToolDescriptor, ToolInvokeOutput,
    host_api::{self, BackgroundActivity, BackgroundActivityKind},
};
