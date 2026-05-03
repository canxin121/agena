//! `enter_worktree` / `exit_worktree` builtin tools.
//!
//! Thin wrappers around the `git` CLI; we don't pull in libgit2 for the
//! sake of keeping the dependency surface tight.  Worktrees are created
//! under `<workspace>/.agena/worktrees/<name>` with a fresh branch
//! `agena/<name>` rooted at the current HEAD.
//!
//! State is held in a process-wide registry keyed by session id so
//! `enter` and `exit` can refer to the same worktree across multiple
//! tool invocations.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::message::{BuiltinToolOutput, EnterWorktreeToolInput, ExitWorktreeToolInput};

use super::{BuiltinExecution, ToolError, ToolExecutionView, ToolExecutor};

#[derive(Debug, Clone)]
pub struct WorktreeSession {
    pub path: PathBuf,
    pub branch: String,
    pub original_workspace: PathBuf,
    /// True when we created the worktree (and so are responsible for
    /// cleaning it up on `remove`).
    pub created_here: bool,
}

pub type WorktreeRegistry = Arc<RwLock<std::collections::HashMap<i64, WorktreeSession>>>;

pub fn registry_for_executor() -> WorktreeRegistry {
    Arc::new(RwLock::new(std::collections::HashMap::new()))
}

pub(super) fn execute_enter(
    executor: &ToolExecutor,
    input: &EnterWorktreeToolInput,
    session_id: Option<i64>,
) -> Result<BuiltinExecution, ToolError> {
    let session_id = session_id.ok_or_else(|| {
        ToolError::Plugin("enter_worktree: no session in execution context".to_string())
    })?;
    let registry = executor
        .worktree_registry()
        .ok_or_else(|| ToolError::Plugin("enter_worktree: registry not configured".to_string()))?;

    if registry.read().contains_key(&session_id) {
        return Err(ToolError::Plugin(
            "enter_worktree: session is already in a worktree; call exit_worktree first"
                .to_string(),
        ));
    }

    let workspace = executor.workspace_root().to_path_buf();
    if input.name.is_some() && input.path.is_some() {
        return Err(ToolError::Plugin(
            "enter_worktree: provide either `name` or `path`, not both".to_string(),
        ));
    }

    let session = if let Some(p) = input.path.as_deref() {
        enter_existing(&workspace, p)?
    } else {
        create_new(&workspace, input.name.as_deref())?
    };

    let view = ToolExecutionView::simple(
        format!("Worktree → {}", session.path.display()),
        format!(
            "Switched to worktree:\n  path:   {}\n  branch: {}\n",
            session.path.display(),
            session.branch
        ),
    );
    let output = BuiltinToolOutput::EnterWorktree {
        path: session.path.to_string_lossy().to_string(),
        branch: session.branch.clone(),
    };

    registry.write().insert(session_id, session);
    Ok(BuiltinExecution::new(output, view))
}

pub(super) fn execute_exit(
    executor: &ToolExecutor,
    input: &ExitWorktreeToolInput,
    session_id: Option<i64>,
) -> Result<BuiltinExecution, ToolError> {
    let session_id = session_id.ok_or_else(|| {
        ToolError::Plugin("exit_worktree: no session in execution context".to_string())
    })?;
    let registry = executor
        .worktree_registry()
        .ok_or_else(|| ToolError::Plugin("exit_worktree: registry not configured".to_string()))?;

    let session = registry
        .write()
        .remove(&session_id)
        .ok_or_else(|| ToolError::Plugin("exit_worktree: not in a worktree".to_string()))?;

    let action = input.action.trim();
    if action == "remove" && session.created_here {
        if !input.discard_changes {
            let dirty = git_is_dirty(&session.path)?;
            if dirty {
                // Re-insert so the model can recover.
                registry.write().insert(session_id, session.clone());
                return Err(ToolError::Plugin(
                    "exit_worktree: worktree has uncommitted changes; \
                     re-call with `discard_changes: true` to force removal"
                        .to_string(),
                ));
            }
        }
        // remove the worktree, then delete the branch.
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
    }

    let view = ToolExecutionView::simple(
        format!("Worktree exited ({action})"),
        format!(
            "Worktree at {} (branch {}) — action: {action}",
            session.path.display(),
            session.branch
        ),
    );
    Ok(BuiltinExecution::new(
        BuiltinToolOutput::ExitWorktree {
            action: action.to_string(),
            path: session.path.to_string_lossy().to_string(),
        },
        view,
    ))
}

fn create_new(workspace: &Path, name: Option<&str>) -> Result<WorktreeSession, ToolError> {
    let slug = name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(generate_slug);
    let base = workspace.join(".agena").join("worktrees");
    std::fs::create_dir_all(&base)
        .map_err(|e| ToolError::Plugin(format!("enter_worktree: mkdir {base:?}: {e}")))?;
    let target = base.join(&slug);
    if target.exists() {
        return Err(ToolError::Plugin(format!(
            "enter_worktree: target {target:?} already exists"
        )));
    }
    let branch = format!("agena/{slug}");
    git(
        workspace,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            target.to_string_lossy().as_ref(),
        ],
    )?;

    Ok(WorktreeSession {
        path: target,
        branch,
        original_workspace: workspace.to_path_buf(),
        created_here: true,
    })
}

fn enter_existing(workspace: &Path, path: &str) -> Result<WorktreeSession, ToolError> {
    let path_buf = PathBuf::from(path);
    if !path_buf.exists() {
        return Err(ToolError::Plugin(format!(
            "enter_worktree: path {path:?} does not exist"
        )));
    }
    let listing = git(workspace, &["worktree", "list", "--porcelain"])?;
    let listing = String::from_utf8_lossy(&listing.stdout).to_string();
    let canonical = path_buf
        .canonicalize()
        .unwrap_or(path_buf.clone())
        .to_string_lossy()
        .to_string();
    let mut found_branch = None;
    let mut current_path: Option<String> = None;
    for line in listing.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current_path = Some(p.to_string());
        } else if let Some(b) = line.strip_prefix("branch ")
            && let Some(cp) = current_path.as_deref()
        {
            let cp_canonical = PathBuf::from(cp)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(cp))
                .to_string_lossy()
                .to_string();
            if cp_canonical == canonical || cp == path {
                found_branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            }
        }
    }
    let branch = found_branch.ok_or_else(|| {
        ToolError::Plugin(format!(
            "enter_worktree: {path:?} is not a registered worktree of this repo"
        ))
    })?;
    Ok(WorktreeSession {
        path: path_buf,
        branch,
        original_workspace: workspace.to_path_buf(),
        created_here: false,
    })
}

fn git_is_dirty(path: &Path) -> Result<bool, ToolError> {
    let out = git(path, &["status", "--porcelain"])?;
    Ok(!out.stdout.is_empty())
}

fn git(cwd: &Path, args: &[&str]) -> Result<Output, ToolError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| ToolError::Plugin(format!("git {args:?}: {e}")))?;
    if !out.status.success() {
        return Err(ToolError::Plugin(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(out)
}

fn generate_slug() -> String {
    let now = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    format!("wt-{now}")
}

// ---------------------------------------------------------------------------
// Public lifecycle API used by TUI / CLI / `/worktrees` slash command.
// ---------------------------------------------------------------------------

/// One entry in the in-memory session → worktree map.
#[derive(Debug, Clone)]
pub struct ActiveWorktree {
    pub session_id: i64,
    pub path: PathBuf,
    pub branch: String,
    pub created_here: bool,
}

/// One entry from the on-disk `.agena/worktrees/` scan, cross-referenced
/// against the registry and `git worktree list`.
#[derive(Debug, Clone)]
pub struct ManagedWorktree {
    pub path: PathBuf,
    pub session_id: Option<i64>,
    pub branch: Option<String>,
    pub registered_with_git: bool,
}

impl ManagedWorktree {
    /// True when no live session owns this directory and git no longer
    /// tracks it. These are orphans `prune_stale` will remove.
    pub fn is_stale(&self) -> bool {
        self.session_id.is_none() && !self.registered_with_git
    }
}

/// Snapshot every session that currently holds a worktree.
pub fn list_active(registry: &WorktreeRegistry) -> Vec<ActiveWorktree> {
    let mut out: Vec<ActiveWorktree> = registry
        .read()
        .iter()
        .map(|(session_id, session)| ActiveWorktree {
            session_id: *session_id,
            path: session.path.clone(),
            branch: session.branch.clone(),
            created_here: session.created_here,
        })
        .collect();
    out.sort_by_key(|entry| entry.session_id);
    out
}

/// Walk `<workspace>/.agena/worktrees/` and report every directory there,
/// joined with what the registry / git know about it.
pub fn list_managed(workspace: &Path, registry: &WorktreeRegistry) -> Vec<ManagedWorktree> {
    let base = workspace.join(".agena").join("worktrees");
    let mut out: Vec<ManagedWorktree> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&base) else {
        return out;
    };

    let registered_paths: std::collections::HashMap<PathBuf, i64> = registry
        .read()
        .iter()
        .map(|(session_id, session)| (session.path.clone(), *session_id))
        .collect();

    let git_paths: std::collections::HashSet<PathBuf> = scan_git_worktrees(workspace);

    for dirent in entries.flatten() {
        let path = dirent.path();
        if !path.is_dir() {
            continue;
        }
        let session_id = registered_paths
            .iter()
            .find(|(p, _)| paths_equal(p.as_path(), &path))
            .map(|(_, id)| *id);
        let registered_with_git = git_paths.iter().any(|p| paths_equal(p, &path));
        let branch = registry
            .read()
            .iter()
            .find_map(|(_, s)| paths_equal(&s.path, &path).then(|| s.branch.clone()));
        out.push(ManagedWorktree {
            path,
            session_id,
            branch,
            registered_with_git,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Drop every managed directory that has no live session AND is not
/// registered with `git worktree list`. Returns the paths that were
/// removed.
///
/// `discard_changes` is currently unused for the orphan path — by
/// definition we already know git does not track these directories — but
/// is preserved on the API to leave room for a future "force-remove
/// dirty managed worktrees" mode.
pub fn prune_stale(workspace: &Path, registry: &WorktreeRegistry) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for entry in list_managed(workspace, registry) {
        if !entry.is_stale() {
            continue;
        }
        if std::fs::remove_dir_all(&entry.path).is_ok() {
            removed.push(entry.path);
        } else {
            tracing::warn!(
                target: "agena::worktree",
                "prune_stale: failed to remove {}",
                entry.path.display()
            );
        }
    }
    removed
}

fn scan_git_worktrees(workspace: &Path) -> std::collections::HashSet<PathBuf> {
    let mut out = std::collections::HashSet::new();
    let Ok(output) = git(workspace, &["worktree", "list", "--porcelain"]) else {
        return out;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            let buf = PathBuf::from(p);
            let canonical = buf.canonicalize().unwrap_or(buf);
            out.insert(canonical);
        }
    }
    out
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let ca = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let cb = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_workspace(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agena-wt-{label}-{suffix}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn list_active_returns_entries_sorted_by_session_id() {
        let registry = registry_for_executor();
        registry.write().insert(
            42,
            WorktreeSession {
                path: PathBuf::from("/tmp/a"),
                branch: "agena/a".into(),
                original_workspace: PathBuf::from("/tmp/repo"),
                created_here: true,
            },
        );
        registry.write().insert(
            7,
            WorktreeSession {
                path: PathBuf::from("/tmp/b"),
                branch: "agena/b".into(),
                original_workspace: PathBuf::from("/tmp/repo"),
                created_here: false,
            },
        );

        let listed = list_active(&registry);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].session_id, 7);
        assert_eq!(listed[1].session_id, 42);
        assert!(!listed[0].created_here);
        assert!(listed[1].created_here);
    }

    #[test]
    fn list_managed_returns_empty_when_no_dir() {
        let workspace = tmp_workspace("no-managed-dir");
        let registry = registry_for_executor();
        let listed = list_managed(&workspace, &registry);
        assert!(listed.is_empty());
    }

    #[test]
    fn list_managed_marks_orphan_dirs_as_stale() {
        let workspace = tmp_workspace("stale");
        let base = workspace.join(".agena").join("worktrees");
        std::fs::create_dir_all(base.join("orphan-1")).unwrap();
        std::fs::create_dir_all(base.join("orphan-2")).unwrap();
        // Not a git repo, so scan_git_worktrees returns empty.
        let registry = registry_for_executor();

        let listed = list_managed(&workspace, &registry);
        assert_eq!(listed.len(), 2);
        for entry in &listed {
            assert!(entry.is_stale(), "expected orphan to be stale: {entry:?}");
            assert!(entry.session_id.is_none());
            assert!(!entry.registered_with_git);
        }
    }

    #[test]
    fn list_managed_recognises_active_session_path() {
        let workspace = tmp_workspace("active");
        let base = workspace.join(".agena").join("worktrees");
        let active_dir = base.join("alive");
        std::fs::create_dir_all(&active_dir).unwrap();

        let registry = registry_for_executor();
        registry.write().insert(
            99,
            WorktreeSession {
                path: active_dir.clone(),
                branch: "agena/alive".into(),
                original_workspace: workspace.clone(),
                created_here: true,
            },
        );

        let listed = list_managed(&workspace, &registry);
        let entry = listed
            .iter()
            .find(|e| paths_equal(&e.path, &active_dir))
            .expect("alive dir present");
        assert_eq!(entry.session_id, Some(99));
        assert_eq!(entry.branch.as_deref(), Some("agena/alive"));
        assert!(!entry.is_stale(), "live session should not be stale");
    }

    #[test]
    fn prune_stale_removes_only_orphan_dirs() {
        let workspace = tmp_workspace("prune");
        let base = workspace.join(".agena").join("worktrees");
        let live = base.join("live");
        let dead = base.join("dead");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::create_dir_all(&dead).unwrap();

        let registry = registry_for_executor();
        registry.write().insert(
            1,
            WorktreeSession {
                path: live.clone(),
                branch: "agena/live".into(),
                original_workspace: workspace.clone(),
                created_here: true,
            },
        );

        let removed = prune_stale(&workspace, &registry);
        assert_eq!(removed.len(), 1);
        assert!(paths_equal(&removed[0], &dead));
        assert!(live.exists(), "live worktree must survive prune");
        assert!(!dead.exists(), "stale worktree must be removed");
    }
}
