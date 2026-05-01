use crate::agent::Agent;
use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, BashToolInput, BuiltinToolInput, CronCreateToolInput,
    CronDeleteToolInput, CronListToolInput, EnterPlanModeToolInput, EnterWorktreeToolInput,
    ExitPlanModeToolInput, ExitWorktreeToolInput, GlobToolInput, GrepToolInput, MonitorToolInput,
    NotebookEditToolInput, PowerShellToolInput, ReadToolInput, ScheduleWakeupToolInput,
    SkillRunToolInput, TaskToolInput, TodoWriteToolInput, ToolSearchToolInput, ViewFileToolInput,
    WebFetchToolInput, WebSearchToolInput,
};

use super::{ToolBehavior, ToolDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelToolProfile {
    Full,
    ReadOnly,
    NoTask,
}

impl ModelToolProfile {
    pub fn infer(model_id: Option<&str>) -> Self {
        let Some(model_id) = model_id else {
            return Self::Full;
        };
        let lowered = model_id.to_ascii_lowercase();
        if lowered.contains("readonly") || lowered.contains("read_only") {
            return Self::ReadOnly;
        }
        if lowered.contains("no-task") || lowered.contains("chat") {
            return Self::NoTask;
        }
        Self::Full
    }
}

#[derive(Debug, Clone)]
pub struct ToolAvailability {
    pub tool_name: &'static str,
    pub enabled: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ToolCatalog {
    profile: ModelToolProfile,
}

impl ToolCatalog {
    pub fn for_model(model_id: Option<&str>) -> Self {
        Self {
            profile: ModelToolProfile::infer(model_id),
        }
    }

    pub fn availability_for_input(
        &self,
        agent: &Agent,
        input: &BuiltinToolInput,
    ) -> ToolAvailability {
        let tool_name = crate::permission::builtin_name(input);
        let enabled = self.is_enabled(tool_name);
        let reason = if agent.disable {
            format!("agent '{}' is disabled", agent.name)
        } else if enabled {
            format!("tool '{tool_name}' enabled for {:?} profile", self.profile)
        } else {
            format!("tool '{tool_name}' disabled for {:?} profile", self.profile)
        };
        ToolAvailability {
            tool_name,
            enabled: enabled && !agent.disable,
            reason,
        }
    }

    pub fn builtin_definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = vec![
            ToolDefinition::builtin::<BashToolInput>(
                "bash",
                "Execute a shell command inside the sandboxed workspace.",
                ToolBehavior::Mutating,
            )
            .with_search_terms(["shell", "terminal", "command", "script"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<ReadToolInput>(
                "read",
                "Read a UTF-8 text file or list a directory with optional pagination.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["open file", "view file", "cat", "inspect"])
            .with_always_load(),
            ToolDefinition::builtin::<ViewFileToolInput>(
                "view_file",
                "Load a local file and attach it back to the conversation as inline multimodal input.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["file", "image", "pdf", "audio", "document"])
            .with_always_load(),
            ToolDefinition::builtin::<ApplyPatchToolInput>(
                "apply_patch",
                "Apply a structured patch that can add, update, move, or delete files.",
                ToolBehavior::Mutating,
            )
            .with_search_terms(["patch", "diff", "multi-file edit"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<GlobToolInput>(
                "glob",
                "Search files by glob pattern from the workspace or a subdirectory.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["find files", "list files", "pattern search"])
            .with_always_load(),
            ToolDefinition::builtin::<GrepToolInput>(
                "grep",
                "Search file contents by regex pattern with optional include glob.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["search text", "regex search", "ripgrep"])
            .with_always_load(),
            ToolDefinition::builtin::<TaskToolInput>(
                "task",
                "Create or resume a typed subagent task session for explore, implement, or verify delegated work.",
                ToolBehavior::Task,
            )
            .with_search_terms(["delegate", "subagent", "parallel work"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<ToolSearchToolInput>(
                "tool_search",
                "Search the tool catalog and optionally load deferred tools for later turns.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["discover tools", "load tools", "find capability"])
            .with_always_load(),
            ToolDefinition::builtin::<TodoWriteToolInput>(
                "todo_write",
                "Replace the session todo list with a short execution plan and updated statuses.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["plan", "todo", "track progress"])
            .with_always_load(),
            ToolDefinition::builtin::<AskUserToolInput>(
                "ask_user",
                "Ask short questions and wait for answers.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms([
                "ask user",
                "clarify requirement",
                "human input",
                "single select",
                "multi select",
                "custom answer",
                "request user input",
            ])
            .with_concurrency_safe(false)
            .with_requires_user_interaction(true)
            .with_always_load(),
            ToolDefinition::builtin::<MonitorToolInput>(
                "monitor",
                "Run a long-lived shell command in the background and stream its stdout/stderr as numbered events. \
                 Actions: start (spawn), list (enumerate), read (pull events with optional blocking wait), stop (kill).",
                ToolBehavior::Mutating,
            )
            .with_search_terms([
                "monitor",
                "background process",
                "long running",
                "watch logs",
                "tail",
                "follow",
                "stream output",
            ])
            .with_deferred_loading(),
            ToolDefinition::builtin::<WebFetchToolInput>(
                "web_fetch",
                "Fetch a URL and return its content as Markdown. HTTP is upgraded to HTTPS; \
                 cached for 15 minutes.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["web", "fetch", "download", "url", "http", "page"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<WebSearchToolInput>(
                "web_search",
                "Search the web. Backend selectable in config (tavily, exa, brave, or \
                 duckduckgo_html as zero-config default).",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["web", "search", "google", "ddg", "find online"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<EnterPlanModeToolInput>(
                "enter_plan_mode",
                "Enter plan mode. Allocates a fresh plan markdown file under \
                 .agena/plans/, blocks mutating tools, and asks the LLM to draft a \
                 plan.  Pair with `exit_plan_mode` once the plan is complete.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["plan", "design", "approach", "outline"])
            .with_always_load(),
            ToolDefinition::builtin::<ExitPlanModeToolInput>(
                "exit_plan_mode",
                "Leave plan mode and return to normal tool execution.  Surfaces a \
                 permission ask so the human can review the plan before approving \
                 the unblock.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["plan", "approve", "exit"])
            .with_always_load(),
            ToolDefinition::builtin::<SkillRunToolInput>(
                "skill_run",
                "Run a discovered or bundled skill by name. Returns the skill's \
                 system body so the model can follow it on subsequent turns.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["skill", "workflow", "macro", "preset"])
            .with_always_load(),
            ToolDefinition::builtin::<EnterWorktreeToolInput>(
                "enter_worktree",
                "Create or attach to a git worktree under .agena/worktrees and \
                 switch the session into it.",
                ToolBehavior::Mutating,
            )
            .with_search_terms(["git", "worktree", "branch", "isolate"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<ExitWorktreeToolInput>(
                "exit_worktree",
                "Leave the current worktree.  action=keep preserves the worktree, \
                 action=remove deletes it (refuses unless discard_changes=true \
                 when there are uncommitted changes).",
                ToolBehavior::Mutating,
            )
            .with_search_terms(["git", "worktree", "exit", "cleanup"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<CronCreateToolInput>(
                "cron_create",
                "Schedule a recurring prompt with a 6-field cron expression.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["cron", "schedule", "recurring", "background"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<CronListToolInput>(
                "cron_list",
                "List all currently scheduled cron jobs and one-shot wakeups.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["cron", "list", "scheduled jobs"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<CronDeleteToolInput>(
                "cron_delete",
                "Delete a scheduled job by id.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["cron", "delete", "remove", "cancel"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<ScheduleWakeupToolInput>(
                "schedule_wakeup",
                "Schedule a one-shot prompt to fire after `delay_seconds`.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["wakeup", "remind", "later", "delay"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<crate::message::LspDefinitionToolInput>(
                "lsp_definition",
                "Resolve the symbol at file_path:line:character to its definition site(s) via the configured LSP server.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["lsp", "definition", "go to def", "jump"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<crate::message::LspReferencesToolInput>(
                "lsp_references",
                "List every reference to the symbol at file_path:line:character via the configured LSP server.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["lsp", "references", "callers", "usages"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<crate::message::LspHoverToolInput>(
                "lsp_hover",
                "Read the hover documentation / type signature for the symbol at file_path:line:character.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["lsp", "hover", "type", "signature", "docs"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<crate::message::LspDiagnosticsToolInput>(
                "lsp_diagnostics",
                "Return the latest LSP-published diagnostics (errors / warnings / hints) for a file.",
                ToolBehavior::ReadOnly,
            )
            .with_search_terms(["lsp", "diagnostics", "errors", "warnings", "lint"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<NotebookEditToolInput>(
                "notebook_edit",
                "Edit a Jupyter .ipynb cell by replacing, inserting, or deleting a cell.",
                ToolBehavior::Mutating,
            )
            .with_search_terms(["notebook", "jupyter", "ipynb", "cell edit"])
            .with_deferred_loading(),
            ToolDefinition::builtin::<PowerShellToolInput>(
                "powershell",
                "Execute a Windows PowerShell command inside the configured workspace.",
                ToolBehavior::Mutating,
            )
            .with_search_terms(["windows", "powershell", "pwsh", "command"])
            .with_deferred_loading(),
        ];
        definitions.retain(|definition| self.is_behavior_enabled(definition.behavior));
        definitions
    }

    pub fn is_behavior_enabled(&self, behavior: ToolBehavior) -> bool {
        match self.profile {
            ModelToolProfile::Full => true,
            ModelToolProfile::ReadOnly => behavior == ToolBehavior::ReadOnly,
            ModelToolProfile::NoTask => behavior != ToolBehavior::Task,
        }
    }

    fn is_enabled(&self, tool_name: &str) -> bool {
        match self.profile {
            ModelToolProfile::Full => true,
            ModelToolProfile::ReadOnly => {
                matches!(
                    tool_name,
                    "read"
                        | "view_file"
                        | "glob"
                        | "grep"
                        | "tool_search"
                        | "todo_write"
                        | "ask_user"
                        | "web_fetch"
                        | "web_search"
                        | "enter_plan_mode"
                        | "exit_plan_mode"
                        | "skill_run"
                        | "lsp_definition"
                        | "lsp_references"
                        | "lsp_hover"
                        | "lsp_diagnostics"
                )
            }
            ModelToolProfile::NoTask => tool_name != "task",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_marks_read_tools_as_always_loaded() {
        let catalog = ToolCatalog::for_model(None);
        let definitions = catalog.builtin_definitions();

        let read = definitions
            .iter()
            .find(|tool| tool.name == "read")
            .expect("read builtin should exist");
        let grep = definitions
            .iter()
            .find(|tool| tool.name == "grep")
            .expect("grep builtin should exist");

        assert!(read.read_only);
        assert!(read.concurrency_safe);
        assert!(!read.is_deferred());
        assert!(grep.should_load_by_default());
    }

    #[test]
    fn builtin_catalog_defers_mutating_and_task_tools() {
        let catalog = ToolCatalog::for_model(None);
        let definitions = catalog.builtin_definitions();

        for tool_name in ["bash", "apply_patch", "task", "notebook_edit", "powershell"] {
            let definition = definitions
                .iter()
                .find(|tool| tool.name == tool_name)
                .unwrap_or_else(|| panic!("missing builtin definition for {tool_name}"));
            assert!(definition.is_deferred(), "{tool_name} should be deferred");
        }
    }

    #[test]
    fn readonly_profile_keeps_lsp_tools_enabled() {
        let catalog = ToolCatalog::for_model(Some("readonly-model"));
        let definitions = catalog.builtin_definitions();

        assert!(
            definitions
                .iter()
                .any(|tool| tool.name == "lsp_diagnostics")
        );
        assert!(!definitions.iter().any(|tool| tool.name == "notebook_edit"));
        assert!(!definitions.iter().any(|tool| tool.name == "powershell"));
    }
}
