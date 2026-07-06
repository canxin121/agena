use std::sync::Arc;

use crate::plugin::PluginError;
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
        PluginManifest::builder(RUNTIME_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("Runtime session, agent, and user-interaction tools.")
            .brief_detailed()
            .hooks(HookSubscription::TOOL_INVOKE)
            .tools([
                AgentToolInput::tool_definition(),
                SessionToolInput::tool_definition(),
                UserToolInput::tool_definition(),
            ])
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        match input.tool_name.as_str() {
            "agent" => {
                let parsed = AgentToolInput::parse_input(input.input)?;
                parsed.dispatch_tool_invoke(&self.inner).await
            }
            "session" => {
                let parsed = SessionToolInput::parse_input(input.input)?;
                parsed.dispatch_tool_invoke(&self.inner).await
            }
            "user" => {
                let parsed = UserToolInput::parse_input(input.input)?;
                parsed.dispatch_tool_invoke(&self.inner).await
            }
            _ => Err(PluginError::not_implemented(format!(
                "tool_invoke({})",
                input.tool_name
            ))),
        }
    }
}
