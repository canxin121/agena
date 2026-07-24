use std::sync::Arc;

use crate::message::TaskToolInput;
use crate::plugins::provided::workflow::{WorkflowPlugin, WorkflowPluginConfig};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    HostCapability, InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput,
};

pub(crate) const TASKS_PLUGIN_ID: &str = "agena.tasks";

pub(crate) struct TasksPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
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

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        summary = "Create or resume a delegated subagent task.",
        task,
        subtask,
        display = detailed,
        capabilities(HostCapability::RunSubtask, HostCapability::PluginStorage)
    )]
    async fn run(&self, input: &TaskToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_task(input).await
    }
}

#[cfg(test)]
mod tests {
    use crate::message::TaskToolInput;
    use agena_plugin_host::sdk::{HostCapability, Plugin};
    use agena_plugin_sdk::ToolInput;

    use super::TasksPlugin;

    #[test]
    fn task_contract_uses_dynamic_profiles_and_terminal_host_capability() {
        let manifest = TasksPlugin::new().manifest();
        let tool = manifest.tools.first().expect("task tool");
        assert_eq!(tool.name, "run");
        assert!(tool.capabilities.contains(&HostCapability::RunSubtask));
        let schema = &tool.contract.input_schema;
        assert!(schema.pointer("/properties/profile").is_some());
        assert!(schema.pointer("/properties/selection").is_some());
        assert!(schema.pointer("/properties/subagent_type").is_none());
        assert!(schema.pointer("/properties/command").is_none());
        assert_eq!(
            schema.pointer("/properties/timeout_ms/minimum"),
            Some(&serde_json::json!(1))
        );
    }

    #[test]
    fn task_input_rejects_zero_timeout_and_unknown_legacy_fields() {
        let valid = serde_json::json!({
            "description": "verify",
            "prompt": "run the checks",
            "profile": "verify",
            "timeout_ms": 1
        });
        assert!(TaskToolInput::parse_input(valid).is_ok());

        for invalid in [
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "profile": "verify",
                "timeout_ms": 0
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "profile": "verify",
                "task_id": "   "
            }),
            serde_json::json!({
                "description": "verify",
                "prompt": "run the checks",
                "profile": "verify",
                "subagent_type": "verify"
            }),
        ] {
            assert!(TaskToolInput::parse_input(invalid).is_err());
        }
    }
}
