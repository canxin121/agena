use std::sync::Arc;

use crate::plugins::provided::workflow::{
    PlanGetInput, PlanSetInput, PlanUpdateInput, WorkflowPlugin, WorkflowPluginConfig,
    planning_plugin_config_schema,
};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    CommandBeforeInput, CommandBeforeResponse, InitContext, InitOutcome, Result as SdkResult,
    ToolBeforeInput, ToolBeforePatch, ToolInvokeOutput,
};

pub(crate) const PLAN_PLUGIN_ID: &str = "agena.plan";

pub(crate) struct PlanPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "plan",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Plan orchestration and plan-autorun tools.",
    config_schema = planning_plugin_config_schema(),
    display = brief_detailed
)]
impl PlanPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let plan = agena_plugin_host::sdk::macro_support::parse_defaulted_config(
            ctx.config.clone(),
            "invalid planning config",
        )?;
        self.inner.initialize(
            ctx,
            WorkflowPluginConfig {
                tool_discovery: Default::default(),
                plan,
            },
            host,
        )?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(query, planning),
        summary = "Inspect the current plan state.",
        planning,
        read_only,
        display = brief,

        concurrency_safe
    )]
    async fn get(&self, input: &PlanGetInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_get(input).await
    }

    #[tool(
        tags(mutate, planning),
        summary = "Create or replace the current plan.",
        help = "Prefer using this tool for implementation tasks unless they are simple. Use it proactively when starting a non-trivial implementation task: getting sign-off on your approach before writing code prevents wasted effort and ensures alignment. Use it when ANY of these conditions apply: new features, multiple valid approaches, changes to existing behavior or structure, architectural decisions, changes touching more than 2-3 files, unclear requirements, or when you would otherwise ask the user to clarify the approach. Only skip it for simple tasks: single-line fixes, adding a single function with clear requirements, very specific detailed instructions, or pure research/read-only work. If unsure whether to use it, err on the side of planning. While the plan is in the `planning` phase, mutating tools are blocked; explore with read-only tools (including parallel `tasks.run` exploration when the scope spans multiple areas), clarify with `ask`, and refine. Present the finished plan for approval through the plan phase transition; never ask whether the plan is acceptable via `ask`.",
        planning,
        mutating,
        display = brief,

    )]
    async fn set(&self, input: &PlanSetInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_set(input).await
    }

    #[tool(
        tags(mutate, planning),
        summary = "Update the current plan.",
        help = "Keep plan-level updates separate from step/check updates: do not send `phase` together with `step`, `check`, `status`, `wait_until_ms`, or `note`. Address steps and checks by 1-based index (`step`, `check`). To complete a plan with steps, mark the required steps/checks `completed` first, then call update separately with `phase: completed`.",
        planning,
        mutating,
        display = brief,

    )]
    async fn update(&self, input: &PlanUpdateInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_update(input).await
    }

    #[tool(
        tags(mutate, planning),
        summary = "Remove the current plan.",
        planning,
        mutating,
        display = brief,

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
        input: agena_plugin_host::AgentStopInput,
    ) -> SdkResult<Option<agena_plugin_host::AgentStopPatch>> {
        self.inner.agent_stop_hook(input).await
    }
}
