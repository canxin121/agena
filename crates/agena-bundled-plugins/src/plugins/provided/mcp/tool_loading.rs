use agena_domain::ChannelAuthority;
use agena_mcp_client::{
    MCP_TOOL_LOADER, McpError, McpOnDemandScope, McpOnDemandSelection, McpOnDemandServer,
    ToolDescriptor,
};
use futures_util::{StreamExt, stream};
use serde_json::json;

use super::*;

static VALIDATE_INPUT: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

pub(super) async fn validate_tool_arguments(
    tool: &ToolDescriptor,
    arguments: Option<&Value>,
) -> SdkResult<()> {
    if let Some(schema) = tool.input_schema.clone() {
        let args = arguments
            .filter(|args| !args.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let valid = VALIDATE_INPUT
            .run(move || agena_plugin_host::schema::validate_json_schema_value(&schema, &args).is_ok())
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        if !valid {
            return Err(PluginError::invalid_params("Arguments do not match the current MCP tool schema."));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoadToolsInput {
    tools: Vec<String>,
}

pub(super) fn current_on_demand_scope() -> Option<McpOnDemandScope> {
    let context = agena_plugin_host::sdk::host_api::current_host_callback_context()?;
    let session = context.session_id.filter(|id| *id > 0)?;
    Some(McpOnDemandScope {
        session,
        workspace: context.workspace_root.unwrap_or_default().into(),
    })
}

fn allows(channel: Option<&ChannelAuthority>, server: &str, name: &str) -> bool {
    channel.is_none_or(|authority| {
        authority.allows_mcp_tool(server, name, chrono::Utc::now().timestamp_millis())
    })
}

impl McpPlugin {
    pub(super) async fn discovery_channel(&self) -> SdkResult<Option<ChannelAuthority>> {
        let context = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("MCP host unavailable"))?
            .companion_context()
            .await?;
        context
            .get("channel_authority")
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .map_err(|_| PluginError::internal("invalid channel tool discovery scope"))
    }

    async fn refresh_on_demand(
        &self,
        server: &McpOnDemandServer,
        scope: &McpOnDemandScope,
        requested: &[String],
    ) -> SdkResult<(Arc<McpConnectionManager>, McpOnDemandSelection)> {
        let observation = self
            .manager
            .begin_on_demand_observation(&server.id, scope)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let manager = self.authorize_server(&server.id).await?;
        let tools = manager
            .connected_tools(&server.id)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let selection = self
            .manager
            .update_on_demand_tools(&server.id, scope, observation, tools, requested)
            .await
            .map_err(|error| match error {
                McpError::Malformed(message) => PluginError::invalid_params(message),
                _ => PluginError::internal_error(&error),
            })?;
        Ok((manager, selection))
    }

    pub(super) async fn invoke_on_demand_tool(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &CallToolInput,
    ) -> SdkResult<Option<ToolInvokeOutput>> {
        let scope = current_on_demand_scope();
        let Some(server) = self
            .manager
            .on_demand_servers(scope.as_ref())
            .await
            .into_iter()
            .find(|server| server.id == input.server)
        else {
            return Ok(None);
        };
        let scope = scope.ok_or_else(|| {
            PluginError::invalid_params("On-demand MCP tools require an owning Agena session")
        })?;
        let channel = self.discovery_channel().await?;
        if !allows(channel.as_ref(), &server.id, &input.name) {
            return Err(PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                "MCP server/tool pair is outside this channel member's explicit allowlist",
            ));
        }
        if input.name == MCP_TOOL_LOADER {
            let request: LoadToolsInput = serde_json::from_value(
                input.arguments.clone().unwrap_or(Value::Null),
            )
            .map_err(|_| {
                PluginError::invalid_params("Pass an object with a tools array of exact tool names")
            })?;
            if request.tools.is_empty() {
                return Err(PluginError::invalid_params("Pass at least one tool name"));
            }
            if request
                .tools
                .iter()
                .any(|name| !allows(channel.as_ref(), &server.id, name))
            {
                return Err(PluginError::from_kind(
                    agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                    "A requested MCP tool is outside this channel member's explicit allowlist",
                ));
            }
            let (_, selection) = self
                .refresh_on_demand(&server, &scope, &request.tools)
                .await?;
            return Ok(Some(load_output(&server, selection)));
        }

        let unavailable = || {
            PluginError::invalid_params(format!(
                "Tool {:?} is unavailable in this session. Load it with {} first.",
                input.name,
                server.alias(MCP_TOOL_LOADER)
            ))
        };
        if !server
            .catalog
            .as_ref()
            .is_some_and(|catalog| catalog.loaded.contains(&input.name))
        {
            return Err(unavailable());
        }
        let (manager, selection) = self.refresh_on_demand(&server, &scope, &[]).await?;
        let connected = manager
            .connected_tools(&server.id)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        let tool = connected
            .iter()
            .find(|tool| tool.name == input.name && selection.catalog.loaded.contains(&tool.name))
            .ok_or_else(unavailable)?;
        validate_tool_arguments(tool, input.arguments.as_ref()).await.map_err(|error| {
            if error.kind == agena_plugin_host::sdk::PluginErrorKind::InvalidParams {
                PluginError::invalid_params(format!(
                    "Arguments do not match the current schema for {}. Read its tools_help again.",
                    server.alias(&input.name)
                ))
            } else {
                error
            }
        })?;
        self.call_remote_tool(context, &manager, input, &input.name)
            .await
            .map(Some)
    }

    pub(super) async fn visible_on_demand_tools(
        &self,
        server: &McpOnDemandServer,
        scope: &McpOnDemandScope,
        channel: Option<&ChannelAuthority>,
    ) -> Vec<ToolDescriptor> {
        let mut tools = Vec::new();
        if allows(channel, &server.id, MCP_TOOL_LOADER) {
            tools.push(server.loader());
        }
        if !server.catalog.as_ref().is_some_and(|catalog| {
            catalog
                .loaded
                .iter()
                .any(|name| allows(channel, &server.id, name))
        }) {
            return tools;
        }
        match self.refresh_on_demand(server, scope, &[]).await {
            Ok((_, selection)) => {
                let catalog = selection.catalog;
                tools.extend(catalog.loaded.iter().filter_map(|name| {
                    let mut tool = catalog.offered.get(name)?.clone();
                    tool.aliases.push(server.alias(name));
                    Some(tool)
                }));
            }
            Err(error) => {
                tracing::warn!(server = %server.id, kind = ?error.kind, "could not list loaded MCP tools");
            }
        }
        tools
    }

    pub(super) async fn on_demand_catalog(
        &self,
        input: &McpToolSearchInput,
        channel: Option<&ChannelAuthority>,
    ) -> SdkResult<ToolInvokeOutput> {
        let scope = current_on_demand_scope().ok_or_else(|| {
            PluginError::invalid_params(
                "The on-demand MCP catalog requires an owning Agena session",
            )
        })?;
        let servers = self
            .manager
            .on_demand_servers(None)
            .await
            .into_iter()
            .filter(|server| allows(channel, &server.id, MCP_TOOL_LOADER))
            .collect::<Vec<_>>();
        let query = input.query.to_ascii_lowercase();
        let updates = stream::iter(servers)
            .map(|server| {
                let scope = &scope;
                async move {
                    let result = self
                        .refresh_on_demand(&server, scope, &[])
                        .await
                        .map(|(_, selection)| selection);
                    (server, result)
                }
            })
            .buffered(8);
        tokio::pin!(updates);
        let mut results = Vec::new();
        let mut lines = Vec::new();
        let limit = input.limit as usize;
        let offset = input.catalog_offset as usize;
        let mut matched = 0;
        while let Some((server, result)) = updates.next().await {
            let alias = server.alias(MCP_TOOL_LOADER);
            let Ok(selection) = result else {
                lines.push(format!(
                    "MCP server {:?} ({alias}) catalog unavailable.",
                    server.source_name
                ));
                continue;
            };
            for (name, tool) in selection.catalog.offered.iter() {
                if !allows(channel, &server.id, name) {
                    continue;
                }
                let summary = server.summary(tool.description.as_deref().unwrap_or_default());
                let searchable = format!("{} {} {name} {summary}", server.id, server.source_name);
                if !query.is_empty() && !searchable.to_ascii_lowercase().contains(&query) {
                    continue;
                }
                matched += 1;
                if matched <= offset || results.len() == limit {
                    continue;
                }
                lines.push(format!("- {name:?} ({alias}): {summary}"));
                results.push(json!({
                    "server": server.id,
                    "name": name,
                    "description": summary,
                    "loader": alias,
                    "loaded": selection.catalog.loaded.contains(name),
                }));
            }
        }
        if lines.is_empty() {
            lines.push("No on-demand MCP tool names matched.".into());
        }
        let end = offset.saturating_add(results.len());
        let next_offset = (end < matched).then_some(end);
        if let Some(next) = next_offset {
            lines.push(format!(
                "Continue with catalog_offset={next} to see more tool names."
            ));
        }
        Ok(ToolInvokeOutput::from_parts(
            "On-demand MCP tool catalog",
            format!("{} of {matched} matching tool names", results.len()),
            lines.join("\n"),
            Some(json!({"results": results, "next_offset": next_offset})),
            Default::default(),
            Vec::new(),
        ))
    }
}

fn load_output(server: &McpOnDemandServer, selection: McpOnDemandSelection) -> ToolInvokeOutput {
    let aliases = |names: &[String]| {
        names
            .iter()
            .map(|name| server.alias(name))
            .collect::<Vec<_>>()
    };
    let loaded = aliases(&selection.loaded);
    let already_loaded = aliases(&selection.already_loaded);
    let mut lines = Vec::new();
    if !loaded.is_empty() {
        lines.push(format!("Loaded: {}.", loaded.join(", ")));
    }
    if !already_loaded.is_empty() {
        lines.push(format!("Already loaded: {}.", already_loaded.join(", ")));
    }
    ToolInvokeOutput::from_parts(
        format!("Load MCP tools from {}", server.source_name),
        format!("{} newly loaded", loaded.len()),
        lines.join("\n"),
        Some(json!({"loaded": loaded, "already_loaded": already_loaded})),
        Default::default(),
        Vec::new(),
    )
}
