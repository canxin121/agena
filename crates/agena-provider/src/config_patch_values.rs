//! Stable provider configuration patch values.
//!
//! These values describe the persisted provider configuration wire shape. They
//! contain no loader, filesystem, or concrete-adapter behavior, so provider
//! draft editors and Core's schema adapter can share them without making the
//! editor depend on Core configuration types.

use merge::Merge as DeriveMerge;
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;
use std::str::FromStr;

use crate::{
    AuthData, CapabilityFamily, CatalogModelDefinition, ConfiguredModelDefinition,
    CredentialIssuer, GeminiStreamMode, OpenAiResponsesBackend, ProviderModelDiscoveryConfig,
    ProviderNativeToolFreshness, ProviderNativeToolHarnessKind, ProviderNativeToolRoute,
    ResolvedProviderModelConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Capability family of a provider (OpenAI, Anthropic, Gemini, ...).
pub enum ProviderCapabilityFamilyConfig {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
    Anthropic,
    Gemini,
    #[serde(rename = "bedrock")]
    Bedrock,
    Gitlab,
}

impl From<ProviderCapabilityFamilyConfig> for CapabilityFamily {
    fn from(value: ProviderCapabilityFamilyConfig) -> Self {
        match value {
            ProviderCapabilityFamilyConfig::OpenAi => CapabilityFamily::OpenAi,
            ProviderCapabilityFamilyConfig::OpenAiCompatible => CapabilityFamily::OpenAiCompatible,
            ProviderCapabilityFamilyConfig::Anthropic => CapabilityFamily::Anthropic,
            ProviderCapabilityFamilyConfig::Gemini => CapabilityFamily::Gemini,
            ProviderCapabilityFamilyConfig::Bedrock => CapabilityFamily::Bedrock,
            ProviderCapabilityFamilyConfig::Gitlab => CapabilityFamily::Gitlab,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Transport mode for streaming responses.
pub enum StreamTransportMode {
    Sse,
    #[serde(rename = "realtime_websocket")]
    RealtimeWebSocket,
}

impl FromStr for StreamTransportMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "sse" => Ok(Self::Sse),
            "realtime_websocket" => Ok(Self::RealtimeWebSocket),
            _ => Err(format!("unknown stream mode `{value}`")),
        }
    }
}

impl From<StreamTransportMode> for GeminiStreamMode {
    fn from(value: StreamTransportMode) -> Self {
        match value {
            StreamTransportMode::Sse => Self::Sse,
            StreamTransportMode::RealtimeWebSocket => Self::RealtimeWebSocket,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
/// Backend flavor for the OpenAI-compatible adapter.
pub enum OpenAiResponsesBackendConfig {
    #[default]
    Api,
    ChatgptCodex,
}

impl FromStr for OpenAiResponsesBackendConfig {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "api" => Ok(Self::Api),
            "chatgpt_codex" => Ok(Self::ChatgptCodex),
            _ => Err(format!("unknown openai backend `{value}`")),
        }
    }
}

impl From<OpenAiResponsesBackendConfig> for OpenAiResponsesBackend {
    fn from(value: OpenAiResponsesBackendConfig) -> Self {
        match value {
            OpenAiResponsesBackendConfig::Api => Self::Api,
            OpenAiResponsesBackendConfig::ChatgptCodex => Self::ChatgptCodex,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Auth mode of a provider (none, api, credential).
pub enum ProviderAuthMode {
    None,
    Api,
    Credential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// API subtype of a provider (custom, cline, gitlab, bedrock).
pub enum ProviderApiSubtype {
    Custom,
    #[serde(rename = "cline_api")]
    ClineApi,
    #[serde(rename = "gitlab_api")]
    Gitlab,
    BedrockSigv4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
/// Source of a secret in an overlay (inline or env).
pub enum ProviderSecretSourceOverlay {
    Inline(String),
    Env(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
/// GitLab API access overlay.
pub enum ProviderGitlabApiAccessOverlay {
    ApiKey { source: ProviderSecretSourceOverlay },
    Credential { credential: AuthData },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Optional protocol path overrides.
pub struct ProviderProtocolPathsOverlay {
    #[merge(strategy = option_override)]
    pub openai: Option<String>,
    #[merge(strategy = option_override)]
    pub anthropic: Option<String>,
    #[merge(strategy = option_override)]
    pub gemini: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Default provider/adapter/model overlay.
pub struct ProviderDefaultsOverlay {
    #[merge(strategy = option_override)]
    pub provider: Option<String>,
    #[merge(strategy = option_override)]
    pub adapter: Option<String>,
    #[merge(strategy = option_override)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Network timeout overlays.
pub struct ProviderNetworkOverlay {
    #[merge(strategy = option_override)]
    pub request_timeout_secs: Option<u64>,
    #[merge(strategy = option_override)]
    pub connect_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Auth overlay for a provider.
pub struct ProviderAuthOverlay {
    #[merge(strategy = option_override)]
    pub mode: Option<ProviderAuthMode>,
    #[merge(strategy = option_override)]
    pub subtype: Option<ProviderApiSubtype>,
    #[merge(strategy = option_override)]
    pub base_url: Option<String>,
    #[merge(strategy = option_struct_merge)]
    pub protocol_paths: Option<ProviderProtocolPathsOverlay>,
    #[merge(strategy = option_override)]
    pub api_key: Option<ProviderSecretSourceOverlay>,
    #[merge(strategy = option_override)]
    pub access: Option<ProviderGitlabApiAccessOverlay>,
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
#[serde(default, deny_unknown_fields)]
/// Overlay applied to a provider adapter.
pub struct ProviderAdapterOverlay {
    #[merge(strategy = option_override)]
    pub backend: Option<OpenAiResponsesBackendConfig>,
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
    pub models: BTreeMap<String, ResolvedProviderModelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Overlay for native tool routes.
pub struct ProviderNativeToolRoutesOverlay {
    #[merge(strategy = option_override)]
    pub web_search: Option<ProviderNativeToolRoute>,
    #[merge(strategy = option_override)]
    pub file_search: Option<ProviderNativeToolRoute>,
    #[merge(strategy = option_override)]
    pub code_execution: Option<ProviderNativeToolRoute>,
    #[merge(strategy = option_override)]
    pub image_generation: Option<ProviderNativeToolRoute>,
    #[merge(strategy = option_override)]
    pub computer: Option<ProviderNativeToolRoute>,
    #[merge(strategy = option_override)]
    pub bash: Option<ProviderNativeToolRoute>,
    #[merge(strategy = option_override)]
    pub text_editor: Option<ProviderNativeToolRoute>,
    #[merge(strategy = option_override)]
    pub url_context: Option<ProviderNativeToolRoute>,
    #[merge(strategy = option_override)]
    pub remote_mcp: Option<ProviderNativeToolRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Overlay for native tool user location.
pub struct ProviderNativeToolUserLocationOverlay {
    #[merge(strategy = option_override)]
    pub country: Option<String>,
    #[merge(strategy = option_override)]
    pub region: Option<String>,
    #[merge(strategy = option_override)]
    pub city: Option<String>,
    #[merge(strategy = option_override)]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Overlay for hosted web search.
pub struct ProviderHostedWebSearchOverlay {
    #[merge(strategy = option_override)]
    pub allowed_domains: Option<Vec<String>>,
    #[merge(strategy = option_override)]
    pub blocked_domains: Option<Vec<String>>,
    #[merge(strategy = option_override)]
    pub freshness: Option<ProviderNativeToolFreshness>,
    #[merge(strategy = option_struct_merge)]
    pub user_location: Option<ProviderNativeToolUserLocationOverlay>,
    #[merge(strategy = option_override)]
    pub max_results: Option<u32>,
    #[merge(strategy = option_override)]
    pub search_context_size: Option<String>,
    #[merge(strategy = option_override)]
    pub provider_options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Overlay for hosted file search.
pub struct ProviderHostedFileSearchOverlay {
    #[merge(strategy = option_override)]
    pub vector_store_ids: Option<Vec<String>>,
    #[merge(strategy = option_override)]
    pub max_results: Option<u32>,
    #[merge(strategy = option_override)]
    pub include_results: Option<bool>,
    #[merge(strategy = option_override)]
    pub provider_options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Container overlay for hosted code execution.
pub struct HostedCodeExecutionContainerOverlay {
    #[merge(strategy = option_override)]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[merge(strategy = option_override)]
    pub id: Option<String>,
    #[merge(strategy = option_override)]
    pub memory_limit: Option<String>,
    #[merge(strategy = option_override)]
    pub file_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Overlay for hosted code execution.
pub struct ProviderHostedCodeExecutionOverlay {
    #[merge(strategy = option_struct_merge)]
    pub container: Option<HostedCodeExecutionContainerOverlay>,
    #[merge(strategy = option_override)]
    pub provider_options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Overlay for hosted image generation.
pub struct ProviderHostedImageGenerationOverlay {
    #[merge(strategy = option_override)]
    pub background: Option<String>,
    #[merge(strategy = option_override)]
    pub size: Option<String>,
    #[merge(strategy = option_override)]
    pub quality: Option<String>,
    #[merge(strategy = option_override)]
    pub moderation: Option<String>,
    #[merge(strategy = option_override)]
    pub provider_options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Patch overlay for hosted URL context of a provider.
pub struct ProviderHostedUrlContextOverlay {
    #[merge(strategy = option_override)]
    pub max_urls: Option<u32>,
    #[merge(strategy = option_override)]
    pub provider_options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Patch overlay for hosted tools of a provider.
pub struct ProviderHostedToolsOverlay {
    #[merge(strategy = option_struct_merge)]
    pub web_search: Option<ProviderHostedWebSearchOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub file_search: Option<ProviderHostedFileSearchOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub code_execution: Option<ProviderHostedCodeExecutionOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub image_generation: Option<ProviderHostedImageGenerationOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub url_context: Option<ProviderHostedUrlContextOverlay>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Patch overlay referencing a native tool harness.
pub struct ProviderNativeToolHarnessRefOverlay {
    #[merge(strategy = option_override)]
    pub kind: Option<ProviderNativeToolHarnessKind>,
    #[merge(strategy = option_override)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Patch overlay binding native tool harnesses.
pub struct ProviderNativeToolHarnessBindingsOverlay {
    #[merge(strategy = option_struct_merge)]
    pub computer: Option<ProviderNativeToolHarnessRefOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub bash: Option<ProviderNativeToolHarnessRefOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub text_editor: Option<ProviderNativeToolHarnessRefOverlay>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Patch overlay for a native tool connector.
pub struct ProviderNativeToolConnectorOverlay {
    #[merge(strategy = option_override)]
    pub server: Option<String>,
    #[merge(strategy = option_override)]
    pub require_approval: Option<bool>,
    #[merge(strategy = option_override)]
    pub tool_filter: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Patch overlay for provider native tools.
pub struct ProviderNativeToolsOverlay {
    #[merge(strategy = option_struct_merge)]
    pub routes: Option<ProviderNativeToolRoutesOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub hosted: Option<ProviderHostedToolsOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub harness: Option<ProviderNativeToolHarnessBindingsOverlay>,
    #[merge(strategy = map_extend)]
    pub connectors: BTreeMap<String, ProviderNativeToolConnectorOverlay>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Full patch overlay applied on top of a provider config.
pub struct ProviderOverlay {
    #[merge(strategy = option_override)]
    pub enabled: Option<bool>,
    #[merge(strategy = option_struct_merge)]
    pub defaults: Option<ProviderDefaultsOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub auth: Option<ProviderAuthOverlay>,
    #[merge(strategy = option_struct_merge)]
    pub network: Option<ProviderNetworkOverlay>,
    #[merge(strategy = map_extend)]
    pub adapters: BTreeMap<String, ProviderAdapterOverlay>,
}

fn option_override<T>(base: &mut Option<T>, overlay: Option<T>) {
    if let Some(value) = overlay {
        *base = Some(value);
    }
}

fn option_struct_merge<T>(base: &mut Option<T>, overlay: Option<T>)
where
    T: merge::Merge,
{
    match (base, overlay) {
        (Some(base), Some(overlay)) => <T as merge::Merge>::merge(base, overlay),
        (slot @ None, Some(overlay)) => *slot = Some(overlay),
        _ => {}
    }
}

fn map_extend<K, V>(base: &mut BTreeMap<K, V>, overlay: BTreeMap<K, V>)
where
    K: Ord,
{
    base.extend(overlay);
}

/// Build the default enabled provider-model patch from catalog data.
pub fn provider_model_overlay_from_catalog_definition(
    definition: &CatalogModelDefinition,
) -> ResolvedProviderModelConfig {
    let mut configured = crate::catalog_definition_to_provider_definition(definition);
    configured.capabilities = configured.capabilities.normalized_resolved_patch();
    provider_model_overlay_from_definition(configured)
}

/// Build the default enabled provider-model patch from a configured definition.
pub fn provider_model_overlay_from_definition(
    definition: ConfiguredModelDefinition,
) -> ResolvedProviderModelConfig {
    let mode = if definition
        .capabilities
        .feature_support(crate::ModelCapabilityFeature::ToolCalling)
        == Some(agena_domain::CapabilitySupport::Supported)
    {
        crate::AgenaToolMode::ProviderProtocol
    } else {
        crate::AgenaToolMode::Disabled
    };
    ResolvedProviderModelConfig {
        enabled: true,
        native_compaction: true,
        agena_tools: crate::AgenaToolsConfig {
            mode,
            provider_native: Default::default(),
        },
        definition,
    }
}
