use std::path::PathBuf;

use thiserror::Error;

use crate::{error::AppError, permission::PermissionMode};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    ParseFile {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("config modes are no longer supported; remove `{field}` and use a single config file")]
    UnsupportedModeConfig { field: &'static str },
    #[error("AGENA_MODE is no longer supported; use a single config file or explicit --set overrides")]
    UnsupportedModeEnvironment,
    #[error("invalid override `{0}`")]
    InvalidOverride(String),
    #[error("invalid permission mode `{0}`")]
    InvalidPermissionMode(String),
    #[error("invalid numeric value for `{key}`: {value}")]
    InvalidNumber { key: String, value: String },
    #[error("provider `{provider_id}` is missing kind")]
    MissingProviderKind { provider_id: String },
    #[error("provider `{provider_id}` field `{field}` is required")]
    MissingProviderField {
        provider_id: String,
        field: &'static str,
    },
    #[error("provider `{provider_id}` has unsupported field combination: {message}")]
    InvalidProviderConfig {
        provider_id: String,
        message: String,
    },
    #[error(
        "provider `{provider_id}` field `{field}` references missing environment variable `{env_key}`"
    )]
    MissingEnvironmentVariable {
        provider_id: String,
        field: &'static str,
        env_key: String,
    },
    #[error("config validation failed: {0}")]
    Validation(String),
    #[error(transparent)]
    App(#[from] AppError),
    #[error("failed to encode config as json: {0}")]
    SerializeJson(#[from] serde_json::Error),
    #[error("failed to encode config as toml: {0}")]
    SerializeToml(#[from] toml::ser::Error),
}

pub(crate) fn parse_permission_mode(value: &str) -> Result<PermissionMode, ConfigError> {
    match value.trim() {
        "allow" => Ok(PermissionMode::Allow),
        "ask" => Ok(PermissionMode::Ask),
        "deny" => Ok(PermissionMode::Deny),
        _ => Err(ConfigError::InvalidPermissionMode(value.to_owned())),
    }
}

pub(crate) fn parse_numeric<T>(value: &str, key: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
{
    value.parse::<T>().map_err(|_| ConfigError::InvalidNumber {
        key: key.to_owned(),
        value: value.to_owned(),
    })
}
