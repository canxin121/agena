//! # agena-cli
//!
//! Command-line presentation for Agena.
//!
//! This crate owns the process argument schema ([`AgenaCli`]), command
//! dispatch, and textual/JSON output formatting. Runtime construction remains
//! the responsibility of the app layer ([`agena_application`]).
//!
//! [`LaunchMode`] describes how a parsed CLI invocation should start: TUI,
//! RPC server, HTTP server, or a one-shot command.

pub mod cli;

pub use cli::*;
