//! Dynamic per-session system prompt sections.
//!
//! Agena execution tools are not declared in the model-visible function
//! protocol: the model only sees the five `agena.tools` gateway functions and
//! discovers execution tools through them. Workflow-relevant decision
//! semantics therefore cannot live only inside tool descriptions; they must
//! also be injected into the system prompt. This module detects which
//! workflow tools are actually available for a session and renders the
//! matching sections, which `crate::identity::system_prompt_with_sections`
//! inserts immediately after `# Plan, ask, and delegate`.
//!
//! The sections mirror Claude Code decision semantics: a strong default
//! toward planning and delegation with explicit exceptions, rather than a
//! neutral when-to-use list, because models respond more reliably to a
//! prefer-unless-simple anchor.
//!
//! Environment facts are intentionally not injected here: they are served on
//! demand by the `context.environment` tool because they can change
//! mid-session.

use crate::session::model::Session;
use crate::tool::ToolExecutor;
use crate::tool::tool_registry::compact_tool_call_name;

use super::SessionManager;
use super::merge_system_prompts;

/// Plan decision semantics injected when the `agena.plan` tools are available.
pub(crate) fn render_planning_section() -> String {
    r#"# Planning

Prefer using `plan.set` for implementation tasks unless they are simple. Use it proactively when starting a non-trivial implementation task: getting sign-off on your approach before writing code prevents wasted effort and ensures alignment. Use it when ANY of these conditions apply:
- New feature implementation
- Multiple valid approaches exist
- Changes affect existing behavior or structure
- An architectural decision is needed
- The change will likely touch more than 2-3 files
- Requirements are unclear and need exploration
- You would otherwise ask the user to clarify the approach — use `plan.set` instead

Only skip planning for simple tasks: single-line or few-line fixes, adding a single function with clear requirements, tasks with very specific detailed instructions, or pure research/read-only work. If unsure whether to plan, err on the side of planning.

`plan.set` never blocks on the user: it saves the plan and returns. With `request_approval: true` (the default) the plan stays in the `planning` phase and you must call `plan.review` to request user approval before it becomes active. Pass `request_approval: false` to `plan.set` or `plan.phase` only when the user has already declared that the plan or the change needs no approval — never default to it.

While the current plan is in the `planning` phase, mutating tools are blocked by the runtime. Explore with read-only tools — delegating parallel exploration to `tasks.run` when the scope spans multiple areas — clarify requirements with `ask`, and refine the plan with `plan.edit` (which never requests approval and never changes phase). When the plan is complete, call `plan.review` to request user approval; never ask whether the plan is acceptable via `ask`."#
        .to_string()
}

/// Ask decision semantics injected when `agena.interaction.ask` is available.
pub(crate) fn render_asking_section() -> String {
    r#"# Asking the user

Use `ask` only when you are blocked on a decision that is genuinely the user's to make: a preference, a direction choice, or a decision with no reasonable default. If a sensible default exists or you can verify the answer yourself, proceed instead of asking. When you do ask, ask all necessary clarifying questions at once. Never use `ask` to ask whether you should proceed or to seek plan approval."#
        .to_string()
}

/// Delegation decision semantics injected when the `agena.tasks` tools are
/// available: an active trigger paired with restraint, mirroring Claude Code.
pub(crate) fn render_delegating_section() -> String {
    r#"# Delegating work

Reach for `tasks.run` when the work matches an available Skill or subagent type, when you have independent work to run in parallel, or when answering would mean reading across several files — delegate it and you keep the conclusion, not the file dumps. Attach `skills` that match the task (for example an explore skill for exploration, a read-only review skill for review). For a single-fact lookup where you already know the file, symbol, or value, search directly. Once you have delegated a search, do not also run it yourself — wait for the result.

Do small tasks yourself instead of delegating them; do not fan out a single task into many subtasks; verify inline instead of delegating when you can; keep the number of concurrent subtasks low. Never delegate understanding: brief the subagent with concrete file paths, line numbers, and what to change, then check its result."#
        .to_string()
}

impl SessionManager {
    fn assemble_system_prompt_for_tool_names(
        &self,
        tool_names: Vec<String>,
        user_system: Option<&str>,
    ) -> String {
        let has_plan = tool_names.iter().any(|name| name == "plan.set");
        let has_ask = tool_names.iter().any(|name| name == "interaction.ask");
        let has_tasks = tool_names.iter().any(|name| name == "tasks.run");

        let mut sections = Vec::new();
        if has_plan {
            sections.push(render_planning_section());
        }
        if has_ask {
            sections.push(render_asking_section());
        }
        if has_tasks {
            sections.push(render_delegating_section());
        }

        let base = crate::identity::system_prompt_with_sections(&sections);
        merge_system_prompts(Some(base.as_str()), user_system).unwrap_or(base)
    }

    /// Render prompt sections from a scoped executor whose definition
    /// snapshot has already been captured. Status/usage code can use this to
    /// avoid rebuilding the plugin catalog once for the prompt and again for
    /// tool bindings.
    pub(crate) fn assemble_session_system_prompt_with_executor(
        &self,
        scoped_executor: &ToolExecutor,
        user_system: Option<&str>,
    ) -> String {
        let tool_names = scoped_executor
            .available_execution_tools()
            .into_iter()
            .map(|tool| compact_tool_call_name(&tool.canonical_name()))
            .collect::<Vec<_>>();
        self.assemble_system_prompt_for_tool_names(tool_names, user_system)
    }

    /// Async catalog path used by model turns and other Tokio request flows.
    /// Definition hooks are awaited directly instead of entering the legacy
    /// synchronous plugin-host bridge.
    pub(crate) async fn assemble_session_system_prompt_async(
        &self,
        session: &Session,
        user_system: Option<&str>,
    ) -> String {
        let state = self.execution_state();
        let scoped_executor = state
            .tool_executor
            .for_session_context_async(&session.runtime.execution)
            .await;
        let tool_names = scoped_executor
            .available_execution_tools_async()
            .await
            .into_iter()
            .map(|tool| compact_tool_call_name(&tool.canonical_name()))
            .collect::<Vec<_>>();
        self.assemble_system_prompt_for_tool_names(tool_names, user_system)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_section_anchors_on_prefer_unless_simple() {
        let section = render_planning_section();
        assert!(section.contains("# Planning"));
        assert!(section.contains("Prefer using `plan.set`"));
        assert!(section.contains("unless they are simple"));
        assert!(section.contains("err on the side of planning"));
        assert!(section.contains("use `plan.set` instead"));
        assert!(section.contains("never ask whether the plan is acceptable"));
        assert!(section.contains("request_approval"));
        assert!(section.contains("has already declared"));
    }

    #[test]
    fn asking_section_carries_red_line() {
        let section = render_asking_section();
        assert!(section.contains("# Asking the user"));
        assert!(section.contains("genuinely the user's to make"));
        assert!(section.contains("Never use `ask`"));
    }

    #[test]
    fn delegating_section_reaches_for_parallel_work_and_keeps_conclusions() {
        let section = render_delegating_section();
        assert!(section.contains("# Delegating work"));
        assert!(section.contains("Reach for `tasks.run`"));
        assert!(section.contains("keep the conclusion, not the file dumps"));
        assert!(section.contains("wait for the result"));
        assert!(section.contains("Do small tasks yourself"));
        assert!(section.contains("Never delegate understanding"));
    }
}
