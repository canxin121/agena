use std::collections::BTreeMap;

use merge::Merge as DeriveMerge;
use serde::{Deserialize, Serialize};

use crate::provider::auth::{AuthData, CredentialIssuer};
use crate::{
    model_catalog::{CatalogModelDefinition, catalog_definition_to_provider_definition},
    provider::ConfiguredModelDefinition,
};

use super::types::{
    OpenAiApiModeConfig, OpenAiBackendConfig, ProviderCapabilityFamilyConfig,
    ProviderModelDiscoveryConfig, ResolvedProviderModelConfig, StreamTransportMode,
};

pub type ProviderModelOverlay = ResolvedProviderModelConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMode {
    None,
    Api,
    #[serde(rename = "gitlab_api")]
    Gitlab,
    Credential,
    BedrockSigv4,
    GoogleAdc,
    SapAiCore,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ProviderProtocolPathsOverlay {
    #[merge(strategy = option_override)]
    pub openai: Option<String>,
    #[merge(strategy = option_override)]
    pub anthropic: Option<String>,
    #[merge(strategy = option_override)]
    pub gemini: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ProviderAuthOverlay {
    #[merge(strategy = option_override)]
    pub mode: Option<ProviderAuthMode>,
    #[merge(strategy = option_override)]
    pub base_url: Option<String>,
    #[merge(strategy = option_struct_merge)]
    pub protocol_paths: Option<ProviderProtocolPathsOverlay>,
    #[merge(strategy = option_override)]
    pub api_key: Option<String>,
    #[merge(strategy = option_override)]
    pub api_key_env: Option<String>,
    #[merge(strategy = option_override)]
    pub instance_url: Option<String>,
    #[merge(strategy = option_override)]
    pub ai_gateway_url: Option<String>,
    #[merge(strategy = map_extend)]
    pub ai_gateway_headers: BTreeMap<String, String>,
    #[merge(strategy = map_extend)]
    pub feature_flags: BTreeMap<String, bool>,
    #[merge(strategy = option_override)]
    pub issuer: Option<CredentialIssuer>,
    #[serde(default)]
    #[merge(strategy = option_override)]
    pub credential: Option<AuthData>,
    #[merge(strategy = option_override)]
    pub profile: Option<String>,
    #[merge(strategy = option_override)]
    pub access_key_id: Option<String>,
    #[merge(strategy = option_override)]
    pub secret_access_key: Option<String>,
    #[merge(strategy = option_override)]
    pub session_token: Option<String>,
    #[merge(strategy = option_override)]
    pub region: Option<String>,
    #[merge(strategy = option_override)]
    pub service_key_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ProviderAdapterOverlay {
    #[merge(strategy = option_override)]
    pub backend: Option<OpenAiBackendConfig>,
    #[merge(strategy = option_override)]
    pub enabled: Option<bool>,
    #[merge(strategy = option_override)]
    pub model_discovery: Option<ProviderModelDiscoveryConfig>,
    #[merge(strategy = option_override)]
    pub base_url: Option<String>,
    #[merge(strategy = option_override)]
    pub models_url: Option<String>,
    #[merge(strategy = option_override)]
    pub capability_family: Option<ProviderCapabilityFamilyConfig>,
    #[merge(strategy = option_override)]
    pub messages_url: Option<String>,
    #[merge(strategy = option_override)]
    pub auth_header: Option<String>,
    #[merge(strategy = option_override)]
    pub auth_scheme: Option<String>,
    #[merge(strategy = option_override)]
    pub user_agent: Option<String>,
    #[merge(strategy = option_override)]
    pub extra_beta_header: Option<String>,
    #[merge(strategy = option_override)]
    pub eager_input_streaming: Option<bool>,
    #[merge(strategy = map_extend)]
    pub extra_headers: BTreeMap<String, String>,
    #[merge(strategy = option_override)]
    pub api_mode: Option<OpenAiApiModeConfig>,
    #[merge(strategy = option_override)]
    pub stream_mode: Option<StreamTransportMode>,
    #[merge(strategy = option_override)]
    pub realtime_ws_url: Option<String>,
    #[merge(strategy = option_override)]
    pub instance_url: Option<String>,
    #[merge(strategy = option_override)]
    pub ai_gateway_url: Option<String>,
    #[merge(strategy = map_extend)]
    pub ai_gateway_headers: BTreeMap<String, String>,
    #[merge(strategy = map_extend)]
    pub feature_flags: BTreeMap<String, bool>,
    #[merge(strategy = map_extend)]
    pub models: BTreeMap<String, ProviderModelOverlay>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ProviderOverlay {
    #[merge(strategy = option_override)]
    pub enabled: Option<bool>,
    #[merge(strategy = option_override)]
    pub default_adapter: Option<String>,
    #[merge(strategy = option_override)]
    pub default_model: Option<String>,
    #[merge(strategy = option_struct_merge)]
    pub auth: Option<ProviderAuthOverlay>,
    #[merge(strategy = map_extend)]
    pub adapters: BTreeMap<String, ProviderAdapterOverlay>,
}

pub(crate) fn option_override<T>(base: &mut Option<T>, overlay: Option<T>) {
    if let Some(value) = overlay {
        *base = Some(value);
    }
}

pub(crate) fn option_struct_merge<T>(base: &mut Option<T>, overlay: Option<T>)
where
    T: merge::Merge,
{
    match (base, overlay) {
        (Some(base), Some(overlay)) => {
            <T as merge::Merge>::merge(base, overlay);
        }
        (slot @ None, Some(overlay)) => {
            *slot = Some(overlay);
        }
        _ => {}
    }
}

pub(crate) fn map_extend<K, V>(base: &mut BTreeMap<K, V>, overlay: BTreeMap<K, V>)
where
    K: Ord,
{
    base.extend(overlay);
}

pub fn provider_model_overlay_from_catalog_definition(
    definition: &CatalogModelDefinition,
) -> ProviderModelOverlay {
    provider_model_overlay_from_definition(catalog_definition_to_provider_definition(definition))
}

pub fn provider_model_overlay_from_definition(
    definition: ConfiguredModelDefinition,
) -> ProviderModelOverlay {
    ProviderModelOverlay {
        enabled: true,
        definition,
    }
}
