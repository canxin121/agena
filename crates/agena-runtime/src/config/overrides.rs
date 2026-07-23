//! Runtime-private adapter that materializes Runtime-owned parsed overrides into the
//! legacy raw configuration schema.

use super::{RawConfig, RawTracingConfig, RawTuiUiConfig, RawUiConfig};
use agena_runtime::ConfigOverride;

pub(crate) fn apply_config_override(override_value: &ConfigOverride, config: &mut RawConfig) {
    match override_value {
        ConfigOverride::TracingFilter(filter) => {
            config
                .tracing
                .get_or_insert_with(RawTracingConfig::default)
                .filter = Some(filter.clone());
        }
        ConfigOverride::TracingDatabase(level) => {
            config
                .tracing
                .get_or_insert_with(RawTracingConfig::default)
                .database = Some(level.clone());
        }
        ConfigOverride::TracingAdapter(mode) => {
            config
                .tracing
                .get_or_insert_with(RawTracingConfig::default)
                .adapter = Some(mode.clone());
        }
        ConfigOverride::UiLocale(locale) => {
            config.ui.get_or_insert_with(RawUiConfig::default).locale = Some(locale.clone());
        }
        ConfigOverride::UiTuiColorScheme(color_scheme) => {
            config
                .ui
                .get_or_insert_with(RawUiConfig::default)
                .tui
                .get_or_insert_with(RawTuiUiConfig::default)
                .color_scheme = Some(*color_scheme);
        }
        ConfigOverride::UiTuiGraphics(graphics) => {
            config
                .ui
                .get_or_insert_with(RawUiConfig::default)
                .tui
                .get_or_insert_with(RawTuiUiConfig::default)
                .graphics = Some(*graphics);
        }
        ConfigOverride::UiTuiTheme(theme) => {
            config
                .ui
                .get_or_insert_with(RawUiConfig::default)
                .tui
                .get_or_insert_with(RawTuiUiConfig::default)
                .theme = Some(theme.clone());
        }
        ConfigOverride::ProvidersDefault(value) => config.providers.default = Some(value.clone()),
        ConfigOverride::AgentsDefault(value) => config.agents.default = Some(value.clone()),
        ConfigOverride::ProviderRequestTimeoutSecs { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .network
                .get_or_insert_with(Default::default)
                .request_timeout_secs = Some(*value);
        }
        ConfigOverride::ProviderConnectTimeoutSecs { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .network
                .get_or_insert_with(Default::default)
                .connect_timeout_secs = Some(*value);
        }
        ConfigOverride::ProviderDefaultsProvider { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .defaults
                .get_or_insert_with(Default::default)
                .provider = Some(value.clone());
        }
        ConfigOverride::ProviderDefaultsAdapter { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .defaults
                .get_or_insert_with(Default::default)
                .adapter = Some(value.clone());
        }
        ConfigOverride::ProviderDefaultsModel { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .defaults
                .get_or_insert_with(Default::default)
                .model = Some(value.clone());
        }
        ConfigOverride::ProviderDefaultsThinkingMode { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .defaults
                .get_or_insert_with(Default::default)
                .thinking_mode = Some(value.clone());
        }
        ConfigOverride::ProviderDefaultsSpeedMode { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .defaults
                .get_or_insert_with(Default::default)
                .speed_mode = Some(value.clone());
        }
        ConfigOverride::ProviderDefaultsVerbosity { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .defaults
                .get_or_insert_with(Default::default)
                .verbosity = Some(value.clone());
        }
        ConfigOverride::ProviderDefaultsParallelToolCalls { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .defaults
                .get_or_insert_with(Default::default)
                .parallel_tool_calls = Some(*value);
        }
        ConfigOverride::AgentDefaultsProvider { agent_name, value } => {
            config
                .agents
                .agents
                .entry(agent_name.clone())
                .or_default()
                .defaults
                .provider = Some(value.clone());
        }
        ConfigOverride::AgentDefaultsAdapter { agent_name, value } => {
            config
                .agents
                .agents
                .entry(agent_name.clone())
                .or_default()
                .defaults
                .adapter = Some(value.clone());
        }
        ConfigOverride::AgentDefaultsModel { agent_name, value } => {
            config
                .agents
                .agents
                .entry(agent_name.clone())
                .or_default()
                .defaults
                .model = Some(value.clone());
        }
        ConfigOverride::AgentDefaultsThinkingMode { agent_name, value } => {
            config
                .agents
                .agents
                .entry(agent_name.clone())
                .or_default()
                .defaults
                .thinking_mode = Some(value.clone());
        }
        ConfigOverride::AgentDefaultsSpeedMode { agent_name, value } => {
            config
                .agents
                .agents
                .entry(agent_name.clone())
                .or_default()
                .defaults
                .speed_mode = Some(value.clone());
        }
        ConfigOverride::AgentDefaultsVerbosity { agent_name, value } => {
            config
                .agents
                .agents
                .entry(agent_name.clone())
                .or_default()
                .defaults
                .verbosity = Some(value.clone());
        }
        ConfigOverride::AgentDefaultsParallelToolCalls { agent_name, value } => {
            config
                .agents
                .agents
                .entry(agent_name.clone())
                .or_default()
                .defaults
                .parallel_tool_calls = Some(*value);
        }
        ConfigOverride::ProviderAuthBaseUrl { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .auth
                .get_or_insert_with(Default::default)
                .base_url = Some(value.clone());
        }
        ConfigOverride::ProviderAuthProtocolPath {
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
        ConfigOverride::ProviderAuthApiKey { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .auth
                .get_or_insert_with(Default::default)
                .api_key = Some(value.clone());
        }
        ConfigOverride::ProviderEnabled { provider_id, value } => {
            config
                .providers
                .providers
                .entry(provider_id.clone())
                .or_default()
                .enabled = Some(*value);
        }
    }
}
