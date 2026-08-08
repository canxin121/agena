//! # agena-plugin-sdk
//!
//! Agena Plugin SDK — write a plugin once, ship it as in-process / cdylib /
//! stdio / HTTP.
//!
//! Implement [`Plugin`] for your type, fill in the hooks you care about
//! (every method has a default no-op), and pick a transport with one of the
//! `export_*!` macros.
//!
//! ## Quick start
//!
//! ```ignore
//! use agena_plugin_sdk::prelude::*;
//!
//! #[derive(Default, PluginConfigStore)]
//! struct MyPlugin;
//!
//! #[agena_plugin(namespace = "demo", name = "hello", version = "0.1.0", export = cdylib)]
//! impl MyPlugin {
//!     #[tool(name = "hello", summary = "Say hello", read_only)]
//!     async fn hello(&self) -> Result<String> {
//!         Ok("hello".into())
//!     }
//! }
//! ```
//!
//! ## Key items
//!
//! - [`Plugin`], [`PluginConfig`], [`InitContext`], [`InitOutcome`] — plugin
//!   contract and lifecycle.
//! - [`ToolStreamSink`] — streaming output sink for tools.
//! - [`PluginError`] — typed plugin errors.
//! - [`PluginKey`] / [`ToolKey`] — stable identifiers.
//! - [`AttachmentKind`] — attachment classification.
//! - [`prelude`] — everything a plugin author needs in one import.
//! - `agena_macros` re-exports: [`agena_plugin`], [`ToolInput`],
//!   [`PluginConfigStore`].

pub extern crate schemars;
extern crate self as agena_plugin_sdk;
pub extern crate serde;
pub extern crate serde_json;

pub mod activity;
pub mod attachment;
pub mod error;
pub mod hooks;
pub mod host_api;
pub mod identity;
#[doc(hidden)]
pub mod macro_support;
pub mod manifest;
mod manifest_support;
pub mod plugin;
pub mod prelude;
pub mod rpc;

#[cfg(feature = "cdylib")]
pub mod cdylib_abi;

#[cfg(any(feature = "cdylib", feature = "stdio", feature = "http"))]
pub mod drivers;

pub use activity::ActivitySourceAdapter;
pub use agena_macros::{PluginConfigStore, ToolInput, agena_plugin};
pub use async_trait::async_trait;
pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
pub use error::{CONFIGURATION_REQUIRED_MARKER, PluginError, PluginErrorKind, Result};
pub use hooks::*;
pub use host_api::{
    HostClient, HostImageExecuteRequest, HostImageExecuteResponse, HostImageInput,
    HostImageOperation, HostPluginDescriptor, HostPluginListResponse, NoopHostClient,
    PluginNotifyAction, PluginNotifyActionTarget, PluginNotifyRequest,
};
pub use identity::{PluginKey, PluginKeyParseError, ToolKey, ToolKeyParseError};
pub use macro_support::{schema_example_texts, schema_usage_text};
pub use manifest::{
    ContributionKind, HookSubscription, InputNetworkSpec, InputPathSpec, NetworkAccessSpec,
    PLUGIN_WORKBENCH_TAB_IDS, PathAccessSpec, PathKind, PluginCommandDefinition,
    PluginDisplayContent, PluginDisplayContribution, PluginManifest, PluginSkillDefinition,
    PluginStudioCommand, PluginStudioControl, PluginStudioControlOption,
    PluginStudioUiContributions, PluginStudioView, PluginTuiColor, PluginTuiThemeColors,
    PluginTuiUiContributions, PluginUiAction, PluginUiContributions, PluginUiThemePalette,
    ToolDefinition, ToolInput, ToolPermissionContract,
    ToolResultPolicy, ToolResultRenderKind, ToolStreamingMode, ToolTag, TransportKind,
    normalize_tool_tag_name, plugin_workbench_tab_id_is_supported,
};
pub use plugin::{InitContext, InitOutcome, Plugin, PluginConfig, ToolStreamSink};
pub use schemars::JsonSchema;

// Re-exports used by macros so plugin authors don't have to add deps directly.
#[doc(hidden)]
#[cfg(feature = "cdylib")]
pub use abi_stable as abi_stable_reexport;
