//! Bundled Agena plugin implementations and their registration factories.

pub mod memory;
pub mod plugins;
pub mod tool;
pub mod web;

pub use agena_runtime_config as config;
pub use agena_runtime_config::{LSP_PLUGIN_ID, LspConfig, MCP_PLUGIN_ID};
pub use agena_runtime_contracts::{agent, message, permission};
