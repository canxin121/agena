mod app;
mod auth;
mod diagnostics;
mod error;
mod fs;
pub(crate) mod server_record;
mod git {
    pub(crate) use agena_git_http::*;
}
mod lifecycle;
pub(crate) mod mcp;
mod path_utils;
mod persistence;
mod preview;
mod state;
mod terminal;
mod user_service;
mod web_ui;

pub(crate) use error::{ApiResult, AppError};

use crate::error::AgenaProcessError;
use agena_cli::{ServerArgs, ServerLaunchRequest};

pub(crate) use state::AppState;

pub(crate) async fn run(request: ServerLaunchRequest) -> Result<(), AgenaProcessError> {
    let result = match request.args.action {
        None => app::run(request.args).await,
        Some(agena_cli::ServerLifecycleAction::Start) => lifecycle::start(request.args).await,
        Some(agena_cli::ServerLifecycleAction::Status) => lifecycle::status().await,
        Some(agena_cli::ServerLifecycleAction::Stop) => lifecycle::stop().await,
        Some(agena_cli::ServerLifecycleAction::Install) => lifecycle::install(request.args).await,
        Some(agena_cli::ServerLifecycleAction::Uninstall) => lifecycle::uninstall().await,
    };
    result.map_err(|error| AgenaProcessError::Internal(error.to_string()))
}

pub(crate) fn issue_token() -> String {
    use base64::Engine as _;

    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("issue_token: getrandom failed");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}
