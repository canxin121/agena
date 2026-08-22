use std::io::ErrorKind;
use std::path::Path;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use serde::Deserialize;

use crate::git2_utils;

use super::super::{
    MAX_BLOB_BYTES, abs_path, git_command_transport_error_response, git_io_error_response,
    git_task_error_response, git2_open_error_response, is_safe_repo_rel_path, map_git_failure,
    run_git, spawn_libgit2,
};

fn is_image_file(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" | "avif"
    )
}

fn image_mime(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        _ => "application/octet-stream",
    }
}

fn git2_read_error(context: &str, error: &git2::Error) -> git2_utils::Git2OpenError {
    git2_utils::Git2OpenError::Other(git2_utils::git2_error_diagnostic(context, error))
}

fn read_head_blob(
    repo: &git2::Repository,
    file_path: &str,
) -> Result<Option<Vec<u8>>, git2_utils::Git2OpenError> {
    let head = match repo.head() {
        Ok(head) => head,
        Err(error)
            if matches!(
                error.code(),
                git2::ErrorCode::NotFound | git2::ErrorCode::UnbornBranch
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(git2_read_error("resolve Git HEAD for a file diff", &error)),
    };
    let tree = head
        .peel_to_tree()
        .map_err(|error| git2_read_error("peel Git HEAD to a tree for a file diff", &error))?;
    let entry = match tree.get_path(Path::new(file_path)) {
        Ok(entry) => entry,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(error) => {
            return Err(git2_read_error(
                "resolve a file in the Git HEAD tree",
                &error,
            ));
        }
    };
    if entry.kind() != Some(git2::ObjectType::Blob) {
        return Ok(None);
    }
    let blob = repo
        .find_blob(entry.id())
        .map_err(|error| git2_read_error("read a blob from the Git HEAD tree", &error))?;
    if blob.size() > MAX_BLOB_BYTES {
        tracing::warn!(
            blob_size = blob.size(),
            max_blob_size = MAX_BLOB_BYTES,
            "Git HEAD file diff content was truncated because the blob is too large"
        );
        return Ok(None);
    }
    Ok(Some(blob.content().to_vec()))
}

fn read_index_blob(
    repo: &git2::Repository,
    file_path: &str,
) -> Result<Option<Vec<u8>>, git2_utils::Git2OpenError> {
    let index = repo
        .index()
        .map_err(|error| git2_read_error("open the Git index for a file diff", &error))?;
    let Some(entry) = index.get_path(Path::new(file_path), 0) else {
        return Ok(None);
    };
    let blob = repo
        .find_blob(entry.id)
        .map_err(|error| git2_read_error("read a blob from the Git index", &error))?;
    if blob.size() > MAX_BLOB_BYTES {
        tracing::warn!(
            blob_size = blob.size(),
            max_blob_size = MAX_BLOB_BYTES,
            "Git index file diff content was truncated because the blob is too large"
        );
        return Ok(None);
    }
    Ok(Some(blob.content().to_vec()))
}

#[derive(Debug, Deserialize)]
/// Query for a file diff.
pub struct GitFileDiffQuery {
    pub directory: Option<String>,
    pub path: Option<String>,
    // If true, compare HEAD -> index; else compare index -> workdir.
    #[serde(default)]
    pub staged: bool,
}

pub async fn git_file_diff(Query(q): Query<GitFileDiffQuery>) -> Response {
    let Some(dir_raw) = q.directory.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "directory parameter is required"})),
        )
            .into_response();
    };
    let dir = match abs_path(dir_raw) {
        Ok(dir) => dir,
        Err(response) => return *response,
    };
    let Some(file_path) = q
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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }

    // Git panel needs different bases depending on whether we're previewing staged or unstaged.
    // - staged=true:  original=HEAD,  modified=index
    // - staged=false: original=index, modified=workdir
    let staged = q.staged;

    let mut original = String::new();
    let mut modified = String::new();

    if is_image_file(file_path) {
        let mime = image_mime(file_path);

        let repo_bytes = spawn_libgit2({
            let dir = dir.clone();
            let file_path = file_path.to_string();
            move || -> Result<(Vec<u8>, Vec<u8>), git2_utils::Git2OpenError> {
                let repo = git2_utils::open_repo_discover(&dir)?;

                let head_bytes = if staged {
                    read_head_blob(&repo, &file_path)?.unwrap_or_default()
                } else {
                    Vec::new()
                };
                let index_bytes = read_index_blob(&repo, &file_path)?.unwrap_or_default();

                Ok((head_bytes, index_bytes))
            }
        })
        .await;

        let (head_bytes, index_bytes) = match repo_bytes {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return git2_open_error_response(e),
            Err(error) => {
                return git_task_error_response("read image content for a Git diff", &error);
            }
        };

        let workdir_bytes = if staged {
            Vec::new()
        } else {
            let full = dir.join(file_path);
            match tokio::fs::read(&full).await {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
                Err(error) => {
                    return git_io_error_response(
                        "failed to read working-tree image for Git diff",
                        &error,
                        "worktree_image_read_failed",
                    );
                }
            }
        };

        let (a, b) = if staged {
            (head_bytes, index_bytes)
        } else {
            (index_bytes, workdir_bytes)
        };

        if !a.is_empty() {
            let b64 = base64::engine::general_purpose::STANDARD.encode(a);
            original = format!("data:{mime};base64,{b64}");
        }
        if !b.is_empty() && b.len() <= MAX_BLOB_BYTES {
            let b64 = base64::engine::general_purpose::STANDARD.encode(b);
            modified = format!("data:{mime};base64,{b64}");
        }
    } else {
        let repo_text = spawn_libgit2({
            let dir = dir.clone();
            let file_path = file_path.to_string();
            move || -> Result<(String, String), git2_utils::Git2OpenError> {
                let repo = git2_utils::open_repo_discover(&dir)?;

                let head_text = if staged {
                    read_head_blob(&repo, &file_path)?
                        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let index_text = read_index_blob(&repo, &file_path)?
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_default();

                Ok((head_text, index_text))
            }
        })
        .await;

        let (head_text, index_text) = match repo_text {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return git2_open_error_response(e),
            Err(error) => {
                return git_task_error_response("read text content for a Git diff", &error);
            }
        };

        let workdir_text = if staged {
            String::new()
        } else {
            let full = dir.join(file_path);
            match tokio::fs::metadata(&full).await {
                Ok(meta) if meta.is_file() && meta.len() <= MAX_BLOB_BYTES as u64 => {
                    match tokio::fs::read_to_string(&full).await {
                        Ok(text) => text,
                        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
                        Err(error) => {
                            return git_io_error_response(
                                "failed to read working-tree text for Git diff",
                                &error,
                                "worktree_text_read_failed",
                            );
                        }
                    }
                }
                Ok(_) => String::new(),
                Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
                Err(error) => {
                    return git_io_error_response(
                        "failed to inspect working-tree file for Git diff",
                        &error,
                        "worktree_metadata_failed",
                    );
                }
            }
        };

        if staged {
            original = head_text;
            modified = index_text;
        } else {
            original = index_text;
            modified = workdir_text;
        }
    }

    Json(serde_json::json!({"original": original, "modified": modified, "path": file_path}))
        .into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Query comparing two revisions.
pub struct GitCompareQuery {
    pub directory: Option<String>,
    pub base: Option<String>,
    pub head: Option<String>,
    pub path: Option<String>,
    #[serde(rename = "contextLines")]
    pub context_lines: Option<String>,
}

pub async fn git_compare(Query(q): Query<GitCompareQuery>) -> Response {
    let Some(dir_raw) = q.directory.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "directory parameter is required"})),
        )
            .into_response();
    };
    let dir = match abs_path(dir_raw) {
        Ok(dir) => dir,
        Err(response) => return *response,
    };
    let Some(base) = q
        .base
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "base is required", "code": "missing_base"})),
        )
            .into_response();
    };
    let Some(head) = q
        .head
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "head is required", "code": "missing_head"})),
        )
            .into_response();
    };

    let path = q
        .path
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    if let Some(p) = path
        && !is_safe_repo_rel_path(p)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }

    let context = q
        .context_lines
        .as_deref()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(3)
        .clamp(0, 500) as u32;

    let range = format!("{}...{}", base, head);
    let mut args: Vec<String> = vec![
        "diff".into(),
        "--no-color".into(),
        "--no-ext-diff".into(),
        format!("-U{}", context),
        range,
    ];
    if let Some(p) = path {
        args.push("--".into());
        args.push(p.to_string());
    }

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (code, out, err) = match run_git(&dir, &args_ref).await {
        Ok(result) => result,
        Err(error) => {
            return git_command_transport_error_response(
                "compare Git revisions",
                &error,
                Some("git_compare_process_failed"),
            );
        }
    };
    if code != 0 {
        if let Some(resp) = map_git_failure(code, &out, &err) {
            return resp;
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err.trim(), "code": "git_compare_failed"})),
        )
            .into_response();
    }

    Json(serde_json::json!({"diff": out})).into_response()
}
