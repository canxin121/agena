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

    #[hook]
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
        name = "list",
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
        name = "search",
        summary = "Search the current tool catalog.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(HostCapability::ListTools, HostCapability::ToolRegistry),
        trim("query"),
        non_empty("query"),
        concurrency_safe
    )]
    async fn search(&self, input: &CatalogSearchInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_search(input).await
    }

    #[tool(
        name = "help",
        summary = "Fetch detailed tool help.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(HostCapability::ListTools),
        trim("tool"),
        non_empty("tool"),
        concurrency_safe
    )]
    async fn help(&self, input: &ToolsHelpInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_help(input).await
    }

    #[tool(
        name = "tags",
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
        name = "call",
        summary = "Invoke a tool after reading its help.",
        discovery,
        ui_display = detailed,
        capabilities(HostCapability::InvokeTool),
        trim("tool"),
        non_empty("tool")
    )]
    async fn call(&self, input: &ToolCallInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_call(input).await
    }
}
