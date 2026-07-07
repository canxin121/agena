use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, Result as SdkResult,
    ToolInvokeInput, ToolInvokeOutput, async_trait,
};
use crate::plugins::provided::workflow::{TaskToolSuite, WorkflowPlugin, WorkflowPluginConfig};

pub(crate) const TASKS_PLUGIN_ID: &str = "agena.tasks";

pub(crate) struct TasksPlugin {
    inner: WorkflowPlugin,
}

impl TasksPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }
}

#[async_trait]
impl Plugin for TasksPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "tasks", env!("CARGO_PKG_VERSION"));
        manifest.summary = Some("Delegated subtask orchestration tools.".to_string());
        manifest.set_display(crate::plugin::sdk::ToolDisplayPreset::BriefDetailed);
        manifest.hooks |= HookSubscription::TOOL_INVOKE;
        manifest.tools.extend(TaskToolSuite::tool_definitions());
        manifest
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let parsed = TaskToolSuite::parse_tool(input.tool_name.as_str(), input.input)?;
        parsed.dispatch_tool_invoke(&self.inner).await
    }
}
