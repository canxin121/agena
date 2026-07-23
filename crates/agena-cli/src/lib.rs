//! Command-line presentation for Agena.
//!
//! This crate owns the process argument schema, command dispatch, textual and
//! JSON output. Runtime construction remains the responsibility of the app.

pub mod cli;

pub use cli::*;
