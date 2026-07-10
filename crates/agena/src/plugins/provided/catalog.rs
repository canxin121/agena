use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HostCapability, InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput,
};
use crate::plugins::provided::workflow::{
    CatalogSearchInput, ToolCallInput, ToolListInput, ToolTagsInput, ToolsHelpInput,
    WorkflowPlanConfig, WorkflowPlugin, WorkflowPluginConfig, tool_catalog_plugin_config_schema,
};

pub(crate) const TOOLS_PLUGIN_ID: &str = "agena.tools";

pub(crate) struct ToolsPlugin {
    inner: WorkflowPlugin,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "tools",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Tool discovery, help, and gateway tools.",
    config_schema = tool_catalog_plugin_config_schema(),
    display = brief_detailed
)]
impl ToolsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let tool_catalog = crate::plugin::sdk::macro_support::parse_defaulted_config(
            ctx.config.clone(),
            "invalid tools config",
        )?;
        self.inner.initialize(
            ctx,
            WorkflowPluginConfig {
                tool_catalog,
                plan: WorkflowPlanConfig::default(),
            },
            host,
        )?;
        Ok(InitOutcome::ack(crate::plugin::sdk::Plugin::manifest(self)))
    }

    #[tool(
        summary = "Enumerate current tools.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(HostCapability::ListTools, HostCapability::ToolRegistry),
        concurrency_safe
    )]
    async fn list(&self, input: &ToolListInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_list(input).await
    }

    #[tool(
        summary = "Search the current tool catalog.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(HostCapability::ListTools, HostCapability::ToolRegistry),
        concurrency_safe
    )]
    async fn search(&self, input: &CatalogSearchInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_search(input).await
    }

    #[tool(
        summary = "Fetch detailed tool help.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(
            HostCapability::ListTools,
            HostCapability::ToolRegistry,
            HostCapability::PluginStorage
        ),
        concurrency_safe
    )]
    async fn help(&self, input: &ToolsHelpInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_help(input).await
    }

    #[tool(
        summary = "List tool tags with pagination.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(HostCapability::ListTools, HostCapability::ToolRegistry),
        concurrency_safe
    )]
    async fn tags(&self, input: &ToolTagsInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_tags(input).await
    }

    #[tool(
        summary = "Invoke a tool after reading its help.",
        discovery,
        ui_display = detailed,
        capabilities(
            HostCapability::ListTools,
            HostCapability::InvokeTool,
            HostCapability::PluginStorage
        )
    )]
    async fn call(&self, input: &ToolCallInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_call(input).await
    }
}

#[cfg(test)]
mod tests {
    use super::ToolsPlugin;
    use crate::plugin::sdk::{HostCapability, Plugin};

    fn capabilities_for(tool_name: &str) -> Vec<HostCapability> {
        ToolsPlugin::new()
            .manifest()
            .tools
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .unwrap_or_else(|| panic!("missing tools plugin tool `{tool_name}`"))
            .capabilities
    }

    #[test]
    fn help_and_call_declare_every_host_capability_they_use() {
        let help = capabilities_for("help");
        assert!(help.contains(&HostCapability::ListTools));
        assert!(help.contains(&HostCapability::ToolRegistry));
        assert!(help.contains(&HostCapability::PluginStorage));

        let call = capabilities_for("call");
        assert!(call.contains(&HostCapability::ListTools));
        assert!(call.contains(&HostCapability::InvokeTool));
        assert!(call.contains(&HostCapability::PluginStorage));
    }
}
