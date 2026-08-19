//! # agena-runtime-contracts
//!
//! Stable, runtime-neutral values and ports shared by feature crates.
//!
//! This crate is intentionally small. Concrete runtime services and
//! application composition belong to their owning crates.
//!
//! It defines the shared authorization, identity, part, permission, and
//! provider-state surfaces ([`authorization`], [`identity`], [`part`],
//! [`permission`], [`provider_state`]) plus cross-cutting contracts such as
//! [`ToolSessionContext`], [`RuntimeRequestContext`],
//! [`NeutralToolExecutionResult`], and [`ConfigSnapshot`].

use serde::{Deserialize, Serialize};

pub mod authorization;
pub mod identity;
pub mod part;
pub mod part_content;
pub mod permission;
pub mod provider_state;

pub use part::*;
pub use provider_state::PartProviderState;

/// Session state needed by tool execution, kept as a neutral port so neither
/// the tool executor nor session core depends on the other's implementation.
pub trait ToolSessionContext {
    /// Runtime-only session identity used to select plugin-scoped capability
    /// overlays. Implementations backed by portable policy/config values may
    /// leave this absent; callers then see only global plugin capabilities.
    fn session_id(&self) -> Option<i64> {
        None
    }
    fn effective_workspace_root(&self) -> Option<&std::path::Path>;
    fn effective_permission(&self) -> &authorization::PermissionConfig;
    fn permission_ceiling(&self) -> &authorization::PermissionConfig;
    fn capability_denied_tool_names(&self) -> &std::collections::BTreeSet<String>;
    fn execution_access(&self) -> agena_domain::ExecutionAccess;
    fn selected_model(&self) -> Option<&str>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Context of a runtime request.
pub struct RuntimeRequestContext {
    pub request_id: String,
    pub cancellation_key: Option<String>,
}

impl RuntimeRequestContext {
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            cancellation_key: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Provider-neutral result of executing a tool.
pub struct NeutralToolExecutionResult {
    pub tool_name: String,
    pub output: serde_json::Value,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Binding of a tool to a plugin and its method.
pub struct PluginToolBinding {
    pub plugin_id: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// An invocation of a plugin method.
pub struct PluginInvocation {
    pub plugin_id: String,
    pub method: String,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A snapshot of configuration affecting a run.
pub struct ConfigSnapshot {
    pub source: String,
    pub revision: Option<String>,
}
