//! Synchronous fast path. Interactive tools always ask; read-only tools,
//! in-workspace reads, managed project-state writes, system temp-directory
//! reads/writes, and exact no-op shell commands are allowed without a model
//! call.
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
Reads and writes inside system temporary directories (for example /tmp, /var/tmp,
/private/tmp, or the platform's temp directory) are routine scratch-space operations
and may be allowed.
BLOCK actions that write outside the configured workspace, are irreversible or destructive,
exfiltrate secrets or private data, probe credentials, weaken persistent security, or run
untrusted code.
When uncertain, BLOCK (fail closed).
Respond only with a strict JSON object: {\"thinking\": string, \"shouldBlock\": boolean,
\"reason\": string}.";

#[derive(Debug, Clone, PartialEq, Eq)]
/// Fast-path decision of the auto-approval classifier.
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
        } => {
            // System temp directories are scratch space: reading and writing
            // inside them is routine, reversible, and safe to auto-approve even
            // when the target lies outside the configured workspace.
            if path_is_within_temp_dir(target) {
                return AutoFastPath::Allow;
            }
            match access.as_str() {
                "read" if path_is_within_workspace(target, workspace_root) => AutoFastPath::Allow,
                "write"
                    if managed_project_root
                        .is_some_and(|root| path_is_within_workspace(target, root)) =>
                {
                    AutoFastPath::Allow
                }
                _ => AutoFastPath::Defer,
            }
        }
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
    path_is_within_root(target, workspace_root)
}

/// Well-known system temporary directories: scratch space that is safe to
/// auto-approve. The platform's configured temp dir (TMPDIR / TMP / TEMP) is
/// checked separately because it varies per user (macOS: /var/folders/.../T,
/// Windows: %LOCALAPPDATA%\Temp).
const SYSTEM_TEMP_DIR_ROOTS: &[&str] = &[
    "/tmp",
    "/private/tmp",
    "/var/tmp",
    "/private/var/tmp",
    "C:/Windows/Temp",
];

/// True when `target` is inside one of the system's temporary directories.
/// Temp directories are scratch space: reads and writes there are routine,
/// reversible, and safe to auto-approve without a model call.
fn path_is_within_temp_dir(target: &str) -> bool {
    let target = target.replace('\\', "/");
    if SYSTEM_TEMP_DIR_ROOTS
        .iter()
        .any(|root| path_is_within_root(&target, root))
    {
        return true;
    }
    // The platform's configured temp dir (TMPDIR / TMP / TEMP) is also scratch
    // space, e.g. macOS /var/folders/.../T or the Windows per-user %TEMP%.
    let env_root = std::env::temp_dir().to_string_lossy().replace('\\', "/");
    !env_root.is_empty() && path_is_within_root(&target, &env_root)
}

fn path_is_within_root(target: &str, root: &str) -> bool {
    let mut target = target.replace('\\', "/");
    let mut root = root.replace('\\', "/");
    if cfg!(windows) {
        // Windows paths are case-insensitive; the policy layer normalizes
        // paths the same way.
        target.make_ascii_lowercase();
        root.make_ascii_lowercase();
    }
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

    #[test]
    fn allows_temp_directory_reads_and_writes() {
        for target in [
            "/tmp/agena_pty.log",
            "/private/tmp/agena_pty.log",
            "/var/tmp/scratch.bin",
            "/private/var/tmp/scratch.bin",
            "C:/Windows/Temp/agena.tmp",
            "/tmp",
        ] {
            assert_eq!(
                auto_fast_path(&path("read", "/work", target), None),
                AutoFastPath::Allow,
                "{target} read should be auto-approved"
            );
            assert_eq!(
                auto_fast_path(&path("write", "/work", target), None),
                AutoFastPath::Allow,
                "{target} write should be auto-approved"
            );
        }
        // The platform's configured temp dir (TMPDIR / TMP / TEMP) is also
        // scratch space and differs per user (macOS /var/folders/.../T,
        // Windows %LOCALAPPDATA%\Temp).
        let platform_temp = std::env::temp_dir().join("agena-approval-test.log");
        let platform_temp = platform_temp.to_string_lossy().replace('\\', "/");
        assert_eq!(
            auto_fast_path(&path("write", "/work", &platform_temp), None),
            AutoFastPath::Allow,
            "platform temp dir write should be auto-approved"
        );
    }

    #[test]
    fn temp_directory_allowance_does_not_leak_outside() {
        for target in [
            "/tmp/../etc/passwd",
            "/tmp-other/scratch.bin",
            "/etc/passwd",
            "/var",
        ] {
            assert_eq!(
                auto_fast_path(&path("write", "/work", target), None),
                AutoFastPath::Defer,
                "{target} must not be auto-approved as a temp write"
            );
        }
    }
}
