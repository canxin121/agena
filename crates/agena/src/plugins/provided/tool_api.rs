use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HostCapability, InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput,
};
use crate::plugins::provided::workflow::{
    ToolApiCallInput, ToolApiHelpInput, ToolApiListInput, ToolApiSearchInput, ToolApiTagsInput,
    WorkflowPlanConfig, WorkflowPlugin, WorkflowPluginConfig, tool_discovery_config_schema,
};

pub(crate) const TOOL_API_PLUGIN_ID: &str = "agena.tools";

pub(crate) struct ToolApiPlugin {
    inner: WorkflowPlugin,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "tools",
    version = env!("CARGO_PKG_VERSION"),
    summary = "The five Tool API functions for discovering and running Agena execution tools.",
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
        let tool_discovery = crate::plugin::sdk::macro_support::parse_defaulted_config(
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
    async fn list(&self, input: &ToolApiListInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_list(input).await
    }

    #[tool(
        summary = "Search the Agena execution tools available in this session.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(HostCapability::ListTools, HostCapability::ToolRegistry),
        concurrency_safe
    )]
    async fn search(&self, input: &ToolApiSearchInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_search(input).await
    }

    #[tool(
        summary = "Get reusable schema, examples, and usage notes for one Agena execution tool.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(
            HostCapability::ListTools,
            HostCapability::ToolRegistry
        ),
        concurrency_safe
    )]
    async fn help(&self, input: &ToolApiHelpInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_help(input).await
    }

    #[tool(
        summary = "List tool tags with pagination.",
        read_only,
        discovery,
        ui_display = detailed,
        capabilities(HostCapability::ListTools, HostCapability::ToolRegistry),
        concurrency_safe
    )]
    async fn tags(&self, input: &ToolApiTagsInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_tags(input).await
    }

    #[tool(
        summary = "Run one Agena execution tool with complete input; validation errors include tool help for a direct retry.",
        discovery,
        ui_display = detailed,
        capabilities(
            HostCapability::ListTools,
            HostCapability::InvokeTool
        )
    )]
    async fn call(&self, input: &ToolApiCallInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_tool_api_call(input).await
    }
}

#[cfg(test)]
mod tests {
    use super::ToolApiPlugin;
    use crate::plugin::sdk::{HostCapability, Plugin, ToolDefinition};

    fn definition_for(tool_name: &str) -> ToolDefinition {
        ToolApiPlugin::new()
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

        let call = definition_for("call");
        assert!(
            call.docs
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("Run one Agena execution tool"))
        );
        let call_tool_description = call
            .contract
            .input_schema
            .pointer("/properties/tool/description")
            .and_then(serde_json::Value::as_str)
            .expect("call tool-name description");
        assert!(call_tool_description.contains("`tools_call`"));
        assert!(call_tool_description.contains("execution tool"));
        let call_input_schema = call
            .contract
            .input_schema
            .pointer("/properties/input")
            .expect("execution-tool input schema");
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
