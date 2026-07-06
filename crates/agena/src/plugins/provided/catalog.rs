use std::sync::Arc;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, Result as SdkResult,
    ToolInvokeInput, ToolInvokeOutput, async_trait,
};
use crate::plugins::provided::workflow::{
    ToolsToolInput, WorkflowPlanConfig, WorkflowPlugin, WorkflowPluginConfig,
    tool_catalog_plugin_config_schema,
};

#[cfg(test)]
use crate::plugin::sdk::host_api::ToolDescriptor;

pub(crate) const CATALOG_PLUGIN_ID: &str = "agena.catalog";

pub(crate) struct CatalogPlugin {
    inner: WorkflowPlugin,
}

impl CatalogPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }
}

#[async_trait]
impl Plugin for CatalogPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena", "catalog", env!("CARGO_PKG_VERSION"))
            .description("Tool catalog discovery and help tools.")
            .config_schema(tool_catalog_plugin_config_schema())
            .brief_detailed()
            .hooks(HookSubscription::TOOL_INVOKE)
            .tool(ToolsToolInput::tool_definition())
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let tool_catalog = crate::plugin::sdk::macro_support::parse_defaulted_config(
            ctx.config.clone(),
            "invalid catalog config",
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
        if input.tool_name != "tools" {
            return Err(PluginError::not_implemented(format!(
                "tool_invoke({})",
                input.tool_name
            )));
        }
        let parsed = ToolsToolInput::parse_input(input.input)?;
        parsed.dispatch_tool_invoke(&self.inner).await
    }
}

#[cfg(test)]
pub(crate) fn tools_tool_descriptor_for_tests() -> ToolDescriptor {
    let definition = ToolsToolInput::tool_definition();
    ToolDescriptor {
        name: crate::plugin::registry::model_tool_name("agena", "catalog", "tools"),
        description: Some(definition.description_text().to_string()),
        before_help: definition.before_help_text().map(ToString::to_string),
        after_help: definition.after_help_text().map(ToString::to_string),
        summary: definition.summary_text().map(ToString::to_string),
        help: definition.help_text().map(ToString::to_string),
        examples: vec![],
        input_schema: Some(definition.sanitized_input_schema()),
        description_mode: None,
        tags: definition.effective_tags(),
        plugin_id: None,
    }
}
