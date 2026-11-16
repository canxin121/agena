//! Bundled Agena plugin implementations and their registration factories.
//!
//! The crate also publishes a generated Markdown reference of every bundled
//! tool — see [`tools_docs`] — so `cargo doc` renders all tool definitions
//! together with their detailed help text, tags, runtime flags, and JSON
//! Schema contracts.

mod artifact_file;
pub mod capability_manifest;
pub mod docs_reference;
pub mod memory;
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

/// Generated Markdown reference for every bundled tool: definitions, detailed
/// help text, tags, runtime flags, and JSON Schema contracts.
///
/// The committed copy lives at `generated/tools-reference.md` (in this crate) and is
/// regenerated with `agena inspect --tools-reference`; a CI drift test keeps
/// it in sync with the real plugin manifests.
#[doc = include_str!("../generated/tools-reference.md")]
pub mod tools_docs {}

pub use capability_manifest::{
    BundledCapabilityManifest, bundled_capability_identity_snapshot_json,
    bundled_capability_manifest,
};
pub use docs_reference::bundled_tools_markdown_reference;
pub use sandbox_worker::{execute_sandbox_tool, execute_sandbox_tool_with_read_filter};

pub use agena_runtime_config as config;
pub use agena_runtime_config::{LSP_PLUGIN_ID, LspConfig, MCP_PLUGIN_ID};
pub use agena_runtime_contracts::{authorization, part, permission, provider_state};
