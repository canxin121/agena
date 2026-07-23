use std::path::PathBuf;

use thiserror::Error;

/// Schema-neutral configuration parsing and validation failure.
#[derive(Debug, Error)]
pub(crate) enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write config file {path}: {source}")]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    ParseFile {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("config modes are no longer supported; remove `{field}` and use a single config file")]
    UnsupportedModeConfig { field: &'static str },
    #[error(
        "AGENA_MODE is no longer supported; use a single config file or explicit --set overrides"
    )]
    UnsupportedModeEnvironment,
    #[error("invalid override `{0}`")]
    InvalidOverride(String),
    #[error("invalid numeric value for `{key}`: {value}")]
    InvalidNumber { key: String, value: String },
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
    #[error("failed to encode config as json: {0}")]
    SerializeJson(#[from] serde_json::Error),
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

/// Parse a JSON configuration document using the Runtime-owned error contract.
pub(crate) fn parse_config_json(
    path: &std::path::Path,
    text: &str,
) -> Result<serde_json::Value, ConfigError> {
    serde_json::from_str(text).map_err(|source| ConfigError::ParseFile {
        path: path.to_path_buf(),
        source,
    })
}

/// Read an optional JSON configuration document using the Runtime-owned file
/// and parse error contract. A missing file is represented as `Ok(None)`.
pub(crate) fn read_config_json(
    path: &std::path::Path,
) -> Result<Option<serde_json::Value>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_config_json(path, &text).map(Some),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::ReadFile {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Parse the permissive boolean spellings accepted by configuration
/// environment overrides.
pub(crate) fn parse_config_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ConfigError::InvalidOverride(format!(
            "{key} expects bool, got `{value}`"
        ))),
    }
}

/// Trim an optional configuration string and discard empty values.
pub(crate) fn normalize_config_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

/// Apply the ordinary most-specific-wins merge rule for optional values.
pub(crate) fn merge_optional_config<T>(base: &mut Option<T>, overlay: Option<T>) {
    if let Some(value) = overlay {
        *base = Some(value);
    }
}

/// Apply a typed numeric environment override when the variable is present.
pub(crate) fn apply_config_env_number<T, F>(
    env: &dyn crate::ConfigEnvironment,
    key: &str,
    mut apply: F,
) -> Result<(), ConfigError>
where
    T: std::str::FromStr,
    F: FnMut(T),
{
    if let Some(value) = env.var(key) {
        apply(parse_numeric::<T>(value.as_str(), key)?);
    }
    Ok(())
}

/// Reject the retired process mode switch before a schema adapter loads files.
pub(crate) fn reject_unsupported_mode_environment(
    env: &dyn crate::ConfigEnvironment,
) -> Result<(), ConfigError> {
    if env.var("AGENA_MODE").is_some() {
        return Err(ConfigError::UnsupportedModeEnvironment);
    }
    Ok(())
}

/// Convert schema-level configuration failures into the stable settings-service
/// error contract used by transport and presentation adapters.
pub(crate) fn config_error_to_settings_error(
    error: ConfigError,
) -> crate::RuntimeConfigSettingsError {
    let message = error.to_string();
    match error {
        ConfigError::ReadFile { .. }
        | ConfigError::WriteFile { .. }
        | ConfigError::SerializeJson(_) => crate::RuntimeConfigSettingsError::internal(message),
        _ => crate::RuntimeConfigSettingsError::invalid_input(message),
    }
}

/// Adapt the transport-facing settings error back into the shared
/// schema-neutral configuration error used by file adapters.
pub(crate) fn settings_error_to_config_error(
    error: crate::RuntimeConfigSettingsError,
) -> ConfigError {
    ConfigError::Validation(error.message().to_owned())
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, parse_config_bool, parse_numeric};

    #[test]
    fn parses_permissive_environment_booleans() {
        assert!(parse_config_bool("flag", " YES ").unwrap());
        assert!(!parse_config_bool("flag", "0").unwrap());
        assert!(matches!(
            parse_config_bool("flag", "maybe"),
            Err(ConfigError::InvalidOverride(_))
        ));
    }

    #[test]
    fn reports_numeric_key_and_value() {
        assert!(matches!(
            parse_numeric::<u64>("not-a-number", "tokens"),
            Err(ConfigError::InvalidNumber { key, value })
                if key == "tokens" && value == "not-a-number"
        ));
    }

    #[test]
    fn reports_json_parse_path_through_runtime_error_contract() {
        let error = super::parse_config_json(std::path::Path::new("settings.json"), "{")
            .expect_err("malformed JSON must be rejected");
        assert!(matches!(
            error,
            ConfigError::ParseFile { path, .. } if path == std::path::Path::new("settings.json")
        ));
    }
}
