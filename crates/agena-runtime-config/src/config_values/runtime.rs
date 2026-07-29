use super::{
    BTreeMap, Deserialize, ProviderAuthConfig, ProviderDefaultsConfig,
    ResolvedProviderAdapterConfig, Serialize,
};
use agena_provider::{ProviderNetworkConfig, ResolvedProviderModelConfig};

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

/// Runtime identity settings that affect provider request headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct RuntimeConfig {
    pub providers: RuntimeProvidersConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct RuntimeProvidersConfig {
    pub client_versions: ProviderClientVersionSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderClientVersionSettings {
    pub codex: String,
    pub claude: String,
    pub gemini: String,
}

impl Default for ProviderClientVersionSettings {
    fn default() -> Self {
        let defaults = agena_provider::ProviderClientVersions::default();
        Self {
            codex: defaults.codex,
            claude: defaults.claude,
            gemini: defaults.gemini,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SessionConfig {
    pub compaction: SessionCompactionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionCompactionConfig {
    pub auto: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved_tokens: Option<u32>,
}

impl Default for SessionCompactionConfig {
    fn default() -> Self {
        Self {
            auto: true,
            reserved_tokens: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProviderConfig {
    pub enabled: bool,
    pub defaults: ProviderDefaultsConfig,
    pub auth: ProviderAuthConfig,
    pub network: ProviderNetworkConfig,
    pub adapters: BTreeMap<String, ResolvedProviderAdapterConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ResolvedProviderModelConfig>,
}
