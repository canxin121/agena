use std::{io::ErrorKind, path::Path, process::Command, time::Duration};

use agena_tool::{SnapshotBackend, SnapshotBackendCapabilities, SnapshotBackendSupport};

use crate::{bounded_process::command_output, snapshot_rift_binary};

const SNAPSHOT_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Detect which managed-snapshot backends are usable for a workspace.
pub fn snapshot_backend_capabilities(workspace: &Path) -> SnapshotBackendCapabilities {
    let git = probe_git_backend(workspace);
    let rift = probe_rift_backend();
    let preferred_backend = if rift.available {
        Some(SnapshotBackend::Rift)
    } else if git.available {
        Some(SnapshotBackend::Git)
    } else {
        None
    };
    SnapshotBackendCapabilities {
        preferred_backend,
        git,
        rift,
    }
}

fn probe_git_backend(workspace: &Path) -> SnapshotBackendSupport {
    if let Err(detail) = probe_command_presence("git", &["--version"], "git CLI") {
        return SnapshotBackendSupport {
            backend: SnapshotBackend::Git,
            available: false,
            detail,
        };
    }

    let output = match command_output(
        Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(workspace),
        SNAPSHOT_PROBE_TIMEOUT,
    ) {
        Ok(output) => output,
        Err(error) => {
            return SnapshotBackendSupport {
                backend: SnapshotBackend::Git,
                available: false,
                detail: format!(
                    "failed to execute `git` in {}: {error}",
                    workspace.display()
                ),
            };
        }
    };

    if output.status.success() {
        SnapshotBackendSupport {
            backend: SnapshotBackend::Git,
            available: true,
            detail: "git CLI is available and the workspace is a git repository".to_string(),
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("workspace {} is not a git repository", workspace.display())
        } else {
            format!(
                "workspace {} is not a git repository: {stderr}",
                workspace.display()
            )
        };
        SnapshotBackendSupport {
            backend: SnapshotBackend::Git,
            available: false,
            detail,
        }
    }
}

fn probe_rift_backend() -> SnapshotBackendSupport {
    let binary = snapshot_rift_binary();
    match probe_command_presence(binary.as_str(), &["--help"], "Rift CLI") {
        Ok(()) => SnapshotBackendSupport {
            backend: SnapshotBackend::Rift,
            available: true,
            detail: format!(
                "Rift CLI `{binary}` is available; filesystem and repository compatibility are verified when a snapshot is created"
            ),
        },
        Err(detail) => SnapshotBackendSupport {
            backend: SnapshotBackend::Rift,
            available: false,
            detail,
        },
    }
}

fn probe_command_presence(command: &str, args: &[&str], label: &str) -> Result<(), String> {
    match command_output(Command::new(command).args(args), SNAPSHOT_PROBE_TIMEOUT) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Err(format!("{label} `{command}` was not found on PATH"))
        }
        Err(error) => Err(format!("failed to start {label} `{command}`: {error}")),
    }
}
