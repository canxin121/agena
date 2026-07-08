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
pub mod plugin;
pub mod prelude;
pub mod rpc;

#[cfg(feature = "cdylib")]
pub mod cdylib_abi;

#[cfg(any(feature = "cdylib", feature = "stdio", feature = "http"))]
pub mod drivers;

pub use agena_macros::{PluginConfigStore, ToolArgs, ToolInputShape, agena_plugin};
pub use async_trait::async_trait;
pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
pub use error::{PluginError, PluginErrorCode, Result};
pub use hooks::*;
pub use host_api::{
    HostClient, HostNetworkPermissionCheckRequest, HostPathPermissionCheckRequest,
    HostPermissionCheckResponse, NoopHostClient,
};
pub use identity::{PluginKey, PluginKeyParseError, ToolKey, ToolKeyParseError};
pub use macro_support::{schema_example_texts, schema_usage_text};
pub use manifest::{
    HookSubscription, HostCapability, InputNetworkSpec, InputPathSpec, NetworkAccessSpec,
    PathAccessSpec, PathKind, PluginManifest, PluginStudioCommand, PluginStudioControl,
    PluginStudioControlOption, PluginStudioUiContributions, PluginStudioView,
    PluginTuiContentBlock, PluginTuiStatuslineSegment, PluginTuiUiContributions, PluginUiAction,
    PluginUiContributions, PluginUiThemePalette, ToolDefinition, ToolDescriptionMode,
    ToolDisplayPreset, ToolInputShape, ToolResultPolicy, ToolResultRenderKind, ToolStreamingMode,
    ToolSurface, ToolTag, TransportKind, UiTextDisplayMode, normalize_tool_tag_name,
};
pub use plugin::{InitContext, InitOutcome, Plugin, PluginConfig, ToolStreamSink};
pub use schemars::JsonSchema;

#[macro_export]
macro_rules! tool_shape_dispatch {
    ($input:expr, $shape:ty, { $($pattern:pat => $body:expr),+ $(,)? }) => {{
        let __input = $input;
        match <$shape as $crate::ToolInputShape>::parse_input(__input)? {
            $($pattern => $body,)+
        }
    }};
}
// Re-exports used by macros so plugin authors don't have to add deps directly.
#[doc(hidden)]
#[cfg(feature = "cdylib")]
pub use abi_stable as abi_stable_reexport;
