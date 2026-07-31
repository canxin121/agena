//! Agena Plugin SDK — write a plugin once, ship it as in-process / cdylib / stdio / HTTP.
//!
//! Implement [`Plugin`] for your type, fill in the hooks you care about (every method
//! has a default no-op), and pick a transport with one of the `export_*!` macros.

pub extern crate schemars;
extern crate self as agena_plugin_sdk;
pub extern crate serde;
pub extern crate serde_json;

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

pub use agena_macros::{PluginConfigStore, ToolInput, agena_plugin};
pub use async_trait::async_trait;
pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
pub use error::{PluginError, PluginErrorCode, Result};
pub use hooks::*;
pub use host_api::{
    HostClient, HostImageExecuteRequest, HostImageExecuteResponse, HostImageInput,
    HostImageOperation, HostNetworkPermissionCheckRequest, HostPathPermissionCheckRequest,
    HostPermissionCheckResponse, HostPermissionOutcome, NoopHostClient,
};
pub use identity::{PluginKey, PluginKeyParseError, ToolKey, ToolKeyParseError};
pub use macro_support::{schema_example_texts, schema_usage_text};
pub use manifest::{
    HookSubscription, HostCapability, InputNetworkSpec, InputPathSpec, NetworkAccessSpec,
    PLUGIN_WORKBENCH_TAB_IDS, PathAccessSpec, PathKind, PluginCommandDefinition, PluginManifest,
    PluginSkillDefinition, PluginStudioCommand, PluginStudioControl, PluginStudioControlOption,
    PluginStudioUiContributions, PluginStudioView, PluginTuiColor, PluginTuiContentBlock,
    PluginTuiStatuslineSegment, PluginTuiThemeColors, PluginTuiUiContributions, PluginUiAction,
    PluginUiContributions, PluginUiThemePalette, ToolDefinition, ToolDescriptionMode,
    ToolDisplayPreset, ToolInput, ToolResultPolicy, ToolResultRenderKind, ToolStreamingMode,
    ToolTag, TransportKind, UiTextDisplayMode, normalize_tool_tag_name,
    plugin_workbench_tab_id_is_supported,
};
pub use plugin::{InitContext, InitOutcome, Plugin, PluginConfig, ToolStreamSink};
pub use schemars::JsonSchema;

// Re-exports used by macros so plugin authors don't have to add deps directly.
#[doc(hidden)]
#[cfg(feature = "cdylib")]
pub use abi_stable as abi_stable_reexport;
