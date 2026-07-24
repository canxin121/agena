//! Plugin runtime lifecycle, contracts, and composition ports.

pub mod plugin_config;
pub mod plugin_runtime_service;
pub mod plugin_shutdown;
pub mod plugin_slot;
pub mod plugins;

mod callback_guard;
pub use callback_guard::CallbackOnDrop;

pub use agena_runtime_config as config;
pub use agena_runtime_config::{LSP_PLUGIN_ID, LspConfig, MCP_PLUGIN_ID};
pub use agena_runtime_contracts::{agent, message, permission};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInvocationRequest {
    pub plugin_id: String,
    pub method: String,
}
