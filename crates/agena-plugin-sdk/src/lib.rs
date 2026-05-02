//! Agena Plugin SDK — write a plugin once, ship it as in-process / cdylib / stdio / HTTP.
//!
//! Implement [`Plugin`] for your type, fill in the hooks you care about (every method
//! has a default no-op), and pick a transport with one of the `export_*!` macros.

pub mod attachment;
pub mod error;
pub mod hooks;
pub mod host_api;
pub mod manifest;
pub mod plugin;
pub mod prelude;
pub mod rpc;

#[cfg(feature = "cdylib")]
pub mod cdylib_abi;

#[cfg(any(feature = "cdylib", feature = "stdio", feature = "http"))]
pub mod drivers;

pub use attachment::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
pub use error::{PluginError, PluginErrorCode, Result};
pub use hooks::*;
pub use host_api::{HostClient, NoopHostClient};
pub use manifest::{
    EntryBehavior, EntryLoadPriority, EntryStreamingMode, HookSubscription, HostCapability,
    InputPathSpec, PathKind, PlanModePolicy, PluginEntryDecl, PluginManifest, TransportKind,
};
pub use plugin::{InitContext, InitOutcome, Plugin, ToolStreamSink};

// Re-exports used by macros so plugin authors don't have to add deps directly.
#[doc(hidden)]
#[cfg(feature = "cdylib")]
pub use abi_stable as abi_stable_reexport;
#[doc(hidden)]
pub use serde_json;
