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
mod platform_compat;
mod server;

pub(crate) use server::{ApiResult, AppError, AppState};

use agena_cli::{AgenaCli, LaunchMode};
use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    // Worker modes construct TLS clients before normal application bootstrap.
    // Cargo can unify rustls-no-provider through the content extraction crates.
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Sandboxed helpers must run before configuration, credentials or plugins load.
    if let Some(code) = agena_process::sandbox::try_run_launcher() {
        std::process::exit(code);
    }
    if let Some(code) = agena_process::sandbox_check::try_run_cli() {
        std::process::exit(code);
    }
    if let Some(code) = agena_process::sandbox_http::try_run_worker() {
        std::process::exit(code);
    }
    if let Some(code) = agena_process::sandbox_file::try_run_worker() {
        std::process::exit(code);
    }
    if let Some(code) = agena_mcp_server::stdio_relay::try_run_worker() {
        std::process::exit(code);
    }
    if let Some(code) = agena_mcp_server::antigravity_guard::try_run_worker() {
        std::process::exit(code);
    }
    if let Some(code) = agena_bundled_plugins::worker::try_run_worker() {
        std::process::exit(code);
    }
    #[cfg(target_os = "macos")]
    if let Some(code) = server::macos_privacy_probe::try_run_cli() {
        std::process::exit(code);
    }
    if let Some(code) = agena_runtime_tools::sandbox_worker::try_run_worker(
        agena_bundled_plugins::execute_sandbox_tool,
    ) {
        std::process::exit(code);
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Returning `Result` directly from `main` makes Rust's blanket
            // `Termination` implementation print `Debug`, which exposes enum
            // wrappers such as `Internal("...")`. Render the deliberate
            // process-level Display text instead; every conversion into this
            // type has already retained the complete diagnostic chain.
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> error::Result<()> {
    agena_runtime::ensure_default_thread_stack();
    let cli = AgenaCli::parse();
    // Capture trusted sandbox configuration before plugins or tools run.
    let _ = agena_process::sandbox::enabled();
    let root = cli.data_dir.clone().unwrap_or_else(|| {
        agena_storage_layout::StorageLayout::from_env()
            .root()
            .to_path_buf()
    });
    let layout = agena_storage_layout::StorageLayout::initialize_process(root)?;
    layout.prepare()?;
    agena_runtime::build_app_runtime()?.block_on(async {
        match cli.into_launch_mode() {
            LaunchMode::Tui(request) => launch::tui::run(request).await,
            LaunchMode::RpcServer(request) => launch::rpc_server::run(request).await,
            LaunchMode::Server(request) => server::run(request).await,
            LaunchMode::Command(cli) => launch::rpc_server::run_command(cli).await,
        }
    })
}
