use std::path::PathBuf;
use std::str::FromStr;

use crate::{TuiColorSchemeConfig, TuiGraphicsModeConfig};

/// Parsed `--set key=value` input. This is a schema value: applying it to a
/// concrete raw configuration document remains the responsibility of that
/// document's owning configuration adapter.
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
        value: agena_provider::ProviderSecretSourceOverlay,
    },
    ProviderEnabled {
        provider_id: String,
        value: bool,
    },
}

/// Runtime-owned request passed to a concrete configuration loader.
/// The loader implementation may still be schema-specific, but process and
/// bootstrap callers do not need a concrete-loader request value.
#[derive(Debug, Clone, Default)]
pub struct LoadConfigRequest {
    pub overrides: Vec<ConfigOverride>,
    pub workspace_root: Option<PathBuf>,
}

/// Parse raw `--set` expressions at the Runtime bootstrap boundary.
pub fn parse_config_override_expressions(
    expressions: &[String],
) -> Result<Vec<ConfigOverride>, RuntimeConfigOverrideError> {
    expressions
        .iter()
        .map(|expression| expression.parse())
        .collect()
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RuntimeConfigOverrideError {
    #[error("config modes are no longer supported; remove `{field}` and use a single config file")]
    UnsupportedModeConfig { field: &'static str },
    #[error("invalid override `{0}")]
    InvalidOverride(String),
    #[error("config validation failed: {0}")]
    Validation(String),
    #[error("invalid numeric value for `{key}`: {value}")]
    InvalidNumber { key: String, value: String },
}

impl FromStr for ConfigOverride {
    type Err = RuntimeConfigOverrideError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (key, raw_value) = value
            .split_once('=')
            .ok_or_else(|| RuntimeConfigOverrideError::InvalidOverride(value.to_owned()))?;
        let key = key.trim();
        let raw_value = raw_value.trim();
        match key {
            "mode" => Err(RuntimeConfigOverrideError::UnsupportedModeConfig { field: "mode" }),
            "tracing.filter" => Ok(Self::TracingFilter(raw_value.to_owned())),
            "tracing.database" => Ok(Self::TracingDatabase(raw_value.to_owned())),
            "tracing.adapter" => Ok(Self::TracingAdapter(raw_value.to_owned())),
            "ui.locale" => Ok(Self::UiLocale(raw_value.to_owned())),
            "ui.tui.color_scheme" => Ok(Self::UiTuiColorScheme(
                raw_value
                    .parse()
                    .map_err(RuntimeConfigOverrideError::Validation)?,
            )),
            "ui.tui.graphics" => Ok(Self::UiTuiGraphics(
                raw_value
                    .parse()
                    .map_err(RuntimeConfigOverrideError::Validation)?,
            )),
            "ui.tui.theme" => Ok(Self::UiTuiTheme(raw_value.to_owned())),
            "providers.default" => Ok(Self::ProvidersDefault(raw_value.to_owned())),
            "agents.default" => Ok(Self::AgentsDefault(raw_value.to_owned())),
            _ if key.starts_with("providers.") => parse_provider_override(key, raw_value),
            _ if key.starts_with("agents.") => parse_agent_override(key, raw_value),
            _ => Err(RuntimeConfigOverrideError::InvalidOverride(key.to_owned())),
        }
    }
}

fn parse_provider_override(
    key: &str,
    raw_value: &str,
) -> Result<ConfigOverride, RuntimeConfigOverrideError> {
    let rest = key
        .strip_prefix("providers.")
        .ok_or_else(|| RuntimeConfigOverrideError::InvalidOverride(key.to_owned()))?;
    let (provider_id, field) = rest
        .split_once('.')
        .ok_or_else(|| RuntimeConfigOverrideError::InvalidOverride(key.to_owned()))?;
    let provider_id = provider_id.trim().to_owned();
    let value = raw_value.to_owned();
    match field {
        "enabled" => Ok(ConfigOverride::ProviderEnabled {
            provider_id,
            value: parse_bool(key, raw_value)?,
        }),
        _ if field.starts_with("defaults.") => match field.trim_start_matches("defaults.").trim() {
            "provider" => Ok(ConfigOverride::ProviderDefaultsProvider { provider_id, value }),
            "adapter" => Ok(ConfigOverride::ProviderDefaultsAdapter { provider_id, value }),
            "model" => Ok(ConfigOverride::ProviderDefaultsModel { provider_id, value }),
            "thinking_mode" => {
                Ok(ConfigOverride::ProviderDefaultsThinkingMode { provider_id, value })
            }
            "speed_mode" => Ok(ConfigOverride::ProviderDefaultsSpeedMode { provider_id, value }),
            "verbosity" => Ok(ConfigOverride::ProviderDefaultsVerbosity { provider_id, value }),
            "parallel_tool_calls" => Ok(ConfigOverride::ProviderDefaultsParallelToolCalls {
                provider_id,
                value: parse_bool(key, raw_value)?,
            }),
            _ => Err(RuntimeConfigOverrideError::InvalidOverride(key.to_owned())),
        },
        "network.request_timeout_secs" => Ok(ConfigOverride::ProviderRequestTimeoutSecs {
            provider_id,
            value: parse_numeric(raw_value, key)?,
        }),
        "network.connect_timeout_secs" => Ok(ConfigOverride::ProviderConnectTimeoutSecs {
            provider_id,
            value: parse_numeric(raw_value, key)?,
        }),
        "base_url" | "api_key" => Err(RuntimeConfigOverrideError::InvalidOverride(format!(
            "{key} is no longer supported; use providers.{provider_id}.auth.{field}"
        ))),
        _ => {
            let auth_field = field
                .strip_prefix("auth.")
                .ok_or_else(|| RuntimeConfigOverrideError::InvalidOverride(key.to_owned()))?;
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
                        _ => Err(RuntimeConfigOverrideError::InvalidOverride(key.to_owned())),
                    }
                }
                _ => Err(RuntimeConfigOverrideError::InvalidOverride(key.to_owned())),
            }
        }
    }
}

fn parse_agent_override(
    key: &str,
    raw_value: &str,
) -> Result<ConfigOverride, RuntimeConfigOverrideError> {
    let rest = key
        .strip_prefix("agents.")
        .ok_or_else(|| RuntimeConfigOverrideError::InvalidOverride(key.to_owned()))?;
    let (agent_name, field) = rest
        .split_once('.')
        .ok_or_else(|| RuntimeConfigOverrideError::InvalidOverride(key.to_owned()))?;
    let agent_name = agent_name.trim().to_owned();
    let value = raw_value.to_owned();
    if !field.starts_with("defaults.") {
        return Err(RuntimeConfigOverrideError::InvalidOverride(key.to_owned()));
    }
    match field.trim_start_matches("defaults.").trim() {
        "provider" => Ok(ConfigOverride::AgentDefaultsProvider { agent_name, value }),
        "adapter" => Ok(ConfigOverride::AgentDefaultsAdapter { agent_name, value }),
        "model" => Ok(ConfigOverride::AgentDefaultsModel { agent_name, value }),
        "thinking_mode" => Ok(ConfigOverride::AgentDefaultsThinkingMode { agent_name, value }),
        "speed_mode" => Ok(ConfigOverride::AgentDefaultsSpeedMode { agent_name, value }),
        "verbosity" => Ok(ConfigOverride::AgentDefaultsVerbosity { agent_name, value }),
        "parallel_tool_calls" => Ok(ConfigOverride::AgentDefaultsParallelToolCalls {
            agent_name,
            value: parse_bool(key, raw_value)?,
        }),
        _ => Err(RuntimeConfigOverrideError::InvalidOverride(key.to_owned())),
    }
}

fn parse_provider_secret_source_override(
    raw_value: &str,
) -> agena_provider::ProviderSecretSourceOverlay {
    if let Some(value) = raw_value.strip_prefix("env:") {
        return agena_provider::ProviderSecretSourceOverlay::Env(value.to_owned());
    }
    if let Some(value) = raw_value.strip_prefix("inline:") {
        return agena_provider::ProviderSecretSourceOverlay::Inline(value.to_owned());
    }
    agena_provider::ProviderSecretSourceOverlay::Inline(raw_value.to_owned())
}

fn parse_numeric(value: &str, key: &str) -> Result<u64, RuntimeConfigOverrideError> {
    value
        .parse::<u64>()
        .map_err(|_| RuntimeConfigOverrideError::InvalidNumber {
            key: key.to_owned(),
            value: value.to_owned(),
        })
}

fn parse_bool(key: &str, value: &str) -> Result<bool, RuntimeConfigOverrideError> {
    match value.trim() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(RuntimeConfigOverrideError::InvalidOverride(format!(
            "{key} expects bool, got `{value}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracing_and_provider_overrides_without_core_schema() {
        assert_eq!(
            "tracing.filter=debug".parse(),
            Ok(ConfigOverride::TracingFilter("debug".to_owned()))
        );
        assert_eq!(
            "providers.demo.auth.api_key=env:DEMO_KEY".parse(),
            Ok(ConfigOverride::ProviderAuthApiKey {
                provider_id: "demo".to_owned(),
                value: agena_provider::ProviderSecretSourceOverlay::Env("DEMO_KEY".to_owned()),
            })
        );
    }

    #[test]
    fn rejects_legacy_and_invalid_override_forms() {
        assert!(matches!(
            "mode=dev".parse::<ConfigOverride>(),
            Err(RuntimeConfigOverrideError::UnsupportedModeConfig { .. })
        ));
        assert!(matches!(
            "providers.demo.defaults.parallel_tool_calls=maybe".parse::<ConfigOverride>(),
            Err(RuntimeConfigOverrideError::InvalidOverride(_))
        ));
    }

    #[test]
    fn parses_bootstrap_override_batches_in_order() {
        let expressions = vec![
            "tracing.filter=debug".to_owned(),
            "providers.demo.defaults.model=gpt-test".to_owned(),
        ];
        let parsed = parse_config_override_expressions(&expressions)
            .expect("bootstrap override batch should parse");
        assert_eq!(parsed.len(), expressions.len());
        assert_eq!(
            parsed.first(),
            Some(&ConfigOverride::TracingFilter("debug".to_owned()))
        );
    }
}
