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

use crate::message::{
    BuiltinToolOutput, EnterWorktreeToolInput, ExitWorktreeToolInput,
};

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
        let _ = git(&session.original_workspace, &[
            "worktree",
            "remove",
            "--force",
            session.path.to_string_lossy().as_ref(),
        ])?;
        let _ = git(&session.original_workspace, &["branch", "-D", &session.branch]);
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
    std::fs::create_dir_all(&base).map_err(|e| {
        ToolError::Plugin(format!("enter_worktree: mkdir {base:?}: {e}"))
    })?;
    let target = base.join(&slug);
    if target.exists() {
        return Err(ToolError::Plugin(format!(
            "enter_worktree: target {target:?} already exists"
        )));
    }
    let branch = format!("agena/{slug}");
    git(workspace, &[
        "worktree",
        "add",
        "-b",
        &branch,
        target.to_string_lossy().as_ref(),
    ])?;

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
        } else if let Some(b) = line.strip_prefix("branch ") {
            if let Some(cp) = current_path.as_deref() {
                let cp_canonical = PathBuf::from(cp)
                    .canonicalize()
                    .unwrap_or_else(|_| PathBuf::from(cp))
                    .to_string_lossy()
                    .to_string();
                if cp_canonical == canonical || cp == path {
                    found_branch =
                        Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
                }
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
