use std::sync::Arc;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, Result as SdkResult,
    ToolInvokeInput, ToolInvokeOutput, async_trait,
};
use crate::plugins::provided::workflow::{
    TaskToolActionInput, WorkflowPlugin, WorkflowPluginConfig,
};

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
        PluginManifest::builder(TASKS_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("Delegated subtask orchestration tools.")
            .brief_detailed()
            .hooks(HookSubscription::TOOL_INVOKE)
            .tool(TaskToolActionInput::tool_decl())
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        if input.tool_name != "task" {
            return Err(PluginError::not_implemented(format!(
                "tool_invoke({})",
                input.tool_name
            )));
        }
        let parsed = TaskToolActionInput::parse_input(input.input)?;
        parsed.dispatch_tool_invoke(&self.inner).await
    }
}
