use axum::{Json, extract::Query, response::Response};
use serde::Deserialize;

use super::super::{
    DirectoryQuery, git_success_response, require_locked_directory, run_git_checked,
};

#[derive(Debug, Deserialize)]
pub struct GitContinueBody {
    pub _dummy: Option<bool>,
}

async fn run_continue_command(q: &DirectoryQuery, args: &[&str]) -> Response {
    let (dir, _guard) = match require_locked_directory(q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(resp) = run_git_checked(&dir, args, None).await {
        return resp;
    }
    git_success_response()
}

pub async fn git_rebase_continue(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitContinueBody>,
) -> Response {
    run_continue_command(&q, &["rebase", "--continue"]).await
}

pub async fn git_rebase_skip(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitContinueBody>,
) -> Response {
    run_continue_command(&q, &["rebase", "--skip"]).await
}

pub async fn git_cherry_pick_continue(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitContinueBody>,
) -> Response {
    run_continue_command(&q, &["cherry-pick", "--continue"]).await
}

pub async fn git_cherry_pick_skip(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitContinueBody>,
) -> Response {
    run_continue_command(&q, &["cherry-pick", "--skip"]).await
}

pub async fn git_revert_continue(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitContinueBody>,
) -> Response {
    run_continue_command(&q, &["revert", "--continue"]).await
}

pub async fn git_revert_skip(
    Query(q): Query<DirectoryQuery>,
    Json(_): Json<GitContinueBody>,
) -> Response {
    run_continue_command(&q, &["revert", "--skip"]).await
}
