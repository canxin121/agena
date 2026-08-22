use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use super::super::{
    DirectoryQuery, git_command_transport_error_response, git_io_error_response,
    git_success_response, is_safe_repo_rel_path, map_git_failure, redact_git_output,
    require_locked_directory, run_git, run_git_checked, run_git_checked_with_status,
    truncate_for_payload,
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

fn collect_body_paths(path: Option<String>, paths: Vec<String>) -> Vec<String> {
    let mut collected: Vec<String> = Vec::new();
    if let Some(path) = path {
        let path = path.trim();
        if !path.is_empty() {
            collected.push(path.to_string());
        }
    }
    for path in paths {
        let path = path.trim();
        if !path.is_empty() {
            collected.push(path.to_string());
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

fn restore_command_is_unsupported(stdout: &str, stderr: &str) -> bool {
    let diagnostic = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    diagnostic.contains("'restore' is not a git command")
        || diagnostic.contains("\"restore\" is not a git command")
        || diagnostic.contains("unknown subcommand: restore")
        || diagnostic.contains("unknown option `staged'")
        || diagnostic.contains("unknown option 'staged'")
        || diagnostic.contains("unknown option `worktree'")
        || diagnostic.contains("unknown option 'worktree'")
}

fn completed_git_failure_response(code: i32, stdout: &str, stderr: &str) -> Response {
    map_git_failure(code, stdout, stderr).unwrap_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "Git command failed without a diagnostic",
                "code": "git_failed",
            })),
        )
            .into_response()
    })
}

async fn remove_untracked_target(path: &std::path::Path) -> Result<(), Response> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(git_io_error_response(
                "inspect untracked target before removal",
                &error,
                "delete_metadata_failed",
            ));
        }
    };

    let result = if metadata.is_dir() {
        tokio::fs::remove_dir_all(path).await
    } else {
        tokio::fs::remove_file(path).await
    };
    result.map_err(|error| {
        git_io_error_response(
            "remove untracked target from the worktree",
            &error,
            "delete_failed",
        )
    })
}

#[derive(Debug, Deserialize)]
/// Body of a git revert request.
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

    if !is_safe_repo_rel_path(file_path) {
        return invalid_file_path_response();
    }
    let absolute_target = dir.join(file_path);

    let tracked = match run_git(&dir, &["ls-files", "--", file_path]).await {
        Ok((0, stdout, _)) => !stdout.trim().is_empty(),
        Ok((code, stdout, stderr)) => {
            return completed_git_failure_response(code, &stdout, &stderr);
        }
        Err(error) => {
            return git_command_transport_error_response(
                "determine whether the revert target is tracked",
                &error,
                Some("revert_status_failed"),
            );
        }
    };

    if !tracked {
        if let Err(resp) = run_git_checked(
            &dir,
            &["clean", "-f", "-d", "--", file_path],
            Some("revert_clean_failed"),
        )
        .await
        {
            return resp;
        }
        if let Err(resp) = remove_untracked_target(&absolute_target).await {
            return resp;
        }
        return git_success_response();
    }

    // Unstage. Only use the legacy command when Git explicitly reports that
    // `restore` is unavailable; transport and repository failures must surface.
    match run_git(&dir, &["restore", "--staged", "--", file_path]).await {
        Ok((0, _, _)) => {}
        Ok((_code, stdout, stderr)) if restore_command_is_unsupported(&stdout, &stderr) => {
            if let Err(resp) = run_git_checked(
                &dir,
                &["reset", "HEAD", "--", file_path],
                Some("revert_unstage_failed"),
            )
            .await
            {
                return resp;
            }
        }
        Ok((code, stdout, stderr)) => {
            return completed_git_failure_response(code, &stdout, &stderr);
        }
        Err(error) => {
            return git_command_transport_error_response(
                "unstage the revert target",
                &error,
                Some("revert_unstage_failed"),
            );
        }
    }

    match run_git(&dir, &["restore", "--", file_path]).await {
        Ok((0, _, _)) => {}
        Ok((_code, stdout, stderr)) if restore_command_is_unsupported(&stdout, &stderr) => {
            if let Err(resp) = run_git_checked(
                &dir,
                &["checkout", "--", file_path],
                Some("revert_worktree_failed"),
            )
            .await
            {
                return resp;
            }
        }
        Ok((code, stdout, stderr)) => {
            return completed_git_failure_response(code, &stdout, &stderr);
        }
        Err(error) => {
            return git_command_transport_error_response(
                "restore the revert target in the worktree",
                &error,
                Some("revert_worktree_failed"),
            );
        }
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
/// Body of a git stage request.
pub struct GitStageBody {
    pub path: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub all: bool,
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

    let scope = if body.all {
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
/// Body of a git unstage request.
pub struct GitUnstageBody {
    pub path: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub all: bool,
}

pub async fn git_unstage(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitUnstageBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    if body.all {
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
    match run_git(&dir, &args).await {
        Ok((0, _, _)) => {}
        Ok((_, stdout, stderr)) if restore_command_is_unsupported(&stdout, &stderr) => {
            let mut reset_args: Vec<&str> = vec!["reset", "HEAD", "--"];
            for p in &paths {
                reset_args.push(p);
            }
            if let Err(resp) = run_git_checked(&dir, &reset_args, Some("unstage_failed")).await {
                return resp;
            }
        }
        Ok((code, stdout, stderr)) => {
            return completed_git_failure_response(code, &stdout, &stderr);
        }
        Err(error) => {
            return git_command_transport_error_response(
                "unstage selected paths",
                &error,
                Some("unstage_failed"),
            );
        }
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Body of a git clean request.
pub struct GitCleanBody {
    // "untracked" (default) | "all" | "tracked"
    pub scope: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
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
        match run_git(&dir, &["restore", "--staged", "--worktree", "--", "."]).await {
            Ok((0, _, _)) => {}
            Ok((_, stdout, stderr)) if restore_command_is_unsupported(&stdout, &stderr) => {
                if let Err(resp) =
                    run_git_checked(&dir, &["reset"], Some("clean_tracked_reset_failed")).await
                {
                    return resp;
                }
                if let Err(resp) = run_git_checked(
                    &dir,
                    &["checkout", "--", "."],
                    Some("clean_tracked_checkout_failed"),
                )
                .await
                {
                    return resp;
                }
            }
            Ok((code, stdout, stderr)) => {
                return completed_git_failure_response(code, &stdout, &stderr);
            }
            Err(error) => {
                return git_command_transport_error_response(
                    "discard tracked index and worktree changes",
                    &error,
                    Some("clean_tracked_failed"),
                );
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
/// Body of a git rename request.
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
    let (code, out, err) = match run_git(&dir, &["mv", "--", from, to]).await {
        Ok(result) => result,
        Err(error) => {
            return git_command_transport_error_response(
                "rename a repository path with Git",
                &error,
                Some("rename_failed"),
            );
        }
    };
    if code != 0 {
        // Fall back to filesystem rename for untracked files.
        let src = dir.join(from);
        let dst = dir.join(to);
        if let Some(parent) = dst.parent() {
            if let Err(error) = tokio::fs::create_dir_all(parent).await {
                tracing::warn!(
                    git_exit_code = code,
                    git_stdout = %redact_git_output(&truncate_for_payload(&out, 16_000)),
                    git_stderr = %redact_git_output(&truncate_for_payload(&err, 16_000)),
                    "Git rename failed before the filesystem fallback could create its destination"
                );
                return git_io_error_response(
                    "create the destination directory for a repository rename",
                    &error,
                    "rename_parent_failed",
                );
            }
        }
        if let Err(error) = tokio::fs::rename(&src, &dst).await {
            tracing::warn!(
                git_exit_code = code,
                git_stdout = %redact_git_output(&truncate_for_payload(&out, 16_000)),
                git_stderr = %redact_git_output(&truncate_for_payload(&err, 16_000)),
                "Git rename failed and the filesystem fallback also failed"
            );
            return git_io_error_response(
                "rename an untracked repository path",
                &error,
                "rename_failed",
            );
        }
    }
    git_success_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Body of a git delete request.
pub struct GitDeleteBody {
    pub path: String,
    #[serde(default)]
    pub force: bool,
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
    let tracked = match run_git(&dir, &["ls-files", "--", path]).await {
        Ok((0, stdout, _)) => !stdout.trim().is_empty(),
        Ok((code, stdout, stderr)) => {
            return completed_git_failure_response(code, &stdout, &stderr);
        }
        Err(error) => {
            return git_command_transport_error_response(
                "determine whether the delete target is tracked",
                &error,
                Some("delete_status_failed"),
            );
        }
    };

    if tracked {
        let mut args: Vec<&str> = vec!["rm"];
        if body.force {
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
    if let Err(resp) = remove_untracked_target(&full).await {
        return resp;
    }

    Json(serde_json::json!({"success": true, "staged": false})).into_response()
}
