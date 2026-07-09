use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    CommandBeforeInput, CommandBeforeResponse, HostCapability, InitContext, InitOutcome,
    Result as SdkResult, ToolBeforeInput, ToolBeforePatch, ToolInvokeOutput,
};
use crate::plugins::provided::workflow::{
    PlanGetInput, PlanSetInput, PlanUpdateInput, WorkflowPlugin, WorkflowPluginConfig,
    planning_plugin_config_schema,
};

pub(crate) const PLAN_PLUGIN_ID: &str = "agena.plan";

pub(crate) struct PlanPlugin {
    inner: WorkflowPlugin,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "plan",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Plan orchestration and plan-autorun tools.",
    config_schema = planning_plugin_config_schema(),
    display = brief_detailed,
    plugin_capabilities(HostCapability::PluginStorage, HostCapability::Statusline)
)]
impl PlanPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook(init)]
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
        Ok(InitOutcome::ack(crate::plugin::sdk::Plugin::manifest(self)))
    }

    #[tool(
        summary = "Inspect the current plan state.",
        planning,
        read_only,
        display = brief,
        capabilities(
            HostCapability::AskUser,
            HostCapability::PluginStorage,
            HostCapability::Statusline
        ),
        concurrency_safe
    )]
    async fn get(&self, input: &PlanGetInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_get(input).await
    }

    #[tool(
        summary = "Create or replace the current plan.",
        planning,
        mutating,
        display = brief,
        capabilities(
            HostCapability::AskUser,
            HostCapability::PluginStorage,
            HostCapability::Statusline
        )
    )]
    async fn set(&self, input: &PlanSetInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_set(input).await
    }

    #[tool(
        summary = "Update the current plan.",
        help = "Keep plan-level updates separate from step/check updates: do not send `phase` together with `step_id`, `check_id`, `status`, `wait_until_ms`, or `note`. To complete a plan with steps, mark the required steps/checks `completed` first, then call update separately with `phase: completed`.",
        planning,
        mutating,
        display = brief,
        capabilities(
            HostCapability::AskUser,
            HostCapability::PluginStorage,
            HostCapability::Statusline
        )
    )]
    async fn update(&self, input: &PlanUpdateInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_update(input).await
    }

    #[tool(
        summary = "Remove the current plan.",
        planning,
        mutating,
        display = brief,
        capabilities(
            HostCapability::AskUser,
            HostCapability::PluginStorage,
            HostCapability::Statusline
        )
    )]
    async fn clear(&self) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_clear().await
    }

    #[hook(tool.before)]
    async fn tool_execute_before(
        &self,
        input: ToolBeforeInput,
    ) -> SdkResult<Option<ToolBeforePatch>> {
        self.inner.tool_execute_before_hook(input).await
    }

    #[hook(shell.before)]
    async fn command_execute_before(
        &self,
        input: CommandBeforeInput,
    ) -> SdkResult<Option<CommandBeforeResponse>> {
        self.inner.command_execute_before_hook(input).await
    }

    #[hook(agent.stop)]
    async fn agent_stop(
        &self,
        input: crate::plugin::AgentStopInput,
    ) -> SdkResult<Option<crate::plugin::AgentStopPatch>> {
        self.inner.agent_stop_hook(input).await
    }
}
