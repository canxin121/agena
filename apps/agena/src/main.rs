//! # agena
//!
//! Unified command-line binary for the Agena agent runtime.
//!
//! The binary parses the shared [`agena_cli`] command surface and launches one
//! of four modes:
//!
//! - **TUI** — the interactive terminal UI.
//! - **RPC server** — the JSON-RPC app-server surface.
//! - **HTTP server** — the REST/WebSocket/SSE API server.
//! - **Command** — one-shot CLI commands (`exec`, `config`, `provider`,
//!   `session`, and friends).
//!
//! The server owns [`agena_runtime`], the session store, scheduler,
//! plugin host, and execution leases. TUI and JSON-RPC modes are thin clients
//! of that server; explicit embedded TUI mode remains available for recovery
//! and development.

#![allow(unused_imports)]

mod error;
mod launch;
mod server;

pub(crate) use server::AppState;

use agena_cli::{AgenaCli, LaunchMode};
use clap::Parser;

fn main() -> error::Result<()> {
    agena_runtime::ensure_default_thread_stack();
    agena_runtime::build_app_runtime()?.block_on(async {
        match AgenaCli::parse().into_launch_mode() {
            LaunchMode::Tui(request) => launch::tui::run(request).await,
            LaunchMode::RpcServer(request) => launch::rpc_server::run(request).await,
            LaunchMode::Server(request) => server::run(request).await,
            LaunchMode::Command(cli) => launch::rpc_server::run_command(cli).await,
        }
    })
}
