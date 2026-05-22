use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use super::super::{
    DirectoryQuery, git_success_response, is_safe_repo_rel_path, map_git_failure,
    require_locked_directory, run_git, run_git_checked, run_git_checked_with_status,
};

fn invalid_file_path_response() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "Invalid file path"})),
    )
        .into_response()
}

fn validate_repo_paths(paths: &[String]) -> Result<(), Box<Response>> {
    if paths.iter().any(|path| !is_safe_repo_rel_path(path)) {
        return Err(Box::new(invalid_file_path_response()));
    }
    Ok(())
}

fn collect_body_paths(path: Option<String>, paths: Option<Vec<String>>) -> Vec<String> {
    let mut collected: Vec<String> = Vec::new();
    if let Some(path) = path {
        let path = path.trim();
        if !path.is_empty() {
            collected.push(path.to_string());
        }
    }
    if let Some(paths) = paths {
        for path in paths {
            let path = path.trim();
            if !path.is_empty() {
                collected.push(path.to_string());
            }
        }
    }
    collected.sort();
    collected.dedup();
    collected
}

fn collect_output_paths(output: &str) -> Vec<String> {
    let mut paths: Vec<String> = output
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

#[derive(Debug, Deserialize)]
pub struct GitRevertBody {
    pub path: Option<String>,
}

pub async fn git_revert(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitRevertBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let Some(file_path) = body
        .path
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path parameter is required"})),
        )
            .into_response();
    };

    let repo_root = dir.clone();
    let absolute_target = repo_root.join(file_path);
    if !absolute_target.starts_with(&repo_root) {
        return invalid_file_path_response();
    }

    let tracked = run_git(&dir, &["ls-files", "--error-unmatch", file_path])
        .await
        .map(|(c, _, _)| c == 0)
        .unwrap_or(false);

    if !tracked {
        // Best-effort clean for untracked.
        let _ = run_git(&dir, &["clean", "-f", "-d", "--", file_path]).await;
        let _ = tokio::fs::remove_file(&absolute_target).await;
        return git_success_response();
    }

    // Unstage.
    if run_git(&dir, &["restore", "--staged", file_path])
        .await
        .is_err()
    {
        let _ = run_git(&dir, &["reset", "HEAD", "--", file_path]).await;
    }
    // Restore worktree.
    if run_git(&dir, &["restore", "--", file_path]).await.is_err() {
        let _ = run_git(&dir, &["checkout", "--", file_path]).await;
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
pub struct GitStageBody {
    pub path: Option<String>,
    pub paths: Option<Vec<String>>,
    pub all: Option<bool>,
    // "tracked" | "untracked" | "merge" | "paths" (default)
    pub scope: Option<String>,
}

pub async fn git_stage(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitStageBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let scope = if body.all.unwrap_or(false) {
        "all".to_string()
    } else if let Some(s) = body.scope.as_deref() {
        s.trim().to_ascii_lowercase()
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "scope is required", "code": "missing_scope"})),
        )
            .into_response();
    };

    if scope == "all" {
        // Stage everything (including deletions), like a "Stage All" action.
        if let Err(resp) = run_git_checked(&dir, &["add", "-A"], None).await {
            return resp;
        }
        return git_success_response();
    }

    if scope == "tracked" {
        // Only stage updates to already tracked files (no new untracked).
        if let Err(resp) = run_git_checked(&dir, &["add", "-u"], None).await {
            return resp;
        }
        return git_success_response();
    }

    if scope == "untracked" {
        // Stage only untracked files.
        let (out, _) = match run_git_checked(
            &dir,
            &["ls-files", "--others", "--exclude-standard"],
            None,
        )
        .await
        {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let files = collect_output_paths(&out);
        if let Err(resp) = validate_repo_paths(&files) {
            return *resp;
        }
        if files.is_empty() {
            return Json(serde_json::json!({"success": true, "staged": 0})).into_response();
        }
        let mut args: Vec<&str> = vec!["add", "--"];
        for p in &files {
            args.push(p);
        }
        if let Err(resp) = run_git_checked(&dir, &args, None).await {
            return resp;
        }
        return Json(serde_json::json!({"success": true, "staged": files.len()})).into_response();
    }

    if scope == "merge" {
        // Attempt to stage all unmerged paths. This will only succeed for files that
        // have been resolved in the worktree.
        let (out, _) =
            match run_git_checked(&dir, &["diff", "--name-only", "--diff-filter=U"], None).await {
                Ok(value) => value,
                Err(resp) => return resp,
            };
        let files = collect_output_paths(&out);
        if let Err(resp) = validate_repo_paths(&files) {
            return *resp;
        }
        if files.is_empty() {
            return Json(serde_json::json!({"success": true, "staged": 0})).into_response();
        }
        let mut args: Vec<&str> = vec!["add", "--"];
        for p in &files {
            args.push(p);
        }
        if let Err(resp) =
            run_git_checked_with_status(&dir, &args, StatusCode::CONFLICT, Some("merge_unresolved"))
                .await
        {
            return resp;
        }
        return Json(serde_json::json!({"success": true, "staged": files.len()})).into_response();
    }

    // Otherwise stage explicit paths.
    if scope != "paths" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid scope", "code": "invalid_scope"})),
        )
            .into_response();
    }

    let paths = collect_body_paths(body.path, body.paths);
    if paths.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path(s) is required"})),
        )
            .into_response();
    }
    if let Err(resp) = validate_repo_paths(&paths) {
        return *resp;
    }

    // Stage full files only (non-interactive).
    let mut args: Vec<&str> = vec!["add", "--"]; // `--` prevents path-as-flag.
    for p in &paths {
        args.push(p);
    }
    if let Err(resp) = run_git_checked(&dir, &args, None).await {
        return resp;
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
pub struct GitUnstageBody {
    pub path: Option<String>,
    pub paths: Option<Vec<String>>,
    pub all: Option<bool>,
}

pub async fn git_unstage(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitUnstageBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    if body.all.unwrap_or(false) {
        // Unstage everything while keeping worktree changes.
        if let Err(resp) = run_git_checked(&dir, &["reset"], None).await {
            return resp;
        }
        return git_success_response();
    }

    let paths = collect_body_paths(body.path, body.paths);
    if paths.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path(s) is required"})),
        )
            .into_response();
    }
    if let Err(resp) = validate_repo_paths(&paths) {
        return *resp;
    }

    // Unstage full files only.
    // Prefer `restore --staged`, fall back to `reset` for older git.
    let mut args: Vec<&str> = vec!["restore", "--staged", "--"]; // `--` prevents path-as-flag.
    for p in &paths {
        args.push(p);
    }
    let (code, _out, _err) =
        run_git(&dir, &args)
            .await
            .unwrap_or((1, "".to_string(), "".to_string()));
    if code != 0 {
        let mut reset_args: Vec<&str> = vec!["reset", "HEAD", "--"]; // `--` prevents path-as-flag.
        for p in &paths {
            reset_args.push(p);
        }
        if let Err(resp) = run_git_checked(&dir, &reset_args, None).await {
            return resp;
        }
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCleanBody {
    // "untracked" (default) | "all" | "tracked"
    pub scope: Option<String>,
    pub paths: Option<Vec<String>>,
}

pub async fn git_clean(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitCleanBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let scope = body
        .scope
        .as_deref()
        .unwrap_or("untracked")
        .trim()
        .to_ascii_lowercase();
    let paths = collect_body_paths(None, body.paths);
    if let Err(resp) = validate_repo_paths(&paths) {
        return *resp;
    }

    if scope == "tracked" {
        // Discard all tracked changes (index + worktree) without touching untracked.
        // Prefer `git restore` and fall back to older git commands.
        let (c1, _o1, _e1) = run_git(&dir, &["restore", "--staged", "--worktree", "--", "."])
            .await
            .unwrap_or((1, "".to_string(), "".to_string()));
        if c1 != 0 {
            let _ = run_git(&dir, &["reset"]).await;
            if let Err(resp) = run_git_checked(&dir, &["checkout", "--", "."], None).await {
                return resp;
            }
        }
        return git_success_response();
    }

    let mut args: Vec<&str> = vec!["clean", "-f", "-d"];
    if scope == "all" {
        args.push("-x");
    } else if scope != "untracked" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid scope", "code": "invalid_scope"})),
        )
            .into_response();
    }
    if !paths.is_empty() {
        args.push("--");
        for p in &paths {
            args.push(p);
        }
    }

    let (out, _) = match run_git_checked(&dir, &args, None).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    Json(serde_json::json!({"success": true, "output": out.trim()})).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRenameBody {
    pub from: String,
    pub to: String,
}

pub async fn git_rename(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitRenameBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let from = body.from.trim();
    let to = body.to.trim();
    if from.is_empty()
        || to.is_empty()
        || !is_safe_repo_rel_path(from)
        || !is_safe_repo_rel_path(to)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }

    // Prefer git mv. It will also handle tracked renames.
    let (code, out, err) =
        run_git(&dir, &["mv", "--", from, to])
            .await
            .unwrap_or((1, "".to_string(), "".to_string()));
    if code != 0 {
        // Fall back to filesystem rename for untracked files.
        let src = dir.join(from);
        let dst = dir.join(to);
        if let Some(parent) = dst.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if tokio::fs::rename(&src, &dst).await.is_err() {
            if let Some(resp) = map_git_failure(code, &out, &err) {
                return resp;
            }
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err.trim(), "code": "rename_failed"})),
            )
                .into_response();
        }
    }
    git_success_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDeleteBody {
    pub path: String,
    pub force: Option<bool>,
}

pub async fn git_delete(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitDeleteBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let path = body.path.trim();
    if path.is_empty() || !is_safe_repo_rel_path(path) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }

    // Determine if tracked.
    let (c_ls, _o_ls, _e_ls) = run_git(&dir, &["ls-files", "--error-unmatch", "--", path])
        .await
        .unwrap_or((1, "".to_string(), "".to_string()));
    let tracked = c_ls == 0;
    let force = body.force.unwrap_or(false);

    if tracked {
        let mut args: Vec<&str> = vec!["rm"];
        if force {
            args.push("-f");
        }
        args.push("-r");
        args.push("--");
        args.push(path);
        if let Err(resp) =
            run_git_checked_with_status(&dir, &args, StatusCode::BAD_REQUEST, Some("delete_failed"))
                .await
        {
            return resp;
        }
        return Json(serde_json::json!({"success": true, "staged": true})).into_response();
    }

    // Untracked: delete from filesystem.
    let full = dir.join(path);
    let meta = tokio::fs::metadata(&full).await;
    if let Ok(m) = meta {
        if m.is_dir() {
            let _ = tokio::fs::remove_dir_all(&full).await;
        } else {
            let _ = tokio::fs::remove_file(&full).await;
        }
    }

    Json(serde_json::json!({"success": true, "staged": false})).into_response()
}
