use super::*;
use agena_mcp_client::{McpSandboxContext, ServerSpec};
use agena_plugin_host::sdk::host_api::{HostPreparedProcessSandbox, HostProcessResources};

pub async fn prepare_stdio_sandbox(
    host: &dyn agena_plugin_host::sdk::HostClient,
    spec: &ServerSpec,
    require_sandbox: bool,
) -> SdkResult<Option<HostPreparedProcessSandbox>> {
    let ServerSpec::Stdio { cwd, resources, .. } = spec else {
        return Ok(None);
    };
    host.prepare_process_sandbox(HostProcessResources {
        cwd: cwd.as_ref().map(|path| path.to_string_lossy().into_owned()),
        reads: resources.reads.clone(),
        writes: resources.writes.clone(),
        network: resources.network.clone(),
        require_sandbox,
        ..Default::default()
    })
    .await
    .map(Some)
}

impl McpPlugin {
    pub(super) async fn prepare_stdio_sandbox(
        &self,
        spec: &ServerSpec,
    ) -> SdkResult<Option<HostPreparedProcessSandbox>> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("MCP permission host is not initialized"))?;
        prepare_stdio_sandbox(host.as_ref(), spec, false).await
    }

    pub(super) async fn authorize_server(
        &self,
        server: &str,
    ) -> SdkResult<Arc<McpConnectionManager>> {
        let callback = agena_plugin_host::sdk::host_api::current_host_callback_context();
        let workspace = callback
            .as_ref()
            .and_then(|context| context.workspace_root.as_deref())
            .map(std::path::Path::new);
        let spec = match workspace {
            Some(workspace) => {
                self.manager
                    .server_spec_in_workspace(server, workspace)
                    .await
            }
            None => self.manager.server_spec(server).await,
        }
        .map_err(|error| PluginError::internal_error(&error))?;
        if let ServerSpec::Http { url, .. } = &spec {
            let host = self
                .host
                .get()
                .ok_or_else(|| PluginError::internal("MCP permission host is not initialized"))?;
            host.require_network_permission(network_permission_target(url.as_str())?)
                .await?;
            let completion =
                agena_plugin_host::current_plugin_call_completion().ok_or_else(|| {
                    PluginError::internal("MCP HTTP requires an active host call lifetime")
                })?;
            let authority = self.connection_network_authority()?;
            if let (Some(workspace), Some(session)) = (
                workspace,
                callback.as_ref().and_then(|context| context.session_id),
            ) && self.manager.is_plugin_server(server).await
                && host.prepare_managed_mcp_http(server.to_owned()).await?
            {
                let manager = authority.finish(
                    self.manager
                        .borrow_managed_http(session, server, workspace, authority.clone())
                        .await,
                )?;
                self.refresh_http_roots(&manager, &authority).await?;
                return Ok(manager);
            }
            let manager = self
                .manager
                .for_http_scope_in_workspace(server, authority.clone(), workspace)
                .await
                .map_err(|error| PluginError::internal_error(&error))?;
            let cleanup = Arc::clone(&manager);
            tokio::spawn(async move {
                completion.wait().await;
                cleanup.shutdown_all().await;
            });
            self.refresh_http_roots(&manager, &authority).await?;
            authority.finish(manager.reconnect(server).await)?;
            return Ok(manager);
        }
        let Some(prepared) = self.prepare_stdio_sandbox(&spec).await? else {
            return Err(PluginError::internal(
                "MCP server has no authorized transport",
            ));
        };
        let completion = if prepared.call_bound {
            Some(
                agena_plugin_host::current_plugin_call_completion().ok_or_else(|| {
                    PluginError::internal(
                        "MCP one-call sandbox requires an active host call lifetime",
                    )
                })?,
            )
        } else {
            None
        };
        let scope = prepared.scope.clone();
        let context = stdio_sandbox_context(prepared)?;
        let manager = self
            .manager
            .for_sandbox_scope(server, context)
            .await
            .map_err(|error| {
                PluginError::internal(format!("MCP sandbox for `{server}` failed: {error}"))
            })?;
        if let Some(completion) = completion {
            let owner = Arc::downgrade(&self.manager);
            tokio::spawn(async move {
                completion.wait().await;
                if let Some(owner) = owner.upgrade() {
                    owner.release_sandbox_scope(&scope).await;
                }
            });
        }
        Ok(manager)
    }

    pub(super) async fn refresh_http_roots(
        &self,
        manager: &McpConnectionManager,
        authority: &ConnectionNetworkAuthority,
    ) -> SdkResult<()> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("MCP permission host is not initialized"))?;
        if let Some(roots) = host.mcp_session_roots().await? {
            authority.finish(manager.replace_roots(roots).await)?;
        }
        Ok(())
    }

    /// Connection probes are separate processes with the current call's exact
    /// policy. They never install an authorized spec into the shared catalog.
    pub(super) async fn prepare_probe_spec(&self, mut spec: ServerSpec) -> SdkResult<ServerSpec> {
        if let Some(prepared) = self.prepare_stdio_sandbox(&spec).await?
            && let ServerSpec::Stdio { cwd, sandbox, .. } = &mut spec
        {
            *cwd = Some(prepared.cwd.into());
            *sandbox = Some(serde_json::from_value(prepared.policy).map_err(|error| {
                PluginError::internal(format!("invalid host sandbox policy: {error}"))
            })?);
        }
        Ok(spec)
    }

    pub(super) async fn scoped_tools(
        &self,
        channel: Option<&agena_domain::ChannelAuthority>,
    ) -> SdkResult<Vec<(String, agena_mcp_client::protocol::ToolDescriptor)>> {
        let mut tools = Vec::new();
        let scope = super::tool_loading::current_on_demand_scope();
        let on_demand = self.manager.on_demand_servers(scope.as_ref()).await;
        for name in self.manager.server_names().await {
            if channel.is_some_and(|authority| {
                !authority.active_at(chrono::Utc::now().timestamp_millis())
                    || (!authority.source_admin && !authority.grant.mcp_tools.contains_key(&name))
            }) {
                continue;
            }
            if let Some(server) = on_demand.iter().find(|server| server.id == name) {
                if let Some(scope) = &scope {
                    tools.extend(
                        self.visible_on_demand_tools(server, scope, channel)
                            .await
                            .into_iter()
                            .map(|tool| (name.clone(), tool)),
                    );
                }
                continue;
            }
            let manager = match self.authorize_server(&name).await {
                Ok(manager) => manager,
                Err(error) if self.manager.is_plugin_server(&name).await => {
                    tracing::warn!(
                        server = %name, kind = ?error.kind, reason = %error.failure.user.fallback,
                        "could not list tools from plugin MCP server"
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            tools.extend(
                manager
                    .all_tools()
                    .await
                    .into_iter()
                    .filter(|(server, _)| server == &name),
            );
        }
        Ok(tools)
    }

    pub(super) async fn scoped_statuses(
        &self,
    ) -> SdkResult<Vec<agena_mcp_client::McpServerStatus>> {
        let mut statuses = Vec::new();
        for name in self.manager.server_names().await {
            let manager = match self.authorize_server(&name).await {
                Ok(manager) => manager,
                Err(_) if self.manager.is_plugin_server(&name).await => Arc::clone(&self.manager),
                Err(error) => return Err(error),
            };
            statuses.extend(
                manager
                    .statuses()
                    .await
                    .into_iter()
                    .filter(|status| status.name == name),
            );
        }
        Ok(statuses)
    }
}

pub(super) fn stdio_sandbox_context(
    prepared: HostPreparedProcessSandbox,
) -> SdkResult<McpSandboxContext> {
    Ok(McpSandboxContext {
        scope: prepared.scope,
        workspace_root: prepared.workspace_root.into(),
        cwd: prepared.cwd.into(),
        additional_working_directories: prepared.additional_working_directories,
        policy: serde_json::from_value(prepared.policy).map_err(|error| {
            PluginError::internal(format!("invalid host sandbox policy: {error}"))
        })?,
    })
}
