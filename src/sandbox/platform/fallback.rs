use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::sandbox::{SandboxCommandRequest, SandboxError, SandboxExecOutput, SandboxPolicy};

use super::command_runner::{configure_piped_stdio, run_command_with_timeout};

pub(super) fn execute(
    request: &SandboxCommandRequest,
    policy: &SandboxPolicy,
    _workspace_root: &Path,
) -> Result<SandboxExecOutput, SandboxError> {
    if !matches!(policy, SandboxPolicy::DangerFullAccess) {
        return Err(SandboxError::Unavailable(format!(
            "OS-level sandboxing is unavailable on target '{}'. Supported adapters currently cover windows/linux/macos. On this platform, use danger-full-access explicitly or implement a platform adapter.",
            std::env::consts::OS
        )));
    }

    let start = Instant::now();
    let mut command = Command::new(&request.command[0]);
    if request.command.len() > 1 {
        command.args(&request.command[1..]);
    }
    command
        .current_dir(&request.cwd)
        .env_clear()
        .envs(request.env.clone());
    configure_piped_stdio(&mut command);

    run_command_with_timeout(&mut command, request.timeout_ms, start)
}
