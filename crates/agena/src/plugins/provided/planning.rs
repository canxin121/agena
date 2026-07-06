use std::sync::Arc;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    CommandBeforeInput, CommandBeforeResponse, HookSubscription, HostCapability, InitContext,
    InitOutcome, Plugin, PluginManifest, Result as SdkResult, ToolBeforeInput, ToolBeforePatch,
    ToolInvokeInput, ToolInvokeOutput, async_trait,
};
use crate::plugins::provided::workflow::{
    PlanToolInput, WorkflowPlugin, WorkflowPluginConfig, planning_plugin_config_schema,
};

pub(crate) const PLAN_PLUGIN_ID: &str = "agena.plan";

pub(crate) struct PlanPlugin {
    inner: WorkflowPlugin,
}

impl PlanPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }
}

#[async_trait]
impl Plugin for PlanPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena", "plan", env!("CARGO_PKG_VERSION"))
            .description("Plan orchestration and plan-autorun tools.")
            .config_schema(planning_plugin_config_schema())
            .brief_detailed()
            .hooks(
                HookSubscription::TOOL_INVOKE
                    | HookSubscription::TOOL_BEFORE
                    | HookSubscription::COMMAND_BEFORE
                    | HookSubscription::AGENT_STOP,
            )
            .plugin_capabilities([HostCapability::PluginStorage, HostCapability::Statusline])
            .tool(PlanToolInput::tool_definition())
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let plan = crate::plugin::sdk::macro_support::parse_defaulted_config(
            ctx.config.clone(),
            "invalid planning config",
        )?;
        self.inner.initialize(
            ctx,
            WorkflowPluginConfig {
                tool_catalog: Default::default(),
                plan,
            },
            host,
        )?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        if input.tool_name != "plan" {
            return Err(PluginError::not_implemented(format!(
                "tool_invoke({})",
                input.tool_name
            )));
        }
        let parsed = PlanToolInput::parse_input(input.input)?;
        parsed.dispatch_tool_invoke(&self.inner).await
    }

    async fn tool_execute_before(
        &self,
        input: ToolBeforeInput,
    ) -> SdkResult<Option<ToolBeforePatch>> {
        self.inner.tool_execute_before_hook(input).await
    }

    async fn command_execute_before(
        &self,
        input: CommandBeforeInput,
    ) -> SdkResult<Option<CommandBeforeResponse>> {
        self.inner.command_execute_before_hook(input).await
    }

    async fn agent_stop(
        &self,
        input: crate::plugin::AgentStopInput,
    ) -> SdkResult<Option<crate::plugin::AgentStopPatch>> {
        self.inner.agent_stop_hook(input).await
    }
}
