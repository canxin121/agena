use std::{
    fmt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use agena_tool::{SnapshotBackend, SnapshotBackendCapabilities};

use crate::{
    SnapshotSession, bounded_process::command_output, snapshot_backend_capabilities,
    snapshot_managed_dir, snapshot_rift_binary, snapshot_rift_database_path,
};

const SNAPSHOT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
/// Result of creating a snapshot.
pub struct SnapshotCreation {
    pub session: SnapshotSession,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
/// Error of a snapshot operation.
pub struct SnapshotOperationError(String);

impl fmt::Display for SnapshotOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SnapshotOperationError {}

/// Create a managed snapshot, preferring Rift and falling back to Git worktree.
pub fn create_managed_snapshot(
    workspace: &Path,
    name: Option<&str>,
) -> Result<SnapshotCreation, SnapshotOperationError> {
    let slug = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(generate_slug);
    let base = snapshot_managed_dir(workspace);
    std::fs::create_dir_all(&base)
        .map_err(|error| operation_error(format!("snapshot.enter: mkdir {base:?}: {error}")))?;
    let target = base.join(&slug);
    if target.exists() {
        return Err(operation_error(format!(
            "snapshot.enter: target {target:?} already exists"
        )));
    }
    let capabilities = snapshot_backend_capabilities(workspace);
    let (rift_failure, note) = if capabilities.rift.available {
        match create_with_rift(workspace, &base, &slug) {
            Ok(session) => {
                return Ok(SnapshotCreation {
                    session,
                    note: None,
                });
            }
            Err(error) => (
                Some(error.to_string()),
                Some(format!(
                    "used git backend because Rift could not create a snapshot here: {error}"
                )),
            ),
        }
    } else {
        (
            Some(capabilities.rift.detail.clone()),
            capabilities.git.available.then(|| {
                format!(
                    "used git backend because Rift is unavailable: {}",
                    capabilities.rift.detail
                )
            }),
        )
    };
    if !capabilities.git.available {
        return Err(create_backend_error(
            &capabilities,
            rift_failure.as_deref(),
            None,
        ));
    }
    match create_with_git(workspace, &target, &slug) {
        Ok(session) => Ok(SnapshotCreation { session, note }),
        Err(error) => Err(create_backend_error(
            &capabilities,
            rift_failure.as_deref(),
            Some(&error.to_string()),
        )),
    }
}

/// Attach an existing Rift snapshot or registered Git worktree to a session.
pub fn attach_existing_snapshot(
    workspace: &Path,
    path: &str,
) -> Result<SnapshotSession, SnapshotOperationError> {
    let path_buf = PathBuf::from(path);
    if !path_buf.exists() {
        return Err(operation_error(format!(
            "snapshot.enter: path {path:?} does not exist"
        )));
    }
    if is_managed_rift_workspace(workspace, &path_buf) {
        return Ok(SnapshotSession {
            path: path_buf.clone(),
            branch: describe_workspace_head(&path_buf),
            original_workspace: workspace.to_path_buf(),
            backend: SnapshotBackend::Rift,
            created_here: false,
        });
    }
    let capabilities = snapshot_backend_capabilities(workspace);
    if !capabilities.git.available {
        return Err(operation_error(format!(
            "snapshot.enter: cannot attach existing git worktree: {}",
            capabilities.git.detail
        )));
    }
    let listing = git(workspace, &["worktree", "list", "--porcelain"])?;
    let canonical = path_buf
        .canonicalize()
        .unwrap_or(path_buf.clone())
        .to_string_lossy()
        .to_string();
    let mut found_branch = None;
    let mut current_path: Option<String> = None;
    for line in String::from_utf8_lossy(&listing.stdout).lines() {
        if let Some(path_value) = line.strip_prefix("worktree ") {
            current_path = Some(path_value.to_owned());
        } else if let Some(branch_value) = line.strip_prefix("branch ")
            && let Some(current_path_value) = current_path.as_deref()
        {
            let current_canonical = PathBuf::from(current_path_value)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(current_path_value))
                .to_string_lossy()
                .to_string();
            if current_canonical == canonical || current_path_value == path {
                found_branch = Some(
                    branch_value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(branch_value)
                        .to_owned(),
                );
            }
        }
    }
    let branch = found_branch.ok_or_else(|| {
        operation_error(format!(
            "snapshot.enter: {path:?} is not a registered git worktree of this repo"
        ))
    })?;
    Ok(SnapshotSession {
        path: path_buf,
        branch,
        original_workspace: workspace.to_path_buf(),
        backend: SnapshotBackend::Git,
        created_here: false,
    })
}

/// Return whether a Git-backed snapshot has uncommitted changes.
pub fn snapshot_has_local_changes(path: &Path) -> bool {
    git(path, &["rev-parse", "--is-inside-work-tree"])
        .and_then(|_| git(path, &["status", "--porcelain"]))
        .map(|output| !output.stdout.is_empty())
        .unwrap_or(false)
}

/// Remove a snapshot created by this process. Permission checks belong to the caller.
pub fn remove_managed_snapshot(session: &SnapshotSession) -> Result<(), SnapshotOperationError> {
    match session.backend {
        SnapshotBackend::Git => {
            let _ = git(
                &session.original_workspace,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    session.path.to_string_lossy().as_ref(),
                ],
            )?;
            let _ = git(
                &session.original_workspace,
                &["branch", "-D", &session.branch],
            );
            Ok(())
        }
        SnapshotBackend::Rift => {
            let db_path = snapshot_rift_database_path(&session.original_workspace);
            let path = session.path.to_string_lossy().to_string();
            let _ = rift(&session.original_workspace, &db_path, &["remove", &path])?;
            if let Err(error) = rift(&session.original_workspace, &db_path, &["gc"]) {
                tracing::warn!(target: "agena_runtime::snapshot", workspace = %session.original_workspace.display(), path = %session.path.display(), error = %error, "rift remove succeeded but garbage collection did not complete");
            }
            Ok(())
        }
    }
}

fn create_with_git(
    workspace: &Path,
    target: &Path,
    slug: &str,
) -> Result<SnapshotSession, SnapshotOperationError> {
    let branch = format!("agena/{slug}");
    let _ = git(
        workspace,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            target.to_string_lossy().as_ref(),
        ],
    )?;
    Ok(SnapshotSession {
        path: target.to_path_buf(),
        branch,
        original_workspace: workspace.to_path_buf(),
        backend: SnapshotBackend::Git,
        created_here: true,
    })
}

fn create_with_rift(
    workspace: &Path,
    base: &Path,
    slug: &str,
) -> Result<SnapshotSession, SnapshotOperationError> {
    let db_path = snapshot_rift_database_path(workspace);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            operation_error(format!(
                "failed to prepare Rift database directory: {error}"
            ))
        })?;
    }
    let workspace_string = workspace.to_string_lossy().to_string();
    let base_string = base.to_string_lossy().to_string();
    let _ = rift(workspace, &db_path, &["init", "--here", &workspace_string])?;
    let output = rift(
        workspace,
        &db_path,
        &[
            "create",
            &workspace_string,
            "--name",
            slug,
            "--into",
            &base_string,
            "--no-hooks",
        ],
    )?;
    let created_path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| base.join(slug));
    let branch = ensure_rift_branch(&created_path, slug)
        .unwrap_or_else(|| describe_workspace_head(&created_path));
    Ok(SnapshotSession {
        path: created_path,
        branch,
        original_workspace: workspace.to_path_buf(),
        backend: SnapshotBackend::Rift,
        created_here: true,
    })
}

fn create_backend_error(
    capabilities: &SnapshotBackendCapabilities,
    rift_failure: Option<&str>,
    git_failure: Option<&str>,
) -> SnapshotOperationError {
    let preferred = capabilities
        .preferred_backend
        .map(|backend| backend.to_string())
        .unwrap_or_else(|| "none".to_owned());
    let mut detail = vec![
        format!("preferred backend: {preferred}"),
        format!(
            "rift: available={} | {}",
            capabilities.rift.available, capabilities.rift.detail
        ),
        format!(
            "git: available={} | {}",
            capabilities.git.available, capabilities.git.detail
        ),
    ];
    if let Some(failure) = rift_failure {
        detail.push(format!("rift create attempt: {failure}"));
    }
    if let Some(failure) = git_failure {
        detail.push(format!("git worktree create attempt: {failure}"));
    }
    operation_error(format!(
        "snapshot.enter: could not create a managed snapshot\n  {}",
        detail.join("\n  ")
    ))
}

fn git(cwd: &Path, args: &[&str]) -> Result<process_control::Output, SnapshotOperationError> {
    let output = command_output(
        Command::new("git").args(args).current_dir(cwd),
        SNAPSHOT_COMMAND_TIMEOUT,
    )
    .map_err(|error| operation_error(format!("git {args:?}: {error}")))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(operation_error(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn rift(
    cwd: &Path,
    db_path: &Path,
    args: &[&str],
) -> Result<process_control::Output, SnapshotOperationError> {
    let output = command_output(
        Command::new(snapshot_rift_binary())
            .arg("--database")
            .arg(db_path)
            .args(args)
            .current_dir(cwd),
        SNAPSHOT_COMMAND_TIMEOUT,
    )
    .map_err(|error| operation_error(format!("rift {args:?}: {error}")))?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(operation_error(format!(
        "rift {args:?} failed: {}",
        if stderr.is_empty() { stdout } else { stderr }
    )))
}

fn is_managed_rift_workspace(workspace: &Path, path: &Path) -> bool {
    if !path.join(".rift").is_file() {
        return false;
    }
    let path = match path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "canonicalize a possible managed Rift workspace",
                    &error,
                ),
                "snapshot path was not accepted as a managed Rift workspace"
            );
            return false;
        }
    };
    let managed = snapshot_managed_dir(workspace);
    let managed = match managed.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "canonicalize the managed snapshot directory",
                    &error,
                ),
                "snapshot path was not accepted as a managed Rift workspace"
            );
            return false;
        }
    };
    path.starts_with(managed)
}

fn ensure_rift_branch(path: &Path, slug: &str) -> Option<String> {
    let branch = format!("agena/{slug}");
    match git(path, &["switch", "-c", &branch]) {
        Ok(_) => Some(branch),
        Err(primary) => match git(path, &["checkout", "-b", &branch]) {
            Ok(_) => {
                tracing::debug!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "create a Rift snapshot branch with git switch",
                        &primary,
                    ),
                    "used legacy git checkout to create the Rift snapshot branch"
                );
                Some(branch)
            }
            Err(fallback) => {
                tracing::warn!(
                    primary = %agena_failure::diagnostic::format_error_chain_with_context(
                        "create a Rift snapshot branch with git switch",
                        &primary,
                    ),
                    fallback = %agena_failure::diagnostic::format_error_chain_with_context(
                        "create a Rift snapshot branch with git checkout",
                        &fallback,
                    ),
                    "Rift snapshot branch creation failed"
                );
                None
            }
        },
    }
}

fn describe_workspace_head(path: &Path) -> String {
    let branch = match git(path, &["branch", "--show-current"]) {
        Ok(output) => {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!branch.is_empty()).then_some(branch)
        }
        Err(error) => {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "read the current snapshot branch",
                    &error,
                ),
                "snapshot branch name is unavailable"
            );
            None
        }
    };
    if let Some(branch) = branch {
        return branch;
    }
    match git(path, &["rev-parse", "--short", "HEAD"]) {
        Ok(output) => {
            let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if head.is_empty() {
                "snapshot".to_owned()
            } else {
                format!("detached@{head}")
            }
        }
        Err(error) => {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "read the current snapshot commit",
                    &error,
                ),
                "using a generic snapshot description"
            );
            "snapshot".to_owned()
        }
    }
}

fn generate_slug() -> String {
    format!("snapshot-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"))
}
fn operation_error(message: String) -> SnapshotOperationError {
    SnapshotOperationError(message)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::attach_existing_snapshot;

    #[test]
    fn attaching_a_missing_snapshot_is_rejected_before_backend_execution() {
        let error = attach_existing_snapshot(
            Path::new("/definitely/not/an/agena/workspace"),
            "/definitely/not/an/agena/snapshot",
        )
        .expect_err("missing snapshot path must not invoke a backend");
        assert!(error.to_string().contains("does not exist"));
    }
}
