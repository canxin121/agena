impl ScopedHostClient {
    fn context(&self) -> HostCallbackContext {
        let mut context = host_api::current_host_callback_context().unwrap_or_default();
        context.plugin_id = Some(self.plugin_id.clone());
        context
    }

    async fn require_capability(
        &self,
        method: &str,
        capability: HostCapability,
    ) -> crate::sdk::Result<()> {
        self.handle
            .require_capability(Some(&self.plugin_id), method, capability)
            .await
    }
}

#[async_trait::async_trait]
impl HostClient for ScopedHostClient {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.log(level, message, fields))
            .await;
    }

    async fn publish_event(&self, env: EventEnvelope) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_EVENT_PUBLISH, HostCapability::PublishEvent)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.publish_event(env)).await
    }

    async fn subscribe_events(&self, filter: EventFilter) -> crate::sdk::Result<EventSubscription> {
        self.require_capability(
            method::HOST_EVENT_SUBSCRIBE,
            HostCapability::SubscribeEvents,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.subscribe_events(filter)).await
    }

    async fn unsubscribe_events(&self, subscription_id: String) -> crate::sdk::Result<()> {
        self.require_capability(
            method::HOST_EVENT_UNSUBSCRIBE,
            HostCapability::SubscribeEvents,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(
            self.context(),
            inner.unsubscribe_events(subscription_id),
        )
        .await
    }

    async fn ask_permission(
        &self,
        req: PermissionAskInput,
    ) -> crate::sdk::Result<PermissionDecision> {
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.ask_permission(req)).await
    }

    async fn check_path_permission(
        &self,
        req: HostPathPermissionCheckRequest,
    ) -> crate::sdk::Result<HostPermissionCheckResponse> {
        self.require_capability(
            method::HOST_PERMISSION_CHECK_PATH,
            HostCapability::PermissionCheck,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.check_path_permission(req))
            .await
    }

    async fn check_network_permission(
        &self,
        req: HostNetworkPermissionCheckRequest,
    ) -> crate::sdk::Result<HostPermissionCheckResponse> {
        self.require_capability(
            method::HOST_PERMISSION_CHECK_NETWORK,
            HostCapability::PermissionCheck,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.check_network_permission(req))
            .await
    }

    async fn read_config(&self, path: Option<String>) -> crate::sdk::Result<serde_json::Value> {
        self.require_capability(method::HOST_CONFIG_READ, HostCapability::ReadConfig)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.read_config(path)).await
    }

    async fn reload_config(&self) -> crate::sdk::Result<HostConfigReloadResponse> {
        self.require_capability(method::HOST_CONFIG_RELOAD, HostCapability::ReloadConfig)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.reload_config()).await
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.require_capability(method::HOST_TOOL_INVOKE, HostCapability::InvokeTool)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.invoke_tool(tool, input)).await
    }

    async fn ask_user(&self, req: AskUserRequest) -> crate::sdk::Result<AskUserResponse> {
        self.require_capability(method::HOST_ASK_USER, HostCapability::AskUser)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.ask_user(req)).await
    }

    async fn run_subtask(&self, req: RunSubtaskRequest) -> crate::sdk::Result<RunSubtaskResponse> {
        self.require_capability(method::HOST_SUBTASK_RUN, HostCapability::RunSubtask)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.run_subtask(req)).await
    }

    async fn list_tools(&self) -> crate::sdk::Result<Vec<ToolDescriptor>> {
        self.require_capability(method::HOST_TOOL_LIST, HostCapability::ListTools)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.list_tools()).await
    }

    async fn get_session(
        &self,
        req: crate::sdk::host_api::HostGetSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostGetSessionResponse> {
        self.require_capability("host/session.get", HostCapability::SessionRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.get_session(req)).await
    }

    async fn rename_session(
        &self,
        req: crate::sdk::host_api::HostRenameSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostRenameSessionResponse> {
        self.require_capability("host/session.rename", HostCapability::SessionRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.rename_session(req)).await
    }

    async fn enter_snapshot(
        &self,
        req: HostEnterSnapshotRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.require_capability(
            method::HOST_SNAPSHOT_ENTER,
            HostCapability::SnapshotRegistry,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.enter_snapshot(req)).await
    }

    async fn exit_snapshot(
        &self,
        req: HostExitSnapshotRequest,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.require_capability(method::HOST_SNAPSHOT_EXIT, HostCapability::SnapshotRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.exit_snapshot(req)).await
    }

    async fn monitor_start(&self, req: MonitorStartRequest) -> crate::sdk::Result<MonitorHandle> {
        self.require_capability(method::HOST_MONITOR_START, HostCapability::MonitorRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.monitor_start(req)).await
    }

    async fn monitor_list(&self) -> crate::sdk::Result<Vec<MonitorHandle>> {
        self.require_capability(method::HOST_MONITOR_LIST, HostCapability::MonitorRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.monitor_list()).await
    }

    async fn monitor_read(
        &self,
        req: MonitorReadRequest,
    ) -> crate::sdk::Result<MonitorReadResponse> {
        self.require_capability(method::HOST_MONITOR_READ, HostCapability::MonitorRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.monitor_read(req)).await
    }

    async fn monitor_stop(&self, req: MonitorStopRequest) -> crate::sdk::Result<MonitorHandle> {
        self.require_capability(method::HOST_MONITOR_STOP, HostCapability::MonitorRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.monitor_stop(req)).await
    }

    async fn register_tool(
        &self,
        req: HostToolRegisterRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.require_capability(
            method::HOST_TOOL_REGISTRY_REGISTER,
            HostCapability::ToolRegistry,
        )
        .await?;
        self.handle
            .tool_upsert_for_plugin(&self.plugin_id, req.tool)
    }

    async fn update_tool(
        &self,
        req: HostToolUpdateRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.require_capability(
            method::HOST_TOOL_REGISTRY_UPDATE,
            HostCapability::ToolRegistry,
        )
        .await?;
        self.handle
            .tool_upsert_for_plugin(&self.plugin_id, req.tool)
    }

    async fn remove_tool(
        &self,
        req: HostToolRemoveRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.require_capability(
            method::HOST_TOOL_REGISTRY_REMOVE,
            HostCapability::ToolRegistry,
        )
        .await?;
        self.handle
            .tool_remove_for_plugin(&self.plugin_id, &req.name, req.by_model_name)
    }

    async fn list_registered_tools(&self) -> crate::sdk::Result<HostRegisteredToolListResponse> {
        self.require_capability(
            method::HOST_TOOL_REGISTRY_LIST,
            HostCapability::ToolRegistry,
        )
        .await?;
        self.handle.registered_tool_list_response()
    }

    async fn storage_get(
        &self,
        req: HostStorageGetRequest,
    ) -> crate::sdk::Result<HostStorageGetResponse> {
        self.require_capability(method::HOST_STORAGE_GET, HostCapability::PluginStorage)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.storage_get(req)).await
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_STORAGE_SET, HostCapability::PluginStorage)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.storage_set(req)).await
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_STORAGE_DELETE, HostCapability::PluginStorage)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.storage_delete(req)).await
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> crate::sdk::Result<HostStorageListResponse> {
        self.require_capability(method::HOST_STORAGE_LIST, HostCapability::PluginStorage)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.storage_list(req)).await
    }

    async fn secret_get(
        &self,
        req: HostSecretGetRequest,
    ) -> crate::sdk::Result<HostSecretGetResponse> {
        self.require_capability(method::HOST_SECRET_GET, HostCapability::PluginSecrets)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.secret_get(req)).await
    }

    async fn secret_set(&self, req: HostSecretSetRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_SECRET_SET, HostCapability::PluginSecrets)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.secret_set(req)).await
    }

    async fn secret_delete(&self, req: HostSecretDeleteRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_SECRET_DELETE, HostCapability::PluginSecrets)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.secret_delete(req)).await
    }

    async fn secret_list(&self) -> crate::sdk::Result<HostSecretListResponse> {
        self.require_capability(method::HOST_SECRET_LIST, HostCapability::PluginSecrets)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.secret_list()).await
    }

    async fn plugin_status_list(&self) -> crate::sdk::Result<HostPluginStatusListResponse> {
        self.require_capability(
            method::HOST_PLUGIN_STATUS_LIST,
            HostCapability::PluginStatus,
        )
        .await?;
        Ok(self.handle.plugin_status_list_response())
    }

    async fn plugin_status_get(
        &self,
        req: HostPluginStatusGetRequest,
    ) -> crate::sdk::Result<HostPluginStatusGetResponse> {
        self.require_capability(method::HOST_PLUGIN_STATUS_GET, HostCapability::PluginStatus)
            .await?;
        Ok(self.handle.plugin_status_get_response(&req.plugin_id))
    }

    async fn lsp_list_servers(&self) -> crate::sdk::Result<HostLspListServersResponse> {
        self.require_capability(method::HOST_LSP_LIST_SERVERS, HostCapability::LspRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.lsp_list_servers()).await
    }

    async fn lsp_list_diagnostics(
        &self,
        req: HostLspListDiagnosticsRequest,
    ) -> crate::sdk::Result<HostLspListDiagnosticsResponse> {
        self.require_capability(
            method::HOST_LSP_LIST_DIAGNOSTICS,
            HostCapability::LspRegistry,
        )
        .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.lsp_list_diagnostics(req))
            .await
    }

    async fn snapshot_list(&self) -> crate::sdk::Result<HostSnapshotListResponse> {
        self.require_capability(method::HOST_SNAPSHOT_LIST, HostCapability::SnapshotRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.snapshot_list()).await
    }

    async fn scheduler_list(&self) -> crate::sdk::Result<HostSchedulerListResponse> {
        self.require_capability(method::HOST_SCHEDULER_LIST, HostCapability::Scheduler)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.scheduler_list()).await
    }

    async fn scheduler_create(
        &self,
        req: HostSchedulerCreateRequest,
    ) -> crate::sdk::Result<HostSchedulerCreateResponse> {
        self.require_capability(method::HOST_SCHEDULER_CREATE, HostCapability::Scheduler)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.scheduler_create(req)).await
    }

    async fn scheduler_delete(
        &self,
        req: HostSchedulerDeleteRequest,
    ) -> crate::sdk::Result<HostSchedulerDeleteResponse> {
        self.require_capability(method::HOST_SCHEDULER_DELETE, HostCapability::Scheduler)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.scheduler_delete(req)).await
    }

    async fn agent_register(&self, req: HostAgentRegisterRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_AGENT_REGISTER, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.agent_register(req)).await
    }

    async fn agent_remove(
        &self,
        req: HostAgentRemoveRequest,
    ) -> crate::sdk::Result<HostAgentRemoveResponse> {
        self.require_capability(method::HOST_AGENT_REMOVE, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.agent_remove(req)).await
    }

    async fn agent_list(&self) -> crate::sdk::Result<HostAgentListResponse> {
        self.require_capability(method::HOST_AGENT_LIST, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.agent_list()).await
    }

    async fn agent_get(
        &self,
        req: HostAgentGetRequest,
    ) -> crate::sdk::Result<HostAgentGetResponse> {
        self.require_capability(method::HOST_AGENT_GET, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.agent_get(req)).await
    }

    async fn agent_switch(
        &self,
        req: HostAgentSwitchRequest,
    ) -> crate::sdk::Result<HostAgentSwitchResponse> {
        self.require_capability(method::HOST_AGENT_SWITCH, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.agent_switch(req)).await
    }

    async fn agent_restore(
        &self,
        req: HostAgentRestoreRequest,
    ) -> crate::sdk::Result<HostAgentRestoreResponse> {
        self.require_capability(method::HOST_AGENT_RESTORE, HostCapability::AgentRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.agent_restore(req)).await
    }

    async fn hook_list(&self) -> crate::sdk::Result<HostHookListResponse> {
        self.require_capability(method::HOST_HOOK_LIST, HostCapability::HookRegistry)
            .await?;
        Ok(self.handle.hook_list_response().await)
    }

    async fn mcp_list_servers(&self) -> crate::sdk::Result<HostMcpListServersResponse> {
        self.require_capability(method::HOST_MCP_LIST_SERVERS, HostCapability::McpRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.mcp_list_servers()).await
    }

    async fn mcp_add_server(&self, req: HostMcpAddServerRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_MCP_ADD_SERVER, HostCapability::McpRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.mcp_add_server(req)).await
    }

    async fn mcp_remove_server(
        &self,
        req: HostMcpRemoveServerRequest,
    ) -> crate::sdk::Result<HostMcpRemoveServerResponse> {
        self.require_capability(method::HOST_MCP_REMOVE_SERVER, HostCapability::McpRegistry)
            .await?;
        let inner = self.handle.inner.read().await.clone();
        host_api::run_in_host_callback_context(self.context(), inner.mcp_remove_server(req)).await
    }

    async fn ui_statusline_contribute(
        &self,
        req: HostStatuslineContributeRequest,
    ) -> crate::sdk::Result<()> {
        self.require_capability(
            method::HOST_UI_STATUSLINE_CONTRIBUTE,
            HostCapability::Statusline,
        )
        .await?;
        self.handle.statusline_contribute(&self.plugin_id, req);
        Ok(())
    }

    async fn ui_statusline_list(&self) -> crate::sdk::Result<HostStatuslineListResponse> {
        self.require_capability(method::HOST_UI_STATUSLINE_LIST, HostCapability::Statusline)
            .await?;
        Ok(self.handle.statusline_list_response())
    }

    async fn ui_statusline_remove(
        &self,
        req: HostStatuslineRemoveRequest,
    ) -> crate::sdk::Result<HostStatuslineRemoveResponse> {
        self.require_capability(
            method::HOST_UI_STATUSLINE_REMOVE,
            HostCapability::Statusline,
        )
        .await?;
        let removed = self
            .handle
            .statusline_remove(&self.plugin_id, &req.segment_id);
        Ok(HostStatuslineRemoveResponse { removed })
    }

    async fn ui_theme_register(&self, req: HostThemeRegisterRequest) -> crate::sdk::Result<()> {
        self.require_capability(method::HOST_UI_THEME_REGISTER, HostCapability::Theme)
            .await?;
        self.handle.theme_register(&self.plugin_id, req)
    }

    async fn ui_theme_list(&self) -> crate::sdk::Result<HostThemeListResponse> {
        self.require_capability(method::HOST_UI_THEME_LIST, HostCapability::Theme)
            .await?;
        Ok(self.handle.theme_list_response())
    }

    async fn ui_theme_remove(
        &self,
        req: HostThemeRemoveRequest,
    ) -> crate::sdk::Result<HostThemeRemoveResponse> {
        self.require_capability(method::HOST_UI_THEME_REMOVE, HostCapability::Theme)
            .await?;
        let removed = self.handle.theme_remove(&self.plugin_id, &req.id);
        Ok(HostThemeRemoveResponse { removed })
    }
}
use super::{
    AskUserRequest, AskUserResponse, EventEnvelope, EventFilter, EventSubscription,
    HostAgentGetRequest, HostAgentGetResponse, HostAgentListResponse, HostAgentRegisterRequest,
    HostAgentRemoveRequest, HostAgentRemoveResponse, HostAgentRestoreRequest,
    HostAgentRestoreResponse, HostAgentSwitchRequest, HostAgentSwitchResponse, HostCallbackContext,
    HostCapability, HostClient, HostConfigReloadResponse, HostEnterSnapshotRequest,
    HostExitSnapshotRequest, HostHookListResponse, HostLspListDiagnosticsRequest,
    HostLspListDiagnosticsResponse, HostLspListServersResponse, HostMcpAddServerRequest,
    HostMcpListServersResponse, HostMcpRemoveServerRequest, HostMcpRemoveServerResponse,
    HostNetworkPermissionCheckRequest, HostPathPermissionCheckRequest, HostPermissionCheckResponse,
    HostPluginStatusGetRequest, HostPluginStatusGetResponse, HostPluginStatusListResponse,
    HostRegisteredToolListResponse, HostSchedulerCreateRequest, HostSchedulerCreateResponse,
    HostSchedulerDeleteRequest, HostSchedulerDeleteResponse, HostSchedulerListResponse,
    HostSecretDeleteRequest, HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse,
    HostSecretSetRequest, HostSnapshotListResponse, HostStatuslineContributeRequest,
    HostStatuslineListResponse, HostStatuslineRemoveRequest, HostStatuslineRemoveResponse,
    HostStorageDeleteRequest, HostStorageGetRequest, HostStorageGetResponse,
    HostStorageListRequest, HostStorageListResponse, HostStorageSetRequest, HostThemeListResponse,
    HostThemeRegisterRequest, HostThemeRemoveRequest, HostThemeRemoveResponse,
    HostToolMutationResponse, HostToolRegisterRequest, HostToolRemoveRequest,
    HostToolUpdateRequest, LogLevel, MonitorHandle, MonitorReadRequest, MonitorReadResponse,
    MonitorStartRequest, MonitorStopRequest, PermissionAskInput, PermissionDecision,
    RunSubtaskRequest, RunSubtaskResponse, ScopedHostClient, ToolDescriptor, ToolInvokeOutput,
    host_api, method,
};
