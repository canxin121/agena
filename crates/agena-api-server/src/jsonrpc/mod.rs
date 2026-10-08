//! JSON-RPC app-server transport (feature-gated).

pub mod protocol;
mod server;

pub use server::{AppServer, AppServerBackend, AppServerError, serve_stdio};
