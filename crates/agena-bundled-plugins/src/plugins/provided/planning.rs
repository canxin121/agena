#[cfg(test)]
mod revision_tests;
use std::sync::Arc;

use crate::plugins::provided::workflow::{
    PlanEditInput, PlanGetInput, PlanPhaseInput, PlanReviewInput, PlanSetInput, WorkflowPlanConfig,
    WorkflowPlugin, WorkflowPluginConfig,
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
    settings = WorkflowPlanConfig,
    settings_default = default,
)]
impl PlanPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let plan = agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
            ctx.settings.clone(),
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
        tags(query, planning, read_only),
        summary = "Inspect the current plan state."
    )]
    async fn get(&self, input: &PlanGetInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_get(input).await
    }

    #[tool(
        tags(mutate, planning),
        summary = "Create or replace the current plan without requesting approval.",
        help = "Prefer using this tool for implementation tasks unless they are simple. Use it proactively when starting a non-trivial implementation task: getting sign-off on your approach before writing code prevents wasted effort and ensures alignment. Use it when ANY of these conditions apply: new features, multiple valid approaches, changes to existing behavior or structure, architectural decisions, changes touching more than 2-3 files, unclear requirements, or when you would otherwise ask the user to clarify the approach. Only skip it for simple tasks: single-line fixes, adding a single function with clear requirements, very specific detailed instructions, or pure research/read-only work. If unsure whether to use it, err on the side of planning. This tool never blocks on the user: it saves the plan and returns. With `request_approval: true` (the default) the plan stays in the `planning` phase and you must call `plan.review` to request user approval before it becomes active. Pass `request_approval: false` only with prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation` — the plan then becomes active immediately. Never change settings to bypass approval. While the plan is in the `planning` phase, mutating tools are blocked; explore with read-only tools (including parallel `tasks.run` exploration when the scope spans multiple areas), clarify with `interaction.ask` when available, replace the plan content with `plan.set`, and update progress/notes with `plan.edit`. When the plan is complete, call `plan.review` to present it for approval; never ask whether the plan is acceptable via `interaction.ask`."
    )]
    async fn set(&self, input: &PlanSetInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_set(input).await
    }

    #[tool(
        tags(mutate, planning),
        summary = "Edit the current plan's steps and checks.",
        help = "Address steps and checks by 1-based index: `step` + `status` (with an optional `note`) updates a step, `step` + `check` + `status` updates a check. This tool NEVER requests user approval and NEVER changes the plan phase — the plan stays in whatever phase it is in. Use `plan.phase` for plan-level phase transitions and `plan.review` to request approval."
    )]
    async fn edit(&self, input: &PlanEditInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_edit(input).await
    }

    #[tool(
        tags(mutate, interactive, planning),
        summary = "Transition the current plan's phase.",
        help = "Plan-level phase transitions between `planning`, `active`, `blocked`, `completed`, and `cancelled`, with optional `autorun` and (for `completed`) `summary`. Transitions into `active`, `blocked`, or `completed` request approval by default only when the current phase is not already approved (`active`, `blocked`, or `completed`). Leave `request_approval` omitted or true for normal transitions; an already approved plan does not request another review for progress or completion. Passing `request_approval: false` requires prior user authorization AND the trusted setting `agena.plan.allow_unreviewed_activation`; never change settings to bypass approval. To complete a plan with steps, mark the required steps/checks `completed` via `plan.edit` first, then call this tool separately with `phase: completed`."
    )]
    async fn phase(&self, input: &PlanPhaseInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_phase(input).await
    }

    #[tool(
        tags(mutate, interactive, planning),
        summary = "Request user approval of the current plan before it becomes active.",
        help = "Request user approval of the current saved plan; this may pause for the user. The plan.phase tool also requests review for transitions that need approval. It reviews the current saved plan and, when the user approves, moves it from `planning` to `active`. Call it after creating or refining the plan with `plan.set` / `plan.edit`. If the user leaves feedback or rejects, the plan stays in `planning` so you can revise it and propose again."
    )]
    async fn review(&self, input: &PlanReviewInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_plan_review(input).await
    }

    #[tool(tags(mutate, planning), summary = "Remove the current plan.")]
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

    #[hook(agent.cancel)]
    async fn agent_cancel(&self, input: agena_plugin_host::AgentCancelInput) -> SdkResult<()> {
        self.inner.agent_cancel_hook(input).await
    }
}
