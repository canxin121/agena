use std::collections::HashSet;
use std::path::{Path, PathBuf};

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::{
    DirectoryQuery, git_success_response, is_safe_repo_rel_path, map_git_failure,
    require_directory, require_locked_directory, run_git, run_git_checked, run_locked_git_checked,
};

fn count_status_paths(status_output: &str) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    for raw in status_output.lines() {
        let line = raw.trim_end();
        if line.len() < 4 {
            continue;
        }
        // Porcelain format: XY<space><path> (or old -> new for renames).
        let mut payload = line[3..].trim().to_string();
        if let Some((_old, new_path)) = payload.split_once(" -> ") {
            payload = new_path.trim().to_string();
        }
        if payload.is_empty() {
            continue;
        }
        seen.insert(payload);
    }
    seen.len()
}

fn resolve_worktree_migrate_source_path(raw: &str, repo: &Path) -> Result<PathBuf, Box<Response>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Box::new((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "sourcePath is required", "code": "missing_source_path"})),
        )
            .into_response()));
    }
    let p = Path::new(trimmed);
    let full =
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            if !is_safe_repo_rel_path(trimmed) {
                return Err(Box::new((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid sourcePath", "code": "invalid_path"})),
            )
                .into_response()));
            }
            repo.join(trimmed)
        };
    Ok(full)
}

async fn git_common_dir(dir: &Path) -> Option<String> {
    let (code, out, _err) = run_git(dir, &["rev-parse", "--git-common-dir"])
        .await
        .ok()?;
    if code != 0 {
        return None;
    }
    let raw = out.trim();
    if raw.is_empty() {
        return None;
    }
    let p = PathBuf::from(raw);
    let full = if p.is_absolute() { p } else { dir.join(p) };
    Some(full.to_string_lossy().into_owned())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorktreeInfo {
    pub worktree: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub locked: bool,
    pub prunable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prunable_reason: Option<String>,
}

fn clean_branch_name(branch: &str) -> String {
    if let Some(rest) = branch.strip_prefix("refs/heads/") {
        return rest.to_string();
    }
    if let Some(rest) = branch.strip_prefix("heads/") {
        return rest.to_string();
    }
    if let Some(rest) = branch.strip_prefix("refs/") {
        return rest.to_string();
    }
    branch.to_string()
}

pub async fn git_worktrees(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let (code, out, err) = run_git(&dir, &["worktree", "list", "--porcelain"])
        .await
        .unwrap_or((1, "".to_string(), "".to_string()));
    if code != 0 {
        // Return [] for non-worktree repos.
        let mut resp = Json(Vec::<GitWorktreeInfo>::new()).into_response();
        resp.headers_mut().insert(
            "X-Agena-Studio-Warning",
            "git worktrees unavailable".parse().unwrap(),
        );
        tracing::warn!("git worktree list failed: {}", err.trim());
        return resp;
    }

    let mut worktrees = Vec::new();
    let mut current: Option<GitWorktreeInfo> = None;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            if let Some(wt) = current.take() {
                worktrees.push(wt);
            }
            current = Some(GitWorktreeInfo {
                worktree: rest.trim().to_string(),
                head: None,
                branch: None,
                locked: false,
                prunable: false,
                locked_reason: None,
                prunable_reason: None,
            });
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            if let Some(ref mut wt) = current {
                wt.head = Some(rest.trim().to_string());
            }
        } else if let Some(rest) = line.strip_prefix("branch ") {
            if let Some(ref mut wt) = current {
                wt.branch = Some(clean_branch_name(rest.trim()));
            }
        } else if let Some(rest) = line.strip_prefix("locked") {
            if let Some(ref mut wt) = current {
                wt.locked = true;
                let reason = rest.trim();
                if !reason.is_empty() {
                    wt.locked_reason = Some(reason.to_string());
                }
            }
        } else if let Some(rest) = line.strip_prefix("prunable") {
            if let Some(ref mut wt) = current {
                wt.prunable = true;
                let reason = rest.trim();
                if !reason.is_empty() {
                    wt.prunable_reason = Some(reason.to_string());
                }
            }
        } else if line.trim().is_empty()
            && let Some(wt) = current.take()
        {
            worktrees.push(wt);
        }
    }
    if let Some(wt) = current.take() {
        worktrees.push(wt);
    }

    Json(worktrees).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorktreeAddBody {
    pub path: Option<String>,
    pub branch: Option<String>,
    pub start_point: Option<String>,
    #[serde(default)]
    pub create_branch: bool,
}

fn default_true() -> bool {
    true
}

fn resolve_worktree_path(repo: &Path, raw: &str) -> Result<PathBuf, Box<Response>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "path is required", "code": "missing_path"})),
            )
                .into_response(),
        ));
    }
    let p = Path::new(trimmed);
    let full = if p.is_absolute() {
        p.to_path_buf()
    } else {
        if !is_safe_repo_rel_path(trimmed) {
            return Err(Box::new(
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
                )
                    .into_response(),
            ));
        }
        repo.join(trimmed)
    };
    if !full.starts_with(repo) {
        return Err(Box::new((
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Worktree path must be within repo", "code": "path_outside_repo"}),
            ),
        )
            .into_response()));
    }
    Ok(full)
}

pub async fn git_worktree_add(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitWorktreeAddBody>,
) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let Some(path_raw) = body.path.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path is required", "code": "missing_path"})),
        )
            .into_response();
    };
    let target = match resolve_worktree_path(&dir, path_raw) {
        Ok(p) => p,
        Err(resp) => return *resp,
    };

    let branch = body
        .branch
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let start_point = body
        .start_point
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let target_str = target.to_string_lossy().to_string();
    let mut args: Vec<String> = vec!["worktree".into(), "add".into()];
    if body.create_branch {
        let Some(name) = branch else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "branch is required", "code": "missing_branch"})),
            )
                .into_response();
        };
        args.push("-b".into());
        args.push(name.to_string());
        args.push(target_str);
        if let Some(sp) = start_point {
            args.push(sp.to_string());
        }
    } else {
        args.push(target_str);
        if let Some(b) = branch {
            args.push(b.to_string());
        }
    }

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if let Err(resp) = run_locked_git_checked(&q, &args_ref, Some("git_worktree_add_failed")).await
    {
        return resp;
    }

    Json(serde_json::json!({"success": true, "path": target.to_string_lossy()})).into_response()
}

#[derive(Debug, Deserialize)]
pub struct GitWorktreeRemoveBody {
    pub path: Option<String>,
}

pub async fn git_worktree_remove(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitWorktreeRemoveBody>,
) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let Some(path_raw) = body.path.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path is required", "code": "missing_path"})),
        )
            .into_response();
    };
    let target = match resolve_worktree_path(&dir, path_raw) {
        Ok(p) => p,
        Err(resp) => return *resp,
    };

    let target_str = target.to_string_lossy().to_string();
    if let Err(resp) = run_locked_git_checked(
        &q,
        &["worktree", "remove", &target_str],
        Some("git_worktree_remove_failed"),
    )
    .await
    {
        return resp;
    }

    git_success_response()
}

pub async fn git_worktree_prune(Query(q): Query<DirectoryQuery>) -> Response {
    let (out, _err) = match run_locked_git_checked(
        &q,
        &["worktree", "prune"],
        Some("git_worktree_prune_failed"),
    )
    .await
    {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    Json(serde_json::json!({"success": true, "output": out.trim()})).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorktreeMigrateBody {
    pub source_path: Option<String>,
    #[serde(default = "default_true")]
    pub include_untracked: bool,
    #[serde(default = "default_true")]
    pub delete_from_source: bool,
}

pub async fn git_worktree_migrate(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitWorktreeMigrateBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let Some(source_raw) = body.source_path.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "sourcePath is required", "code": "missing_source_path"}),
            ),
        )
            .into_response();
    };
    let source = match resolve_worktree_migrate_source_path(source_raw, &dir) {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    if source == dir {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "sourcePath must be different from target worktree",
                "code": "invalid_source_path"
            })),
        )
            .into_response();
    }
    if !source.is_dir() {
        return (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": "Source worktree not found", "code": "source_not_found"}),
            ),
        )
            .into_response();
    }

    let Some(target_common) = git_common_dir(&dir).await else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Target directory is not a git worktree",
                "code": "target_not_git_repo"
            })),
        )
            .into_response();
    };
    let Some(source_common) = git_common_dir(&source).await else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "sourcePath is not a git worktree",
                "code": "source_not_git_repo"
            })),
        )
            .into_response();
    };
    if source_common != target_common {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Source and target worktrees belong to different repositories",
                "code": "worktree_repo_mismatch"
            })),
        )
            .into_response();
    }

    let (status_out, _status_err) = match run_git_checked(
        &source,
        &["status", "--porcelain"],
        Some("git_status_failed"),
    )
    .await
    {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let changed_files = count_status_paths(&status_out);
    if changed_files == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "No local changes to migrate from source worktree",
                "code": "git_no_changes_to_migrate"
            })),
        )
            .into_response();
    }

    let source_name = source
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.to_string_lossy().into_owned());
    let target_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.to_string_lossy().into_owned());
    let stash_name = format!("migration:{source_name}->{target_name}");

    let mut stash_args: Vec<String> = vec![
        "stash".to_string(),
        "push".to_string(),
        "-m".to_string(),
        stash_name,
    ];
    if body.include_untracked {
        stash_args.push("--include-untracked".to_string());
    }
    let stash_args_ref: Vec<&str> = stash_args.iter().map(|s| s.as_str()).collect();
    if let Err(resp) =
        run_git_checked(&source, &stash_args_ref, Some("git_stash_push_failed")).await
    {
        return resp;
    }

    let (ref_out, _ref_err) = match run_git_checked(
        &source,
        &["stash", "list", "-n", "1", "--format=%gd"],
        Some("git_stash_ref_failed"),
    )
    .await
    {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let stash_ref = ref_out
        .lines()
        .next()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("stash@{0}")
        .to_string();

    let apply_cmd = if body.delete_from_source {
        ["stash", "pop", stash_ref.as_str()]
    } else {
        ["stash", "apply", stash_ref.as_str()]
    };
    let (apply_code, apply_out, apply_err) =
        run_git(&dir, &apply_cmd)
            .await
            .unwrap_or((1, "".to_string(), "".to_string()));
    if apply_code != 0 {
        // Best-effort restore source changes so migration failures are not destructive.
        let _ = run_git(&source, &["stash", "pop", "--index", &stash_ref]).await;
        if let Some(resp) = map_git_failure(apply_code, &apply_out, &apply_err) {
            return resp;
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": apply_err.trim(),
                "code": "git_worktree_migrate_failed"
            })),
        )
            .into_response();
    }

    if !body.delete_from_source
        && let Err(resp) = run_git_checked(
            &source,
            &["stash", "pop", "--index", &stash_ref],
            Some("git_worktree_restore_failed"),
        )
        .await
    {
        return resp;
    }

    Json(serde_json::json!({
        "success": true,
        "sourcePath": source.to_string_lossy(),
        "targetPath": dir.to_string_lossy(),
        "migratedFiles": changed_files,
        "deleteFromSource": body.delete_from_source,
        "includeUntracked": body.include_untracked,
    }))
    .into_response()
}
