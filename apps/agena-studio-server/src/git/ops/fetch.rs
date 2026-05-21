use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use super::super::{
    DirectoryQuery, GitAuthInput, TempGitAskpass, git_http_auth_env, normalize_http_auth,
    run_locked_git_env_checked,
};

#[derive(Debug, Deserialize)]
pub struct GitFetchBody {
    pub remote: Option<String>,
    pub branch: Option<String>,
    #[serde(default)]
    pub prune: Option<bool>,
    #[serde(default)]
    pub all: Option<bool>,
    // Alias for refspec/branch name.
    pub r#ref: Option<String>,
    #[serde(default)]
    pub auth: Option<GitAuthInput>,
}

pub async fn git_fetch(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitFetchBody>,
) -> Response {
    let remote = body
        .remote
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let branch = body
        .branch
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let rf = body
        .r#ref
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let prune = body.prune.unwrap_or(false);
    let fetch_all = body.all.unwrap_or(false);

    let mut args: Vec<String> = Vec::new();
    let mut extra_env: Vec<(String, String)> = Vec::new();
    let mut _askpass: Option<TempGitAskpass> = None;
    if let Some((u, p)) = body.auth.as_ref().and_then(normalize_http_auth) {
        match git_http_auth_env(&u, &p).await {
            Ok((prefix, env, guard)) => {
                args.extend(prefix);
                extra_env.extend(env);
                _askpass = Some(guard);
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": e, "code": "git_auth_setup_failed"})),
                )
                    .into_response();
            }
        }
    }

    args.push("fetch".into());
    if prune {
        args.push("--prune".into());
    }
    if fetch_all {
        if remote.is_some() || branch.is_some() || rf.is_some() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "remote/branch/ref are not allowed when all=true",
                    "code": "invalid_fetch_args"
                })),
            )
                .into_response();
        }
        args.push("--all".into());
    } else {
        if let Some(r) = remote {
            args.push(r.into());
        }
        if let Some(b) = branch {
            if remote.is_none() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(
                        serde_json::json!({"error": "remote is required when branch is provided"}),
                    ),
                )
                    .into_response();
            }
            args.push(b.into());
        } else if let Some(rf) = rf {
            if remote.is_none() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "remote is required when ref is provided"})),
                )
                    .into_response();
            }
            args.push(rf.into());
        }
    }
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let env_ref: Vec<(&str, &str)> = extra_env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    if let Err(resp) = run_locked_git_env_checked(&q, &args_ref, &env_ref, None).await {
        return resp;
    }
    Json(serde_json::json!({"success": true})).into_response()
}
