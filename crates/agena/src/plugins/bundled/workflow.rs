//! First-party `agena.workflow` plugin: orchestration tools (task, tool_search,
//! todo_write, ask_user, enter_plan_mode, exit_plan_mode, enter_worktree,
//! exit_worktree).

use crate::message::{
    AskUserToolInput, EnterPlanModeToolInput, EnterWorktreeToolInput, ExitPlanModeToolInput,
    ExitWorktreeToolInput, TaskToolInput, TodoWriteToolInput, ToolSearchToolInput,
};
use crate::plugin::sdk::{
    EntryBehavior as SdkEntryBehavior, HostCapability, PlanModePolicy, PluginEntryDecl,
};
use crate::plugins::bundled::router::BuiltinRouterPlugin;

pub(crate) const WORKFLOW_PLUGIN_ID: &str = "agena.workflow";

pub(crate) fn new_plugin() -> BuiltinRouterPlugin {
    BuiltinRouterPlugin::new(
        "agena-workflow",
        "Workflow orchestration tools bridged to the built-in implementations.",
        entries(),
    )
}

fn entries() -> Vec<PluginEntryDecl> {
    vec![
        PluginEntryDecl::new(
            "task",
            crate::entry::definition::json_schema_for::<TaskToolInput>(),
        )
        .description(
            "Create or resume a typed subagent task session for explore, implement, or verify delegated work.",
        )
        .behavior(SdkEntryBehavior::Task)
        .search_terms(["delegate", "subagent", "parallel work"])
        .deferred_load()
        .host_capability(HostCapability::SpawnSubtask),
        PluginEntryDecl::new(
            "tool_search",
            crate::entry::definition::json_schema_for::<ToolSearchToolInput>(),
        )
        .description("Search the tool catalog and optionally load deferred tools for later turns.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["discover tools", "load tools", "find capability"])
        .always_load()
        .host_capability(HostCapability::ListTools),
        PluginEntryDecl::new(
            "todo_write",
            crate::entry::definition::json_schema_for::<TodoWriteToolInput>(),
        )
        .description("Replace the session todo list with a short execution plan and updated statuses.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["plan", "todo", "track progress"])
        .always_load(),
        PluginEntryDecl::new(
            "ask_user",
            crate::entry::definition::json_schema_for::<AskUserToolInput>(),
        )
        .description("Ask short questions and wait for answers.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms([
            "ask user",
            "clarify requirement",
            "human input",
            "single select",
            "multi select",
            "custom answer",
            "request user input",
        ])
        .always_load()
        .concurrency_safe(false)
        .requires_user_interaction(true)
        .host_capability(HostCapability::AskUser),
        PluginEntryDecl::new(
            "enter_plan_mode",
            crate::entry::definition::json_schema_for::<EnterPlanModeToolInput>(),
        )
        .description(
            "Enter plan mode. Allocates a fresh plan markdown file under .agena/plans/, blocks mutating tools, and asks the LLM to draft a plan. Pair with `exit_plan_mode` once the plan is complete.",
        )
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["plan", "design", "approach", "outline"])
        .always_load()
        .plan_mode_policy(PlanModePolicy::Allowed)
        .host_capability(HostCapability::PlanRegistry),
        PluginEntryDecl::new(
            "exit_plan_mode",
            crate::entry::definition::json_schema_for::<ExitPlanModeToolInput>(),
        )
        .description(
            "Leave plan mode and return to normal tool execution. Surfaces a permission ask so the human can review the plan before approving the unblock.",
        )
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["plan", "approve", "exit"])
        .always_load()
        .plan_mode_policy(PlanModePolicy::Allowed)
        .host_capability(HostCapability::PlanRegistry),
        PluginEntryDecl::new(
            "enter_worktree",
            crate::entry::definition::json_schema_for::<EnterWorktreeToolInput>(),
        )
        .description(
            "Create or attach to a git worktree under .agena/worktrees and switch the session into it.",
        )
        .behavior(SdkEntryBehavior::WriteSandboxed)
        .search_terms(["git", "worktree", "branch", "isolate"])
        .deferred_load()
        .host_capability(HostCapability::WorktreeRegistry),
        PluginEntryDecl::new(
            "exit_worktree",
            crate::entry::definition::json_schema_for::<ExitWorktreeToolInput>(),
        )
        .description(
            "Leave the current worktree. action=keep preserves the worktree, action=remove deletes it (refuses unless discard_changes=true when there are uncommitted changes).",
        )
        .behavior(SdkEntryBehavior::WriteSandboxed)
        .search_terms(["git", "worktree", "exit", "cleanup"])
        .deferred_load()
        .host_capability(HostCapability::WorktreeRegistry),
    ]
}
