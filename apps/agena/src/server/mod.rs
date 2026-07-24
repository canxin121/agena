mod app;
mod attachment;
mod auth;
mod config;
mod cors;
mod diagnostics;
mod error;
mod fs;
mod git {
    pub(crate) use agena_git_http::*;
}
mod path_utils;
mod persistence;
mod preview;
mod providers;
mod settings;
mod state;
mod terminal;

use crate::error::AgenaProcessError;
use agena_cli::{ServerArgs, ServerLaunchRequest, UiCookieSameSite};

pub(crate) use error::{ApiResult, AppError};
pub(crate) use state::AppState;

pub(crate) async fn run(request: ServerLaunchRequest) -> Result<(), AgenaProcessError> {
    app::run(request.args)
        .await
        .map_err(|error| AgenaProcessError::Internal(error.to_string()))
}

pub(crate) fn issue_token() -> String {
    use base64::Engine as _;

    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("issue_token: getrandom failed");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}
