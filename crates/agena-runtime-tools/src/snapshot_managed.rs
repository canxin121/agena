use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use agena_tool::SnapshotBackend;

use crate::{
    SnapshotRegistry, SnapshotSession, bounded_process::command_output, snapshot_managed_dir,
    snapshot_rift_binary, snapshot_rift_database_path,
};

const SNAPSHOT_INSPECTION_TIMEOUT: Duration = Duration::from_secs(60);

/// One managed snapshot directory, joined with its active-session and backend state.
#[derive(Debug, Clone)]
pub struct ManagedSnapshot {
    pub path: PathBuf,
    pub session_id: Option<i64>,
    pub branch: Option<String>,
    pub backend: Option<SnapshotBackend>,
    pub registered_with_git: bool,
    pub registered_with_rift: bool,
}

impl ManagedSnapshot {
    pub fn is_stale(&self) -> bool {
        self.session_id.is_none() && !self.registered_with_git && !self.registered_with_rift
    }
}

/// Report managed snapshot directories and their current backend registration.
pub fn list_managed_snapshots(
    workspace: &Path,
    registry: &SnapshotRegistry,
) -> Vec<ManagedSnapshot> {
    let base = snapshot_managed_dir(workspace);
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };

    let sessions = registry.read();
    let sessions_by_path: HashMap<PathBuf, SnapshotSession> = sessions
        .values()
        .map(|session| (session.path.clone(), session.clone()))
        .collect();
    let session_ids_by_path: HashMap<PathBuf, i64> = sessions
        .iter()
        .map(|(session_id, session)| (session.path.clone(), *session_id))
        .collect();
    drop(sessions);
    let git_paths = scan_git_backend_paths(workspace);
    let mut snapshots = Vec::new();

    for dirent in entries.flatten() {
        let path = dirent.path();
        if !path.is_dir() || is_reserved_managed_entry(&path) {
            continue;
        }
        let session_id = session_ids_by_path
            .iter()
            .find(|(candidate, _)| paths_equal(candidate, &path))
            .map(|(_, session_id)| *session_id);
        let active_session = sessions_by_path
            .iter()
            .find_map(|(candidate, session)| paths_equal(candidate, &path).then_some(session));
        let registered_with_git = git_paths
            .iter()
            .any(|candidate| paths_equal(candidate, &path));
        let registered_with_rift = path.join(".rift").is_file();
        let backend = active_session.map(|session| session.backend).or_else(|| {
            registered_with_rift
                .then_some(SnapshotBackend::Rift)
                .or_else(|| registered_with_git.then_some(SnapshotBackend::Git))
        });
        let branch = active_session
            .map(|session| session.branch.clone())
            .or_else(|| {
                (registered_with_git || registered_with_rift)
                    .then(|| describe_workspace_head(&path))
            });
        snapshots.push(ManagedSnapshot {
            path,
            session_id,
            branch,
            backend,
            registered_with_git,
            registered_with_rift,
        });
    }
    snapshots.sort_by(|left, right| left.path.cmp(&right.path));
    snapshots
}

/// Remove managed directories no longer known to a session or snapshot backend.
pub fn prune_stale_managed_snapshots(
    workspace: &Path,
    registry: &SnapshotRegistry,
) -> Vec<PathBuf> {
    let mut removed = rift_gc(workspace);
    for entry in list_managed_snapshots(workspace, registry) {
        if entry.is_stale() && std::fs::remove_dir_all(&entry.path).is_ok() {
            removed.push(entry.path);
        } else if entry.is_stale() {
            tracing::warn!(target: "agena_runtime::snapshot", "prune_stale: failed to remove {}", entry.path.display());
        }
    }
    removed
}

fn rift_gc(workspace: &Path) -> Vec<PathBuf> {
    let db_path = snapshot_rift_database_path(workspace);
    if !db_path.exists() {
        return Vec::new();
    }
    let Ok(output) = rift(workspace, &db_path, &["gc"]) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn scan_git_backend_paths(workspace: &Path) -> HashSet<PathBuf> {
    let output = match git(workspace, &["worktree", "list", "--porcelain"]) {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(
                diagnostic = %error,
                "managed snapshot Git worktree discovery failed; backend path exclusion is partial"
            );
            return HashSet::new();
        }
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .map(|path| match path.canonicalize() {
            Ok(canonical) => canonical,
            Err(error) => {
                tracing::warn!(
                    worktree_path = %path.display(),
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "canonicalize a Git worktree backend path",
                        &error,
                    ),
                    "managed snapshot backend exclusion will use an unresolved worktree path"
                );
                path
            }
        })
        .collect()
}

fn git(cwd: &Path, args: &[&str]) -> Result<process_control::Output, String> {
    let output = command_output(
        Command::new("git").args(args).current_dir(cwd),
        SNAPSHOT_INSPECTION_TIMEOUT,
    )
    .map_err(|error| format!("git {args:?}: {error}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn rift(cwd: &Path, db_path: &Path, args: &[&str]) -> Result<process_control::Output, String> {
    let output = command_output(
        Command::new(snapshot_rift_binary())
            .arg("--database")
            .arg(db_path)
            .args(args)
            .current_dir(cwd),
        SNAPSHOT_INSPECTION_TIMEOUT,
    )
    .map_err(|error| format!("rift {args:?}: {error}"))?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(format!(
        "rift {args:?} failed: {}",
        if stderr.is_empty() { stdout } else { stderr }
    ))
}

fn describe_workspace_head(path: &Path) -> String {
    let branch = match git(path, &["branch", "--show-current"]) {
        Ok(output) => {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!branch.is_empty()).then_some(branch)
        }
        Err(error) => {
            tracing::debug!(
                diagnostic = %error,
                "managed snapshot branch name is unavailable"
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
            tracing::debug!(diagnostic = %error, "using a generic managed snapshot description");
            "snapshot".to_owned()
        }
    }
}

fn is_reserved_managed_entry(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".trash")
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let left = match a.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "canonicalize the first managed snapshot path",
                    &error,
                ),
                "falling back to lexical snapshot path comparison"
            );
            a.to_path_buf()
        }
    };
    let right = match b.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "canonicalize the second managed snapshot path",
                    &error,
                ),
                "falling back to lexical snapshot path comparison"
            );
            b.to_path_buf()
        }
    };
    left == right
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use agena_tool::SnapshotBackend;

    use super::ManagedSnapshot;

    #[test]
    fn stale_snapshot_requires_no_session_or_backend_registration() {
        let stale = ManagedSnapshot {
            path: PathBuf::from("/tmp/orphan"),
            session_id: None,
            branch: None,
            backend: None,
            registered_with_git: false,
            registered_with_rift: false,
        };
        assert!(stale.is_stale());

        let active = ManagedSnapshot {
            session_id: Some(7),
            backend: Some(SnapshotBackend::Git),
            ..stale
        };
        assert!(!active.is_stale());
    }
}
