use agena_mcp_client::{McpHttpNetworkAuthorizer, McpHttpTransport};

use super::*;

impl RuntimeHostClient {
    pub(in crate::runtime::host_client) async fn admit_managed_mcp_http(
        &self,
        id: &str,
    ) -> Result<bool> {
        let context = self.callback_context()?;
        if context.plugin_id.as_deref() != Some("agena.mcp") {
            return Ok(false);
        }
        let (Some(session), Some(raw_tool)) = (context.session_id, context.tool_name) else {
            return Ok(false);
        };
        if session <= 0 {
            return Ok(false);
        }
        let runtime = self.runtime()?;
        let snapshot = runtime.current_snapshot();
        let generation = snapshot.generation();
        let cleanup_ledger = &runtime.deepseek_mcp_cleanup;
        let plugins = snapshot.plugin_manager();
        let declarations = managed_servers(plugins.plugins());
        let Some(target) = declarations.iter().find(|target| target.id == id) else {
            // A call from the retired host can still see a removed server.
            if generation != self.generation {
                let _operation = cleanup_ledger.lock_server(session, id).await;
                cleanup_ledger.check_replacement(session, id, generation)?;
            }
            return Ok(false);
        };
        let _operation = cleanup_ledger.lock_server(session, id).await;
        cleanup_ledger.check_replacement(session, id, generation)?;
        if !target.definition.enabled
            || context.mcp_app.is_some()
            || !matches!(
                raw_tool.as_str(),
                "tools.call"
                    | "tools.search"
                    | "resources.list"
                    | "resources.templates.list"
                    | "resources.read"
                    | "resources.directory.list"
                    | "skills.list"
                    | "skills.read"
                    | "prompts.get"
                    | "prompts.list"
            )
        {
            return Ok(false);
        }
        let Some(mcp) = snapshot.mcp_manager() else {
            return Ok(false);
        };
        let Some(sessions) = snapshot.session_manager() else {
            return Ok(false);
        };
        let Some(policy) = target
            .definition
            .server
            .as_ref()
            .and_then(|entry| entry.settings.deepseek_reconnect)
            .filter(|_| mcp.reconnect_supervisor_running())
        else {
            return Ok(false);
        };
        // Only a borrowed connection needs the live manager to match the
        // bundled tool's generation. A call-owned scope uses its own spec.
        if generation != self.generation || snapshot.replaced.is_cancelled() {
            return Err(host_unavailable(
                "the DeepSeek MCP manager configuration changed",
            ));
        }
        let tool = format!("agena.mcp.{raw_tool}");
        // This check resolves remembered policy without consuming the live
        // tool's grant cache or asking for another approval.
        let authorization = sessions
            .authorize_deepseek_mcp_manager(session, &target.owner_key, &tool)
            .await;
        if snapshot.replaced.is_cancelled() {
            return Err(host_unavailable(
                "the DeepSeek MCP manager configuration changed",
            ));
        }
        let Ok(session_executor) = authorization else {
            return Ok(false);
        };
        if !Arc::ptr_eq(session_executor.plugin_manager(), &plugins) {
            return Err(host_unavailable(
                "the DeepSeek MCP manager configuration changed",
            ));
        }
        if context.workspace_root.as_deref().map(std::path::Path::new)
            != Some(session_executor.workspace_root())
        {
            return Ok(false);
        }
        let workspace = session_executor.workspace_root().to_owned();
        let server = mcp
            .plugin_server_in_workspace(id, &workspace)
            .await
            .map_err(transport_error)?;
        let ServerSpec::Http {
            url,
            transport: McpHttpTransport::Streamable,
            ..
        } = server.spec
        else {
            return Ok(false);
        };
        let lifetime = snapshot.replaced.child_token();
        let authority = Arc::new(reconnect::ManagedAuthority {
            sessions,
            plugins: plugins.clone(),
            caller: target.owner_key.clone(),
            owner: target.owner_key.clone(),
            tool,
            workspace: workspace.clone(),
            session,
            cancellation: lifetime.clone(),
            cleanup: false,
        });
        let authorization = authority.authorize(url.as_str()).await;
        if snapshot.replaced.is_cancelled() {
            return Err(host_unavailable(
                "the DeepSeek MCP manager configuration changed",
            ));
        }
        if authorization.is_err() {
            // The original call can still use its own grant with its own
            // transport. It cannot create a background effect.
            return Ok(false);
        }
        if let Some(scope) = mcp
            .reserve_managed_connection(session, id, &workspace, lifetime)
            .await
            .map_err(transport_error)?
        {
            cleanup_ledger.track(
                target.identity(session, &workspace),
                generation,
                scope.clone(),
            );
            drop(
                reconnect::ManagedRestart {
                    authority,
                    mcp,
                    name: id.to_owned(),
                    scope,
                    previous: None,
                    policy,
                }
                .start(),
            );
        }
        Ok(true)
    }
}
