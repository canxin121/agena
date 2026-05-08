use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::message::FirstPartyToolInput;
use crate::permission::{
    AccessKind, PermissionDecision, PermissionMode, PermissionPolicy, ToolPermissionPolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AgentMode {
    #[default]
    Primary,
    Subagent,
    All,
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

    pub fn with_tool_policy(mut self, tool_policy: ToolPermissionPolicy) -> Self {
        self.tool_policy = tool_policy;
        self
    }

    pub fn with_allowed_tools<I, S>(mut self, allowed_tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut tool_policy = ToolPermissionPolicy::new(PermissionMode::Deny)
            .with_execution_mode(self.tool_policy.execution_mode());
        for tool_name in allowed_tools {
            let name = tool_name.as_ref().trim();
            if name.is_empty() {
                continue;
            }
            tool_policy = tool_policy.with_tool_mode(name.to_string(), PermissionMode::Allow);
        }
        self.tool_policy = tool_policy;
        self
    }

    pub fn authorize_first_party_tool(&self, input: &FirstPartyToolInput) -> PermissionDecision {
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
            };
        }
        self.tool_policy.check_first_party(input)
    }

    pub fn authorize_tool_name(&self, tool_name: &str) -> PermissionDecision {
        self.authorize_tool_call(tool_name, false)
    }

    pub fn authorize_tool_call(&self, tool_name: &str, sensitive: bool) -> PermissionDecision {
        if self.disable {
            return PermissionDecision::Deny {
                reason: format!("agent '{}' is disabled", self.name),
            };
        }
        self.tool_policy.check_tool(tool_name, None, sensitive)
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

    use crate::message::{FirstPartyToolInput, ReadToolInput};
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
    fn disabled_agent_denies_first_party_tools() {
        let mut agent = Agent::new("build", PermissionPolicy::allow_all());
        agent.disable = true;
        let input = FirstPartyToolInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });

        match agent.authorize_first_party_tool(&input) {
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
