//! Bundled Agena plugin implementations and their registration factories.
//!
//! Browse [`tools_docs`] for the bundled plugin and tool reference. The index
//! links to the settings, commands, service declarations, and tool pages.
//! Search Rustdoc by a complete tool ID such as `agena.tool_script.run`.

#![recursion_limit = "256"]

mod artifact_file;
mod browser;
pub mod capability_manifest;
pub mod docs_reference;
pub mod memory;
mod network_policy;
mod permissions;
mod plugin_metadata;
mod plugin_settings_docs;
mod plugin_tool_docs;
pub mod plugins;
mod sandbox_operations;
mod sandbox_worker;
pub mod tool;
pub mod web;
pub mod worker;

pub(crate) static BLOCKING_PLUGIN_WORKERS: tokio::sync::Semaphore =
    tokio::sync::Semaphore::const_new(32);

#[cfg(doc)]
include!(concat!(env!("OUT_DIR"), "/tools_docs.rs"));

#[cfg(not(doc))]
/// Bundled plugin and tool reference rendered by Rustdoc.
pub mod tools_docs {}

pub use capability_manifest::{
    BundledCapabilityManifest, bundled_capability_identity_snapshot_json,
    bundled_capability_manifest,
};
pub use docs_reference::bundled_tools_markdown_reference;
pub use sandbox_worker::execute_sandbox_tool;

pub use agena_runtime_config as config;
pub use agena_runtime_config::{LSP_PLUGIN_ID, LspConfig, MCP_PLUGIN_ID};
pub use agena_runtime_contracts::{authorization, part, permission, provider_state};
