//! Git checkout ownership for delegated agents. Use libgit2 so provisioning
//! cannot run a repository's shell hooks or configured Git executables.
use super::*;
use agena_domain::{
    AccessKind, AgentWorktree, PathAccessModes, PathPermissionConfig, PathPermissionMode,
    PermissionConfig, PermissionDecision,
};
use agena_runtime_contracts::permission::{SandboxPathPattern, SandboxPermissionSnapshot};
use git2::{Repository, TreeWalkMode, TreeWalkResult, WorktreeAddOptions};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

fn config_error(error: impl std::fmt::Display) -> AppError {
    AppError::Config(format!("Agent worktree: {error}"))
}

pub(super) fn revision(value: Option<String>) -> Result<String, AppError> {
    let value = value.unwrap_or_else(|| "HEAD".into());
    if value.is_empty() || value.len() > 256 || value.contains(['\0', '\r', '\n']) {
        return Err(config_error("worktree_base must be a local Git revision"));
    }
    Ok(value)
}

fn open_checkout(path: &Path) -> Result<Repository, AppError> {
    let repository = Repository::open(path).map_err(config_error)?;
    if repository
        .workdir()
        .and_then(|root| root.canonicalize().ok())
        .as_deref()
        != Some(path)
    {
        return Err(config_error(
            "select the root of a Git checkout for worktree isolation",
        ));
    }
    Ok(repository)
}

fn permitted(snapshot: &SandboxPermissionSnapshot, kind: AccessKind, path: &Path) -> bool {
    matches!(
        snapshot.authorize_path_access(kind, path),
        PermissionDecision::Allow
    )
}

fn require_repository_access(
    repository: &Repository,
    snapshot: &SandboxPermissionSnapshot,
) -> Result<(), AppError> {
    let common = repository
        .commondir()
        .canonicalize()
        .map_err(config_error)?;
    for path in [common.join("worktrees"), common.join("refs/heads")] {
        if !permitted(snapshot, AccessKind::Write, &path) {
            return Err(config_error(format!(
                "creating a checkout requires Git metadata write access at {}",
                path.display()
            )));
        }
    }
    for path in [
        repository.path().join("HEAD"),
        common.join("objects"),
        common.join("refs"),
        common.join("packed-refs"),
    ] {
        if !permitted(snapshot, AccessKind::Read, &path) {
            return Err(config_error(format!(
                "creating a checkout requires Git metadata read access at {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub(super) fn prepare(
    source: &Path,
    revision: &str,
    snapshot: &SandboxPermissionSnapshot,
) -> Result<(AgentWorktree, PermissionConfig), AppError> {
    let source_path = source.canonicalize().map_err(config_error)?;
    if snapshot
        .workspace_root
        .canonicalize()
        .map_err(config_error)?
        != source_path
    {
        return Err(config_error(
            "the requested repository is outside the parent workspace",
        ));
    }
    let repository = open_checkout(&source_path)?;
    require_repository_access(&repository, snapshot)?;
    let commit = repository
        .revparse_single(revision)
        .and_then(|object| object.peel_to_commit())
        .map_err(config_error)?;
    // Object-database access alone cannot grant a copy of a forbidden tracked file.
    let mut denied = None;
    commit
        .tree()
        .map_err(config_error)?
        .walk(TreeWalkMode::PreOrder, |prefix, entry| {
            let Ok(name) = std::str::from_utf8(entry.name_bytes()) else {
                denied = Some("a tracked path has no UTF-8 permission identity".to_owned());
                return TreeWalkResult::Abort;
            };
            let path = source_path.join(prefix).join(name);
            if permitted(snapshot, AccessKind::Read, &path) {
                TreeWalkResult::Ok
            } else {
                denied = Some(format!(
                    "read permission is required for {}",
                    path.display()
                ));
                TreeWalkResult::Abort
            }
        })
        .map_err(|error| config_error(denied.clone().unwrap_or_else(|| error.to_string())))?;
    if let Some(denied) = denied {
        return Err(config_error(denied));
    }
    let policy = checkout_policy(snapshot, &source_path)?;
    let branch = format!("agena-agent-{}", uuid::Uuid::new_v4().simple());
    let path = managed_root(&source_path).join(&branch);
    Ok((
        AgentWorktree {
            path,
            source_path,
            branch,
            base_commit: commit.id().to_string(),
        },
        policy,
    ))
}

fn managed_root(source: &Path) -> PathBuf {
    agena_storage_layout::StorageLayout::current()
        .root()
        .join("agent-worktrees")
        .join(agena_storage_layout::workspace_key(source))
}

fn checkout_policy(
    snapshot: &SandboxPermissionSnapshot,
    source: &Path,
) -> Result<PermissionConfig, AppError> {
    let modes = |path: &Path| {
        PathAccessModes::from_mode(if !permitted(snapshot, AccessKind::Read, path) {
            PathPermissionMode::Deny
        } else if permitted(snapshot, AccessKind::Write, path) {
            PathPermissionMode::Readwrite
        } else {
            PathPermissionMode::Readonly
        })
    };
    let mut boundaries = BTreeSet::new();
    for pattern in snapshot
        .path_layers
        .iter()
        .flat_map(|policy| &policy.rules)
        .flat_map(|rule| &rule.patterns)
    {
        let (root, pattern) = match pattern {
            SandboxPathPattern::Absolute { pattern } => (Path::new(""), pattern.as_str()),
            SandboxPathPattern::Relative { root, pattern } => (root.as_path(), pattern.as_str()),
        };
        let literal = pattern.strip_suffix("/**").unwrap_or(pattern);
        let path = root.join(if literal == "**" { "." } else { literal });
        if let Ok(suffix) = path.strip_prefix(source)
            && !suffix.as_os_str().is_empty()
            && suffix != Path::new(".")
        {
            if suffix.to_string_lossy().contains(['*', '?']) {
                return Err(config_error(
                    "this path rule cannot be projected onto a separate checkout",
                ));
            }
            boundaries.insert(path);
        }
    }
    for rule in &snapshot.persisted_rules {
        if let agena_domain::PermissionAction::PathAccess { target_path, .. } = &rule.action {
            let path = PathBuf::from(target_path);
            if path != source && path.starts_with(source) {
                boundaries.insert(path);
            }
        }
    }
    let rules = boundaries
        .into_iter()
        .map(|path| {
            let suffix = path.strip_prefix(source).expect("workspace path");
            (
                format!(
                    "<workspace>/{}",
                    suffix.to_string_lossy().replace('\\', "/")
                ),
                modes(&path),
            )
        })
        .collect();
    Ok(PermissionConfig {
        path: Some(PathPermissionConfig {
            workspace: Some(modes(source)),
            external: Some(PathAccessModes::from_mode(PathPermissionMode::Readonly)),
            rules,
        }),
        ..Default::default()
    })
}

fn validate_owned(info: &AgentWorktree) -> Result<(), AppError> {
    if info.path != managed_root(&info.source_path).join(&info.branch)
        || info.path.file_name().and_then(|name| name.to_str()) != Some(&info.branch)
        || !info
            .branch
            .strip_prefix("agena-agent-")
            .is_some_and(|value| {
                value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    {
        return Err(config_error("invalid managed checkout identity"));
    }
    Ok(())
}

pub(super) fn create(
    info: &AgentWorktree,
    snapshot: &SandboxPermissionSnapshot,
) -> Result<(), AppError> {
    validate_owned(info)?;
    let repository = open_checkout(&info.source_path)?;
    require_repository_access(&repository, snapshot)?;
    agena_storage_layout::secure_directory(&managed_root(&info.source_path))
        .map_err(config_error)?;
    let commit = repository
        .find_commit(git2::Oid::from_str(&info.base_commit).map_err(config_error)?)
        .map_err(config_error)?;
    let mut branch = repository
        .branch(&info.branch, &commit, false)
        .map_err(config_error)?;
    let mut options = WorktreeAddOptions::new();
    options.reference(Some(branch.get()));
    if let Err(error) = repository.worktree(&info.branch, &info.path, Some(&options)) {
        // Keep any checkout Git managed to publish. Otherwise retire the ref
        // reserved by this call without removing user files.
        if repository.find_worktree(&info.branch).is_err()
            && let Err(cleanup) = branch.delete()
        {
            tracing::warn!(%cleanup, branch = %info.branch, "worktree reservation could not be retired");
        }
        return Err(config_error(error));
    }
    verify(info)
}

fn verify(info: &AgentWorktree) -> Result<(), AppError> {
    validate_owned(info)?;
    let source = open_checkout(&info.source_path)?;
    let checkout = source.find_worktree(&info.branch).map_err(config_error)?;
    if checkout.path().canonicalize().ok().as_deref() != Some(info.path.as_path()) {
        return Err(config_error(format!(
            "the checkout moved or was removed: {}",
            info.path.display()
        )));
    }
    checkout.validate().map_err(config_error)?;
    let repository = open_checkout(&info.path)?;
    if repository.path().canonicalize().map_err(config_error)?
        != source
            .commondir()
            .join("worktrees")
            .join(&info.branch)
            .canonicalize()
            .map_err(config_error)?
    {
        return Err(config_error(
            "the managed checkout belongs to a different Git repository",
        ));
    }
    Ok(())
}

pub(super) async fn verify_async(info: AgentWorktree) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || verify(&info))
        .await
        .map_err(|error| AppError::internal_error(&error))?
}

pub(super) async fn followup_policy(
    info: AgentWorktree,
    snapshot: SandboxPermissionSnapshot,
) -> Result<PermissionConfig, AppError> {
    tokio::task::spawn_blocking(move || {
        if snapshot
            .workspace_root
            .canonicalize()
            .map_err(config_error)?
            != info.source_path
        {
            return Err(config_error(
                "the parent's checkout has changed since launch",
            ));
        }
        checkout_policy(&snapshot, &info.source_path)
    })
    .await
    .map_err(|error| AppError::internal_error(&error))?
}

pub(super) fn describe(
    info: &AgentWorktree,
    snapshot: &SandboxPermissionSnapshot,
) -> Result<String, AppError> {
    verify(info)?;
    let repository = open_checkout(&info.path)?;
    let head = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(config_error)?
        .id();
    let base = repository
        .find_commit(git2::Oid::from_str(&info.base_commit).map_err(config_error)?)
        .and_then(|commit| commit.tree())
        .map_err(config_error)?;
    let mut options = git2::DiffOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    let diff = repository
        .diff_tree_to_workdir_with_index(Some(&base), Some(&mut options))
        .map_err(config_error)?;
    let mut count = 0;
    let mut paths = Vec::new();
    for delta in diff.deltas() {
        if let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path())
            && permitted(snapshot, AccessKind::Read, &info.path.join(path))
        {
            count += 1;
            if paths.len() < 12 {
                let mut label = path.to_string_lossy().escape_debug().to_string();
                truncate(&mut label, 384);
                paths.push(label);
            }
        }
    }
    let mut summary = format!(
        "Git worktree {} on {}. Base {}, HEAD {}. {} changed paths.",
        info.path.display(),
        info.branch,
        info.base_commit,
        head,
        count
    );
    for path in paths {
        summary.push_str(&format!("\n- {path}"));
    }
    if count > 12 {
        summary.push_str(&format!("\n- {} additional paths", count - 12));
    }
    Ok(summary)
}

impl SessionManager {
    pub(super) async fn agent_worktree_permissions(
        &self,
        session: i64,
        executor: &ToolExecutor,
        context: &agena_runtime_session_core::SessionExecutionContext,
    ) -> Result<SandboxPermissionSnapshot, AppError> {
        let executor = executor.for_session_resource_context(context);
        let mut snapshot = self
            .sandbox_executor_for_session(&executor, session)
            .await
            .map_err(tool_error)?
            .sandbox_permission_snapshot();
        // Interactive grants are scoped to the original invocation or
        // resource. Project only restrictions into a newly allocated copy.
        snapshot
            .persisted_rules
            .retain(|rule| rule.mode == agena_domain::PermissionMode::Deny);
        Ok(snapshot)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use agena_runtime_contracts::{
        authorization::ExecutionPrincipal,
        permission::{PermissionPolicy, ToolPermissionPolicy},
    };

    #[test]
    fn worktree_permissions_keep_source_restrictions_at_the_new_root() {
        let source = Path::new("/source-project");
        let checkout = Path::new("/agent-checkout");
        let permissions = serde_json::from_value(serde_json::json!({
            "path": {"rules": {
                "/source-project/restricted.txt": "readonly",
                "<workspace>/private": "deny"
            }}
        }))
        .unwrap();
        let base = ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        );
        let snapshot = base
            .clone()
            .try_apply_permission_config(&permissions)
            .unwrap()
            .sandbox_permission_snapshot(source);
        let projection = checkout_policy(&snapshot, source).unwrap();
        let scoped = base
            .try_apply_permission_ceiling(&projection)
            .unwrap()
            .sandbox_permission_snapshot(checkout);
        assert!(permitted(
            &scoped,
            AccessKind::Write,
            &checkout.join("ordinary.txt")
        ));
        assert!(!permitted(
            &scoped,
            AccessKind::Write,
            &checkout.join("restricted.txt")
        ));
        assert!(!permitted(
            &scoped,
            AccessKind::Read,
            &checkout.join("private/child.txt")
        ));
        assert!(!permitted(
            &scoped,
            AccessKind::Write,
            &source.join("ordinary.txt")
        ));
    }
}
