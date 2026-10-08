use super::{BTreeMap, Deserialize, ProviderAuthConfig, ResolvedProviderAdapterConfig, Serialize};
use agena_provider::{ProviderNetworkConfig, ResolvedProviderModelConfig};

/// Opaque client preferences. Runtime and storage preserve this document;
/// clients validate and interpret their own entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct UiConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(flatten)]
    pub preferences: BTreeMap<String, serde_json::Value>,
}

/// Runtime identity settings that affect provider request headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct RuntimeConfig {
    pub providers: RuntimeProvidersConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
/// Resolved runtime provider settings.
pub struct RuntimeProvidersConfig {
    pub client_versions: ProviderClientVersionSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Preferred client versions for built-in providers.
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
/// Resolved session configuration.
pub struct SessionConfig {
    pub compaction: SessionCompactionConfig,
    /// Cap on model turns within one stable run. `0` means unlimited;
    /// `None` falls back to `DEFAULT_MAX_MODEL_TURNS` (500).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
/// Resolved session compaction configuration.
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
/// Resolved configuration of one provider.
pub struct ResolvedProviderConfig {
    pub enabled: bool,
    pub auth: ProviderAuthConfig,
    pub network: ProviderNetworkConfig,
    pub adapters: BTreeMap<String, ResolvedProviderAdapterConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ResolvedProviderModelConfig>,
}
