//! Dynamic per-session system prompt sections.
//!
//! Agena execution tools are not declared in the model-visible function
//! protocol: the model only sees the five `agena.tools` gateway functions and
//! discovers execution tools through them. Workflow-relevant decision
//! semantics therefore cannot live only inside tool descriptions; they must
//! also be injected into the system prompt. This module renders those dynamic
//! sections from live session facts and available-tool detection, mirroring
//! the conditional prompt-segment pattern Claude Code uses.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::session::model::Session;
use crate::tool::tool_registry::compact_tool_call_name;

use super::merge_system_prompts;
use super::SessionManager;

/// How long cached git facts stay valid before a refresh.
const GIT_FACTS_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GitFacts {
    pub branch: Option<String>,
    pub short_sha: Option<String>,
    pub dirty: bool,
}

static GIT_CACHE: OnceLock<Mutex<HashMap<PathBuf, (Instant, Option<GitFacts>)>>> =
    OnceLock::new();

fn git_cache() -> &'static Mutex<HashMap<PathBuf, (Instant, Option<GitFacts>)>> {
    GIT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn run_git(workspace: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Resolve git branch/sha/dirty facts for a workspace, cached per workspace
/// for a short TTL so repeated prompt assembly stays cheap and stable.
pub(crate) fn git_facts(workspace: &Path) -> Option<GitFacts> {
    {
        let cache = git_cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((cached_at, facts)) = cache.get(workspace) {
            if cached_at.elapsed() < GIT_FACTS_TTL {
                return facts.clone();
            }
        }
    }
    let facts = run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|branch| {
        let short_sha = run_git(workspace, &["rev-parse", "--short", "HEAD"]);
        let dirty = run_git(workspace, &["status", "--porcelain"])
            .map(|status| !status.trim().is_empty())
            .unwrap_or(false);
        GitFacts {
            branch: Some(branch),
            short_sha,
            dirty,
        }
    });
    let mut cache = git_cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(workspace.to_path_buf(), (Instant::now(), facts.clone()));
    facts
}

/// Shell the session should assume for shell commands, preferring the
/// `SHELL` environment variable and falling back to a platform default.
pub(crate) fn shell_for_session() -> Option<String> {
    std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Render the `# Environment` block Claude-Code-style: workspace, git state,
/// shell, OS, and session identity, so the model does not need to discover
/// these facts with extra tool calls.
pub(crate) fn render_environment_block(
    workspace: Option<&Path>,
    shell: Option<&str>,
    session_id: i64,
    is_subagent: bool,
) -> String {
    let mut lines = vec!["# Environment".to_string()];
    if let Some(workspace) = workspace {
        lines.push(format!("- Working directory: {}", workspace.display()));
        if let Some(facts) = git_facts(workspace) {
            if let (Some(branch), Some(short_sha)) = (facts.branch.as_deref(), facts.short_sha.as_deref())
            {
                let dirty = if facts.dirty { " (dirty)" } else { "" };
                lines.push(format!("- Git: {branch} @ {short_sha}{dirty}"));
            } else if let Some(branch) = facts.branch.as_deref() {
                lines.push(format!("- Git branch: {branch}"));
            }
        }
    }
    let shell = shell
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(if cfg!(windows) { "powershell" } else { "/bin/bash" });
    lines.push(format!("- Shell: {shell}"));
    lines.push(format!(
        "- OS: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    if is_subagent {
        lines.push(format!("- Session: subagent {session_id}"));
    } else {
        lines.push(format!("- Session: {session_id}"));
    }
    lines.join("\n")
}

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

/// Append dynamic sections to a base system prompt, each separated by a blank
/// line, skipping empty sections.
pub(crate) fn assemble_dynamic_sections(base: &str, sections: &[String]) -> String {
    let mut prompt = base.trim_end().to_string();
    for section in sections {
        let section = section.trim();
        if section.is_empty() {
            continue;
        }
        prompt.push_str("\n\n");
        prompt.push_str(section);
    }
    prompt
}

impl SessionManager {
    /// Assemble the full system prompt for one session: the static identity,
    /// the dynamic `# Environment` block, and workflow sections injected only
    /// when the corresponding execution tools are actually available. The
    /// caller-supplied `user_system` (if any) is appended last so it keeps
    /// precedence.
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

        let workspace = session
            .runtime
            .execution
            .effective_workspace_root
            .clone()
            .or_else(|| Some(state.tool_executor.workspace_root().to_path_buf()));

        let mut sections = Vec::new();
        sections.push(render_environment_block(
            workspace.as_deref(),
            shell_for_session().as_deref(),
            session.id,
            session.is_subagent(),
        ));
        if has_plan {
            sections.push(render_planning_section());
        }
        if has_ask {
            sections.push(render_asking_section());
        }
        if has_tasks {
            sections.push(render_delegating_section());
        }

        let base = crate::identity::system_prompt();
        let assembled = assemble_dynamic_sections(&base, &sections);
        merge_system_prompts(Some(assembled.as_str()), user_system).unwrap_or(assembled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_block_lists_workspace_shell_os_and_session() {
        let block = render_environment_block(
            Some(Path::new("/tmp/example-ws")),
            Some("/bin/zsh"),
            42,
            false,
        );
        assert!(block.contains("# Environment"));
        assert!(block.contains("/tmp/example-ws"));
        assert!(block.contains("Shell: /bin/zsh"));
        assert!(block.contains("Session: 42"));
        assert!(!block.contains("subagent"));
    }

    #[test]
    fn environment_block_marks_subagent_session() {
        let block = render_environment_block(None, None, 9, true);
        assert!(block.contains("Session: subagent 9"));
    }

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

    #[test]
    fn assembly_joins_sections_with_blank_lines() {
        let assembled = assemble_dynamic_sections(
            "base",
            &["# One".to_string(), "# Two".to_string(), String::new()],
        );
        assert_eq!(assembled, "base\n\n# One\n\n# Two");
    }
}
