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
//! Environment facts are intentionally not injected here: they are served on
//! demand by the `context.environment` tool because they can change
//! mid-session.

use crate::session::model::Session;
use crate::tool::tool_registry::compact_tool_call_name;

use super::merge_system_prompts;
use super::SessionManager;

/// Plan decision semantics injected when the `agena.plan` tools are available.
pub(crate) fn render_planning_section() -> String {
    r#"# Planning

Use `plan.set` for non-trivial implementation work: new features, multiple viable approaches, changing existing behavior or structure, architectural decisions, changes touching 2-3 or more files, unclear requirements, or when user preference affects the direction. Skip planning for single-line fixes, clearly scoped single-function changes, or pure research/read-only work.

While the current plan is in the `planning` phase, mutating tools are blocked by the runtime. Explore with read-only tools, clarify requirements with `ask`, and refine the plan. When the plan is complete, present it for approval through the plan phase transition; never ask whether the plan is acceptable via `ask`."#
        .to_string()
}

/// Ask decision semantics injected when `agena.interaction.ask` is available.
pub(crate) fn render_asking_section() -> String {
    r#"# Asking the user

Use `ask` only when you are blocked on a decision that is genuinely the user's to make: a preference, a direction choice, or a decision with no reasonable default. If a sensible default exists or you can verify the answer yourself, proceed instead of asking. When you do ask, ask all necessary clarifying questions at once. Never use `ask` to ask whether you should proceed or to seek plan approval."#
        .to_string()
}

/// Delegation restraint semantics injected when the `agena.tasks` tools are available.
pub(crate) fn render_delegating_section() -> String {
    r#"# Delegating work

Use `tasks.run` only for work that is genuinely parallel, independent, or read-heavy across many files. Do small tasks yourself instead of delegating them; do not fan out a single task into many subtasks; verify inline instead of delegating when you can; do not redo work you already delegated; keep the number of concurrent subtasks low."#
        .to_string()
}

impl SessionManager {
    /// Assemble the full system prompt for one session: the static identity
    /// with workflow sections injected after `# Plan, ask, and delegate` only
    /// when the corresponding execution tools are actually available. The
    /// caller-supplied `user_system` (if any) is appended last so it keeps
    /// precedence. Environment facts are queried on demand via
    /// `context.environment`, not injected here.
    pub(crate) fn assemble_session_system_prompt(
        &self,
        session: &Session,
        user_system: Option<&str>,
    ) -> String {
        let state = self.execution_state();
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
        let tool_names = scoped_executor
            .available_execution_tools()
            .into_iter()
            .map(|tool| compact_tool_call_name(&tool.canonical_name()))
            .collect::<Vec<_>>();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_section_carries_decision_criteria() {
        let section = render_planning_section();
        assert!(section.contains("# Planning"));
        assert!(section.contains("plan.set"));
        assert!(section.contains("never ask whether the plan is acceptable"));
    }

    #[test]
    fn asking_section_carries_red_line() {
        let section = render_asking_section();
        assert!(section.contains("# Asking the user"));
        assert!(section.contains("genuinely the user's to make"));
        assert!(section.contains("Never use `ask`"));
    }

    #[test]
    fn delegating_section_carries_restraint() {
        let section = render_delegating_section();
        assert!(section.contains("# Delegating work"));
        assert!(section.contains("Do small tasks yourself"));
    }
}
