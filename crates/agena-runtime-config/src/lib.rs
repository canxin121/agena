//! Stable configuration contracts for runtime composition.

pub mod config;
pub mod config_error;
pub mod config_override;
pub mod config_values;
pub mod lsp_config;
pub mod mcp_config;
pub mod runtime_config_settings_service;
pub mod runtime_configuration_service;

pub mod config_environment;
pub mod config_paths;

pub use config::{
    ConfigLoader, RawConfig, RawConfigFile, RawTracingConfig, RawTuiUiConfig, RawUiConfig,
    apply_config_override, delete_layered_file_setting, list_file_settings, list_json_path,
    parse_settings_path, patch_layered_file_settings, read_file_setting, set_layered_file_setting,
    validate_config_text, validate_layered_file_settings,
};
pub use config_environment::{ConfigEnvironment, ProcessEnvironment};
pub use config_error::ConfigError;
pub use config_error::*;
pub use config_override::{
    ConfigOverride, LoadConfigRequest, RuntimeConfigOverrideError,
    parse_config_override_expressions,
};
pub use config_paths::{default_config_path, default_workspace_root, project_config_path};
pub use config_values::*;
pub use lsp_config::*;
pub use mcp_config::*;
pub use runtime_config_settings_service::*;
pub use runtime_configuration_service::*;

/// Process-level tracing values consumed by runtime bootstrap and config
/// resolution. Database connection setup remains owned by `agena-runtime`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RuntimeTracingConfiguration {
    pub filter: String,
    pub database: String,
    pub adapter: String,
}

impl Default for RuntimeTracingConfiguration {
    fn default() -> Self {
        Self {
            filter: "info".to_owned(),
            database: "error".to_owned(),
            adapter: "off".to_owned(),
        }
    }
}
