impl ScopedHostClient {
    fn ensure_current_generation(&self) -> crate::sdk::Result<()> {
        if self.effect_scope.state() != crate::effect_scope::PluginEffectScopeState::Active
            || !self
                .handle
                .is_current_effect_scope(&self.plugin_key, &self.effect_scope)
        {
            return Err(PluginError::internal(format!(
                "stale plugin generation {} for `{}`",
                self.effect_scope.generation(),
                self.plugin_id
            ))
            .with_plugin(self.plugin_id.clone()));
        }
        Ok(())
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
        if self.ensure_current_generation().is_err() {
            return;
        }
        let Ok(context) = self.context() else {
            return;
        };
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(context, inner.log(level, message, fields)).await;
    }

    async fn publish_event(&self, env: EventEnvelope) -> crate::sdk::Result<()> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.publish_event(env)).await
    }

    async fn subscribe_events(&self, filter: EventFilter) -> crate::sdk::Result<EventSubscription> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.subscribe_events(filter))
            .await
    }

    async fn unsubscribe_events(&self, subscription_id: String) -> crate::sdk::Result<()> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(
            self.context()?,
            inner.unsubscribe_events(subscription_id),
        )
        .await
    }

    async fn read_config(&self, path: Option<String>) -> crate::sdk::Result<serde_json::Value> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.read_config(path)).await
    }

    async fn reload_config(&self) -> crate::sdk::Result<HostConfigReloadResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.reload_config()).await
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.invoke_tool(tool, input))
            .await
    }

    async fn invoke_service(
        &self,
        req: crate::sdk::PluginServiceInvokeInput,
    ) -> crate::sdk::Result<crate::sdk::PluginServiceInvokeOutput> {
        self.ensure_current_generation()?;
        self.handle
            .invoke_service_for_plugin(&self.plugin_id, req, Some(self.context()?))
            .await
    }

    async fn ask_user(&self, req: AskUserRequest) -> crate::sdk::Result<AskUserResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.ask_user(req)).await
    }

    async fn run_subtask(&self, req: RunSubtaskRequest) -> crate::sdk::Result<RunSubtaskResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.run_subtask(req)).await
    }

    async fn cancel_subtask(
        &self,
        req: CancelSubtaskRequest,
    ) -> crate::sdk::Result<SubtaskControlResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.cancel_subtask(req)).await
    }

    async fn message_subtask(
        &self,
        req: MessageSubtaskRequest,
    ) -> crate::sdk::Result<SubtaskControlResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.message_subtask(req)).await
    }

    async fn read_subtask_output(
        &self,
        req: ReadSubtaskOutputRequest,
    ) -> crate::sdk::Result<ReadSubtaskOutputResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.read_subtask_output(req))
            .await
    }

    async fn list_tools(&self) -> crate::sdk::Result<Vec<ToolDescriptor>> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.list_tools()).await
    }

    async fn get_context_status(
        &self,
        req: HostContextStatusRequest,
    ) -> crate::sdk::Result<HostContextStatusResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.get_context_status(req)).await
    }

    async fn get_session(
        &self,
        req: crate::sdk::host_api::HostGetSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostGetSessionResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.get_session(req)).await
    }

    async fn rename_session(
        &self,
        req: crate::sdk::host_api::HostRenameSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostRenameSessionResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.rename_session(req)).await
    }

    async fn set_session_model(
        &self,
        req: crate::sdk::host_api::HostSetSessionModelRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostSetSessionModelResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.set_session_model(req)).await
    }

    async fn image_execute(
        &self,
        req: HostImageExecuteRequest,
    ) -> crate::sdk::Result<HostImageExecuteResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.image_execute(req)).await
    }

    async fn enter_snapshot(
        &self,
        req: HostEnterSnapshotRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.enter_snapshot(req)).await
    }

    async fn exit_snapshot(
        &self,
        req: HostExitSnapshotRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.exit_snapshot(req)).await
    }

    async fn monitor_start(&self, req: MonitorStartRequest) -> crate::sdk::Result<MonitorHandle> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.monitor_start(req)).await
    }

    async fn monitor_list(&self) -> crate::sdk::Result<Vec<MonitorHandle>> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.monitor_list()).await
    }

    async fn monitor_read(
        &self,
        req: MonitorReadRequest,
    ) -> crate::sdk::Result<MonitorReadResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.monitor_read(req)).await
    }

    async fn monitor_stop(&self, req: MonitorStopRequest) -> crate::sdk::Result<MonitorHandle> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.monitor_stop(req)).await
    }

    async fn register_tool(
        &self,
        req: HostToolRegisterRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.ensure_current_generation()?;
        self.handle
            .tool_upsert_for_plugin(&self.plugin_id, req.tool)
    }

    async fn update_tool(
        &self,
        req: HostToolUpdateRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.ensure_current_generation()?;
        self.handle
            .tool_upsert_for_plugin(&self.plugin_id, req.tool)
    }

    async fn remove_tool(
        &self,
        req: HostToolRemoveRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.ensure_current_generation()?;
        self.handle
            .tool_remove_for_plugin(&self.plugin_id, &req.name, req.by_model_name)
    }

    async fn list_registered_tools(&self) -> crate::sdk::Result<HostRegisteredToolListResponse> {
        self.ensure_current_generation()?;
        self.handle.registered_tool_list_response()
    }

    async fn list_plugins(&self) -> crate::sdk::Result<crate::sdk::HostPluginListResponse> {
        self.ensure_current_generation()?;
        Ok(self.handle.plugin_list_response())
    }

    async fn storage_get(
        &self,
        req: HostStorageGetRequest,
    ) -> crate::sdk::Result<HostStorageGetResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.storage_get(req)).await
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> crate::sdk::Result<()> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.storage_set(req)).await
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> crate::sdk::Result<()> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.storage_delete(req)).await
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> crate::sdk::Result<HostStorageListResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.storage_list(req)).await
    }

    async fn secret_get(
        &self,
        req: HostSecretGetRequest,
    ) -> crate::sdk::Result<HostSecretGetResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.secret_get(req)).await
    }

    async fn secret_set(&self, req: HostSecretSetRequest) -> crate::sdk::Result<()> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.secret_set(req)).await
    }

    async fn secret_delete(&self, req: HostSecretDeleteRequest) -> crate::sdk::Result<()> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.secret_delete(req)).await
    }

    async fn secret_list(&self) -> crate::sdk::Result<HostSecretListResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.secret_list()).await
    }

    async fn plugin_status_list(&self) -> crate::sdk::Result<HostPluginStatusListResponse> {
        self.ensure_current_generation()?;
        Ok(self.handle.plugin_status_list_response())
    }

    async fn plugin_status_get(
        &self,
        req: HostPluginStatusGetRequest,
    ) -> crate::sdk::Result<HostPluginStatusGetResponse> {
        self.ensure_current_generation()?;
        Ok(self.handle.plugin_status_get_response(&req.plugin_id))
    }

    async fn lsp_list_servers(&self) -> crate::sdk::Result<HostLspListServersResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.lsp_list_servers()).await
    }

    async fn lsp_list_diagnostics(
        &self,
        req: HostLspListDiagnosticsRequest,
    ) -> crate::sdk::Result<HostLspListDiagnosticsResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.lsp_list_diagnostics(req))
            .await
    }

    async fn snapshot_list(&self) -> crate::sdk::Result<HostSnapshotListResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.snapshot_list()).await
    }

    async fn scheduler_list(&self) -> crate::sdk::Result<HostSchedulerListResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.scheduler_list()).await
    }

    async fn scheduler_create(
        &self,
        req: HostSchedulerCreateRequest,
    ) -> crate::sdk::Result<HostSchedulerCreateResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.scheduler_create(req)).await
    }

    async fn scheduler_delete(
        &self,
        req: HostSchedulerDeleteRequest,
    ) -> crate::sdk::Result<HostSchedulerDeleteResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.scheduler_delete(req)).await
    }

    async fn hook_list(&self) -> crate::sdk::Result<HostHookListResponse> {
        self.ensure_current_generation()?;
        Ok(self.handle.hook_list_response().await)
    }

    async fn mcp_list_servers(&self) -> crate::sdk::Result<HostMcpListServersResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.mcp_list_servers()).await
    }

    async fn mcp_add_server(&self, req: HostMcpAddServerRequest) -> crate::sdk::Result<()> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.mcp_add_server(req)).await
    }

    async fn mcp_remove_server(
        &self,
        req: HostMcpRemoveServerRequest,
    ) -> crate::sdk::Result<HostMcpRemoveServerResponse> {
        self.ensure_current_generation()?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context()?, inner.mcp_remove_server(req)).await
    }

    async fn display_contribute(
        &self,
        req: HostDisplayContributeRequest,
    ) -> crate::sdk::Result<()> {
        self.ensure_current_generation()?;
        self.handle.display_contribute(&self.plugin_id, req);
        Ok(())
    }

    async fn display_remove(
        &self,
        req: HostDisplayRemoveRequest,
    ) -> crate::sdk::Result<HostDisplayRemoveResponse> {
        self.ensure_current_generation()?;
        let removed = self
            .handle
            .display_remove(&self.plugin_id, &req.contribution_id);
        Ok(HostDisplayRemoveResponse { removed })
    }

    async fn ui_theme_register(&self, req: HostThemeRegisterRequest) -> crate::sdk::Result<()> {
        self.ensure_current_generation()?;
        self.handle.theme_register(&self.plugin_id, req)
    }

    async fn ui_theme_list(&self) -> crate::sdk::Result<HostThemeListResponse> {
        self.ensure_current_generation()?;
        Ok(self.handle.theme_list_response())
    }

    async fn ui_theme_remove(
        &self,
        req: HostThemeRemoveRequest,
    ) -> crate::sdk::Result<HostThemeRemoveResponse> {
        self.ensure_current_generation()?;
        let removed = self.handle.theme_remove(&self.plugin_id, &req.id);
        Ok(HostThemeRemoveResponse { removed })
    }
}
use super::{
    AskUserRequest, AskUserResponse, CancelSubtaskRequest, EventEnvelope, EventFilter,
    EventSubscription, HostCallbackContext, HostClient, HostConfigReloadResponse,
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
    ReadSubtaskOutputRequest, ReadSubtaskOutputResponse, RunSubtaskRequest, RunSubtaskResponse,
    ScopedHostClient, SubtaskControlResponse, ToolDescriptor, ToolInvokeOutput, host_api,
};
