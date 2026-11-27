use super::{Deserialize, Serialize};
use agena_api::resource::PluginListItem;
use agena_mcp_server::{StatelessMcpToolMetadata, is_stateless_mcp_tool_exposed};
use agena_runtime_tools::tool::definition::{schema_example_texts, schema_usage_text};
use agena_tool_api::{CatalogPlugin, CatalogTool, ToolApiCatalog};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Public descriptor for a Runtime tool owned by the server.
pub struct OperatorToolResource {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_help: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub input_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    /// Whether this tool is marked as interactive by its declared tags or
    /// discovery metadata. Consumers that cannot service interaction
    /// should hide it.
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub open_world: bool,
    #[serde(default)]
    pub agent: bool,
    /// Stable full plugin id (for example, `agena.fs`).
    pub plugin_id: String,
}

impl OperatorToolResource {
    pub fn canonical_name(&self) -> &str {
        self.canonical_name.as_deref().unwrap_or(&self.name)
    }

    pub fn stateless_mcp_metadata(&self) -> StatelessMcpToolMetadata<'_> {
        StatelessMcpToolMetadata {
            name: &self.name,
            plugin_id: &self.plugin_id,
            interactive: self.interactive,
            agent: self.agent,
        }
    }

    pub fn stateless_mcp_catalog(tools: Vec<Self>, plugins: Vec<PluginListItem>) -> ToolApiCatalog {
        let mut tools: Vec<_> = tools
            .into_iter()
            .filter(|tool| is_stateless_mcp_tool_exposed(tool.stateless_mcp_metadata()))
            .map(|tool| CatalogTool {
                canonical_name: tool.canonical_name().to_owned(),
                name: tool.name,
                plugin_id: tool.plugin_id,
                summary: tool.summary.unwrap_or_default(),
                help: [tool.before_help, tool.help, tool.after_help]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                tags: tool.tags,
                input_usage: schema_usage_text(&tool.input_schema),
                input_examples: schema_example_texts(&tool.input_schema),
                input_schema: tool.input_schema,
                output_schema: tool.output_schema.unwrap_or(serde_json::Value::Null),
            })
            .collect();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        let mut owners = std::collections::BTreeMap::<_, usize>::new();
        for tool in &tools {
            *owners.entry(&tool.plugin_id).or_default() += 1;
        }
        let mut plugins: Vec<_> = plugins
            .into_iter()
            .filter_map(|plugin| {
                let id = plugin.status.plugin_id.to_string();
                let tool_count = *owners.get(&id)?;
                Some(CatalogPlugin {
                    id,
                    version: plugin.version.unwrap_or_default(),
                    summary: plugin.summary.unwrap_or_default(),
                    tags: plugin.tags.iter().map(ToString::to_string).collect(),
                    tool_count,
                })
            })
            .collect();
        plugins.sort_by(|left, right| left.id.cmp(&right.id));
        ToolApiCatalog { tools, plugins }
    }
}

#[derive(Debug, Clone, Deserialize)]
/// Server-owned invocation of a Runtime tool outside a session lifecycle.
pub struct OperatorToolInvokeRequest {
    /// Database-backed workspace identity the caller intends to operate on.
    /// The server resolves this id and requires its canonical path to equal
    /// the workspace root of the composed Runtime tool executor.
    pub workspace_id: i64,
    pub tool: String,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    /// Trusted API caller's execution mode, outside the tool input object.
    /// Ordinary operator calls preserve isolation; MCP bridges send false.
    #[serde(default = "operator_sandbox_enabled")]
    pub sandbox_enabled: bool,
}

fn operator_sandbox_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_sandbox_defaults_to_isolation_and_accepts_explicit_direct_execution() {
        let ordinary: OperatorToolInvokeRequest = serde_json::from_value(serde_json::json!({
            "workspace_id": 1, "tool": "agena.fs.read"
        }))
        .unwrap();
        assert!(ordinary.sandbox_enabled);
        let direct: OperatorToolInvokeRequest = serde_json::from_value(serde_json::json!({
            "workspace_id": 1, "tool": "agena.fs.read", "sandbox_enabled": false,
            "input": {"sandbox_enabled": true}
        }))
        .unwrap();
        assert!(!direct.sandbox_enabled);
    }
}
