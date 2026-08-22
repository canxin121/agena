//! Read-only configuration projection from a composed runtime.

use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;

#[derive(Debug, Clone)]
/// Snapshot of the effective runtime configuration.
pub struct RuntimeConfigurationSnapshot {
    pub config_path: PathBuf,
    pub config_found: bool,
    /// Workspace-local configuration resolution details. These are paths and
    /// provenance only, not concrete configuration-schema values.
    pub project_config_path: PathBuf,
    pub project_config_found: bool,
    pub applied_layers: Vec<String>,
    pub ui: RuntimeUiConfiguration,
    /// The resolved effective configuration is intentionally a JSON document:
    /// configuration is user-extensible and its public settings API is path
    /// based rather than a fixed record schema.
    pub effective_config: serde_json::Value,
    /// Complete resolved configuration document, including the stable `config`
    /// and `meta` sections used by read-only CLI diagnostics. This preserves
    /// resolution-layer evidence without exposing concrete configuration structs.
    pub configuration_document: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
/// UI-relevant runtime configuration.
pub struct RuntimeUiConfiguration {
    pub locale: Option<String>,
    pub theme: Option<String>,
    pub color_scheme: RuntimeTuiColorScheme,
    pub graphics: RuntimeTuiGraphicsMode,
    /// Default transcript expansion for activities without a kind override.
    pub transcript_activity_default_expanded: bool,
    /// Per-kind transcript expansion overrides keyed by activity kind id.
    pub transcript_activity_kinds: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Copy, Default)]
/// TUI color scheme preference.
pub enum RuntimeTuiColorScheme {
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, Default)]
/// TUI graphics mode preference.
pub enum RuntimeTuiGraphicsMode {
    #[default]
    Auto,
    Native,
    Unicode,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("runtime configuration projection failed: {message}")]
/// Error reading the runtime configuration snapshot.
pub struct RuntimeConfigurationError {
    message: String,
}

impl RuntimeConfigurationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn from_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::new(agena_failure::diagnostic::format_error_chain(error))
    }
}

#[async_trait]
/// Service providing the effective runtime configuration.
pub trait RuntimeConfigurationService: Send + Sync {
    fn runtime_configuration(
        &self,
    ) -> Result<RuntimeConfigurationSnapshot, RuntimeConfigurationError>;
}
