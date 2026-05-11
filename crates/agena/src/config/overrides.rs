use std::{path::PathBuf, str::FromStr};

use super::{
    AuthStoreBackend, ConfigError, RawAuthConfig, RawConfig, RawPermissionConfig,
    RawProviderHttpConfig, RawRequestRetryConfig, RawRuntimeConfig, RawStreamReplayConfig,
    RawTracingConfig, RawUiConfig, parse_numeric, parse_permission_mode,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOverride {
    AuthStorePath(PathBuf),
    AuthStoreBackend(AuthStoreBackend),
    TracingFilter(String),
    UiLocale(String),
    PermissionDefaultRead(crate::permission::PermissionMode),
    PermissionDefaultWrite(crate::permission::PermissionMode),
    PermissionDefaultExternalDirectory(crate::permission::PermissionMode),
    ProviderHttpTimeoutSecs(u64),
    ProviderConnectTimeoutSecs(u64),
    RequestRetryMaxRetries(u32),
    RequestRetryBaseDelayMs(u64),
    RequestRetryMaxDelayMs(u64),
    StreamReplayMaxRetriesAfterOutput(u32),
    StreamReplayMaxTrackedEvents(usize),
    ProviderDefaultModel { provider_id: String, value: String },
    ProviderBaseUrl { provider_id: String, value: String },
    ProviderApiKey { provider_id: String, value: String },
    ProviderApiKeyEnv { provider_id: String, value: String },
    ProviderEnabled { provider_id: String, value: bool },
}

impl FromStr for ConfigOverride {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (key, raw_value) = value
            .split_once('=')
            .ok_or_else(|| ConfigError::InvalidOverride(value.to_owned()))?;
        let key = key.trim();
        let raw_value = raw_value.trim();

        match key {
            "mode" => Err(ConfigError::UnsupportedModeConfig { field: "mode" }),
            "auth.store_path" => Ok(Self::AuthStorePath(PathBuf::from(raw_value))),
            "auth.store_backend" => {
                Ok(Self::AuthStoreBackend(parse_auth_store_backend(raw_value)?))
            }
            "tracing.filter" => Ok(Self::TracingFilter(raw_value.to_owned())),
            "ui.locale" => Ok(Self::UiLocale(raw_value.to_owned())),
            "permission.default_read" => Ok(Self::PermissionDefaultRead(parse_permission_mode(
                raw_value,
            )?)),
            "permission.default_write" => Ok(Self::PermissionDefaultWrite(parse_permission_mode(
                raw_value,
            )?)),
            "permission.default_external_directory" => Ok(
                Self::PermissionDefaultExternalDirectory(parse_permission_mode(raw_value)?),
            ),
            "runtime.provider_http.timeout_secs" => Ok(Self::ProviderHttpTimeoutSecs(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.provider_http.connect_timeout_secs" => Ok(Self::ProviderConnectTimeoutSecs(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.request_retry.max_retries" => {
                Ok(Self::RequestRetryMaxRetries(parse_numeric(raw_value, key)?))
            }
            "runtime.request_retry.base_delay_ms" => Ok(Self::RequestRetryBaseDelayMs(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.request_retry.max_delay_ms" => {
                Ok(Self::RequestRetryMaxDelayMs(parse_numeric(raw_value, key)?))
            }
            "runtime.stream_replay.max_retries_after_output" => Ok(
                Self::StreamReplayMaxRetriesAfterOutput(parse_numeric(raw_value, key)?),
            ),
            "runtime.stream_replay.max_tracked_events" => Ok(Self::StreamReplayMaxTrackedEvents(
                parse_numeric(raw_value, key)?,
            )),
            _ => parse_provider_override(key, raw_value),
        }
    }
}

impl ConfigOverride {
    pub(crate) fn apply_to(&self, config: &mut RawConfig) {
        match self {
            Self::AuthStorePath(path) => {
                config
                    .auth
                    .get_or_insert_with(RawAuthConfig::default)
                    .store_path = Some(path.clone());
            }
            Self::AuthStoreBackend(backend) => {
                config
                    .auth
                    .get_or_insert_with(RawAuthConfig::default)
                    .store_backend = Some(*backend);
            }
            Self::TracingFilter(filter) => {
                config
                    .tracing
                    .get_or_insert_with(RawTracingConfig::default)
                    .filter = Some(filter.clone());
            }
            Self::UiLocale(locale) => {
                config.ui.get_or_insert_with(RawUiConfig::default).locale = Some(locale.clone());
            }
            Self::PermissionDefaultRead(mode) => {
                config
                    .permission
                    .get_or_insert_with(RawPermissionConfig::default)
                    .default_read = Some(*mode);
            }
            Self::PermissionDefaultWrite(mode) => {
                config
                    .permission
                    .get_or_insert_with(RawPermissionConfig::default)
                    .default_write = Some(*mode);
            }
            Self::PermissionDefaultExternalDirectory(mode) => {
                config
                    .permission
                    .get_or_insert_with(RawPermissionConfig::default)
                    .default_external_directory = Some(*mode);
            }
            Self::ProviderHttpTimeoutSecs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .provider_http
                    .get_or_insert_with(RawProviderHttpConfig::default)
                    .timeout_secs = Some(*value);
            }
            Self::ProviderConnectTimeoutSecs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .provider_http
                    .get_or_insert_with(RawProviderHttpConfig::default)
                    .connect_timeout_secs = Some(*value);
            }
            Self::RequestRetryMaxRetries(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .request_retry
                    .get_or_insert_with(RawRequestRetryConfig::default)
                    .max_retries = Some(*value);
            }
            Self::RequestRetryBaseDelayMs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .request_retry
                    .get_or_insert_with(RawRequestRetryConfig::default)
                    .base_delay_ms = Some(*value);
            }
            Self::RequestRetryMaxDelayMs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .request_retry
                    .get_or_insert_with(RawRequestRetryConfig::default)
                    .max_delay_ms = Some(*value);
            }
            Self::StreamReplayMaxRetriesAfterOutput(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .stream_replay
                    .get_or_insert_with(RawStreamReplayConfig::default)
                    .max_retries_after_output = Some(*value);
            }
            Self::StreamReplayMaxTrackedEvents(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .stream_replay
                    .get_or_insert_with(RawStreamReplayConfig::default)
                    .max_tracked_events = Some(*value);
            }
            Self::ProviderDefaultModel { provider_id, value } => {
                config.provider_mut(provider_id).default_model = Some(value.clone());
            }
            Self::ProviderBaseUrl { provider_id, value } => {
                config.provider_mut(provider_id).base_url = Some(value.clone());
            }
            Self::ProviderApiKey { provider_id, value } => {
                config.provider_mut(provider_id).api_key = Some(value.clone());
            }
            Self::ProviderApiKeyEnv { provider_id, value } => {
                config.provider_mut(provider_id).api_key_env = Some(value.clone());
            }
            Self::ProviderEnabled { provider_id, value } => {
                config.provider_mut(provider_id).enabled = Some(*value);
            }
        }
    }
}

fn parse_provider_override(key: &str, raw_value: &str) -> Result<ConfigOverride, ConfigError> {
    let Some(rest) = key.strip_prefix("providers.") else {
        return Err(ConfigError::InvalidOverride(key.to_owned()));
    };
    let (provider_id, field) = rest
        .split_once('.')
        .ok_or_else(|| ConfigError::InvalidOverride(key.to_owned()))?;
    let provider_id = provider_id.trim().to_owned();
    let value = raw_value.to_owned();

    match field {
        "default_model" => Ok(ConfigOverride::ProviderDefaultModel { provider_id, value }),
        "base_url" => Ok(ConfigOverride::ProviderBaseUrl { provider_id, value }),
        "api_key" => Ok(ConfigOverride::ProviderApiKey { provider_id, value }),
        "api_key_env" => Ok(ConfigOverride::ProviderApiKeyEnv { provider_id, value }),
        "enabled" => Ok(ConfigOverride::ProviderEnabled {
            provider_id,
            value: parse_bool(key, raw_value)?,
        }),
        _ => Err(ConfigError::InvalidOverride(key.to_owned())),
    }
}

fn parse_auth_store_backend(value: &str) -> Result<AuthStoreBackend, ConfigError> {
    match value.trim() {
        "auto" => Ok(AuthStoreBackend::Auto),
        "file" => Ok(AuthStoreBackend::File),
        "keyring" => Ok(AuthStoreBackend::Keyring),
        _ => Err(ConfigError::InvalidOverride(format!(
            "auth.store_backend expects auto, file, or keyring, got `{value}`"
        ))),
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.trim() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ConfigError::InvalidOverride(format!(
            "{key} expects bool, got `{value}`"
        ))),
    }
}
