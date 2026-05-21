use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::super::{
    DirectoryQuery, git_success_response, require_directory, require_directory_raw,
    require_locked_directory, run_git_checked,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStashEntry {
    pub r#ref: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStashListResponse {
    pub stashes: Vec<GitStashEntry>,
}

fn invalid_stash_ref_response() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "Invalid stash ref", "code": "invalid_stash_ref"})),
    )
        .into_response()
}

async fn run_stash_ref_command(
    q: &DirectoryQuery,
    raw_ref: Option<&str>,
    args_for: impl FnOnce(&str) -> Vec<&str>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let Some(stash_ref) = normalize_stash_ref(raw_ref) else {
        return invalid_stash_ref_response();
    };
    let args = args_for(stash_ref.as_str());
    if let Err(resp) = run_git_checked(&dir, &args, None).await {
        return resp;
    }
    git_success_response()
}

pub async fn git_stash_list(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let (out, _err) = match run_git_checked(&dir, &["stash", "list"], None).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let mut stashes: Vec<GitStashEntry> = Vec::new();
    for line in out.lines() {
        // stash@{0}: On branch: message
        if let Some((r, rest)) = line.split_once(':') {
            let r = r.trim();
            let title = rest.trim();
            if !r.is_empty() {
                stashes.push(GitStashEntry {
                    r#ref: r.to_string(),
                    title: title.to_string(),
                });
            }
        }
    }
    Json(GitStashListResponse { stashes }).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStashPushBody {
    pub message: Option<String>,
    pub include_untracked: Option<bool>,
    pub keep_index: Option<bool>,
    pub staged: Option<bool>,
}

pub async fn git_stash_push(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitStashPushBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let mut args: Vec<String> = vec!["stash".into(), "push".into()];
    let staged = body.staged.unwrap_or(false);
    if staged && (body.include_untracked.unwrap_or(false) || body.keep_index.unwrap_or(false)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "staged cannot be combined with includeUntracked or keepIndex",
                "code": "invalid_stash_args"
            })),
        )
            .into_response();
    }
    if staged {
        args.push("--staged".into());
    }
    if body.include_untracked.unwrap_or(false) {
        args.push("--include-untracked".into());
    }
    if body.keep_index.unwrap_or(false) {
        args.push("--keep-index".into());
    }
    if let Some(m) = body
        .message
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        args.push("-m".into());
        args.push(m.to_string());
    }

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if let Err(resp) = run_git_checked(&dir, &args_ref, None).await {
        return resp;
    }
    git_success_response()
}

#[derive(Debug, Deserialize)]
pub struct GitStashShowQuery {
    pub directory: Option<String>,
    pub r#ref: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStashShowResponse {
    pub r#ref: String,
    pub diff: String,
}

pub async fn git_stash_show(Query(q): Query<GitStashShowQuery>) -> Response {
    let dir = match require_directory_raw(q.directory.as_deref()) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };
    let Some(r) = normalize_stash_ref(q.r#ref.as_deref()) else {
        return invalid_stash_ref_response();
    };

    let (out, _err) = match run_git_checked(&dir, &["stash", "show", "-p", &r], None).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    Json(GitStashShowResponse {
        r#ref: r,
        diff: out,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStashRefBody {
    pub r#ref: Option<String>,
}

fn normalize_stash_ref(r: Option<&str>) -> Option<String> {
    let s = r.unwrap_or("").trim();
    if s.is_empty() {
        return Some("stash@{0}".to_string());
    }
    // allow "stash@{0}" or "0"
    if s.starts_with("stash@{") && s.ends_with('}') {
        return Some(s.to_string());
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("stash@{{{}}}", s));
    }
    None
}

pub async fn git_stash_apply(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitStashRefBody>,
) -> Response {
    run_stash_ref_command(&q, body.r#ref.as_deref(), |stash_ref| {
        vec!["stash", "apply", stash_ref]
    })
    .await
}

pub async fn git_stash_pop(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitStashRefBody>,
) -> Response {
    run_stash_ref_command(&q, body.r#ref.as_deref(), |stash_ref| {
        vec!["stash", "pop", stash_ref]
    })
    .await
}

pub async fn git_stash_drop(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitStashRefBody>,
) -> Response {
    run_stash_ref_command(&q, body.r#ref.as_deref(), |stash_ref| {
        vec!["stash", "drop", stash_ref]
    })
    .await
}

pub async fn git_stash_drop_all(Query(q): Query<DirectoryQuery>) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let (list_out, _list_err) = match run_git_checked(&dir, &["stash", "list"], None).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let cleared = list_out
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();

    if let Err(resp) = run_git_checked(&dir, &["stash", "clear"], None).await {
        return resp;
    }

    Json(serde_json::json!({"success": true, "cleared": cleared})).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStashBranchBody {
    pub branch: Option<String>,
    pub r#ref: Option<String>,
}

pub async fn git_stash_branch(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitStashBranchBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let Some(branch) = body
        .branch
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "branch is required"})),
        )
            .into_response();
    };
    let Some(r) = normalize_stash_ref(body.r#ref.as_deref()) else {
        return invalid_stash_ref_response();
    };

    if let Err(resp) = run_git_checked(&dir, &["stash", "branch", branch, &r], None).await {
        return resp;
    }
    git_success_response()
}
