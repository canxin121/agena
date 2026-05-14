use std::str::FromStr;

use super::{
    ConfigError, RawConfig, RawProviderHttpConfig, RawRequestRetryConfig, RawRuntimeConfig,
    RawStreamReplayConfig, RawTracingConfig, RawUiConfig, SharedGatewayEndpointLayout,
    parse_numeric,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOverride {
    TracingFilter(String),
    TracingDatabaseLevel(String),
    UiLocale(String),
    ProviderHttpTimeoutSecs(u64),
    ProviderConnectTimeoutSecs(u64),
    RequestRetryMaxRetries(u32),
    RequestRetryBaseDelayMs(u64),
    RequestRetryMaxDelayMs(u64),
    StreamReplayMaxRetriesAfterOutput(u32),
    StreamReplayMaxTrackedEvents(usize),
    ProviderDefaultModel { provider_id: String, value: String },
    ProviderAuthBaseUrl { provider_id: String, value: String },
    ProviderAuthEndpointLayout {
        provider_id: String,
        value: SharedGatewayEndpointLayout,
    },
    ProviderAuthApiKey { provider_id: String, value: String },
    ProviderAuthApiKeyEnv { provider_id: String, value: String },
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
            "tracing.filter" => Ok(Self::TracingFilter(raw_value.to_owned())),
            "tracing.database_level" => Ok(Self::TracingDatabaseLevel(raw_value.to_owned())),
            "ui.locale" => Ok(Self::UiLocale(raw_value.to_owned())),
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
            Self::TracingFilter(filter) => {
                config
                    .tracing
                    .get_or_insert_with(RawTracingConfig::default)
                    .filter = Some(filter.clone());
            }
            Self::TracingDatabaseLevel(level) => {
                config
                    .tracing
                    .get_or_insert_with(RawTracingConfig::default)
                    .database_level = Some(level.clone());
            }
            Self::UiLocale(locale) => {
                config.ui.get_or_insert_with(RawUiConfig::default).locale = Some(locale.clone());
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
                config
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .default_model = Some(value.clone());
            }
            Self::ProviderAuthBaseUrl { provider_id, value } => {
                config
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .auth
                    .get_or_insert_with(Default::default)
                    .base_url = Some(value.clone());
            }
            Self::ProviderAuthEndpointLayout { provider_id, value } => {
                config
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .auth
                    .get_or_insert_with(Default::default)
                    .endpoint_layout = Some(*value);
            }
            Self::ProviderAuthApiKey { provider_id, value } => {
                config
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .auth
                    .get_or_insert_with(Default::default)
                    .api_key = Some(value.clone());
            }
            Self::ProviderAuthApiKeyEnv { provider_id, value } => {
                config
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .auth
                    .get_or_insert_with(Default::default)
                    .api_key_env = Some(value.clone());
            }
            Self::ProviderEnabled { provider_id, value } => {
                config
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .enabled = Some(*value);
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
        "enabled" => Ok(ConfigOverride::ProviderEnabled {
            provider_id,
            value: parse_bool(key, raw_value)?,
        }),
        "base_url" | "api_key" | "api_key_env" => Err(ConfigError::InvalidOverride(format!(
            "{key} is no longer supported; use providers.{provider_id}.auth.{field}"
        ))),
        _ => {
            let Some(auth_field) = field.strip_prefix("auth.") else {
                return Err(ConfigError::InvalidOverride(key.to_owned()));
            };
            match auth_field {
                "base_url" => Ok(ConfigOverride::ProviderAuthBaseUrl { provider_id, value }),
                "endpoint_layout" => Ok(ConfigOverride::ProviderAuthEndpointLayout {
                    provider_id,
                    value: raw_value.parse()?,
                }),
                "api_key" => Ok(ConfigOverride::ProviderAuthApiKey { provider_id, value }),
                "api_key_env" => Ok(ConfigOverride::ProviderAuthApiKeyEnv { provider_id, value }),
                _ => Err(ConfigError::InvalidOverride(key.to_owned())),
            }
        }
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
