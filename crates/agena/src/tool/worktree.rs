//! `enter_worktree` / `exit_worktree` plugin tools.
//!
//! Agena now prefers `rift` snapshots when available and falls back to
//! `git worktree` when Rift cannot be used. Both backends write into the
//! same managed directory under `~/agena/projects/<workspace-key>/worktrees`,
//! so the model-facing workflow stays stable while Rift can reduce clone
//! storage through copy-on-write snapshots and filtered copies.
//!
//! State is held in a process-wide registry keyed by session id so
//! `enter` and `exit` can refer to the same managed workspace across
//! multiple tool invocations.

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::message::{EnterWorktreeToolInput, ExitWorktreeToolInput};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeBackend {
    Rift,
    Git,
}

impl WorktreeBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rift => "rift",
            Self::Git => "git_worktree",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorktreeBackendSupport {
    pub backend: WorktreeBackend,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct WorktreeBackendCapabilities {
    pub preferred_backend: Option<WorktreeBackend>,
    pub git: WorktreeBackendSupport,
    pub rift: WorktreeBackendSupport,
}

impl WorktreeBackendCapabilities {
    pub fn for_backend(&self, backend: WorktreeBackend) -> &WorktreeBackendSupport {
        match backend {
            WorktreeBackend::Rift => &self.rift,
            WorktreeBackend::Git => &self.git,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorktreeSession {
    pub path: PathBuf,
    pub branch: String,
    pub original_workspace: PathBuf,
    pub backend: WorktreeBackend,
    /// True when we created the worktree (and so are responsible for
    /// cleaning it up on `remove`).
    pub created_here: bool,
}

pub type WorktreeRegistry = Arc<RwLock<std::collections::HashMap<i64, WorktreeSession>>>;

pub fn registry_for_executor() -> WorktreeRegistry {
    Arc::new(RwLock::new(std::collections::HashMap::new()))
}

#[derive(Debug, Clone)]
struct EnterWorkspaceResolution {
    session: WorktreeSession,
    note: Option<String>,
}

impl EnterWorkspaceResolution {
    fn without_note(session: WorktreeSession) -> Self {
        Self {
            session,
            note: None,
        }
    }
}

pub(super) fn execute_enter(
    executor: &ToolExecutor,
    input: &EnterWorktreeToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
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

    let resolution = if let Some(path) = input.path.as_deref() {
        EnterWorkspaceResolution::without_note(enter_existing(&workspace, path)?)
    } else {
        create_new(&workspace, input.name.as_deref())?
    };
    let EnterWorkspaceResolution { session, note } = resolution;
    let note_line = note
        .as_deref()
        .map(|note| format!("  note:    {note}\n"))
        .unwrap_or_default();

    let view = ToolExecutionView::simple(
        format!("Workspace → {}", session.path.display()),
        format!(
            "Switched to managed workspace:\n  backend: {}\n  path:    {}\n  branch:  {}\n{}",
            session.backend.as_str(),
            session.path.display(),
            session.branch,
            note_line,
        ),
    );
    let output = ToolPayloadOutput::EnterWorktree {
        path: session.path.to_string_lossy().to_string(),
        branch: session.branch.clone(),
        backend: Some(session.backend.as_str().to_string()),
        note,
    };

    registry.write().insert(session_id, session);
    Ok(ToolPayloadExecution::new(output, view))
}

pub(super) fn execute_exit(
    executor: &ToolExecutor,
    input: &ExitWorktreeToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
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
        if let Err(error) = remove_created_workspace(executor, &session, input.discard_changes) {
            registry.write().insert(session_id, session.clone());
            return Err(error);
        }
    }

    let view = ToolExecutionView::simple(
        format!("Workspace exited ({action})"),
        format!(
            "Workspace at {} (backend {}, branch {}) — action: {action}",
            session.path.display(),
            session.backend.as_str(),
            session.branch,
        ),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::ExitWorktree {
            action: action.to_string(),
            path: session.path.to_string_lossy().to_string(),
        },
        view,
    ))
}

fn create_new(workspace: &Path, name: Option<&str>) -> Result<EnterWorkspaceResolution, ToolError> {
    let slug = name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(generate_slug);
    let base = managed_worktrees_dir(workspace);
    std::fs::create_dir_all(&base)
        .map_err(|error| ToolError::Plugin(format!("enter_worktree: mkdir {base:?}: {error}")))?;
    let target = base.join(&slug);
    if target.exists() {
        return Err(ToolError::Plugin(format!(
            "enter_worktree: target {target:?} already exists"
        )));
    }

    let capabilities = backend_capabilities(workspace);
    let (rift_failure, note) = if capabilities.rift.available {
        match try_create_new_with_rift(workspace, &base, &slug) {
            Ok(session) => return Ok(EnterWorkspaceResolution::without_note(session)),
            Err(error) => {
                tracing::info!(
                    target: "agena::worktree",
                    workspace = %workspace.display(),
                    slug = %slug,
                    error = %error,
                    "falling back to git worktree because the rift backend could not be used"
                );
                (
                    Some(error.clone()),
                    Some(format!(
                        "used git_worktree because Rift could not create a snapshot here: {error}"
                    )),
                )
            }
        }
    } else {
        (
            Some(capabilities.rift.detail.clone()),
            capabilities.git.available.then(|| {
                format!(
                    "used git_worktree because Rift is unavailable: {}",
                    capabilities.rift.detail
                )
            }),
        )
    };

    if !capabilities.git.available {
        return Err(create_new_backend_error(
            &capabilities,
            rift_failure.as_deref(),
            None,
        ));
    }

    match create_new_with_git(workspace, &target, &slug) {
        Ok(session) => Ok(EnterWorkspaceResolution { session, note }),
        Err(git_error) => {
            let git_failure = git_error.to_string();
            Err(create_new_backend_error(
                &capabilities,
                rift_failure.as_deref(),
                Some(git_failure.as_str()),
            ))
        }
    }
}

fn create_new_backend_error(
    capabilities: &WorktreeBackendCapabilities,
    rift_failure: Option<&str>,
    git_failure: Option<&str>,
) -> ToolError {
    let preferred = capabilities
        .preferred_backend
        .map(WorktreeBackend::as_str)
        .unwrap_or("none");
    let mut detail = vec![
        format!("preferred backend: {preferred}"),
        format!(
            "rift: available={} | {}",
            capabilities.rift.available, capabilities.rift.detail
        ),
        format!(
            "git_worktree: available={} | {}",
            capabilities.git.available, capabilities.git.detail
        ),
    ];
    if let Some(rift_failure) = rift_failure {
        detail.push(format!("rift create attempt: {rift_failure}"));
    }
    if let Some(git_failure) = git_failure {
        detail.push(format!("git worktree create attempt: {git_failure}"));
    }
    ToolError::Plugin(format!(
        "enter_worktree: could not create a managed workspace\n  {}",
        detail.join("\n  ")
    ))
}

pub fn backend_capabilities(workspace: &Path) -> WorktreeBackendCapabilities {
    let git = probe_git_backend(workspace);
    let rift = probe_rift_backend();
    let preferred_backend = if rift.available {
        Some(WorktreeBackend::Rift)
    } else if git.available {
        Some(WorktreeBackend::Git)
    } else {
        None
    };
    WorktreeBackendCapabilities {
        preferred_backend,
        git,
        rift,
    }
}

fn probe_git_backend(workspace: &Path) -> WorktreeBackendSupport {
    if let Err(detail) = probe_command_presence("git", &["--version"], "git CLI") {
        return WorktreeBackendSupport {
            backend: WorktreeBackend::Git,
            available: false,
            detail,
        };
    }

    let output = match Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return WorktreeBackendSupport {
                backend: WorktreeBackend::Git,
                available: false,
                detail: format!(
                    "failed to execute `git` in {}: {error}",
                    workspace.display()
                ),
            };
        }
    };

    if output.status.success() {
        WorktreeBackendSupport {
            backend: WorktreeBackend::Git,
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
        WorktreeBackendSupport {
            backend: WorktreeBackend::Git,
            available: false,
            detail,
        }
    }
}

fn probe_rift_backend() -> WorktreeBackendSupport {
    let binary = rift_bin();
    match probe_command_presence(binary.as_str(), &["--help"], "Rift CLI") {
        Ok(()) => WorktreeBackendSupport {
            backend: WorktreeBackend::Rift,
            available: true,
            detail: format!(
                "Rift CLI `{binary}` is available; filesystem and repository compatibility are verified when a snapshot is created"
            ),
        },
        Err(detail) => WorktreeBackendSupport {
            backend: WorktreeBackend::Rift,
            available: false,
            detail,
        },
    }
}

fn probe_command_presence(command: &str, args: &[&str], label: &str) -> Result<(), String> {
    match Command::new(command).args(args).output() {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Err(format!("{label} `{command}` was not found on PATH"))
        }
        Err(error) => Err(format!("failed to start {label} `{command}`: {error}")),
    }
}

fn create_new_with_git(
    workspace: &Path,
    target: &Path,
    slug: &str,
) -> Result<WorktreeSession, ToolError> {
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
        path: target.to_path_buf(),
        branch,
        original_workspace: workspace.to_path_buf(),
        backend: WorktreeBackend::Git,
        created_here: true,
    })
}

fn try_create_new_with_rift(
    workspace: &Path,
    base: &Path,
    slug: &str,
) -> Result<WorktreeSession, String> {
    let db_path = rift_database_path(workspace);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to prepare Rift database directory: {error}"))?;
    }

    let workspace_str = workspace.to_string_lossy().to_string();
    let base_str = base.to_string_lossy().to_string();
    rift(
        workspace,
        &db_path,
        &["init", "--here", workspace_str.as_str()],
    )
    .map_err(|error| error.to_string())?;
    let output = rift(
        workspace,
        &db_path,
        &[
            "create",
            workspace_str.as_str(),
            "--name",
            slug,
            "--into",
            base_str.as_str(),
            "--no-hooks",
        ],
    )
    .map_err(|error| error.to_string())?;

    let created_path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| base.join(slug));
    let branch = ensure_rift_branch(&created_path, slug)
        .unwrap_or_else(|| describe_workspace_head(&created_path));

    Ok(WorktreeSession {
        path: created_path,
        branch,
        original_workspace: workspace.to_path_buf(),
        backend: WorktreeBackend::Rift,
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

    if is_managed_rift_workspace(workspace, &path_buf) {
        return Ok(WorktreeSession {
            path: path_buf.clone(),
            branch: describe_workspace_head(&path_buf),
            original_workspace: workspace.to_path_buf(),
            backend: WorktreeBackend::Rift,
            created_here: false,
        });
    }

    let capabilities = backend_capabilities(workspace);
    if !capabilities.git.available {
        return Err(ToolError::Plugin(format!(
            "enter_worktree: cannot attach existing git worktree: {}",
            capabilities.git.detail
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
        if let Some(path_value) = line.strip_prefix("worktree ") {
            current_path = Some(path_value.to_string());
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
                        .to_string(),
                );
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
        backend: WorktreeBackend::Git,
        created_here: false,
    })
}

fn remove_created_workspace(
    executor: &ToolExecutor,
    session: &WorktreeSession,
    discard_changes: bool,
) -> Result<(), ToolError> {
    executor.ensure_read_permission(&session.path)?;
    executor.ensure_edit_permission(&session.path)?;
    if !discard_changes && workspace_has_local_changes(&session.path)? {
        return Err(ToolError::Plugin(
            "exit_worktree: workspace has local changes; re-call with `discard_changes: true` to force removal"
                .to_string(),
        ));
    }

    match session.backend {
        WorktreeBackend::Rift => remove_with_rift(&session.original_workspace, &session.path),
        WorktreeBackend::Git => remove_with_git(&session.original_workspace, session),
    }
}

fn remove_with_git(workspace: &Path, session: &WorktreeSession) -> Result<(), ToolError> {
    git(
        workspace,
        &[
            "worktree",
            "remove",
            "--force",
            session.path.to_string_lossy().as_ref(),
        ],
    )?;
    let _ = git(workspace, &["branch", "-D", &session.branch]);
    Ok(())
}

fn remove_with_rift(workspace: &Path, path: &Path) -> Result<(), ToolError> {
    let db_path = rift_database_path(workspace);
    let path_str = path.to_string_lossy().to_string();
    rift(workspace, &db_path, &["remove", path_str.as_str()])?;
    if let Err(error) = rift(workspace, &db_path, &["gc"]) {
        tracing::warn!(
            target: "agena::worktree",
            workspace = %workspace.display(),
            path = %path.display(),
            error = %error,
            "rift remove succeeded but garbage collection did not complete"
        );
    }
    Ok(())
}

fn workspace_has_local_changes(path: &Path) -> Result<bool, ToolError> {
    match git(path, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(_) => {
            let output = git(path, &["status", "--porcelain"])?;
            Ok(!output.stdout.is_empty())
        }
        Err(_) => Ok(false),
    }
}

fn git(cwd: &Path, args: &[&str]) -> Result<Output, ToolError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| ToolError::Plugin(format!("git {args:?}: {error}")))?;
    if !output.status.success() {
        return Err(ToolError::Plugin(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output)
}

fn rift(cwd: &Path, db_path: &Path, args: &[&str]) -> Result<Output, ToolError> {
    let db_path_str = db_path.to_string_lossy().to_string();
    let mut command = Command::new(rift_bin());
    command.arg("--database").arg(db_path_str);
    command.args(args).current_dir(cwd);
    let output = command
        .output()
        .map_err(|error| ToolError::Plugin(format!("rift {args:?}: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(ToolError::Plugin(format!("rift {args:?} failed: {detail}")));
    }
    Ok(output)
}

fn rift_bin() -> String {
    std::env::var("AGENA_RIFT_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "rift".to_string())
}

fn managed_worktrees_dir(workspace: &Path) -> PathBuf {
    crate::project_paths::project_state_dir(workspace).join("worktrees")
}

fn rift_database_path(workspace: &Path) -> PathBuf {
    crate::project_paths::project_state_dir(workspace).join("rift.sqlite")
}

fn is_managed_rift_workspace(workspace: &Path, path: &Path) -> bool {
    if !has_rift_marker(path) {
        return false;
    }
    let base = managed_worktrees_dir(workspace);
    let canonical_base = base.canonicalize().unwrap_or(base);
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical_path.starts_with(canonical_base)
}

fn has_rift_marker(path: &Path) -> bool {
    path.join(".rift").is_file()
}

fn ensure_rift_branch(path: &Path, slug: &str) -> Option<String> {
    let branch = format!("agena/{slug}");
    if git(path, &["switch", "-c", branch.as_str()]).is_ok() {
        return Some(branch);
    }
    if git(path, &["checkout", "-b", branch.as_str()]).is_ok() {
        return Some(branch);
    }
    None
}

fn describe_workspace_head(path: &Path) -> String {
    git_current_branch(path)
        .or_else(|| git_detached_head_label(path))
        .unwrap_or_else(|| "snapshot".to_string())
}

fn git_current_branch(path: &Path) -> Option<String> {
    let output = git(path, &["branch", "--show-current"]).ok()?;
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}

fn git_detached_head_label(path: &Path) -> Option<String> {
    let output = git(path, &["rev-parse", "--short", "HEAD"]).ok()?;
    let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!head.is_empty()).then(|| format!("detached@{head}"))
}

fn generate_slug() -> String {
    let now = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    format!("wt-{now}")
}

// ---------------------------------------------------------------------------
// Public lifecycle API used by TUI / CLI / `/worktrees` slash command.
// ---------------------------------------------------------------------------

/// One record in the in-memory session → worktree map.
#[derive(Debug, Clone)]
pub struct ActiveWorktree {
    pub session_id: i64,
    pub path: PathBuf,
    pub branch: String,
    pub backend: WorktreeBackend,
    pub created_here: bool,
}

/// One record from the on-disk managed worktrees scan, cross-referenced
/// against the registry plus both managed backends.
#[derive(Debug, Clone)]
pub struct ManagedWorktree {
    pub path: PathBuf,
    pub session_id: Option<i64>,
    pub branch: Option<String>,
    pub backend: Option<WorktreeBackend>,
    pub registered_with_git: bool,
    pub registered_with_rift: bool,
}

impl ManagedWorktree {
    /// True when no live session owns this directory and neither managed
    /// backend still recognizes it. These are orphans `prune_stale` will remove.
    pub fn is_stale(&self) -> bool {
        self.session_id.is_none() && !self.registered_with_git && !self.registered_with_rift
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
            backend: session.backend,
            created_here: session.created_here,
        })
        .collect();
    out.sort_by_key(|entry| entry.session_id);
    out
}

/// Walk the home-level managed worktrees directory and report every directory there,
/// joined with what the registry, Rift markers, and `git worktree list` know about it.
pub fn list_managed(workspace: &Path, registry: &WorktreeRegistry) -> Vec<ManagedWorktree> {
    let base = managed_worktrees_dir(workspace);
    let mut out: Vec<ManagedWorktree> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&base) else {
        return out;
    };

    let sessions_by_path: HashMap<PathBuf, WorktreeSession> = registry
        .read()
        .iter()
        .map(|(_session_id, session)| (session.path.clone(), session.clone()))
        .collect();
    let session_ids_by_path: HashMap<PathBuf, i64> = registry
        .read()
        .iter()
        .map(|(session_id, session)| (session.path.clone(), *session_id))
        .collect();
    let git_paths: HashSet<PathBuf> = scan_git_worktrees(workspace);

    for dirent in entries.flatten() {
        let path = dirent.path();
        if !path.is_dir() || is_reserved_managed_entry(&path) {
            continue;
        }

        let session_id = session_ids_by_path
            .iter()
            .find(|(candidate, _)| paths_equal(candidate.as_path(), &path))
            .map(|(_, session_id)| *session_id);
        let active_session = sessions_by_path
            .iter()
            .find_map(|(candidate, session)| paths_equal(candidate, &path).then_some(session));
        let registered_with_git = git_paths
            .iter()
            .any(|candidate| paths_equal(candidate, &path));
        let registered_with_rift = has_rift_marker(&path);
        let backend = active_session.map(|session| session.backend).or_else(|| {
            if registered_with_rift {
                Some(WorktreeBackend::Rift)
            } else if registered_with_git {
                Some(WorktreeBackend::Git)
            } else {
                None
            }
        });
        let branch = active_session
            .map(|session| session.branch.clone())
            .or_else(|| {
                (registered_with_git || registered_with_rift)
                    .then(|| describe_workspace_head(&path))
            });

        out.push(ManagedWorktree {
            path,
            session_id,
            branch,
            backend,
            registered_with_git,
            registered_with_rift,
        });
    }
    out.sort_by(|left, right| left.path.cmp(&right.path));
    out
}

/// Drop every managed directory that has no live session AND is no longer
/// registered with either managed backend. Also triggers a best-effort
/// Rift garbage collection pass so removed snapshots do not linger in
/// `.trash`. Returns the paths that were removed.
pub fn prune_stale(workspace: &Path, registry: &WorktreeRegistry) -> Vec<PathBuf> {
    let mut removed = rift_gc(workspace);
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

fn rift_gc(workspace: &Path) -> Vec<PathBuf> {
    let db_path = rift_database_path(workspace);
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

fn scan_git_worktrees(workspace: &Path) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    let Ok(output) = git(workspace, &["worktree", "list", "--porcelain"]) else {
        return out;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(path_value) = line.strip_prefix("worktree ") {
            let path = PathBuf::from(path_value);
            let canonical = path.canonicalize().unwrap_or(path);
            out.insert(canonical);
        }
    }
    out
}

fn is_reserved_managed_entry(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".trash")
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let canonical_a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let canonical_b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    canonical_a == canonical_b
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::sync::{Mutex, OnceLock};

    use tempfile::TempDir;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct TestHomeGuard {
        home: Option<String>,
        userprofile: Option<String>,
    }

    impl TestHomeGuard {
        fn set(path: &Path) -> Self {
            let value = path.to_string_lossy().to_string();
            let home = std::env::var("HOME").ok();
            let userprofile = std::env::var("USERPROFILE").ok();
            // SAFETY: tests serialize access to process-global environment via `env_lock`.
            unsafe {
                std::env::set_var("HOME", &value);
                std::env::set_var("USERPROFILE", &value);
            }
            Self { home, userprofile }
        }
    }

    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            // SAFETY: tests serialize access to process-global environment via `env_lock`.
            unsafe {
                if let Some(home) = &self.home {
                    std::env::set_var("HOME", home);
                } else {
                    std::env::remove_var("HOME");
                }
                if let Some(userprofile) = &self.userprofile {
                    std::env::set_var("USERPROFILE", userprofile);
                } else {
                    std::env::remove_var("USERPROFILE");
                }
            }
        }
    }

    struct RiftBinGuard {
        previous: Option<String>,
    }

    impl RiftBinGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::var("AGENA_RIFT_BIN").ok();
            // SAFETY: tests serialize access to process-global environment via `env_lock`.
            unsafe {
                std::env::set_var("AGENA_RIFT_BIN", path);
            }
            Self { previous }
        }
    }

    impl Drop for RiftBinGuard {
        fn drop(&mut self) {
            // SAFETY: tests serialize access to process-global environment via `env_lock`.
            unsafe {
                if let Some(previous) = &self.previous {
                    std::env::set_var("AGENA_RIFT_BIN", previous);
                } else {
                    std::env::remove_var("AGENA_RIFT_BIN");
                }
            }
        }
    }

    #[test]
    fn create_new_prefers_rift_when_available() {
        let _env_guard = env_lock().lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _home_guard = TestHomeGuard::set(temp.path());

        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let fake_rift = temp.path().join("fake-rift.sh");
        fs::write(
            &fake_rift,
            r#"#!/usr/bin/env bash
set -euo pipefail
args=("$@")
cmd=""
cmd_index=0
for ((i=0; i<${#args[@]}; i++)); do
  case "${args[$i]}" in
    init|create|remove|gc)
      cmd="${args[$i]}"
      cmd_index=$i
      break
      ;;
  esac
done
if [[ "$cmd" == "init" ]]; then
  target="${args[$((cmd_index + 2))]}"
  touch "$target/.rift"
  exit 0
fi
if [[ "$cmd" == "create" ]]; then
  name=""
  into=""
  for ((i=cmd_index + 1; i<${#args[@]}; i++)); do
    if [[ "${args[$i]}" == "--name" ]]; then
      name="${args[$((i + 1))]}"
    elif [[ "${args[$i]}" == "--into" ]]; then
      into="${args[$((i + 1))]}"
    fi
  done
  mkdir -p "$into/$name"
  touch "$into/$name/.rift"
  printf '%s\n' "$into/$name"
  exit 0
fi
exit 0
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&fake_rift).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&fake_rift, perms).unwrap();
        }
        let _rift_guard = RiftBinGuard::set(&fake_rift);

        let created = create_new(&workspace, Some("demo")).unwrap();
        let EnterWorkspaceResolution { session, note } = created;

        assert_eq!(session.backend, WorktreeBackend::Rift);
        assert!(session.created_here);
        assert_eq!(session.path, managed_worktrees_dir(&workspace).join("demo"));
        assert_eq!(session.branch, "snapshot");
        assert!(note.is_none());
    }

    #[test]
    fn create_new_falls_back_to_git_when_rift_fails() {
        let _env_guard = env_lock().lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _home_guard = TestHomeGuard::set(temp.path());

        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let fake_rift = temp.path().join("fake-rift.sh");
        fs::write(
            &fake_rift,
            "#!/usr/bin/env bash\nset -euo pipefail\nprintf 'unsupported filesystem\\n' >&2\nexit 1\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&fake_rift).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&fake_rift, perms).unwrap();
        }
        let _rift_guard = RiftBinGuard::set(&fake_rift);

        let output = Command::new("git")
            .args(["init"])
            .current_dir(&workspace)
            .output()
            .unwrap();
        assert!(output.status.success());
        fs::write(workspace.join("README.md"), "hello\n").unwrap();
        let output = Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&workspace)
            .output()
            .unwrap();
        assert!(output.status.success());
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Agena Test",
                "-c",
                "user.email=agena@example.com",
                "commit",
                "-m",
                "init",
            ])
            .current_dir(&workspace)
            .output()
            .unwrap();
        assert!(output.status.success());

        let created = create_new(&workspace, Some("demo")).unwrap();
        let EnterWorkspaceResolution { session, note } = created;

        assert_eq!(session.backend, WorktreeBackend::Git);
        assert_eq!(session.branch, "agena/demo");
        assert!(session.path.exists());
        assert!(
            note.as_deref()
                .is_some_and(|note| note.contains("unsupported filesystem"))
        );
    }

    #[test]
    fn backend_capabilities_report_preferred_backend_and_rift_caveat() {
        let _env_guard = env_lock().lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _home_guard = TestHomeGuard::set(temp.path());

        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let fake_rift = temp.path().join("fake-rift.sh");
        fs::write(&fake_rift, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&fake_rift).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&fake_rift, perms).unwrap();
        }
        let _rift_guard = RiftBinGuard::set(&fake_rift);

        let output = Command::new("git")
            .args(["init"])
            .current_dir(&workspace)
            .output()
            .unwrap();
        assert!(output.status.success());

        let capabilities = backend_capabilities(&workspace);

        assert_eq!(capabilities.preferred_backend, Some(WorktreeBackend::Rift));
        assert!(capabilities.git.available);
        assert!(capabilities.rift.available);
        assert!(capabilities.rift.detail.contains(
            "filesystem and repository compatibility are verified when a snapshot is created"
        ));
    }

    #[test]
    fn list_managed_recognizes_rift_entries_and_skips_trash() {
        let _env_guard = env_lock().lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _home_guard = TestHomeGuard::set(temp.path());

        let workspace = temp.path().join("workspace");
        let base = managed_worktrees_dir(&workspace);
        let rift_dir = base.join("demo");
        let trash_dir = base.join(".trash");

        fs::create_dir_all(&rift_dir).unwrap();
        fs::create_dir_all(&trash_dir).unwrap();
        fs::write(rift_dir.join(".rift"), "marker").unwrap();

        let registry = registry_for_executor();
        let managed = list_managed(&workspace, &registry);

        assert_eq!(managed.len(), 1);
        assert_eq!(managed[0].path, rift_dir);
        assert_eq!(managed[0].backend, Some(WorktreeBackend::Rift));
        assert!(managed[0].registered_with_rift);
        assert!(!managed[0].is_stale());
    }
}
