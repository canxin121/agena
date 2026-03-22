use std::collections::HashMap;
use std::path::Path;

use thiserror::Error;

use crate::message::BuiltinToolInput;
use crate::permission::{AccessKind, PermissionDecision, PermissionMode, PermissionPolicy};

#[derive(Debug, Clone)]
pub struct Agent {
    pub name: String,
    pub permission_policy: PermissionPolicy,
    pub tool_policy: ToolPermissionPolicy,
}

impl Agent {
    pub fn new(name: impl Into<String>, permission_policy: PermissionPolicy) -> Self {
        Self {
            name: name.into(),
            permission_policy,
            tool_policy: ToolPermissionPolicy::allow_all(),
        }
    }

    pub fn read_all_write_workspace_only(name: impl Into<String>) -> Self {
        Self::new(name, PermissionPolicy::read_all_write_workspace_only())
    }

    pub fn with_tool_policy(mut self, tool_policy: ToolPermissionPolicy) -> Self {
        self.tool_policy = tool_policy;
        self
    }

    pub fn key(&self) -> &str {
        &self.name
    }

    pub fn authorize_builtin_tool(&self, input: &BuiltinToolInput) -> PermissionDecision {
        self.tool_policy.check_builtin(input)
    }

    pub fn authorize_path_access(
        &self,
        access: AccessKind,
        workspace_root: &Path,
        target_path: &Path,
    ) -> PermissionDecision {
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
