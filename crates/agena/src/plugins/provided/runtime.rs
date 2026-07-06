use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, Result as SdkResult,
    ToolInvokeInput, ToolInvokeOutput, async_trait,
};
use crate::plugins::provided::workflow::{
    AgentToolInput, SessionToolInput, UserToolInput, WorkflowPlugin, WorkflowPluginConfig,
};

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
        PluginManifest::builder("agena", "runtime", env!("CARGO_PKG_VERSION"))
            .description("Runtime session, agent, and user-interaction tools.")
            .brief_detailed()
            .hooks(HookSubscription::TOOL_INVOKE)
            .tools(AgentToolInput::tool_definitions())
            .tools(SessionToolInput::tool_definitions())
            .tools(UserToolInput::tool_definitions())
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let tool_name = input.tool_name.as_str();
        if AgentToolInput::tool_definitions()
            .iter()
            .any(|definition| definition.name == tool_name)
        {
            let parsed = AgentToolInput::parse_tool(tool_name, input.input)?;
            return parsed.dispatch_tool_invoke(&self.inner).await;
        }
        if SessionToolInput::tool_definitions()
            .iter()
            .any(|definition| definition.name == tool_name)
        {
            let parsed = SessionToolInput::parse_tool(tool_name, input.input)?;
            return parsed.dispatch_tool_invoke(&self.inner).await;
        }
        let parsed = UserToolInput::parse_tool(tool_name, input.input)?;
        parsed.dispatch_tool_invoke(&self.inner).await
    }
}
