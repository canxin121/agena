use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::message::BuiltinToolInput;
use crate::permission::{AccessKind, PermissionDecision, PermissionMode, PermissionPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    Primary,
    Subagent,
    All,
}

impl Default for AgentMode {
    fn default() -> Self {
        Self::Primary
    }
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub name: String,
    pub description: Option<String>,
    pub mode: AgentMode,
    pub prompt: Option<String>,
    pub disable: bool,
    pub permission_policy: PermissionPolicy,
    pub tool_policy: ToolPermissionPolicy,
}

impl Agent {
    pub fn new(name: impl Into<String>, permission_policy: PermissionPolicy) -> Self {
        let name = name.into();
        Self {
            description: None,
            mode: AgentMode::Primary,
            prompt: None,
            disable: false,
            name,
            permission_policy,
            tool_policy: ToolPermissionPolicy::allow_all(),
        }
    }

    pub fn authorize_builtin_tool(&self, input: &BuiltinToolInput) -> PermissionDecision {
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
            };
        }
        self.tool_policy.check_builtin(input)
    }

    pub fn authorize_path_access(
        &self,
        access: AccessKind,
        workspace_root: &Path,
        target_path: &Path,
    ) -> PermissionDecision {
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
            };
        }
        self.permission_policy
            .check_access(access, workspace_root, target_path)
    }
}

#[derive(Debug, Clone)]
pub struct ToolPermissionPolicy {
    default_mode: PermissionMode,
    builtin_modes: HashMap<&'static str, PermissionMode>,
}

impl ToolPermissionPolicy {
    pub fn new(default_mode: PermissionMode) -> Self {
        Self {
            default_mode,
            builtin_modes: HashMap::new(),
        }
    }

    pub fn allow_all() -> Self {
        Self::new(PermissionMode::Allow)
    }

    pub fn with_builtin_mode(mut self, builtin_name: &'static str, mode: PermissionMode) -> Self {
        self.builtin_modes.insert(builtin_name, mode);
        self
    }

    pub fn check_builtin(&self, input: &BuiltinToolInput) -> PermissionDecision {
        let name = builtin_name(input);
        let mode = self
            .builtin_modes
            .get(name)
            .copied()
            .unwrap_or(self.default_mode);
        match mode {
            PermissionMode::Allow => PermissionDecision::Allow,
            PermissionMode::Ask => PermissionDecision::Ask {
                reason: format!("tool '{name}' requires confirmation by policy"),
            },
            PermissionMode::Deny => PermissionDecision::Deny {
                reason: format!("tool '{name}' denied by policy"),
            },
        }
    }
}

fn builtin_name(input: &BuiltinToolInput) -> &'static str {
    match input {
        BuiltinToolInput::Bash(_) => "bash",
        BuiltinToolInput::Read(_) => "read",
        BuiltinToolInput::Write(_) => "write",
        BuiltinToolInput::Edit(_) => "edit",
        BuiltinToolInput::ApplyPatch(_) => "apply_patch",
        BuiltinToolInput::Glob(_) => "glob",
        BuiltinToolInput::Grep(_) => "grep",
        BuiltinToolInput::Task(_) => "task",
    }
}

#[derive(Debug, Error)]
pub enum AgentPolicyError {
    #[error("agent '{agent_name}' cannot use tool '{tool_name}': {reason}")]
    ToolDenied {
        agent_name: String,
        tool_name: String,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::message::{BuiltinToolInput, ReadToolInput};
    use crate::permission::{AccessKind, PermissionPolicy};

    use super::{Agent, AgentMode};

    #[test]
    fn new_agent_has_reasonable_defaults() {
        let agent = Agent::new("build", PermissionPolicy::allow_all());

        assert_eq!(agent.name, "build");
        assert_eq!(agent.description, None);
        assert_eq!(agent.mode, AgentMode::Primary);
        assert_eq!(agent.prompt, None);
        assert!(!agent.disable);
    }

    #[test]
    fn agent_fields_can_be_set_directly() {
        let mut agent = Agent::new("explore", PermissionPolicy::allow_all());
        agent.description = Some("Read-only explorer".to_string());
        agent.mode = AgentMode::Subagent;
        agent.prompt = Some("You are a focused exploration agent.".to_string());
        agent.disable = true;

        assert_eq!(agent.description.as_deref(), Some("Read-only explorer"));
        assert_eq!(agent.mode, AgentMode::Subagent);
        assert_eq!(
            agent.prompt.as_deref(),
            Some("You are a focused exploration agent.")
        );
        assert!(agent.disable);
    }

    #[test]
    fn disabled_agent_denies_builtin_tools() {
        let mut agent = Agent::new("build", PermissionPolicy::allow_all());
        agent.disable = true;
        let input = BuiltinToolInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });

        match agent.authorize_builtin_tool(&input) {
            crate::permission::PermissionDecision::Deny { reason } => {
                assert!(reason.contains("disabled"));
            }
            other => panic!("expected deny decision for disabled agent, got {other:?}"),
        }
    }

    #[test]
    fn disabled_agent_denies_path_access() {
        let mut agent = Agent::new("build", PermissionPolicy::allow_all());
        agent.disable = true;

        match agent.authorize_path_access(AccessKind::Read, Path::new("."), Path::new("README.md"))
        {
            crate::permission::PermissionDecision::Deny { reason } => {
                assert!(reason.contains("disabled"));
            }
            other => panic!("expected deny decision for disabled agent, got {other:?}"),
        }
    }
}
