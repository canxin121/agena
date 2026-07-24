#![allow(unused_imports)]

mod error;
mod launch;
mod server;

pub(crate) use server::{ApiResult, AppError, AppState};

use agena_cli::{AgenaCli, LaunchMode};
use clap::Parser;

fn main() -> error::Result<()> {
    agena_runtime::build_app_runtime()?.block_on(async {
        match AgenaCli::parse().into_launch_mode() {
            LaunchMode::Tui(request) => launch::tui::run(request).await,
            LaunchMode::RpcServer(request) => launch::rpc_server::run(request).await,
            LaunchMode::Server(request) => server::run(request).await,
            LaunchMode::Command(cli) => launch::rpc_server::run_command(cli).await,
        }
    })
}
