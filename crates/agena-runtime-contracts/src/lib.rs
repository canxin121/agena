//! Stable, runtime-neutral values and ports shared by feature crates.
//!
//! This crate is intentionally small. Concrete runtime services and
//! application composition belong to their owning crates.

use serde::{Deserialize, Serialize};

pub mod agent;
pub mod agents;
pub mod message;
pub mod permission;

pub use message::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct NeutralToolExecutionResult {
    pub tool_name: String,
    pub output: serde_json::Value,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginToolBinding {
    pub plugin_id: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInvocation {
    pub plugin_id: String,
    pub method: String,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub source: String,
    pub revision: Option<String>,
}
