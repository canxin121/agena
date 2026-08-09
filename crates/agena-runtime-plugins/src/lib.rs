//! # agena-runtime-plugins
//!
//! Plugin runtime lifecycle, contracts, and composition ports.
//!
//! Owns plugin configuration ([`plugin_config`]), the runtime service that
//! loads and manages plugins ([`plugin_runtime_service`]), plugin slots and
//! shutdown coordination ([`plugin_slot`], [`plugin_shutdown`]), and the
//! bundled plugin set ([`plugins`]). Also re-exports configuration and
//! contract surfaces used across the runtime.

pub mod plugin_config;
pub mod plugin_runtime_service;
pub mod plugin_shutdown;
pub mod plugin_slot;
pub mod plugins;

mod callback_guard;
pub use callback_guard::CallbackOnDrop;

pub use agena_runtime_config as config;
pub use agena_runtime_config::{LSP_PLUGIN_ID, LspConfig, MCP_PLUGIN_ID};
pub use agena_runtime_contracts::{authorization, part, permission, provider_state};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Request to invoke a plugin method.
pub struct PluginInvocationRequest {
    pub plugin_id: String,
    pub method: String,
}
