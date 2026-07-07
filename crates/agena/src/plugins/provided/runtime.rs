use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, Result as SdkResult,
    ToolInvokeInput, ToolInvokeOutput, async_trait,
};
use crate::plugins::provided::workflow::{RuntimeToolSuite, WorkflowPlugin, WorkflowPluginConfig};

pub(crate) const RUNTIME_PLUGIN_ID: &str = "agena.runtime";

pub(crate) struct RuntimePlugin {
    inner: WorkflowPlugin,
}

impl RuntimePlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }
}

#[async_trait]
impl Plugin for RuntimePlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "runtime", env!("CARGO_PKG_VERSION"));
        manifest.summary = Some("Runtime session, agent, and user-interaction tools.".to_string());
        manifest.set_display(crate::plugin::sdk::ToolDisplayPreset::BriefDetailed);
        manifest.hooks |= HookSubscription::TOOL_INVOKE;
        manifest.tools.extend(RuntimeToolSuite::tool_definitions());
        manifest
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let parsed = RuntimeToolSuite::parse_tool(input.tool_name.as_str(), input.input)?;
        parsed.dispatch_tool_invoke(&self.inner).await
    }
}
