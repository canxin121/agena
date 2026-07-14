use std::str::FromStr;

use super::{
    ConfigError, RawConfig, RawTracingConfig, RawTuiUiConfig, RawUiConfig, TuiColorSchemeConfig,
    TuiGraphicsModeConfig, parse_numeric,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOverride {
    TracingFilter(String),
    TracingDatabase(String),
    TracingAdapter(String),
    UiLocale(String),
    UiTuiColorScheme(TuiColorSchemeConfig),
    UiTuiGraphics(TuiGraphicsModeConfig),
    UiTuiTheme(String),
    ProvidersDefault(String),
    AgentsDefault(String),
    ProviderRequestTimeoutSecs {
        provider_id: String,
        value: u64,
    },
    ProviderConnectTimeoutSecs {
        provider_id: String,
        value: u64,
    },
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
            "ui.tui.graphics" => Ok(Self::UiTuiGraphics(
                raw_value.parse().map_err(ConfigError::Validation)?,
            )),
            "ui.tui.theme" => Ok(Self::UiTuiTheme(raw_value.to_owned())),
            "providers.default" => Ok(Self::ProvidersDefault(raw_value.to_owned())),
            "agents.default" => Ok(Self::AgentsDefault(raw_value.to_owned())),
            _ if key.starts_with("providers.") => parse_provider_override(key, raw_value),
            _ if key.starts_with("agents.") => parse_agent_override(key, raw_value),
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
            Self::UiTuiGraphics(graphics) => {
                config
                    .ui
                    .get_or_insert_with(RawUiConfig::default)
                    .tui
                    .get_or_insert_with(RawTuiUiConfig::default)
                    .graphics = Some(*graphics);
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
            Self::ProviderRequestTimeoutSecs { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .network
                    .get_or_insert_with(Default::default)
                    .request_timeout_secs = Some(*value);
            }
            Self::ProviderConnectTimeoutSecs { provider_id, value } => {
                config
                    .providers
                    .providers
                    .entry(provider_id.clone())
                    .or_default()
                    .network
                    .get_or_insert_with(Default::default)
                    .connect_timeout_secs = Some(*value);
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
        "network.request_timeout_secs" => Ok(ConfigOverride::ProviderRequestTimeoutSecs {
            provider_id,
            value: parse_numeric(raw_value, key)?,
        }),
        "network.connect_timeout_secs" => Ok(ConfigOverride::ProviderConnectTimeoutSecs {
            provider_id,
            value: parse_numeric(raw_value, key)?,
        }),
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
