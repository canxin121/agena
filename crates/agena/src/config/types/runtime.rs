use super::{
    BTreeMap, Deserialize, EnvFilter, ProviderAuthConfig, ProviderDefaultsConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderModelConfig, Serialize,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TracingConfig {
    pub filter: String,
    pub database: String,
    pub adapter: String,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            filter: "info".to_string(),
            database: "error".to_string(),
            adapter: "off".to_string(),
        }
    }
}

impl TracingConfig {
    pub fn env_filter(&self) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
        let mut filter = EnvFilter::try_new(self.filter.as_str())?;
        for target in ["sqlx", "sea_orm"] {
            filter = filter.add_directive(format!("{target}={}", self.database).parse()?);
        }
        filter = filter.add_directive(format!("agena::adapter={}", self.adapter).parse()?);
        Ok(filter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UiConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    pub tui: TuiUiConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TuiColorSchemeConfig {
    #[default]
    Auto,
    Dark,
    Light,
}

impl std::str::FromStr for TuiColorSchemeConfig {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            _ => Err(format!(
                "ui.tui.color_scheme expects one of auto,dark,light, got `{value}`"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TuiGraphicsModeConfig {
    /// Negotiate the best native image protocol when the complete terminal
    /// path can be established, otherwise retain semantic Unicode/text output.
    #[default]
    Auto,
    /// Probe for native graphics even when the transport path cannot be
    /// established automatically. Intended for expert-configured paths.
    Native,
    /// Skip native graphics negotiation and keep all rich content in the
    /// deterministic Unicode/text renderer.
    Unicode,
}

impl std::str::FromStr for TuiGraphicsModeConfig {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "native" | "image" | "images" | "on" | "1" => Ok(Self::Native),
            "unicode" | "text" | "halfblocks" | "off" | "0" => Ok(Self::Unicode),
            _ => Err(format!(
                "ui.tui.graphics expects one of auto,native,unicode, got `{value}`"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct TuiUiConfig {
    pub color_scheme: TuiColorSchemeConfig,
    pub graphics: TuiGraphicsModeConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub permission: crate::agent::PermissionConfig,
    #[serde(
        default,
        skip_serializing_if = "crate::agents::AgentSelectionConfig::is_empty"
    )]
    pub defaults: crate::agents::AgentSelectionConfig,
    #[serde(
        default,
        skip_serializing_if = "crate::agents::AgentToolsConfig::is_empty"
    )]
    pub tools: crate::agents::AgentToolsConfig,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

pub use agena_plugin_host::PluginsConfig as PluginConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProviderConfig {
    pub enabled: bool,
    pub defaults: ProviderDefaultsConfig,
    pub auth: ProviderAuthConfig,
    pub network: super::ProviderNetworkConfig,
    pub adapters: BTreeMap<String, ResolvedProviderAdapterConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ResolvedProviderModelConfig>,
}
