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
        summary = "Inspect reusable schema and examples for a dotted catalog target; the target itself is never a provider function.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(
            HostCapability::ListTools,
            HostCapability::ToolRegistry
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
        summary = "Execute a dotted catalog target with one complete input; schema mismatches return embedded target help for a direct retry.",
        discovery,
        ui_display = detailed,
        capabilities(
            HostCapability::ListTools,
            HostCapability::InvokeTool
        )
    )]
    async fn call(&self, input: &ToolCallInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_call(input).await
    }
}

#[cfg(test)]
mod tests {
    use super::ToolsPlugin;
    use crate::plugin::sdk::{HostCapability, Plugin, ToolDefinition};

    fn definition_for(tool_name: &str) -> ToolDefinition {
        ToolsPlugin::new()
            .manifest()
            .tools
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .unwrap_or_else(|| panic!("missing tools plugin tool `{tool_name}`"))
    }

    fn capabilities_for(tool_name: &str) -> Vec<HostCapability> {
        definition_for(tool_name).capabilities
    }

    #[test]
    fn help_and_call_declare_every_host_capability_they_use() {
        let help = capabilities_for("help");
        assert!(help.contains(&HostCapability::ListTools));
        assert!(help.contains(&HostCapability::ToolRegistry));
        assert!(!help.contains(&HostCapability::PluginStorage));

        let call = capabilities_for("call");
        assert!(call.contains(&HostCapability::ListTools));
        assert!(call.contains(&HostCapability::InvokeTool));
        assert!(!call.contains(&HostCapability::PluginStorage));
    }

    #[test]
    fn provider_definitions_distinguish_functions_from_catalog_targets() {
        let help = definition_for("help");
        assert!(
            help.docs
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("never a provider function"))
        );
        let help_tool_description = help
            .contract
            .input_schema
            .pointer("/properties/tool/description")
            .and_then(serde_json::Value::as_str)
            .expect("help target description");
        assert!(help_tool_description.contains("payload data"));
        assert!(help_tool_description.contains("never be used as a provider function name"));

        let call = definition_for("call");
        assert!(
            call.docs
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("Execute a dotted catalog target"))
        );
        let call_tool_description = call
            .contract
            .input_schema
            .pointer("/properties/tool/description")
            .and_then(serde_json::Value::as_str)
            .expect("call target description");
        assert!(call_tool_description.contains("`tools_call`"));
        assert!(call_tool_description.contains("never call this target directly"));
        let call_input_schema = call
            .contract
            .input_schema
            .pointer("/properties/input")
            .expect("call target input schema");
        assert_eq!(
            call_input_schema
                .get("additionalProperties")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(
            call_input_schema
                .get("description")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|description| description.contains("collapse a populated object"))
        );
    }
}
