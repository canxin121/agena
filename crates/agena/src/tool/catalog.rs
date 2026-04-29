use crate::agent::Agent;
use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, BashToolInput, BuiltinToolInput, GlobToolInput,
    GrepToolInput, MonitorToolInput, ReadToolInput, TaskToolInput, TodoWriteToolInput,
    ToolSearchToolInput, ViewFileToolInput, WebFetchToolInput, WebSearchToolInput,
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

        for tool_name in ["bash", "apply_patch", "task"] {
            let definition = definitions
                .iter()
                .find(|tool| tool.name == tool_name)
                .unwrap_or_else(|| panic!("missing builtin definition for {tool_name}"));
            assert!(definition.is_deferred(), "{tool_name} should be deferred");
        }
    }
}
