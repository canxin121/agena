//! `enter_snapshot` / `exit_snapshot` plugin tools.
//!
//! Agena now prefers `rift` snapshots when available and falls back to
//! `git worktree` when Rift cannot be used. Both backends write into the
//! same managed directory under `~/agena/projects/<workspace-key>/snapshots`,
//! so the model-facing workflow stays stable while Rift can reduce clone
//! storage through copy-on-write snapshots and filtered copies.
//!
//! State is held in a process-wide registry keyed by session id so
//! `enter` and `exit` can refer to the same managed snapshot across
//! multiple tool invocations.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::message::{EnterSnapshotToolInput, ExitSnapshotToolInput};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotBackend {
    Rift,
    Git,
}

impl AsRef<str> for SnapshotBackend {
    fn as_ref(&self) -> &str {
        match self {
            Self::Rift => "rift",
            Self::Git => "git",
        }
    }
}

impl fmt::Display for SnapshotBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotBackendSupport {
    pub backend: SnapshotBackend,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct SnapshotBackendCapabilities {
    pub preferred_backend: Option<SnapshotBackend>,
    pub git: SnapshotBackendSupport,
    pub rift: SnapshotBackendSupport,
}

impl SnapshotBackendCapabilities {
    pub fn for_backend(&self, backend: SnapshotBackend) -> &SnapshotBackendSupport {
        match backend {
            SnapshotBackend::Rift => &self.rift,
            SnapshotBackend::Git => &self.git,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotSession {
    pub path: PathBuf,
    pub branch: String,
    pub original_workspace: PathBuf,
    pub backend: SnapshotBackend,
    /// True when we created the snapshot (and so are responsible for
    /// cleaning it up on `remove`).
    pub created_here: bool,
}

pub type SnapshotRegistry = Arc<RwLock<std::collections::HashMap<i64, SnapshotSession>>>;

pub fn registry_for_executor() -> SnapshotRegistry {
    Arc::new(RwLock::new(std::collections::HashMap::new()))
}

#[derive(Debug, Clone)]
struct EnterWorkspaceResolution {
    session: SnapshotSession,
    note: Option<String>,
}

impl EnterWorkspaceResolution {
    fn without_note(session: SnapshotSession) -> Self {
        Self {
            session,
            note: None,
        }
    }
}

pub(super) fn execute_enter(
    executor: &ToolExecutor,
    input: &EnterSnapshotToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
    let session_id = session_id.ok_or_else(|| {
        ToolError::Plugin("snapshot.enter: no session in execution context".to_string())
    })?;
    let registry = executor
        .snapshot_registry()
        .ok_or_else(|| ToolError::Plugin("snapshot.enter: registry not configured".to_string()))?;

    if registry.read().contains_key(&session_id) {
        return Err(ToolError::Plugin(
            "snapshot.enter: session is already in a snapshot; call `snapshot exit` first"
                .to_string(),
        ));
    }

    let workspace = executor.workspace_root().to_path_buf();
    if input.name.is_some() && input.path.is_some() {
        return Err(ToolError::Plugin(
            "snapshot.enter: provide either `name` or `path`, not both".to_string(),
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
        format!("Snapshot → {}", session.path.display()),
        format!(
            "Switched to managed snapshot:\n  backend: {}\n  path:    {}\n  branch:  {}\n{}",
            session.backend,
            session.path.display(),
            session.branch,
            note_line,
        ),
    );
    let output = ToolPayloadOutput::EnterSnapshot {
        path: session.path.to_string_lossy().to_string(),
        branch: session.branch.clone(),
        backend: Some(session.backend.to_string()),
        note,
    };

    registry.write().insert(session_id, session);
    Ok(ToolPayloadExecution::new(output, view))
}

pub(super) fn execute_exit(
    executor: &ToolExecutor,
    input: &ExitSnapshotToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
    let session_id = session_id.ok_or_else(|| {
        ToolError::Plugin("snapshot.exit: no session in execution context".to_string())
    })?;
    let registry = executor
        .snapshot_registry()
        .ok_or_else(|| ToolError::Plugin("snapshot.exit: registry not configured".to_string()))?;

    let session = registry
        .write()
        .remove(&session_id)
        .ok_or_else(|| ToolError::Plugin("snapshot.exit: not in a snapshot".to_string()))?;

    let action = input.action.trim();
    if action == "remove" && session.created_here {
        if let Err(error) = remove_created_workspace(executor, &session, input.discard_changes) {
            registry.write().insert(session_id, session.clone());
            return Err(error);
        }
    }

    let view = ToolExecutionView::simple(
        format!("Snapshot exited ({action})"),
        format!(
            "Snapshot at {} (backend {}, branch {}) — action: {action}",
            session.path.display(),
            session.backend,
            session.branch,
        ),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::ExitSnapshot {
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
    let base = managed_snapshots_dir(workspace);
    std::fs::create_dir_all(&base)
        .map_err(|error| ToolError::Plugin(format!("snapshot.enter: mkdir {base:?}: {error}")))?;
    let target = base.join(&slug);
    if target.exists() {
        return Err(ToolError::Plugin(format!(
            "snapshot.enter: target {target:?} already exists"
        )));
    }

    let capabilities = backend_capabilities(workspace);
    let (rift_failure, note) = if capabilities.rift.available {
        match try_create_new_with_rift(workspace, &base, &slug) {
            Ok(session) => return Ok(EnterWorkspaceResolution::without_note(session)),
            Err(error) => {
                tracing::info!(
                    target: "agena::snapshot",
                    workspace = %workspace.display(),
                    slug = %slug,
                    error = %error,
                    "falling back to git worktree because the rift backend could not be used"
                );
                (
                    Some(error.clone()),
                    Some(format!(
                        "used git backend because Rift could not create a snapshot here: {error}"
                    )),
                )
            }
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
    capabilities: &SnapshotBackendCapabilities,
    rift_failure: Option<&str>,
    git_failure: Option<&str>,
) -> ToolError {
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
    if let Some(rift_failure) = rift_failure {
        detail.push(format!("rift create attempt: {rift_failure}"));
    }
    if let Some(git_failure) = git_failure {
        detail.push(format!("git worktree create attempt: {git_failure}"));
    }
    ToolError::Plugin(format!(
        "snapshot.enter: could not create a managed snapshot\n  {}",
        detail.join("\n  ")
    ))
}

pub fn backend_capabilities(workspace: &Path) -> SnapshotBackendCapabilities {
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

    let output = match Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace)
        .output()
    {
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
    let binary = rift_bin();
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
) -> Result<SnapshotSession, ToolError> {
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

    Ok(SnapshotSession {
        path: target.to_path_buf(),
        branch,
        original_workspace: workspace.to_path_buf(),
        backend: SnapshotBackend::Git,
        created_here: true,
    })
}

fn try_create_new_with_rift(
    workspace: &Path,
    base: &Path,
    slug: &str,
) -> Result<SnapshotSession, String> {
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

    Ok(SnapshotSession {
        path: created_path,
        branch,
        original_workspace: workspace.to_path_buf(),
        backend: SnapshotBackend::Rift,
        created_here: true,
    })
}

fn enter_existing(workspace: &Path, path: &str) -> Result<SnapshotSession, ToolError> {
    let path_buf = PathBuf::from(path);
    if !path_buf.exists() {
        return Err(ToolError::Plugin(format!(
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

    let capabilities = backend_capabilities(workspace);
    if !capabilities.git.available {
        return Err(ToolError::Plugin(format!(
            "snapshot.enter: cannot attach existing git worktree: {}",
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

fn remove_created_workspace(
    executor: &ToolExecutor,
    session: &SnapshotSession,
    discard_changes: bool,
) -> Result<(), ToolError> {
    executor.ensure_read_permission(&session.path)?;
    executor.ensure_edit_permission(&session.path)?;
    if !discard_changes && workspace_has_local_changes(&session.path)? {
        return Err(ToolError::Plugin(
            "snapshot.exit: snapshot has local changes; re-call with `discard_changes: true` to force removal"
                .to_string(),
        ));
    }

    match session.backend {
        SnapshotBackend::Rift => remove_with_rift(&session.original_workspace, &session.path),
        SnapshotBackend::Git => remove_with_git(&session.original_workspace, session),
    }
}

fn remove_with_git(workspace: &Path, session: &SnapshotSession) -> Result<(), ToolError> {
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
            target: "agena::snapshot",
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

fn managed_snapshots_dir(workspace: &Path) -> PathBuf {
    crate::project_paths::project_state_dir(workspace).join("snapshots")
}

fn rift_database_path(workspace: &Path) -> PathBuf {
    crate::project_paths::project_state_dir(workspace).join("rift.sqlite")
}

fn is_managed_rift_workspace(workspace: &Path, path: &Path) -> bool {
    if !has_rift_marker(path) {
        return false;
    }
    let base = managed_snapshots_dir(workspace);
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
    format!("snapshot-{now}")
}

// ---------------------------------------------------------------------------
// Public lifecycle API used by TUI / CLI / `/snapshot` slash command.
// ---------------------------------------------------------------------------

/// One record in the in-memory session → snapshot map.
#[derive(Debug, Clone)]
pub struct ActiveSnapshot {
    pub session_id: i64,
    pub path: PathBuf,
    pub branch: String,
    pub backend: SnapshotBackend,
    pub created_here: bool,
}

/// One record from the on-disk managed snapshots scan, cross-referenced
/// against the registry plus both managed backends.
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
    /// True when no live session owns this directory and neither managed
    /// backend still recognizes it. These are orphans `prune_stale` will remove.
    pub fn is_stale(&self) -> bool {
        self.session_id.is_none() && !self.registered_with_git && !self.registered_with_rift
    }
}

/// Snapshot every session that currently holds an active snapshot.
pub fn list_active(registry: &SnapshotRegistry) -> Vec<ActiveSnapshot> {
    let mut out: Vec<ActiveSnapshot> = registry
        .read()
        .iter()
        .map(|(session_id, session)| ActiveSnapshot {
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

/// Walk the home-level managed snapshots directory and report every directory there,
/// joined with what the registry, Rift markers, and `git worktree list` know about it.
pub fn list_managed(workspace: &Path, registry: &SnapshotRegistry) -> Vec<ManagedSnapshot> {
    let base = managed_snapshots_dir(workspace);
    let mut out: Vec<ManagedSnapshot> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&base) else {
        return out;
    };

    let sessions_by_path: HashMap<PathBuf, SnapshotSession> = registry
        .read()
        .iter()
        .map(|(_session_id, session)| (session.path.clone(), session.clone()))
        .collect();
    let session_ids_by_path: HashMap<PathBuf, i64> = registry
        .read()
        .iter()
        .map(|(session_id, session)| (session.path.clone(), *session_id))
        .collect();
    let git_paths: HashSet<PathBuf> = scan_git_backend_paths(workspace);

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
                Some(SnapshotBackend::Rift)
            } else if registered_with_git {
                Some(SnapshotBackend::Git)
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

        out.push(ManagedSnapshot {
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
pub fn prune_stale(workspace: &Path, registry: &SnapshotRegistry) -> Vec<PathBuf> {
    let mut removed = rift_gc(workspace);
    for entry in list_managed(workspace, registry) {
        if !entry.is_stale() {
            continue;
        }
        if std::fs::remove_dir_all(&entry.path).is_ok() {
            removed.push(entry.path);
        } else {
            tracing::warn!(
                target: "agena::snapshot",
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

fn scan_git_backend_paths(workspace: &Path) -> HashSet<PathBuf> {
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
