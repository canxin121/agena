use std::collections::HashMap;
use std::io::ErrorKind;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::super::{
    DirectoryQuery, git_io_error_response, is_safe_repo_rel_path, require_directory,
    require_directory_raw, require_locked_directory, run_git_checked, run_git_checked_with_status,
};

use super::file_diff::GitFileDiffQuery;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// List of conflicted files.
pub struct GitConflictsListResponse {
    pub files: Vec<String>,
}

pub async fn git_conflicts_list(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };
    // `ls-files -u` is the most reliable source for unresolved entries,
    // including cases where conflict markers are not textual.
    let (out, _) = match run_git_checked(&dir, &["ls-files", "-u"], None).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let mut files: Vec<String> = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let path = if let Some((_left, right)) = t.split_once('\t') {
            right.trim()
        } else {
            t.split_whitespace().last().unwrap_or("").trim()
        };
        if !path.is_empty() {
            files.push(path.to_string());
        }
    }
    files.sort();
    files.dedup();
    Json(GitConflictsListResponse { files }).into_response()
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
/// A conflict block of a file.
pub struct ConflictBlock {
    pub id: usize,
    pub ours_label: Option<String>,
    pub base_label: Option<String>,
    pub theirs_label: Option<String>,
    pub ours: String,
    pub base: String,
    pub theirs: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Conflict view of one file.
pub struct GitConflictFileResponse {
    pub path: String,
    pub text: String,
    pub blocks: Vec<ConflictBlock>,
    pub has_markers: bool,
    pub is_unmerged: bool,
}

async fn git_path_is_unmerged(dir: &std::path::Path, path: &str) -> Result<bool, Response> {
    let (out, _) = run_git_checked(
        dir,
        &["ls-files", "-u", "--", path],
        Some("conflict_state_failed"),
    )
    .await?;
    Ok(out.lines().any(|line| !line.trim().is_empty()))
}

fn parse_conflict_markers(text: &str) -> Vec<ConflictBlock> {
    let mut blocks: Vec<ConflictBlock> = Vec::new();
    let mut state = 0;
    let mut ours: Vec<String> = Vec::new();
    let mut base: Vec<String> = Vec::new();
    let mut theirs: Vec<String> = Vec::new();
    let mut ours_label: Option<String> = None;
    let mut base_label: Option<String> = None;
    let mut id: usize = 0;

    for line in text.lines() {
        if line.starts_with("<<<<<<<") {
            state = 1;
            ours.clear();
            base.clear();
            theirs.clear();
            ours_label = line
                .strip_prefix("<<<<<<<")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            base_label = None;
            continue;
        }
        if state == 1 && line.starts_with("|||||||") {
            state = 2;
            base_label = line
                .strip_prefix("|||||||")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            continue;
        }
        if (state == 1 || state == 2) && line.starts_with("=======") {
            state = 3;
            continue;
        }
        if state == 3 && line.starts_with(">>>>>>>") {
            let theirs_label = line
                .strip_prefix(">>>>>>>")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            blocks.push(ConflictBlock {
                id,
                ours_label: ours_label.clone(),
                base_label: base_label.clone(),
                theirs_label,
                ours: ours.join("\n"),
                base: base.join("\n"),
                theirs: theirs.join("\n"),
            });
            id += 1;
            state = 0;
            continue;
        }

        if state == 1 {
            ours.push(line.to_string());
        } else if state == 2 {
            base.push(line.to_string());
        } else if state == 3 {
            theirs.push(line.to_string());
        }
    }

    blocks
}

pub async fn git_conflict_file(Query(q): Query<GitFileDiffQuery>) -> Response {
    // Reuse `GitFileDiffQuery` for directory+path query params.
    let dir = match require_directory_raw(q.directory.as_deref()) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };
    let Some(path) = q
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
    if !is_safe_repo_rel_path(path) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }

    let full = dir.join(path);
    let meta = match tokio::fs::metadata(&full).await {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found", "code": "not_found"})),
            )
                .into_response();
        }
        Err(error) => {
            return git_io_error_response(
                "failed to inspect conflicted file",
                &error,
                "conflict_file_metadata_failed",
            );
        }
    };
    if !meta.is_file() || meta.len() > (512 * 1024) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File too large", "code": "too_large"})),
        )
            .into_response();
    }

    let is_unmerged = match git_path_is_unmerged(&dir, path).await {
        Ok(is_unmerged) => is_unmerged,
        Err(response) => return response,
    };
    let text = match tokio::fs::read_to_string(&full).await {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found", "code": "not_found"})),
            )
                .into_response();
        }
        Err(error) => {
            return git_io_error_response(
                "failed to read conflicted file",
                &error,
                "conflict_file_read_failed",
            );
        }
    };
    let blocks = parse_conflict_markers(&text);
    let has_markers = !blocks.is_empty()
        && text.contains("<<<<<<<")
        && text.contains(">>>>>>>")
        && text.contains("=======");
    Json(GitConflictFileResponse {
        path: path.to_string(),
        text,
        blocks,
        has_markers,
        is_unmerged,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Body of a conflict resolution request.
pub struct GitConflictResolveBody {
    pub path: Option<String>,
    // "ours" | "theirs" | "base" | "both" | "manual"
    pub strategy: Option<String>,
    #[serde(default = "default_stage_conflict_resolution")]
    pub stage: bool,
    // For "manual": list of (block id -> choice)
    #[serde(default)]
    pub choices: Vec<serde_json::Value>,
}

fn default_stage_conflict_resolution() -> bool {
    true
}

fn apply_conflict_choices(
    text: &str,
    choices: &HashMap<usize, String>,
    default_choice: &str,
) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut state = 0;
    let mut ours: Vec<String> = Vec::new();
    let mut base: Vec<String> = Vec::new();
    let mut theirs: Vec<String> = Vec::new();
    let mut id: usize = 0;

    for line in text.lines() {
        if line.starts_with("<<<<<<<") {
            state = 1;
            ours.clear();
            base.clear();
            theirs.clear();
            continue;
        }
        if state == 1 && line.starts_with("|||||||") {
            state = 2;
            continue;
        }
        if (state == 1 || state == 2) && line.starts_with("=======") {
            state = 3;
            continue;
        }
        if state == 3 && line.starts_with(">>>>>>>") {
            let choice = choices
                .get(&id)
                .map(|s| s.as_str())
                .unwrap_or(default_choice);
            if choice == "ours" {
                out.extend(ours.clone());
            } else if choice == "base" {
                out.extend(base.clone());
            } else if choice == "theirs" {
                out.extend(theirs.clone());
            } else {
                // both
                out.extend(ours.clone());
                out.extend(theirs.clone());
            }
            id += 1;
            state = 0;
            continue;
        }

        if state == 0 {
            out.push(line.to_string());
        } else if state == 1 {
            ours.push(line.to_string());
        } else if state == 2 {
            base.push(line.to_string());
        } else {
            theirs.push(line.to_string());
        }
    }

    out.join("\n") + "\n"
}

pub async fn git_conflict_resolve(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitConflictResolveBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let Some(path) = body
        .path
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path is required", "code": "missing_path"})),
        )
            .into_response();
    };
    if !is_safe_repo_rel_path(path) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }
    let strategy = body
        .strategy
        .as_deref()
        .unwrap_or("manual")
        .trim()
        .to_ascii_lowercase();
    if strategy == "ours" || strategy == "theirs" {
        let flag = if strategy == "ours" {
            "--ours"
        } else {
            "--theirs"
        };
        if let Err(resp) = run_git_checked_with_status(
            &dir,
            &["checkout", flag, "--", path],
            StatusCode::BAD_REQUEST,
            Some("checkout_conflict_failed"),
        )
        .await
        {
            return resp;
        }
    } else {
        let full = dir.join(path);
        let text = match tokio::fs::read_to_string(&full).await {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "File not found", "code": "not_found"})),
                )
                    .into_response();
            }
            Err(error) => {
                return git_io_error_response(
                    "failed to read conflicted file before resolution",
                    &error,
                    "conflict_file_read_failed",
                );
            }
        };
        if !text.contains("<<<<<<<") {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "No conflict markers found",
                    "code": "no_markers"
                })),
            )
                .into_response();
        }

        let mut map: HashMap<usize, String> = HashMap::new();
        for item in body.choices {
            let Some(id) = item.get("id").and_then(|v| v.as_u64()) else {
                continue;
            };
            let Some(choice) = item.get("choice").and_then(|v| v.as_str()) else {
                continue;
            };
            let c = choice.trim().to_ascii_lowercase();
            if c == "ours" || c == "theirs" || c == "base" || c == "both" {
                map.insert(id as usize, c);
            }
        }

        let default_choice = if strategy == "both" {
            "both"
        } else if strategy == "base" {
            "base"
        } else {
            "ours"
        };
        let new_text = apply_conflict_choices(&text, &map, default_choice);
        if let Err(e) =
            super::super::atomic_file::write_file_atomically(full, new_text.into_bytes()).await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string(), "code": "write_failed"})),
            )
                .into_response();
        }
    }

    if body.stage
        && let Err(resp) = run_git_checked_with_status(
            &dir,
            &["add", "--", path],
            StatusCode::CONFLICT,
            Some("stage_failed"),
        )
        .await
    {
        return resp;
    }

    Json(serde_json::json!({"success": true})).into_response()
}
