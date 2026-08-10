//! Bundled Agena plugin implementations and their registration factories.
//!
//! The crate also publishes a generated Markdown reference of every bundled
//! tool — see [`tools_docs`] — so `cargo doc` renders all tool definitions
//! together with their detailed help text, examples, tags, runtime flags, and
//! JSON Schema contracts.

pub mod capability_manifest;
pub mod docs_reference;
pub mod memory;
pub mod plugins;
pub mod tool;
pub mod web;

/// Generated Markdown reference for every bundled tool: definitions, detailed
/// help text, examples, tags, runtime flags, and JSON Schema contracts.
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

pub use agena_runtime_config as config;
pub use agena_runtime_config::{LSP_PLUGIN_ID, LspConfig, MCP_PLUGIN_ID};
pub use agena_runtime_contracts::{authorization, part, permission, provider_state};
