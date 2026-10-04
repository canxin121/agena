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
//! Each section preserves the corresponding tool workflow while keeping
//! schemas and detailed usage in live tool help.
//!
//! Environment facts are intentionally not injected here: they are served on
//! demand by the `session.environment` tool because they can change
//! mid-session.

use crate::session::model::Session;
use crate::tool::ToolExecutor;
use crate::tool::tool_registry::compact_tool_call_name;

use super::SessionManager;
use super::merge_system_prompts;

/// Plan decision semantics injected when the `agena.plan` tools are available.
pub(crate) fn render_planning_section() -> String {
    r#"# Planning

Prefer `plan.set` for non-trivial implementation: new features, architectural choices, uncertain requirements, or coordinated changes. Skip it for small clear fixes and pure research. Explore and refine the plan before requesting review.

`plan.set` records a plan without blocking. With `request_approval: true` (default), mutating tools remain blocked in the planning phase; call `plan.review` for approval. Use `request_approval: false` only with prior user authorization AND trusted configuration allowing it. Reviews bind to a revision; review again after changing an approved plan. Use `plan.edit` for edits, not approval requests."#
        .to_string()
}

/// Ask decision semantics injected when `agena.interaction.ask` is available.
///
/// The ask tool is named first-class in the system prompt so the model knows
/// it exists, but its contract is never embedded: the live input contract is
/// served by `tools_help`. The five `agena.tools` gateway functions stay the
/// only protocol surface; this section just names the tool and its decision
/// semantics, exactly like `session.rename` and `plan.set` are named directly
/// in other sections.
pub(crate) fn render_asking_section() -> String {
    r#"# Asking the user

Use `interaction.ask` only for a decision the user must make with no reasonable default. Read its live help first; include at least two distinct choices and bundle related questions. Your turn suspends until answers arrive, then continue the same task. Read and repair rejected input. Do not ask whether to proceed with already authorized work or use this tool for plan approval (`plan.review`)."#
        .to_string()
}

/// Delegation decision semantics injected when the `agena.tasks` tools are
/// available: bounded independent work with retained responsibility.
pub(crate) fn render_delegating_section() -> String {
    r#"# Delegating work

Use `tasks.run` for bounded independent work, a suitable available command/subagent, or exploration that benefits from returning conclusions. Give concrete scope and checks, attach relevant `commands`, keep concurrency low, and verify results. Handle simple lookups yourself; do not redo delegated work or delegate your responsibility for understanding it.

Default execution waits for the task's result. Use `run_in_background: true` when other useful work can proceed; completion follows the background notification rules below."#
        .to_string()
}

/// Background-execution discipline injected when any tool that can launch
/// background work is available (`shell.run` and friends, `tasks.run`,
/// `monitor.start`): a background launch returns immediately, the session is
/// *notified* when the work settles (the `system_notification` part), and the
/// model must consume the notification instead of polling for completion.
pub(crate) fn render_background_section() -> String {
    r#"# Background execution

`shell.run` and `tasks.run` with `run_in_background: true` return a handle and later deliver a `system_notification` on completion, failure, timeout or cancellation. Continue useful work while waiting; incorporate notifications even after an earlier turn ended. Never poll status/logs merely to wait for completion.

`monitor.start` is continuous: each event has a sequence and arrives as a notification. Do not restart it, poll or sleep waiting for events. `cron.create` schedules wakes at safe turn boundaries and retains the originating assistant run. Use the IANA timezone from `<environment_context>`; returned timestamps are explicit RFC 3339 instants. Jobs are session-only and expire after seven days.

Interactive terminals use `shell.run(tty: true)` and the returned `process_id`. Read incremental output, then send exact input with `shell.write`: `\r` is Enter, `\u0003` is Ctrl-C. Empty input or bounded `shell.logs` reads may observe a prompt. Omit `since_seq` to consume unread output, or pass a cursor for replay. Silence/yield is not process exit. Use resize/signal/stop for lifecycle control, declare subsequent effects, and never resend a whole input after a partial write."#
        .to_string()
}

/// Whether the available tool set can launch background work, so the
/// `# Background execution` discipline section must be injected. Covers
/// `shell.run` and friends, `tasks.run`, the continuous `monitor.start`, and
/// `cron.create` (a scheduled job fires later and wakes the session again).
fn wants_background_section(tool_names: &[String]) -> bool {
    let has_tasks = tool_names.iter().any(|name| name == "tasks.run");
    let has_monitor = tool_names.iter().any(|name| name == "monitor.start");
    let has_cron = tool_names.iter().any(|name| name == "cron.create");
    let has_shell = tool_names.iter().any(|name| name == "shell.run");
    has_shell || has_tasks || has_monitor || has_cron
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
        if wants_background_section(&tool_names) {
            sections.push(render_background_section());
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
        let base = self.assemble_system_prompt_for_tool_names(tool_names, user_system);
        let guidance = scoped_executor.project_instruction_section(&[]);
        if guidance.is_empty() {
            base
        } else {
            format!("{base}\n\n{guidance}")
        }
    }

    /// Async catalog path used by model turns and other Tokio request flows.
    /// Definition hooks are awaited directly through the current plugin host.
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
        let base = self.assemble_system_prompt_for_tool_names(tool_names, user_system);
        let guidance = scoped_executor.project_instruction_section(&[]);
        if guidance.is_empty() {
            base
        } else {
            format!("{base}\n\n{guidance}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_workflow_sections_preserve_required_behavior() {
        let planning = render_planning_section();
        for rule in [
            "plan.set",
            "pure research",
            "plan.review",
            "prior user authorization AND trusted configuration",
            "mutating tools remain blocked",
            "revision",
            "plan.edit",
        ] {
            assert!(planning.contains(rule), "planning rule: {rule}");
        }
        let asking = render_asking_section();
        for rule in [
            "interaction.ask",
            "at least two distinct choices",
            "turn suspends",
            "already authorized work",
            "plan.review",
        ] {
            assert!(asking.contains(rule), "asking rule: {rule}");
        }
        let delegation = render_delegating_section();
        for rule in [
            "tasks.run",
            "bounded independent work",
            "commands",
            "verify results",
            "do not redo delegated work",
            "waits for the task",
            "run_in_background: true",
        ] {
            assert!(delegation.contains(rule), "delegation rule: {rule}");
        }
        let background = render_background_section();
        for rule in [
            "system_notification",
            "Never poll",
            "each event has a sequence",
            "IANA timezone",
            "RFC 3339",
            "originating assistant run",
            "seven days",
            "since_seq",
            "partial write",
            "Silence/yield is not process exit",
        ] {
            assert!(background.contains(rule), "background rule: {rule}");
        }
        assert!(background.contains("`\\r` is Enter"));
        assert!(background.contains("`\\u0003` is Ctrl-C"));
    }

    #[test]
    fn assembled_prompt_budget_and_heading_layout_are_bounded() {
        let sections = vec![
            render_planning_section(),
            render_asking_section(),
            render_delegating_section(),
            render_background_section(),
        ];
        let prompt = crate::identity::system_prompt_with_sections(&sections);
        assert!(
            prompt.len() <= 9_000,
            "assembled prompt grew to {} bytes",
            prompt.len()
        );
        assert!(!prompt.contains("\n\n\n"));
        let mut headings = std::collections::HashSet::new();
        let lines = prompt.lines().collect::<Vec<_>>();
        for (index, line) in lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with("# "))
        {
            assert!(headings.insert(*line), "duplicate heading: {line}");
            assert!(index == 0 || lines[index - 1].is_empty());
            assert!(lines[index + 1].is_empty());
        }
    }

    #[test]
    fn background_guidance_tracks_launch_capabilities() {
        for name in ["monitor.start", "cron.create", "shell.run", "tasks.run"] {
            assert!(wants_background_section(&[name.to_owned()]), "{name}");
        }
        for name in ["monitor.stop", "cron.list", "cron.history", "fs.read"] {
            assert!(!wants_background_section(&[name.to_owned()]), "{name}");
        }
    }
}
