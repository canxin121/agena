use agena_mcp_client::{McpConnectionManager, McpOnDemandScope, McpOnDemandServer};

use super::{Arc, ToolExecutor};

impl ToolExecutor {
    pub fn with_mcp_manager(mut self, manager: Option<Arc<McpConnectionManager>>) -> Self {
        self.mcp_manager = manager;
        self
    }

    pub fn mcp_manager(&self) -> Option<&Arc<McpConnectionManager>> {
        self.mcp_manager.as_ref()
    }

    pub fn mcp_tool_available(&self, name: &str) -> bool {
        self.available_registered_tools().iter().any(|entry| {
            entry.canonical_name() == name && self.tool_is_discoverable_under_permission(entry)
        })
    }

    pub fn allows_mcp_tool_alias(&self, server: &str, raw_name: &str) -> bool {
        self.channel_authority.as_ref().is_none_or(|authority| {
            authority.allows_mcp_tool(server, raw_name, chrono::Utc::now().timestamp_millis())
        })
    }

    pub async fn on_demand_mcp_servers(&self) -> Vec<McpOnDemandServer> {
        let (Some(manager), Some(session)) = (&self.mcp_manager, self.session_id) else {
            return Vec::new();
        };
        if !manager.has_on_demand_servers().await
            || !self.mcp_tool_available("agena.mcp.tools.call")
        {
            return Vec::new();
        }
        manager
            .on_demand_servers(Some(&McpOnDemandScope {
                session,
                workspace: self.workspace_root.clone(),
            }))
            .await
    }

    pub async fn release_session_mcp(&self, session: i64) {
        if let Some(manager) = &self.mcp_manager {
            manager.release_session(session).await;
        }
    }
}
