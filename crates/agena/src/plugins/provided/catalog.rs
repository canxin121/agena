use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, Result as SdkResult,
    ToolInvokeInput, ToolInvokeOutput, async_trait,
};
use crate::plugins::provided::workflow::{
    CatalogToolSuite, WorkflowPlanConfig, WorkflowPlugin, WorkflowPluginConfig,
    tool_catalog_plugin_config_schema,
};

pub(crate) const TOOLS_PLUGIN_ID: &str = "agena.tools";

pub(crate) struct ToolsPlugin {
    inner: WorkflowPlugin,
}

impl ToolsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }
}

#[async_trait]
impl Plugin for ToolsPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "tools", env!("CARGO_PKG_VERSION"));
        manifest.summary = Some("Tool discovery, help, and gateway tools.".to_string());
        manifest.config_schema = Some(tool_catalog_plugin_config_schema());
        manifest.set_display(crate::plugin::sdk::ToolDisplayPreset::BriefDetailed);
        manifest.hooks |= HookSubscription::TOOL_INVOKE;
        manifest.tools.extend(CatalogToolSuite::tool_definitions());
        manifest
    }

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
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let parsed = CatalogToolSuite::parse_tool(input.tool_name.as_str(), input.input)?;
        parsed.dispatch_tool_invoke(&self.inner).await
    }
}
