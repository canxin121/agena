//! Synchronous fast path. Interactive tools always ask; read-only tools,
//! in-workspace reads, managed project-state writes, and exact no-op shell
//! commands are allowed without a model call.
//!
//! Read-only detection is contract-driven ([`ToolCapabilities::read_only`]),
//! never tool-name allowlists and never tool tags, so MCP and future plugins
//! are covered automatically.

use agena_domain::ActionSpec;

pub const AUTO_APPROVAL_SYSTEM_PROMPT: &str = "\
You are Agena's automatic permission safety classifier. You are a safety gate, not an agent.
You evaluate one proposed tool action in the context of a recent conversation transcript.
ALLOW actions that are clearly safe, local to the machine, and reversible: routine
development actions inside the configured workspace, and read-only access to external
resources (reading files or fetching data outside the workspace) when the action is
reasonable and does not expose secrets, credentials, private keys, or personal data.
BLOCK actions that write outside the configured workspace, are irreversible or destructive,
exfiltrate secrets or private data, probe credentials, weaken persistent security, or run
untrusted code.
When uncertain, BLOCK (fail closed).
Respond only with a strict JSON object: {\"thinking\": string, \"shouldBlock\": boolean,
\"reason\": string}.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoFastPath {
    Allow,
    Ask { reason: String },
    Defer,
}

pub fn auto_fast_path(action: &ActionSpec, managed_project_root: Option<&str>) -> AutoFastPath {
    match action {
        ActionSpec::Tool {
            tool_name,
            contract,
            command,
        } => {
            if is_interaction_tool(tool_name) {
                return AutoFastPath::Ask {
                    reason: "interactive tools always require explicit user confirmation"
                        .to_owned(),
                };
            }
            if contract.read_only && !contract.shell && !contract.interactive {
                return AutoFastPath::Allow;
            }
            if command.as_deref().is_some_and(is_exact_noop_command) {
                return AutoFastPath::Allow;
            }
            AutoFastPath::Defer
        }
        ActionSpec::Path {
            access,
            workspace_root,
            target,
        } => match access.as_str() {
            "read" if path_is_within_workspace(target, workspace_root) => AutoFastPath::Allow,
            "write"
                if managed_project_root
                    .is_some_and(|root| path_is_within_workspace(target, root)) =>
            {
                AutoFastPath::Allow
            }
            _ => AutoFastPath::Defer,
        },
        ActionSpec::Network { .. } => AutoFastPath::Defer,
    }
}

fn is_interaction_tool(tool_name: &str) -> bool {
    let name = tool_name.strip_prefix("agena.").unwrap_or(tool_name);
    name == "interaction.ask" || name.starts_with("interaction.")
}

/// Exact no-op shell commands: they cannot change anything.
fn is_exact_noop_command(command: &str) -> bool {
    matches!(command.trim(), "true" | ":" | "false")
}

pub(crate) fn path_is_within_workspace(target: &str, workspace_root: &str) -> bool {
    let target = target.replace('\\', "/");
    let root = workspace_root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return false;
    }
    if target == root {
        return true;
    }
    let Some(prefix) = target.strip_prefix(&format!("{root}/")) else {
        return false;
    };
    !prefix.split('/').any(|segment| segment == "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{ActionSpec, ToolPermissionContract};

    fn tool(name: &str, contract: ToolPermissionContract, command: Option<&str>) -> ActionSpec {
        ActionSpec::Tool {
            tool_name: name.to_owned(),
            contract,
            command: command.map(ToOwned::to_owned),
        }
    }

    fn path(access: &str, root: &str, target: &str) -> ActionSpec {
        ActionSpec::Path {
            access: access.to_owned(),
            workspace_root: root.to_owned(),
            target: target.to_owned(),
        }
    }

    fn read_only_contract() -> ToolPermissionContract {
        ToolPermissionContract {
            read_only: true,
            input_paths: vec![agena_domain::InputPathSpec {
                jsonpath: "$.path".to_owned(),
                kind: agena_domain::PathKind::Read,
                fallback: None,
                optional: false,
            }],
            ..ToolPermissionContract::default()
        }
    }

    #[test]
    fn asks_for_interactive_tools() {
        for name in ["interaction.ask", "agena.interaction.ask"] {
            assert!(matches!(
                auto_fast_path(&tool(name, ToolPermissionContract::default(), None), None),
                AutoFastPath::Ask { .. }
            ));
        }
    }

    #[test]
    fn allows_read_only_contract_tools_without_name_allowlists() {
        assert_eq!(
            auto_fast_path(&tool("any.plugin.read", read_only_contract(), None), None),
            AutoFastPath::Allow
        );
        assert_eq!(
            auto_fast_path(&tool("mcp.read_file", read_only_contract(), None), None),
            AutoFastPath::Allow
        );
        let write_contract = ToolPermissionContract {
            input_paths: vec![agena_domain::InputPathSpec {
                jsonpath: "$.path".to_owned(),
                kind: agena_domain::PathKind::Write,
                fallback: None,
                optional: false,
            }],
            ..ToolPermissionContract::default()
        };
        assert_eq!(
            auto_fast_path(&tool("fs.write", write_contract, None), None),
            AutoFastPath::Defer
        );
    }

    #[test]
    fn allows_workspace_reads_and_managed_writes() {
        assert_eq!(
            auto_fast_path(&path("read", "/work", "/work/src/main.rs"), None),
            AutoFastPath::Allow
        );
        assert_eq!(
            auto_fast_path(&path("read", "/work", "/work/../etc/passwd"), None),
            AutoFastPath::Defer
        );
        assert_eq!(
            auto_fast_path(
                &path("write", "/work", "/work/.agena/state.json"),
                Some("/work/.agena")
            ),
            AutoFastPath::Allow
        );
        assert_eq!(
            auto_fast_path(&path("write", "/work", "/work/src/main.rs"), None),
            AutoFastPath::Defer
        );
    }

    #[test]
    fn allows_exact_noop_commands() {
        let shell_contract = ToolPermissionContract {
            shell: true,
            ..ToolPermissionContract::default()
        };
        for command in ["true", ":", "false"] {
            assert_eq!(
                auto_fast_path(
                    &tool("shell.run", shell_contract.clone(), Some(command)),
                    None
                ),
                AutoFastPath::Allow
            );
        }
    }
}
