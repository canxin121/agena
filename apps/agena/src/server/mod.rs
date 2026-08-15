mod app;
mod auth;
pub(crate) mod server_record;
mod diagnostics;
mod lifecycle;
mod state;
mod user_service;

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
