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
        let spec = self
            .manager
            .server_spec(server)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let Some(prepared) = self.prepare_stdio_sandbox(&spec).await? else {
            if let ServerSpec::Http { url, .. } = spec {
                self.host
                    .get()
                    .ok_or_else(|| PluginError::internal("MCP permission host is not initialized"))?
                    .require_network_permission(url.to_string())
                    .await?;
            }
            return Ok(Arc::clone(&self.manager));
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
        let manager = self
            .manager
            .for_sandbox_scope(server, context)
            .await
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

    pub(super) async fn scoped_statuses(&self) -> SdkResult<Vec<agena_mcp_client::McpServerStatus>> {
        let mut statuses = Vec::new();
        for name in self.manager.server_names().await {
            let manager = self.authorize_server(&name).await?;
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
