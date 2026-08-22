use std::path::PathBuf;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tokio::sync::Semaphore;

mod atomic_file;
mod auth;
mod blame;
mod branches;
mod commit;
mod diff;
mod exec;
mod gpg;
mod history;
mod ignore;
mod lfs;
mod ops;
mod policy;
mod remote;
mod repos;
mod status;
mod submodule;
mod utils;
mod worktrees;

pub(crate) const MAX_BLOB_BYTES: usize = 50 * 1024 * 1024;
const LIBGIT2_WORKER_LIMIT: usize = 16;
static LIBGIT2_WORKERS: Semaphore = Semaphore::const_new(LIBGIT2_WORKER_LIMIT);

/// Runs synchronous libgit2 work without occupying a Tokio runtime worker.
///
/// `spawn_blocking` has a deliberately large pool and an unbounded submission
/// queue. HTTP clients can otherwise enqueue arbitrary numbers of expensive
/// repository scans, so acquire a process-wide budget before submitting the
/// blocking job and retain the permit until the job actually returns.
pub(crate) async fn spawn_libgit2<F, T>(job: F) -> Result<T, tokio::task::JoinError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let permit = LIBGIT2_WORKERS
        .acquire()
        .await
        .expect("the static libgit2 worker semaphore is never closed");
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job()
    })
    .await
}

#[derive(Debug, Deserialize)]
/// Query carrying an optional git directory.
pub struct DirectoryQuery {
    pub directory: Option<String>,
}

pub(crate) fn require_directory_raw(dir: Option<&str>) -> Result<PathBuf, Box<Response>> {
    let Some(dir) = dir.map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "directory parameter is required",
                    "code": "missing_directory",
                })),
            )
                .into_response(),
        ));
    };
    abs_path(dir)
}

pub(crate) fn require_directory(q: &DirectoryQuery) -> Result<PathBuf, Box<Response>> {
    require_directory_raw(q.directory.as_deref())
}

pub(crate) async fn require_locked_directory(
    q: &DirectoryQuery,
) -> Result<(PathBuf, exec::RepoLockGuard), Response> {
    let dir = require_directory(q).map_err(|resp| *resp)?;
    let guard = lock_repo(&dir).await?;
    Ok((dir, guard))
}

// Shared helpers/types re-exported for submodules.
pub use auth::GitAuthInput;
pub(crate) use auth::{TempGitAskpass, git_http_auth_env, normalize_http_auth};
pub use blame::*;

pub(crate) use exec::{
    git_command_result_or_log, git_command_transport_error_response, git_success_response,
    lock_repo, run_git, run_git_checked, run_git_checked_with_status, run_git_env,
    run_git_with_input, run_locked_git_checked, run_locked_git_checked_with_status,
    run_locked_git_env_checked, run_locked_git_env_checked_with_status,
};
pub(crate) use policy::{
    GitBranchProtectionPrompt, git_allow_force_push, git_allow_no_verify_commit,
    git_branch_protection_for_branch, git_enforce_branch_protection, git_strict_patch_validation,
};

pub(crate) use utils::{
    abs_path, git_config_get, git_io_error_response, git_task_error_response,
    git2_open_error_response, is_safe_repo_rel_path, map_git_failure, path_slash,
    redact_git_output, rel_path_slash, truncate_for_payload,
};

// Public HTTP handlers.
pub use branches::*;
pub use commit::*;
pub use diff::*;
pub use gpg::*;
pub use history::*;
pub use ignore::*;
pub use lfs::*;
pub use ops::*;
pub use remote::*;
pub use repos::*;
pub use status::*;
pub use submodule::*;
pub use worktrees::*;
