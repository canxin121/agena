use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use super::super::{
    DirectoryQuery, git_success_response, require_locked_directory, run_git_checked,
};

#[derive(Debug, Deserialize)]
/// Body of a git merge request.
pub struct GitMergeBody {
    pub branch: Option<String>,
}

async fn run_branch_operation(
    q: &DirectoryQuery,
    branch: Option<&str>,
    args_for: impl FnOnce(&str) -> Vec<&str>,
    failure_code: &'static str,
) -> Response {
    let (dir, _guard) = match require_locked_directory(q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let Some(branch) = branch.map(str::trim).filter(|value| !value.is_empty()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "branch is required", "code": "missing_branch"})),
        )
            .into_response();
    };
    let args = args_for(branch);
    if let Err(resp) = run_git_checked(&dir, &args, Some(failure_code)).await {
        return resp;
    }
    git_success_response()
}

pub async fn git_merge(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitMergeBody>,
) -> Response {
    run_branch_operation(
        &q,
        body.branch.as_deref(),
        |branch| vec!["merge", "--no-edit", branch],
        "git_merge_failed",
    )
    .await
}

#[derive(Debug, Deserialize)]
/// Body of a git rebase request.
pub struct GitRebaseBody {
    pub branch: Option<String>,
}

pub async fn git_rebase(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitRebaseBody>,
) -> Response {
    run_branch_operation(
        &q,
        body.branch.as_deref(),
        |branch| vec!["rebase", branch],
        "git_rebase_failed",
    )
    .await
}
