use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::sandbox::cap_fs;
use crate::sandbox::{SandboxCommandRequest, SandboxError, SandboxExecOutput, SandboxPolicy};

use super::command_runner::{configure_piped_stdio, run_command_with_timeout};

const SANDBOX_EXEC_PATH: &str = "/usr/bin/sandbox-exec";

pub(super) fn execute(
    request: &SandboxCommandRequest,
    policy: &SandboxPolicy,
    workspace_root: &Path,
) -> Result<SandboxExecOutput, SandboxError> {
    if matches!(policy, SandboxPolicy::DangerFullAccess) {
        return execute_without_sandbox(request);
    }

    if !cap_fs::is_file(Path::new(SANDBOX_EXEC_PATH)) {
        return Err(SandboxError::Unavailable(format!(
            "{SANDBOX_EXEC_PATH} is not available",
        )));
    }

    let mut sandbox_command = vec![
        SANDBOX_EXEC_PATH.to_string(),
        "-p".to_string(),
        build_seatbelt_policy(policy, workspace_root),
        "--".to_string(),
    ];
    sandbox_command.extend(request.command.clone());

    execute_command(
        &sandbox_command,
        &request.cwd,
        &request.env,
        request.timeout_ms,
    )
}

fn build_seatbelt_policy(policy: &SandboxPolicy, workspace_root: &Path) -> String {
    let mut policy_text = include_str!("macos_base_policy.sbpl")
        .trim_end()
        .to_string();

    if policy.has_full_network_access() {
        append_rule(&mut policy_text, "(allow network-outbound)");
        append_rule(&mut policy_text, "(allow network-inbound)");
        append_rule(&mut policy_text, "(allow system-socket)");
    }

    if policy.has_full_disk_write_access() {
        append_rule(&mut policy_text, "(allow file-write*)");
    } else {
        let writable_roots = policy.writable_roots_with_workspace(workspace_root);
        if !writable_roots.is_empty() {
            let clauses = writable_roots
                .iter()
                .map(write_clause_for_root)
                .collect::<Vec<_>>()
                .join("\n  ");
            append_rule(
                &mut policy_text,
                &format!("(allow file-write* (require-any\n  {clauses}\n))"),
            );
        }
    }

    policy_text
}

fn append_rule(policy: &mut String, rule: &str) {
    policy.push('\n');
    policy.push_str(rule);
}

fn write_clause_for_root(root: &crate::sandbox::WritableRoot) -> String {
    let escaped_root = escape_sbpl_path(&root.root);
    if root.read_only_subpaths.is_empty() {
        return format!("(subpath \"{escaped_root}\")");
    }

    let mut require_not = Vec::new();
    for path in &root.read_only_subpaths {
        let escaped = escape_sbpl_path(path);
        require_not.push(format!("(require-not (subpath \"{escaped}\"))"));
    }

    format!(
        "(require-all (subpath \"{escaped_root}\") {})",
        require_not.join(" ")
    )
}

fn escape_sbpl_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn execute_without_sandbox(
    request: &SandboxCommandRequest,
) -> Result<SandboxExecOutput, SandboxError> {
    execute_command(
        &request.command,
        &request.cwd,
        &request.env,
        request.timeout_ms,
    )
}

fn execute_command(
    argv: &[String],
    cwd: &Path,
    env_map: &std::collections::HashMap<String, String>,
    timeout_ms: Option<u64>,
) -> Result<SandboxExecOutput, SandboxError> {
    let start = Instant::now();
    let mut command = Command::new(&argv[0]);
    if argv.len() > 1 {
        command.args(&argv[1..]);
    }

    command.current_dir(cwd).env_clear().envs(env_map.clone());
    configure_piped_stdio(&mut command);

    run_command_with_timeout(&mut command, timeout_ms, start)
}
