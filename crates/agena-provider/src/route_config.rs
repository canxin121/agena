//! Provider request-route configuration shared by schema parsers and adapters.

use serde::{Deserialize, Serialize};

pub const CLINE_API_BASE_URL: &str = "https://api.cline.bot";
pub const CLINE_API_OPENAI_PROTOCOL_PATH: &str = "/api/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderProtocolPathsConfig {
    pub openai: String,
    pub anthropic: String,
    pub gemini: String,
}

impl Default for ProviderProtocolPathsConfig {
    fn default() -> Self {
        Self {
            openai: "/v1".to_owned(),
            anthropic: "/v1".to_owned(),
            gemini: "/v1beta".to_owned(),
        }
    }
}

pub fn cline_api_protocol_paths() -> ProviderProtocolPathsConfig {
    ProviderProtocolPathsConfig {
        openai: CLINE_API_OPENAI_PROTOCOL_PATH.to_owned(),
        ..ProviderProtocolPathsConfig::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderModelDiscoveryConfig {
    #[default]
    Live,
    ConfiguredOnly,
}

#[cfg(test)]
mod tests {
    use super::{
        CLINE_API_OPENAI_PROTOCOL_PATH, ProviderModelDiscoveryConfig, ProviderProtocolPathsConfig,
        cline_api_protocol_paths,
    };

    #[test]
    fn cline_route_overrides_only_the_openai_protocol_path() {
        let paths = cline_api_protocol_paths();
        assert_eq!(paths.openai, CLINE_API_OPENAI_PROTOCOL_PATH);
        assert_eq!(
            paths.anthropic,
            ProviderProtocolPathsConfig::default().anthropic
        );
        assert_eq!(paths.gemini, ProviderProtocolPathsConfig::default().gemini);
        assert_eq!(
            serde_json::to_string(&ProviderModelDiscoveryConfig::ConfiguredOnly)
                .expect("serialize model discovery mode"),
            "\"configured_only\""
        );
    }
}
