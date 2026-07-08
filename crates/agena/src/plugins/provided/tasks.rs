use std::sync::Arc;

use crate::message::TaskToolInput;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HostCapability, InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput,
};
use crate::plugins::provided::workflow::{WorkflowPlugin, WorkflowPluginConfig};

pub(crate) const TASKS_PLUGIN_ID: &str = "agena.tasks";

pub(crate) struct TasksPlugin {
    inner: WorkflowPlugin,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "tasks",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Delegated subtask orchestration tools.",
    display = brief_detailed
)]
impl TasksPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(crate::plugin::sdk::Plugin::manifest(self)))
    }

    #[tool(
        summary = "Create or resume a delegated subagent task.",
        task,
        subtask,
        display = detailed,
        capabilities(HostCapability::SpawnSubtask, HostCapability::PluginStorage)
    )]
    async fn run(&self, input: &TaskToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_task(input).await
    }
}
