use super::*;
use agena_mcp_client::{McpSandboxContext, ServerSpec};
use agena_plugin_host::sdk::host_api::{HostPreparedProcessSandbox, HostProcessResources};

impl McpPlugin {
    async fn prepare_stdio_sandbox(
        &self,
        spec: &ServerSpec,
    ) -> SdkResult<Option<HostPreparedProcessSandbox>> {
        let ServerSpec::Stdio { cwd, resources, .. } = spec else {
            return Ok(None);
        };
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("MCP permission host is not initialized"))?;
        host.prepare_process_sandbox(HostProcessResources {
            cwd: cwd.as_ref().map(|path| path.to_string_lossy().into_owned()),
            reads: resources.reads.clone(),
            writes: resources.writes.clone(),
            network: resources.network.clone(),
        })
        .await
        .map(Some)
    }

    pub(super) async fn authorize_server(
        &self,
        server: &str,
    ) -> SdkResult<Arc<McpConnectionManager>> {
        self.authorized_server_manager(server, false).await
    }

    pub(super) async fn authorized_server_manager(
        &self,
        server: &str,
        reconnect: bool,
    ) -> SdkResult<Arc<McpConnectionManager>> {
        let spec = self
            .manager
            .server_spec(server)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        if let ServerSpec::Http { url, .. } = &spec {
            self.host
                .get()
                .ok_or_else(|| PluginError::internal("MCP permission host is not initialized"))?
                .require_network_permission(network_permission_target(url.as_str())?)
                .await?;
            let completion =
                agena_plugin_host::current_plugin_call_completion().ok_or_else(|| {
                    PluginError::internal("MCP HTTP requires an active host call lifetime")
                })?;
            let authority = self.connection_network_authority()?;
            let manager = self
                .manager
                .for_http_scope(server, authority.clone())
                .await
                .map_err(|error| PluginError::internal_error(&error))?;
            let cleanup = Arc::clone(&manager);
            tokio::spawn(async move {
                completion.wait().await;
                cleanup.shutdown_all().await;
            });
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
        let context = McpSandboxContext {
            scope: prepared.scope.clone(),
            workspace_root: prepared.workspace_root.into(),
            cwd: prepared.cwd.into(),
            policy: serde_json::from_value(prepared.policy).map_err(|error| {
                PluginError::internal(format!("invalid host sandbox policy: {error}"))
            })?,
        };
        let manager = if reconnect {
            self.manager
                .reconnect_in_sandbox_scope(server, context)
                .await
        } else {
            self.manager.for_sandbox_scope(server, context).await
        }
        .map_err(|error| {
            PluginError::internal(format!("MCP sandbox for `{server}` failed: {error}"))
        })?;
        if let Some(completion) = completion {
            let owner = Arc::downgrade(&self.manager);
            let scope = prepared.scope;
            tokio::spawn(async move {
                completion.wait().await;
                if let Some(owner) = owner.upgrade() {
                    owner.release_sandbox_scope(&scope).await;
                }
            });
        }
        Ok(manager)
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
        for name in self.manager.server_names().await {
            if channel.is_some_and(|authority| {
                !authority.active_at(chrono::Utc::now().timestamp_millis())
                    || (!authority.source_admin && !authority.grant.mcp_tools.contains_key(&name))
            }) {
                continue;
            }
            let manager = self.authorize_server(&name).await?;
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
}
