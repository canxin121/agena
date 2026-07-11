use std::str::FromStr;

use super::{
    ConfigError, RawConfig, RawProviderHttpConfig, RawRequestRetryConfig, RawRuntimeConfig,
    RawRuntimeGcConfig, RawRuntimeModelCatalogConfig, RawRuntimeProvidersConfig,
    RawRuntimeSessionConfig, RawSessionCacheConfig, RawStreamReplayConfig, RawTracingConfig,
    RawTuiUiConfig, RawUiConfig, TuiColorSchemeConfig, parse_numeric,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOverride {
    TracingFilter(String),
    TracingDatabase(String),
    TracingAdapter(String),
    UiLocale(String),
    UiTuiColorScheme(TuiColorSchemeConfig),
    UiTuiTheme(String),
    ProvidersDefault(String),
    AgentsDefault(String),
    ProviderHttpTimeoutSecs(u64),
    ProviderConnectTimeoutSecs(u64),
    RequestRetryMaxRetries(u32),
    RequestRetryBaseDelayMs(u64),
    RequestRetryMaxDelayMs(u64),
    StreamReplayMaxRetriesAfterOutput(u32),
    StreamReplayMaxTrackedEvents(usize),
    ModelCatalogCacheMaxAgeSecs(u64),
    RuntimeSessionCacheMaxSessions(usize),
    RuntimeSessionCacheTtlSecs(u64),
    RuntimeSessionCacheMaxBytes(usize),
    RuntimeSessionGcEnabled(bool),
    RuntimeSessionGcIntervalSecs(u64),
    ProviderDefaultsProvider {
        provider_id: String,
        value: String,
    },
    ProviderDefaultsAdapter {
        provider_id: String,
        value: String,
    },
    ProviderDefaultsModel {
        provider_id: String,
        value: String,
    },
    ProviderDefaultsThinkingMode {
        provider_id: String,
        value: String,
    },
    ProviderDefaultsSpeedMode {
        provider_id: String,
        value: String,
    },
    ProviderDefaultsVerbosity {
        provider_id: String,
        value: String,
    },
    ProviderDefaultsParallelToolCalls {
        provider_id: String,
        value: bool,
    },
    AgentDefaultsProvider {
        agent_name: String,
        value: String,
    },
    AgentDefaultsAdapter {
        agent_name: String,
        value: String,
    },
    AgentDefaultsModel {
        agent_name: String,
        value: String,
    },
    AgentDefaultsThinkingMode {
        agent_name: String,
        value: String,
    },
    AgentDefaultsSpeedMode {
        agent_name: String,
        value: String,
    },
    AgentDefaultsVerbosity {
        agent_name: String,
        value: String,
    },
    AgentDefaultsParallelToolCalls {
        agent_name: String,
        value: bool,
    },
    ProviderAuthBaseUrl {
        provider_id: String,
        value: String,
    },
    ProviderAuthProtocolPath {
        provider_id: String,
        protocol: String,
        value: String,
    },
    ProviderAuthApiKey {
        provider_id: String,
        value: super::overlay::ProviderSecretSourceOverlay,
    },
    ProviderEnabled {
        provider_id: String,
        value: bool,
    },
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
            "tracing.database" => Ok(Self::TracingDatabase(raw_value.to_owned())),
            "tracing.adapter" => Ok(Self::TracingAdapter(raw_value.to_owned())),
            "ui.locale" => Ok(Self::UiLocale(raw_value.to_owned())),
            "ui.tui.color_scheme" => Ok(Self::UiTuiColorScheme(
                raw_value.parse().map_err(ConfigError::Validation)?,
            )),
            "ui.tui.theme" => Ok(Self::UiTuiTheme(raw_value.to_owned())),
            "providers.default" => Ok(Self::ProvidersDefault(raw_value.to_owned())),
            "agents.default" => Ok(Self::AgentsDefault(raw_value.to_owned())),
            _ if key.starts_with("providers.") => parse_provider_override(key, raw_value),
            _ if key.starts_with("agents.") => parse_agent_override(key, raw_value),
            "runtime.providers.http.timeout_secs" => Ok(Self::ProviderHttpTimeoutSecs(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.providers.http.connect_timeout_secs" => Ok(Self::ProviderConnectTimeoutSecs(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.providers.retry.max_retries" => {
                Ok(Self::RequestRetryMaxRetries(parse_numeric(raw_value, key)?))
            }
            "runtime.providers.retry.base_delay_ms" => Ok(Self::RequestRetryBaseDelayMs(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.providers.retry.max_delay_ms" => {
                Ok(Self::RequestRetryMaxDelayMs(parse_numeric(raw_value, key)?))
            }
            "runtime.providers.stream_replay.max_retries_after_output" => Ok(
                Self::StreamReplayMaxRetriesAfterOutput(parse_numeric(raw_value, key)?),
            ),
            "runtime.providers.stream_replay.max_tracked_events" => Ok(
                Self::StreamReplayMaxTrackedEvents(parse_numeric(raw_value, key)?),
            ),
            "runtime.model_catalog.cache_max_age_secs" => Ok(Self::ModelCatalogCacheMaxAgeSecs(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.session.cache.max_sessions" => Ok(Self::RuntimeSessionCacheMaxSessions(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.session.cache.ttl_secs" => Ok(Self::RuntimeSessionCacheTtlSecs(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.session.cache.max_bytes" => Ok(Self::RuntimeSessionCacheMaxBytes(
                parse_numeric(raw_value, key)?,
            )),
            "runtime.session.gc.enabled" => {
                Ok(Self::RuntimeSessionGcEnabled(parse_bool(key, raw_value)?))
            }
            "runtime.session.gc.interval_secs" => Ok(Self::RuntimeSessionGcIntervalSecs(
                parse_numeric(raw_value, key)?,
            )),
            _ => Err(ConfigError::InvalidOverride(key.to_owned())),
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
            Self::TracingDatabase(level) => {
                config
                    .tracing
                    .get_or_insert_with(RawTracingConfig::default)
                    .database = Some(level.clone());
            }
            Self::TracingAdapter(mode) => {
                config
                    .tracing
                    .get_or_insert_with(RawTracingConfig::default)
                    .adapter = Some(mode.clone());
            }
            Self::UiLocale(locale) => {
                config.ui.get_or_insert_with(RawUiConfig::default).locale = Some(locale.clone());
            }
            Self::UiTuiColorScheme(color_scheme) => {
                config
                    .ui
                    .get_or_insert_with(RawUiConfig::default)
                    .tui
                    .get_or_insert_with(RawTuiUiConfig::default)
                    .color_scheme = Some(*color_scheme);
            }
            Self::UiTuiTheme(theme) => {
                config
                    .ui
                    .get_or_insert_with(RawUiConfig::default)
                    .tui
                    .get_or_insert_with(RawTuiUiConfig::default)
                    .theme = Some(theme.clone());
            }
            Self::ProvidersDefault(value) => {
                config.providers.default = Some(value.clone());
            }
            Self::AgentsDefault(value) => {
                config.agents.default = Some(value.clone());
            }
            Self::ProviderHttpTimeoutSecs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .providers
                    .get_or_insert_with(RawRuntimeProvidersConfig::default)
                    .http
                    .get_or_insert_with(RawProviderHttpConfig::default)
                    .timeout_secs = Some(*value);
            }
            Self::ProviderConnectTimeoutSecs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .providers
                    .get_or_insert_with(RawRuntimeProvidersConfig::default)
                    .http
                    .get_or_insert_with(RawProviderHttpConfig::default)
                    .connect_timeout_secs = Some(*value);
            }
            Self::RequestRetryMaxRetries(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .providers
                    .get_or_insert_with(RawRuntimeProvidersConfig::default)
                    .retry
                    .get_or_insert_with(RawRequestRetryConfig::default)
                    .max_retries = Some(*value);
            }
            Self::RequestRetryBaseDelayMs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .providers
                    .get_or_insert_with(RawRuntimeProvidersConfig::default)
                    .retry
                    .get_or_insert_with(RawRequestRetryConfig::default)
                    .base_delay_ms = Some(*value);
            }
            Self::RequestRetryMaxDelayMs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .providers
                    .get_or_insert_with(RawRuntimeProvidersConfig::default)
                    .retry
                    .get_or_insert_with(RawRequestRetryConfig::default)
                    .max_delay_ms = Some(*value);
            }
            Self::StreamReplayMaxRetriesAfterOutput(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .providers
                    .get_or_insert_with(RawRuntimeProvidersConfig::default)
                    .stream_replay
                    .get_or_insert_with(RawStreamReplayConfig::default)
                    .max_retries_after_output = Some(*value);
            }
            Self::StreamReplayMaxTrackedEvents(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .providers
                    .get_or_insert_with(RawRuntimeProvidersConfig::default)
                    .stream_replay
                    .get_or_insert_with(RawStreamReplayConfig::default)
                    .max_tracked_events = Some(*value);
            }
            Self::ModelCatalogCacheMaxAgeSecs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .model_catalog
                    .get_or_insert_with(RawRuntimeModelCatalogConfig::default)
                    .cache_max_age_secs = Some(*value);
            }
            Self::RuntimeSessionCacheMaxSessions(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .session
                    .get_or_insert_with(RawRuntimeSessionConfig::default)
                    .cache
                    .get_or_insert_with(RawSessionCacheConfig::default)
                    .max_sessions = Some(*value);
            }
            Self::RuntimeSessionCacheTtlSecs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .session
                    .get_or_insert_with(RawRuntimeSessionConfig::default)
                    .cache
                    .get_or_insert_with(RawSessionCacheConfig::default)
                    .ttl_secs = Some(*value);
            }
            Self::RuntimeSessionCacheMaxBytes(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .session
                    .get_or_insert_with(RawRuntimeSessionConfig::default)
                    .cache
                    .get_or_insert_with(RawSessionCacheConfig::default)
                    .max_bytes = Some(*value);
            }
            Self::RuntimeSessionGcEnabled(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .session
                    .get_or_insert_with(RawRuntimeSessionConfig::default)
                    .gc
                    .get_or_insert_with(RawRuntimeGcConfig::default)
                    .enabled = Some(*value);
            }
            Self::RuntimeSessionGcIntervalSecs(value) => {
                config
                    .runtime
                    .get_or_insert_with(RawRuntimeConfig::default)
                    .session
                    .get_or_insert_with(RawRuntimeSessionConfig::default)
                    .gc
                    .get_or_insert_with(RawRuntimeGcConfig::default)
                    .interval_secs = Some(*value);
            }
            Self::ProviderDefaultsProvider { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .defaults
                    .get_or_insert_with(Default::default)
                    .provider = Some(value.clone());
            }
            Self::ProviderDefaultsAdapter { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .defaults
                    .get_or_insert_with(Default::default)
                    .adapter = Some(value.clone());
            }
            Self::ProviderDefaultsModel { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .defaults
                    .get_or_insert_with(Default::default)
                    .model = Some(value.clone());
            }
            Self::ProviderDefaultsThinkingMode { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .defaults
                    .get_or_insert_with(Default::default)
                    .thinking_mode = Some(value.clone());
            }
            Self::ProviderDefaultsSpeedMode { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .defaults
                    .get_or_insert_with(Default::default)
                    .speed_mode = Some(value.clone());
            }
            Self::ProviderDefaultsVerbosity { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .defaults
                    .get_or_insert_with(Default::default)
                    .verbosity = Some(value.clone());
            }
            Self::ProviderDefaultsParallelToolCalls { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .defaults
                    .get_or_insert_with(Default::default)
                    .parallel_tool_calls = Some(*value);
            }
            Self::AgentDefaultsProvider { agent_name, value } => {
                config
                    .agents
                    .agents
                    .entry(agent_name.clone())
                    .or_default()
                    .defaults
                    .provider = Some(value.clone());
            }
            Self::AgentDefaultsAdapter { agent_name, value } => {
                config
                    .agents
                    .agents
                    .entry(agent_name.clone())
                    .or_default()
                    .defaults
                    .adapter = Some(value.clone());
            }
            Self::AgentDefaultsModel { agent_name, value } => {
                config
                    .agents
                    .agents
                    .entry(agent_name.clone())
                    .or_default()
                    .defaults
                    .model = Some(value.clone());
            }
            Self::AgentDefaultsThinkingMode { agent_name, value } => {
                config
                    .agents
                    .agents
                    .entry(agent_name.clone())
                    .or_default()
                    .defaults
                    .thinking_mode = Some(value.clone());
            }
            Self::AgentDefaultsSpeedMode { agent_name, value } => {
                config
                    .agents
                    .agents
                    .entry(agent_name.clone())
                    .or_default()
                    .defaults
                    .speed_mode = Some(value.clone());
            }
            Self::AgentDefaultsVerbosity { agent_name, value } => {
                config
                    .agents
                    .agents
                    .entry(agent_name.clone())
                    .or_default()
                    .defaults
                    .verbosity = Some(value.clone());
            }
            Self::AgentDefaultsParallelToolCalls { agent_name, value } => {
                config
                    .agents
                    .agents
                    .entry(agent_name.clone())
                    .or_default()
                    .defaults
                    .parallel_tool_calls = Some(*value);
            }
            Self::ProviderAuthBaseUrl { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .auth
                    .get_or_insert_with(Default::default)
                    .base_url = Some(value.clone());
            }
            Self::ProviderAuthProtocolPath {
                provider_id,
                protocol,
                value,
            } => {
                let auth = config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .auth
                    .get_or_insert_with(Default::default);
                let protocol_paths = auth.protocol_paths.get_or_insert_with(Default::default);
                match protocol.as_str() {
                    "openai" => protocol_paths.openai = Some(value.clone()),
                    "anthropic" => protocol_paths.anthropic = Some(value.clone()),
                    "gemini" => protocol_paths.gemini = Some(value.clone()),
                    _ => {}
                }
            }
            Self::ProviderAuthApiKey { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .auth
                    .get_or_insert_with(Default::default)
                    .api_key = Some(value.clone());
            }
            Self::ProviderEnabled { provider_id, value } => {
                config
                    .providers
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
        "enabled" => Ok(ConfigOverride::ProviderEnabled {
            provider_id,
            value: parse_bool(key, raw_value)?,
        }),
        _ if field.starts_with("defaults.") => {
            let name = field.trim_start_matches("defaults.").trim();
            match name {
                "provider" => Ok(ConfigOverride::ProviderDefaultsProvider { provider_id, value }),
                "adapter" => Ok(ConfigOverride::ProviderDefaultsAdapter { provider_id, value }),
                "model" => Ok(ConfigOverride::ProviderDefaultsModel { provider_id, value }),
                "thinking_mode" => {
                    Ok(ConfigOverride::ProviderDefaultsThinkingMode { provider_id, value })
                }
                "speed_mode" => {
                    Ok(ConfigOverride::ProviderDefaultsSpeedMode { provider_id, value })
                }
                "verbosity" => Ok(ConfigOverride::ProviderDefaultsVerbosity { provider_id, value }),
                "parallel_tool_calls" => Ok(ConfigOverride::ProviderDefaultsParallelToolCalls {
                    provider_id,
                    value: parse_bool(key, raw_value)?,
                }),
                _ => Err(ConfigError::InvalidOverride(key.to_owned())),
            }
        }
        "base_url" | "api_key" => Err(ConfigError::InvalidOverride(format!(
            "{key} is no longer supported; use providers.{provider_id}.auth.{field}"
        ))),
        _ => {
            let Some(auth_field) = field.strip_prefix("auth.") else {
                return Err(ConfigError::InvalidOverride(key.to_owned()));
            };
            match auth_field {
                "base_url" => Ok(ConfigOverride::ProviderAuthBaseUrl { provider_id, value }),
                "api_key" => Ok(ConfigOverride::ProviderAuthApiKey {
                    provider_id,
                    value: parse_provider_secret_source_override(raw_value),
                }),
                _ if auth_field.starts_with("protocol_paths.") => {
                    let protocol = auth_field
                        .trim_start_matches("protocol_paths.")
                        .trim()
                        .to_owned();
                    match protocol.as_str() {
                        "openai" | "anthropic" | "gemini" => {
                            Ok(ConfigOverride::ProviderAuthProtocolPath {
                                provider_id,
                                protocol,
                                value,
                            })
                        }
                        _ => Err(ConfigError::InvalidOverride(key.to_owned())),
                    }
                }
                _ => Err(ConfigError::InvalidOverride(key.to_owned())),
            }
        }
    }
}

fn parse_provider_secret_source_override(
    raw_value: &str,
) -> super::overlay::ProviderSecretSourceOverlay {
    if let Some(value) = raw_value.strip_prefix("env:") {
        return super::overlay::ProviderSecretSourceOverlay::Env(value.to_owned());
    }
    if let Some(value) = raw_value.strip_prefix("inline:") {
        return super::overlay::ProviderSecretSourceOverlay::Inline(value.to_owned());
    }
    super::overlay::ProviderSecretSourceOverlay::Inline(raw_value.to_owned())
}

fn parse_agent_override(key: &str, raw_value: &str) -> Result<ConfigOverride, ConfigError> {
    let Some(rest) = key.strip_prefix("agents.") else {
        return Err(ConfigError::InvalidOverride(key.to_owned()));
    };
    let (agent_name, field) = rest
        .split_once('.')
        .ok_or_else(|| ConfigError::InvalidOverride(key.to_owned()))?;
    let agent_name = agent_name.trim().to_owned();
    let value = raw_value.to_owned();

    match field {
        _ if field.starts_with("defaults.") => {
            let name = field.trim_start_matches("defaults.").trim();
            match name {
                "provider" => Ok(ConfigOverride::AgentDefaultsProvider { agent_name, value }),
                "adapter" => Ok(ConfigOverride::AgentDefaultsAdapter { agent_name, value }),
                "model" => Ok(ConfigOverride::AgentDefaultsModel { agent_name, value }),
                "thinking_mode" => {
                    Ok(ConfigOverride::AgentDefaultsThinkingMode { agent_name, value })
                }
                "speed_mode" => Ok(ConfigOverride::AgentDefaultsSpeedMode { agent_name, value }),
                "verbosity" => Ok(ConfigOverride::AgentDefaultsVerbosity { agent_name, value }),
                "parallel_tool_calls" => Ok(ConfigOverride::AgentDefaultsParallelToolCalls {
                    agent_name,
                    value: parse_bool(key, raw_value)?,
                }),
                _ => Err(ConfigError::InvalidOverride(key.to_owned())),
            }
        }
        _ => Err(ConfigError::InvalidOverride(key.to_owned())),
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
