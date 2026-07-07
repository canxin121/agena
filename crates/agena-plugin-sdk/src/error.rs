use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Top-level plugin error. Carried over JSON-RPC as the `error.data` field of
/// the response, with a stable `code` from [`PluginErrorCode`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginError {
    pub code: PluginErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginErrorCode {
    Generic,
    NotImplemented,
    InvalidParams,
    Timeout,
    Disconnected,
    Panicked,
    HostUnavailable,
}

impl PluginError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: PluginErrorCode::Generic,
            message: message.into(),
            hook: None,
            plugin: None,
            data: None,
        }
    }

    pub fn not_implemented(hook: impl Into<String>) -> Self {
        Self {
            code: PluginErrorCode::NotImplemented,
            message: format!("hook not implemented: {}", hook.into()),
            hook: None,
            plugin: None,
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: PluginErrorCode::InvalidParams,
            message: message.into(),
            hook: None,
            plugin: None,
            data: None,
        }
    }

    pub fn invalid_params_with_data(message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            code: PluginErrorCode::InvalidParams,
            message: message.into(),
            hook: None,
            plugin: None,
            data: Some(data),
        }
    }
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PluginError {}

impl From<&str> for PluginError {
    fn from(value: &str) -> Self {
        PluginError::new(value)
    }
}

impl From<String> for PluginError {
    fn from(value: String) -> Self {
        PluginError::new(value)
    }
}

impl From<serde_json::Error> for PluginError {
    fn from(value: serde_json::Error) -> Self {
        PluginError::invalid_params(value.to_string())
    }
}

#[derive(Debug, Error)]
#[error("plugin transport error: {message}")]
pub struct TransportErrorRepr {
    pub message: String,
}

pub type Result<T> = std::result::Result<T, PluginError>;
