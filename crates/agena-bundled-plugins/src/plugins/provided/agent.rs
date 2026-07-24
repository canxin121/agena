use std::sync::Arc;

use crate::message::AgentSwitchToolInput;
use crate::plugins::provided::workflow::{WorkflowPlugin, WorkflowPluginConfig};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    HostCapability, InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput,
};

pub(crate) const AGENT_PLUGIN_ID: &str = "agena.agent";

pub(crate) struct AgentPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "agent",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Runtime agent profile tools.",
    display = brief_detailed
)]
impl AgentPlugin {
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
        summary = "Switch the current runtime agent profile.",
        display = brief,
        capabilities(HostCapability::AgentRegistry)
    )]
    async fn switch(&self, input: &AgentSwitchToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_agent_switch(input).await
    }

    #[tool(
        summary = "Restore the previous runtime agent profile.",
        display = brief,
        capabilities(HostCapability::AgentRegistry)
    )]
    async fn restore(&self) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_agent_restore().await
    }
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::Plugin;

    use super::AgentPlugin;

    #[test]
    fn manifest_contains_only_agent_profile_tools() {
        let manifest = AgentPlugin::new().manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "agent");
        assert_eq!(tool_names, ["switch", "restore"]);
    }
}
