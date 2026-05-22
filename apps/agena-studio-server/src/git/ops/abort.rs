use axum::{Json, extract::Query, response::Response};
use serde::Deserialize;

use super::super::{
    DirectoryQuery, git_success_response, require_locked_directory, run_git_checked,
};

#[derive(Debug, Deserialize)]
pub struct GitAbortBody {
    // reserved for future
    pub _dummy: Option<bool>,
}

async fn run_abort_command(q: &DirectoryQuery, args: &[&str]) -> Response {
    let (dir, _guard) = match require_locked_directory(q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(resp) = run_git_checked(&dir, args, None).await {
        return resp;
    }
    git_success_response()
}

pub async fn git_merge_abort(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitAbortBody>,
) -> Response {
    run_abort_command(&q, &["merge", "--abort"]).await
}

pub async fn git_rebase_abort(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitAbortBody>,
) -> Response {
    run_abort_command(&q, &["rebase", "--abort"]).await
}

pub async fn git_cherry_pick_abort(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitAbortBody>,
) -> Response {
    run_abort_command(&q, &["cherry-pick", "--abort"]).await
}

pub async fn git_revert_abort(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitAbortBody>,
) -> Response {
    run_abort_command(&q, &["revert", "--abort"]).await
}
