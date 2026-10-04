//! Dynamic per-session system prompt sections.
//!
//! Agena execution tools are not declared in the model-visible function
//! protocol: the model only sees the declared Tool API gateway functions and
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

/// Keep workflow advice usable even when only part of a plugin is enabled.
fn render_planning_section(tool_names: &[String]) -> String {
    let has = |name: &str| tool_names.iter().any(|tool| tool == name);
    let mut paragraphs = vec![
        "# Planning",
        "Plan non-trivial implementation: new features, architectural choices, uncertain requirements, or coordinated changes. Skip formal planning for small clear fixes and pure research. Explore and refine the plan before requesting review.",
    ];
    if has("plan.set") {
        if has("plan.review") || has("plan.phase") {
            paragraphs.push("Prefer `plan.set` to record the plan. It returns without waiting for the user. With `request_approval: true` (default), creating or replacing a plan puts it in the planning phase and blocks mutating work.");
            if has("plan.review") {
                paragraphs.push("Call `plan.review` for approval before implementation.");
            } else {
                paragraphs.push("Use `plan.phase` with `phase: active` to request the required approval before implementation.");
            }
        } else {
            paragraphs.push("`plan.set` normally saves a plan in the planning phase, which blocks mutating work. A review tool is unavailable: keep new plans in prose unless unreviewed activation is authorized and permitted by trusted configuration. Do not create a blocking plan without a way to approve it. Existing planning-phase restrictions still apply.");
        }
    } else if has("plan.review") {
        paragraphs.push("Use `plan.review` to request approval of the current saved plan when it needs approval. It can suspend the turn until the user responds.");
    }
    if has("plan.set") || has("plan.phase") {
        paragraphs.push("Use `request_approval: false` only with prior user authorization AND trusted configuration allowing it; never change configuration to bypass review.");
    }
    if has("plan.review") || has("plan.phase") {
        paragraphs.push("Pending reviews bind to a revision. If the saved plan changes during review, inspect the current version and request review again when required; an old approval cannot authorize that changed version.");
    }
    if has("plan.edit") {
        paragraphs.push("`plan.edit` updates step/check progress and notes without changing the phase or requesting approval. Ordinary progress updates to an active plan do not require another review.");
    }
    if has("plan.phase") {
        paragraphs.push("Use `plan.phase` for phase changes and completion. For completion, finish required steps/checks first. Transitions within an already approved plan need no new review; other transitions follow its live help and current phase.");
    }
    paragraphs.join("\n\n")
}

/// Ask decision semantics injected when `agena.interaction.ask` is available.
///
/// The ask tool is named first-class in the system prompt so the model knows
/// it exists, but its contract is never embedded: the live input contract is
/// served by `tools_help`. The declared Tool API gateway functions stay the
/// only protocol surface; this section just names the tool and its decision
/// semantics, exactly like `session.rename` and `plan.set` are named directly
/// in other sections.
fn render_asking_section(has_plan_review: bool) -> String {
    let mut section = r#"# Asking the user

Use `interaction.ask` for a decision the user must make with no reasonable default, or authorization for a specific dangerous action. This includes discarding changes, rewriting Git history, and choosing whether to squash before an authorized push. Read its live help first; include at least two distinct choices and bundle related questions. Ask before a wrong assumption would cause substantial rework. Your turn suspends until answers arrive, then continue the same task. Read and repair rejected input. Reuse authorization already given for the same action and scope. Do not ask whether to proceed with already authorized work or use this tool for plan approval.

Use the tool for every question; do not end your turn with a plain-text question. Wait for its result before the dependent action. A timeout, cancellation, or empty answer is not approval: continue only independent, already-authorized work and report any remaining blocker."#
        .to_string();
    if has_plan_review {
        section.push_str(" Use `plan.review` for plan approval.");
    }
    section
}

/// Delegation decision semantics injected when the `agena.tasks` tools are
/// available: bounded independent work with retained responsibility.
fn render_delegating_section() -> String {
    r#"# Delegating work

Use `tasks.run` for bounded independent work, a suitable available command/subagent, or exploration that benefits from returning conclusions. Give concrete scope and checks, attach relevant `commands`, keep concurrency low, and verify results. Handle simple lookups yourself; do not redo delegated work or delegate your responsibility for understanding it.

Default execution waits for the task's result. Use `run_in_background: true` when other useful work can proceed; completion follows the background notification rules below."#
        .to_string()
}

/// Notification and terminal advice mentions only available execution tools.
fn render_background_section(tool_names: &[String]) -> String {
    let has = |name: &str| tool_names.iter().any(|tool| tool == name);
    let mut paragraphs = vec!["# Background execution".to_owned()];
    let launchers = ["shell.run", "tasks.run"]
        .into_iter()
        .filter(|name| has(name))
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>();
    if !launchers.is_empty() {
        paragraphs.push(format!("{} with `run_in_background: true` return a handle and later deliver a `system_notification` on completion, failure, timeout or cancellation.", launchers.join(" and ")));
    }
    if has("monitor.start") {
        paragraphs.push("`monitor.start` listens for events; each event has a sequence and arrives as a `system_notification`. A monitor can end on source exit, timeout, cancellation or session end. Do not restart it just to check for events.".to_owned());
        if has("monitor.stop") {
            paragraphs.push("Use `monitor.stop` when monitoring is no longer needed.".to_owned());
        }
    }
    if has("cron.create") {
        paragraphs.push("`cron.create` schedules wakes as `system_notification` messages at safe turn boundaries and retains the originating assistant run. Use the IANA timezone from `<environment_context>`; returned timestamps are explicit RFC 3339 instants. Jobs are session-only and expire after seven days.".to_owned());
    }
    paragraphs.push("Continue useful work while waiting. If only a future notification remains, end the current turn with the work still pending; resume from the notification even after an earlier turn ended. Waiting is not task completion: inspect the outcome and verify results before claiming success. Never poll status/logs or sleep merely to wait for completion or events. Bounded output/log reads are appropriate for a concrete diagnosis or missing result details.".to_owned());
    if has("shell.run") && has("shell.write") {
        paragraphs.push(r#"Interactive terminals use `shell.run` with `tty: true` and the returned `process_id`. Read incremental output, then send exact input with `shell.write`: `\r` is Enter, `\u0003` is Ctrl-C. Empty `chars` may perform a bounded read to observe a prompt; interactive I/O is allowed. Omit `since_seq` to consume unread output, or pass a cursor for replay. Silence/yield is not process exit. Declare subsequent effects and never resend a whole input after a partial write."#.to_owned());
        let controls = ["shell.resize", "shell.signal", "shell.stop"]
            .into_iter()
            .filter(|name| has(name))
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>();
        if !controls.is_empty() {
            paragraphs.push(format!("Available terminal lifecycle controls: {}. Read their live help for dimensions, interruption or cleanup.", controls.join(", ")));
        }
    } else if has("shell.run") {
        paragraphs.push("Use non-interactive commands when no terminal input tool is available; do not launch an interactive program that needs later keystrokes.".to_owned());
    }
    if has("shell.run") && has("shell.logs") {
        paragraphs.push("Use bounded `shell.logs` reads for diagnostics or to observe an interactive prompt, not as a completion wait loop.".to_owned());
    }
    paragraphs.join("\n\n")
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

fn workflow_sections(tool_names: &[String]) -> Vec<String> {
    let has = |name: &str| tool_names.iter().any(|tool| tool == name);
    let mut sections = Vec::new();
    if ["plan.set", "plan.review", "plan.edit", "plan.phase"]
        .into_iter()
        .any(has)
    {
        sections.push(render_planning_section(tool_names));
    }
    if has("interaction.ask") {
        sections.push(render_asking_section(has("plan.review")));
    }
    if has("tasks.run") {
        sections.push(render_delegating_section());
    }
    if wants_background_section(tool_names) {
        sections.push(render_background_section(tool_names));
    }
    sections
}

impl SessionManager {
    fn assemble_system_prompt_for_tool_names(
        &self,
        tool_names: Vec<String>,
        user_system: Option<&str>,
    ) -> String {
        let sections = workflow_sections(&tool_names);
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
        let mut base = self.assemble_system_prompt_for_tool_names(tool_names, user_system);
        if let Some(instruction) = scoped_executor.conversation_instruction() {
            base.push_str("\n\n");
            base.push_str(&instruction);
        }
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
        let mut base = self.assemble_system_prompt_for_tool_names(tool_names, user_system);
        if let Some(instruction) = scoped_executor.conversation_instruction() {
            base.push_str("\n\n");
            base.push_str(&instruction);
        }
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

    const WORKFLOW_TOOLS: &[&str] = &[
        "plan.set",
        "plan.review",
        "plan.edit",
        "plan.phase",
        "interaction.ask",
        "tasks.run",
        "shell.run",
        "shell.write",
        "shell.logs",
        "shell.resize",
        "shell.signal",
        "shell.stop",
        "monitor.start",
        "monitor.stop",
        "cron.create",
    ];

    fn names(tools: &[&str]) -> Vec<String> {
        tools.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn workflow_capability_matrix_never_advertises_unavailable_tools() {
        let cases: &[&[&str]] = &[
            &[],
            &["fs.read"],
            &["cron.list"],
            &["monitor.stop"],
            &["cron.create"],
            &["monitor.start"],
            &["monitor.start", "monitor.stop"],
            &["tasks.run"],
            &["shell.run"],
            &["shell.run", "shell.logs"],
            &["shell.run", "shell.write"],
            &["shell.run", "shell.write", "shell.resize"],
            &["shell.run", "shell.write", "shell.signal"],
            &["shell.run", "shell.write", "shell.stop"],
            &["shell.run", "shell.write", "shell.logs"],
            &["plan.set"],
            &["plan.review"],
            &["plan.edit"],
            &["plan.phase"],
            &["plan.set", "plan.review"],
            &["plan.set", "plan.phase"],
            &["interaction.ask"],
            &["interaction.ask", "plan.review"],
            WORKFLOW_TOOLS,
        ];
        for available in cases {
            let sections = workflow_sections(&names(available));
            let rendered = sections.join("\n\n");
            for tool in WORKFLOW_TOOLS {
                if !available.contains(tool) {
                    assert!(
                        !rendered.contains(tool),
                        "{tool} advertised with {available:?}"
                    );
                }
            }
            let wants_background = ["cron.create", "monitor.start", "tasks.run", "shell.run"]
                .iter()
                .any(|name| available.contains(name));
            assert_eq!(
                rendered.contains("# Background execution"),
                wants_background,
                "{available:?}"
            );
            let wants_terminal =
                available.contains(&"shell.run") && available.contains(&"shell.write");
            assert_eq!(
                rendered.contains("tty: true"),
                wants_terminal,
                "{available:?}"
            );
            assert_eq!(
                rendered.contains("# Asking the user"),
                available.contains(&"interaction.ask")
            );
            assert_eq!(
                rendered.contains("# Delegating work"),
                available.contains(&"tasks.run")
            );
            if sections.is_empty() {
                assert_eq!(
                    crate::identity::system_prompt_with_sections(&sections),
                    crate::identity::system_prompt()
                );
            }
        }
    }

    #[test]
    fn partial_plan_capabilities_keep_approval_and_progress_distinct() {
        let set_only = workflow_sections(&names(&["plan.set"])).join("\n\n");
        assert!(set_only.contains("keep new plans in prose"));
        assert!(set_only.contains("Existing planning-phase restrictions still apply"));
        assert!(!set_only.contains("plan.review"));
        let with_phase = workflow_sections(&names(&["plan.set", "plan.phase"])).join("\n\n");
        assert!(with_phase.contains("`plan.phase` with `phase: active`"));
        assert!(!with_phase.contains("review tool is unavailable"));
        assert!(!with_phase.contains("plan.review"));
        let full = workflow_sections(&names(&[
            "plan.set",
            "plan.review",
            "plan.edit",
            "plan.phase",
        ]))
        .join("\n\n");
        for rule in [
            "creating or replacing a plan puts it in the planning phase",
            "prior user authorization AND trusted configuration",
            "Pending reviews bind to a revision",
            "changes during review",
            "Ordinary progress updates to an active plan do not require another review",
            "For completion, finish required steps/checks first",
        ] {
            assert!(full.contains(rule), "planning rule: {rule}");
        }
    }

    #[test]
    fn workflow_sections_preserve_wait_and_interactive_io_obligations() {
        let sections = workflow_sections(&names(WORKFLOW_TOOLS));
        let asking = &sections[1];
        for rule in [
            "interaction.ask",
            "at least two distinct choices",
            "turn suspends",
            "already authorized work",
            "plan.review",
            "specific dangerous action",
            "same action and scope",
            "Use the tool for every question",
            "A timeout, cancellation, or empty answer is not approval",
        ] {
            assert!(asking.contains(rule), "asking rule: {rule}");
        }
        let delegation = &sections[2];
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
        let background = &sections[3];
        for rule in [
            "system_notification",
            "Never poll",
            "end the current turn with the work still pending",
            "resume from the notification",
            "Waiting is not task completion",
            "concrete diagnosis",
            "each event has a sequence",
            "source exit",
            "IANA timezone",
            "RFC 3339",
            "originating assistant run",
            "seven days",
            "since_seq",
            "partial write",
            "Silence/yield is not process exit",
            "interactive I/O is allowed",
        ] {
            assert!(background.contains(rule), "background rule: {rule}");
        }
        assert!(background.contains(r#"`\r` is Enter"#));
        assert!(background.contains(r#"`\u0003` is Ctrl-C"#));
    }

    #[test]
    fn assembled_prompt_budget_and_heading_layout_are_bounded() {
        let sections = workflow_sections(&names(WORKFLOW_TOOLS));
        let prompt = crate::identity::system_prompt_with_sections(&sections);
        // Budget necessary decision guidance; schemas remain in live help.
        assert!(
            prompt.len() <= 12_000 + crate::identity::AGENA_GIT_WORKFLOW_PROMPT.trim().len() + 2,
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
        assert_eq!(sections.len(), 4);
        let mut position = 0;
        for heading in [
            "# Planning",
            "# Asking the user",
            "# Delegating work",
            "# Background execution",
            "# Git and file recovery",
            "# Communication and delivery",
        ] {
            let next = prompt.find(heading).unwrap();
            assert!(next > position, "out-of-order section {heading}");
            position = next;
        }
        println!(
            "prompt bytes: base={}, all_workflows={}",
            crate::identity::system_prompt().len(),
            prompt.len()
        );
    }
}
