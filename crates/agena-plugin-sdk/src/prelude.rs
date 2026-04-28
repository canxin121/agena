//! Convenience re-exports for plugin authors.
//!
//! ```ignore
//! use agena_plugin_sdk::prelude::*;
//! ```

pub use async_trait::async_trait;
pub use serde::{Deserialize, Serialize};
pub use serde_json::{json, Value};
pub use std::sync::Arc;

pub use crate::error::{PluginError, PluginErrorCode, Result};
pub use crate::hooks::*;
pub use crate::host_api::{EventSubscription, HostClient, LogLevel, NoopHostClient};
pub use crate::manifest::{
    HookSubscription, PluginManifest, PluginManifestBuilder, ToolBehavior, ToolDecl,
    TransportKind,
};
pub use crate::plugin::{InitContext, InitOutcome, Plugin};

#[cfg(feature = "cdylib")]
pub use crate::cdylib_abi::{AgenaPluginCdylib, AgenaPluginCdylib_Ref};
