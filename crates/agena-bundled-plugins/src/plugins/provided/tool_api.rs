use std::sync::Arc;

use crate::plugins::provided::workflow::{
    ToolApiHelpInput, ToolApiListInput, ToolApiSearchInput, ToolApiTagsInput, WorkflowPlanConfig,
    WorkflowPlugin, WorkflowPluginConfig, tool_discovery_config_schema,
};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput};

pub(crate) const TOOL_API_PLUGIN_ID: &str = "agena.tools";

pub(crate) struct ToolApiPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "tools",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Tool API discovery functions. The runtime resolves tools_call directly to its execution target.",
    config_schema = tool_discovery_config_schema(),
    display = brief_detailed
)]
impl ToolApiPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let tool_discovery = agena_plugin_host::sdk::macro_support::parse_defaulted_config(
            ctx.config.clone(),
            "invalid tools config",
        )?;
        self.inner.initialize(
            ctx,
            WorkflowPluginConfig {
                tool_discovery,
                plan: WorkflowPlanConfig::default(),
            },
            host,
        )?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(query, discovery),
        summary = "Enumerate current tools.",
        read_only,
        discovery,
        ui_display = detailed,

        concurrency_safe
    )]
    async fn list(&self, input: &ToolApiListInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_list(input).await
    }

    #[tool(
        tags(query, discovery),
        summary = "Search the Agena execution tools available in this session.",
        read_only,
        discovery,
        ui_display = detailed,

        concurrency_safe
    )]
    async fn search(&self, input: &ToolApiSearchInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_search(input).await
    }

    #[tool(
        tags(query, discovery),
        summary = "Get reusable schema, examples, and usage notes for one Agena execution tool.",
        read_only,
        discovery,
        ui_display = detailed,
                concurrency_safe
    )]
    async fn help(&self, input: &ToolApiHelpInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_help(input).await
    }

    #[tool(
        tags(query, discovery),
        summary = "List tool tags with pagination.",
        read_only,
        discovery,
        ui_display = detailed,

        concurrency_safe
    )]
    async fn tags(&self, input: &ToolApiTagsInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_tags(input).await
    }

    #[tool(
        tags(query, discovery),
        summary = "Enumerate the current live plugin inventory with version, summary, tags, and tool count.",
        read_only,
        discovery,
        ui_display = detailed,

        concurrency_safe
    )]
    async fn plugins_list(&self, input: &ToolApiListInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plugins_list(input).await
    }

    #[tool(
        tags(query, discovery),
        summary = "Search the loaded plugins by id, summary, or tag.",
        read_only,
        discovery,
        ui_display = detailed,

        concurrency_safe
    )]
    async fn plugins_search(&self, input: &ToolApiSearchInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plugins_search(input).await
    }

    #[tool(
        tags(query, discovery),
        summary = "List plugin tags with pagination.",
        read_only,
        discovery,
        ui_display = detailed,

        concurrency_safe
    )]
    async fn plugins_tags(&self, input: &ToolApiTagsInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plugins_tags(input).await
    }
}

#[cfg(test)]
mod tests {
    use super::ToolApiPlugin;
    use agena_plugin_host::sdk::{Plugin, ToolDefinition};

    fn definition_for(tool_name: &str) -> ToolDefinition {
        ToolApiPlugin::new()
            .manifest()
            .tools
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .unwrap_or_else(|| panic!("missing tools plugin tool `{tool_name}`"))
    }

    #[test]
    fn definitions_distinguish_tool_api_functions_from_execution_tools() {
        let help = definition_for("help");
        assert!(
            help.docs
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("one Agena execution tool"))
        );
        let help_tool_description = help
            .contract
            .input_schema
            .pointer("/properties/tool/description")
            .and_then(serde_json::Value::as_str)
            .expect("help tool-name description");
        assert!(help_tool_description.contains("Exact name"));
        assert!(help_tool_description.contains("`tools_list` or `tools_search`"));
    }
}
