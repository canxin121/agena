use crate::agent::Agent;
use crate::message::{
    ApplyPatchToolInput, BashToolInput, BuiltinToolInput, EditToolInput, GlobToolInput,
    GrepToolInput, ReadToolInput, TaskToolInput, WriteToolInput,
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
            ),
            ToolDefinition::builtin::<ReadToolInput>(
                "read",
                "Read a UTF-8 text file or list a directory with optional pagination.",
                ToolBehavior::ReadOnly,
            ),
            ToolDefinition::builtin::<WriteToolInput>(
                "write",
                "Create or overwrite a file inside the workspace.",
                ToolBehavior::Mutating,
            ),
            ToolDefinition::builtin::<EditToolInput>(
                "edit",
                "Replace an exact string inside a file, optionally for all matches.",
                ToolBehavior::Mutating,
            ),
            ToolDefinition::builtin::<ApplyPatchToolInput>(
                "apply_patch",
                "Apply a structured patch that can add, update, move, or delete files.",
                ToolBehavior::Mutating,
            ),
            ToolDefinition::builtin::<GlobToolInput>(
                "glob",
                "Search files by glob pattern from the workspace or a subdirectory.",
                ToolBehavior::ReadOnly,
            ),
            ToolDefinition::builtin::<GrepToolInput>(
                "grep",
                "Search file contents by regex pattern with optional include glob.",
                ToolBehavior::ReadOnly,
            ),
            ToolDefinition::builtin::<TaskToolInput>(
                "task",
                "Create or resume a subagent task session for delegated work.",
                ToolBehavior::Task,
            ),
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
                matches!(tool_name, "read" | "glob" | "grep")
            }
            ModelToolProfile::NoTask => tool_name != "task",
        }
    }
}
