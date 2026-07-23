//! Provider-facing ports shared by application services and concrete adapters.
//!
//! This crate deliberately owns contracts only. Provider SDK clients, runtime
//! composition, configuration parsing, and catalog decoration remain in their
//! respective concrete layers until each has a dedicated migration slice.

use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use agena_domain::{
    AdapterId, CapabilitySupport, Model, ModelCapabilities, ModelId, ModelInputModality,
    ModelLifecycle, ModelMetadata, ModelPricing, ModelRef, ModelSpeedMode,
    ModelSpeedModeRequestOverride, ModelThinkingMode, ProviderId, ReasoningEffort, ThinkingDisplay,
    ThinkingRequest,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

mod auth_values;
pub use auth_values::{
    AuthData, CopilotDeployment, CredentialIssuer, OAuthTokenResponse, OAuthUserInfo,
};
mod config_patch_values;
pub use config_patch_values::{
    HostedCodeExecutionContainerOverlay, OpenAiResponsesBackendConfig, ProviderAdapterOverlay,
    ProviderApiSubtype, ProviderAuthMode, ProviderAuthOverlay, ProviderCapabilityFamilyConfig,
    ProviderDefaultsOverlay, ProviderGitlabApiAccessOverlay, ProviderHostedCodeExecutionOverlay,
    ProviderHostedFileSearchOverlay, ProviderHostedImageGenerationOverlay,
    ProviderHostedToolsOverlay, ProviderHostedUrlContextOverlay, ProviderHostedWebSearchOverlay,
    ProviderNativeToolConnectorOverlay, ProviderNativeToolHarnessBindingsOverlay,
    ProviderNativeToolHarnessRefOverlay, ProviderNativeToolRoutesOverlay,
    ProviderNativeToolUserLocationOverlay, ProviderNativeToolsOverlay, ProviderNetworkOverlay,
    ProviderOverlay, ProviderProtocolPathsOverlay, ProviderSecretSourceOverlay,
    StreamTransportMode, provider_model_overlay_from_catalog_definition,
    provider_model_overlay_from_definition,
};
mod copilot_models;
pub use copilot_models::CopilotModelExtension;
mod bedrock_auth;
pub use bedrock_auth::BedrockSigv4AuthConfig;
mod http_utils;
pub use http_utils::{
    auth_header_value, ensure_header_case_insensitive, insert_header_case_insensitive,
    merge_json_object_patch_map, merged_request_headers, normalize_base_url,
    normalize_optional_text, optional_non_empty, prompt_cache_header_entries,
    prompt_cache_ignores_header, request_shape_fingerprint,
};
mod usage_cost;
pub use usage_cost::{
    CompletionUsageCostContribution, completion_usage_cost_contribution,
    estimate_completion_usage_cost_usd,
};
mod anthropic_wire_text;
pub use anthropic_wire_text::{AnthropicBinarySource, AnthropicTextBlock};
mod anthropic_wire;
pub use anthropic_wire::{
    AnthropicCacheCreationUsage, AnthropicMessage, AnthropicMessagesRequest,
    AnthropicMessagesResponse, AnthropicModel, AnthropicModelListResponse, AnthropicOutputConfig,
    AnthropicOutputTokensDetails, AnthropicSseContentBlock, AnthropicSseDelta, AnthropicSseEvent,
    AnthropicSseMessage, AnthropicSseMessageDelta, AnthropicToolCallState, AnthropicUsage,
};
mod anthropic_thinking;
pub use anthropic_thinking::{
    AnthropicThinkingBlockState, AnthropicThinkingParts, anthropic_adaptive_parts,
    anthropic_budget_for_effort, anthropic_default_display, anthropic_effort_for_budget,
    anthropic_enabled_parts, anthropic_model_defaults_to_omitted_thinking,
    anthropic_model_rejects_disabled_thinking, anthropic_model_rejects_sampling,
    anthropic_model_requires_adaptive_thinking, anthropic_model_supports_adaptive_thinking,
    anthropic_model_supports_effort, anthropic_model_supports_max_effort,
    anthropic_model_supports_xhigh_effort, anthropic_thinking_metadata, anthropic_thinking_parts,
    anthropic_wire_tool_name, json_value_to_string, map_anthropic_usage,
    merge_anthropic_cache_creation_usage, merge_anthropic_usage,
};
mod gemini_thinking;
pub use gemini_thinking::{GeminiThinkingConfig, gemini_thinking_config};
mod gemini_usage;
pub use gemini_usage::{GeminiUsageMetadata, gemini_usage_to_completion};
mod gemini_models;
pub use gemini_models::{GeminiModel, GeminiModelListResponse};
mod gemini_content_wire;
pub use gemini_content_wire::{
    GeminiContent, GeminiFunctionCall, GeminiFunctionResponse, GeminiInlineData, GeminiPart,
};
mod gemini_request_wire;
pub use gemini_request_wire::{
    GeminiFunctionCallingConfig, GeminiFunctionDeclaration, GeminiGenerateRequest,
    GeminiGenerationConfig, GeminiInstruction, GeminiLiveClientContent,
    GeminiLiveConversationRequest, GeminiLiveSetup, GeminiToolConfig,
};
mod gemini_response_wire;
pub use gemini_response_wire::{GeminiCandidate, GeminiGenerateResponse};
mod gemini_live_response_wire;
pub use gemini_live_response_wire::{
    GeminiLiveServerContent, GeminiLiveServerMessage, GeminiLiveToolCall,
};
mod ollama_wire;
pub use ollama_wire::{
    OllamaChatMessage, OllamaChatMessageResponse, OllamaChatRequest, OllamaChatResponse,
    OllamaFunctionCall, OllamaFunctionDefinition, OllamaModelDetails, OllamaOptions,
    OllamaTagModel, OllamaTagsResponse, OllamaToolCall, OllamaToolDefinition,
};
mod ollama_usage;
pub use ollama_usage::ollama_usage_to_completion;

mod prompt_cache_shape;
pub use prompt_cache_shape::{PromptCacheShape, PromptCacheShapeChange, PromptCacheShapeDiff};
mod prompt_cache_control;
pub use prompt_cache_control::{PromptCacheControl, select_cache_target_indices};
mod protocol_ids;
pub use protocol_ids::{
    ModelToolCallId, ProviderItemId, ProviderStreamKey, openai_responses_call_id,
    valid_openai_responses_call_id,
};
mod tool_stream;
pub use tool_stream::{
    ToolStreamAccumulator, ToolStreamError, ToolStreamInput, ToolStreamInputKind, ToolStreamUpdate,
};
mod tool_mode_policy;
pub use tool_mode_policy::{
    ProviderToolModeViolation, apply_configured_tool_request, prepare_disabled_tool_request,
    project_disabled_completion_input_history, strip_provider_native_tool_body_fields,
    validate_disabled_tool_response,
};
mod prompt_tool_envelope;
pub use prompt_tool_envelope::{
    PromptToolCall, PromptToolCallsEnvelope, PromptToolDefinition, PromptToolResult,
};
mod prompt_tool_decoder;
pub use prompt_tool_decoder::{
    PromptToolDecodedItem, PromptToolTextDecoder, decode_prompt_tool_calls,
};
mod wire_values;
pub use wire_values::{
    ChatStreamChoice, ChatStreamChunk, ChatStreamDelta, ResponsesToolEvent, ResponsesToolEventKind,
};
mod model_metadata;
pub use model_metadata::{ModelMetadataRegistry, default_model_metadata_registry};
mod capabilities;
pub use capabilities::{CapabilityRegistry, default_capability_registry};
mod model_modes;
pub use model_modes::{ModelModeRegistry, default_model_mode_registry};
mod configured_models;
pub use configured_models::{
    apply_configured_modes, apply_configured_thinking_modes, configured_thinking_mode_selector,
    configured_thinking_mode_to_model,
};
mod configured_model_config;
pub use configured_model_config::ResolvedProviderModelConfig;
mod credential_config;
pub use credential_config::{
    ProviderCredentialAuthConfig, ProviderGitlabCredentialAuthConfig,
    ProviderHttpCredentialAuthConfig, ProviderInlineCredentialAuthConfig,
    ProviderSapAiCoreCredentialAuthConfig,
};
mod network_config;
pub use network_config::{
    DEFAULT_PROVIDER_CONNECT_TIMEOUT_SECS, DEFAULT_PROVIDER_REQUEST_TIMEOUT_SECS,
    ProviderNetworkConfig,
};
mod openai_responses_wire;
pub use openai_responses_wire::{
    OpenAiIncompleteDetails, OpenAiInputTokenDetails, OpenAiOutputContent, OpenAiOutputItem,
    OpenAiOutputTokenDetails, OpenAiReasoningSummaryContent, OpenAiResponsesResponse, OpenAiUsage,
    openai_responses_reasoning_delta,
};
mod openai_chat_usage;
pub use openai_chat_usage::{
    ChatInputTokensDetails, ChatOutputTokensDetails, ChatUsage, chat_usage_to_completion,
};
mod openai_chat_response_format;
pub use openai_chat_response_format::{
    ChatJsonSchemaSpec, ChatResponseFormat, openai_chat_response_format,
};
mod openai_chat_reasoning;
pub use openai_chat_reasoning::{
    openai_chat_reasoning_effort, openai_chat_supports_reasoning_effort,
};
mod openai_chat_response_wire;
pub use openai_chat_response_wire::{
    ChatCompletionChoice, ChatCompletionResponse, ChatDeltaOrMessage, ChatFunctionCallWire,
    ChatToolCallWire,
};
mod openai_chat_tool_definition;
pub use openai_chat_tool_definition::{ChatFunctionDefinition, ChatToolDefinition};
mod openai_chat_stream_options;
pub use openai_chat_stream_options::ChatStreamOptions;
mod openai_chat_tool_call_request;
pub use openai_chat_tool_call_request::{ChatFunctionCallRequest, ChatToolCallRequest};
mod openai_chat_message;
pub use openai_chat_message::ChatMessage;
mod openai_chat_completion_request;
pub use openai_chat_completion_request::ChatCompletionRequest;
mod openai_chat_text;
pub use openai_chat_text::openai_chat_extract_text;
mod openai_chat_reasoning_text;
pub use openai_chat_reasoning_text::{
    openai_chat_extract_reasoning_text, openai_chat_reasoning_field,
    openai_chat_reasoning_field_from_delta,
};
mod openai_chat_reasoning_details;
pub use openai_chat_reasoning_details::merge_openai_chat_reasoning_details;
mod route_config;
pub use route_config::{
    CLINE_API_BASE_URL, CLINE_API_OPENAI_PROTOCOL_PATH, ProviderModelDiscoveryConfig,
    ProviderProtocolPathsConfig, cline_api_protocol_paths,
};
mod secret_config;
pub use secret_config::{ProviderGitlabApiAccessConfig, ProviderSecretSourceConfig};
mod catalog_definition;
pub use catalog_definition::{
    CatalogDefinitionSourcePriority, CatalogModelDefinition, ModelCatalogDocument,
    ModelCatalogProviderRecord, ModelCatalogResponse, ModelCatalogSnapshot,
};
mod catalog_model_id;
pub use catalog_model_id::{catalog_model_id_for_raw, normalized_catalog_model_id};
mod catalog_projection;
pub use catalog_projection::{capability_patch_from_model, catalog_definition_from_model};
mod catalog_merge;
pub use catalog_merge::{
    merge_capability_patch, merge_catalog_definition, merge_json_patch_maps_fill_missing,
    merge_json_value_fill_missing, merge_live_provider_catalog_document, merge_model_pricing,
    merge_selection_patch, merge_speed_mode_request_override_fill_missing, merge_unique,
};
mod catalog_public_merge;
pub use catalog_public_merge::{
    merge_public_source_catalog_definition, merge_public_source_catalog_document,
};
mod catalog_thinking_modes;
pub use catalog_thinking_modes::{
    catalog_thinking_mode_for_effort, enrich_catalog_document_thinking_modes,
    inferred_catalog_thinking_modes, insert_catalog_thinking_effort,
    openai_catalog_reasoning_efforts,
};
mod catalog_collector;
pub use catalog_collector::collect_live_provider_models;
mod catalog_decoration;
pub use catalog_decoration::{
    apply_catalog_definition_as_baseline, apply_configured_definition_as_baseline,
    catalog_definition_to_provider_definition, merge_catalog_baseline_speed_modes,
    merge_catalog_baseline_thinking_modes,
};
mod catalog_model_decoration;
pub use catalog_model_decoration::{CatalogModelDecorationSource, decorate_provider_models};

/// Selects how Agena tools are exposed to a provider model.
///
/// This is a provider-facing request policy. Core configuration owns the
/// surrounding provider settings, but the runtime adapters consume this value
/// directly from the provider contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgenaToolMode {
    ProviderProtocol,
    PromptEnvelope,
    #[default]
    Disabled,
}

impl AgenaToolMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ProviderProtocol => "provider_protocol",
            Self::PromptEnvelope => "prompt_envelope",
            Self::Disabled => "disabled",
        }
    }

    pub const fn is_provider_protocol(&self) -> bool {
        matches!(self, Self::ProviderProtocol)
    }

    pub const fn is_prompt_envelope(&self) -> bool {
        matches!(self, Self::PromptEnvelope)
    }

    pub const fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }
}

/// Provider-facing tool exposure configuration for one model route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AgenaToolsConfig {
    #[serde(default)]
    pub mode: AgenaToolMode,
    #[serde(default, skip_serializing_if = "ProviderNativeToolsConfig::is_empty")]
    pub provider_native: ProviderNativeToolsConfig,
}

impl AgenaToolsConfig {
    pub fn is_default(&self) -> bool {
        self.mode.is_disabled() && self.provider_native.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderCatalogError {
    #[error("provider catalog request is invalid: {0}")]
    InvalidRequest(String),
    #[error("provider catalog entry was not found: {0}")]
    NotFound(String),
    #[error("provider catalog operation failed: {0}")]
    Operation(String),
}

/// Classifies provider failures that affect retry and recovery policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    ApiError,
    ContextOverflow,
}

/// Persistence provenance for a provider model-catalog snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogSnapshotSourceKind {
    Generated,
    Cache,
}

impl ModelCatalogSnapshotSourceKind {
    /// Stable persistence tag used by storage adapters.
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Cache => "cache",
        }
    }

    /// Parses the stable persistence tag without depending on a concrete
    /// database or application error type.
    pub fn from_persisted(value: &str) -> Result<Self, String> {
        match value {
            "generated" => Ok(Self::Generated),
            "cache" => Ok(Self::Cache),
            other => Err(format!("invalid model catalog cache source `{other}`")),
        }
    }
}

/// Stable key identifying one configured provider adapter/model route.
pub type ProviderModelRouteKey = (String, String);

/// Runtime model-catalog data supplied to provider adapters.
///
/// Persistence formats, source-ranking metadata, and refresh timing policy
/// remain outside this crate; those are concrete catalog/runtime concerns.
/// This value contains only the resolved provider-facing model definitions
/// needed while listing and enriching models at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderModelCatalog {
    pub models: BTreeMap<String, ConfiguredModelDefinition>,
    pub appendable_model_ids: std::collections::BTreeSet<String>,
}

/// API-facing model catalog record independent of core persistence and ranking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CatalogModelRecord {
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_temperature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_p: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_interleaved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "ConfiguredModelModeMap::is_empty")]
    pub thinking_modes: ConfiguredModelModeMap<ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "ConfiguredModelModeMap::is_empty")]
    pub speed_modes: ConfiguredModelModeMap<ConfiguredModelSpeedMode>,
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatch,
}

/// Provider-specific ordering hints used when composing live model catalogs.
/// The ranking policy is concrete-layer-owned; this value keeps the catalog
/// service independent of provider configuration structs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderModelPriorities {
    values: BTreeMap<String, i32>,
}

impl ProviderModelPriorities {
    pub fn new(values: BTreeMap<String, i32>) -> Self {
        Self { values }
    }

    pub fn get(&self, provider_id: &str) -> i32 {
        self.values.get(provider_id).copied().unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Version values used when provider adapters identify their client protocol.
/// Fetching and configuration persistence remain concrete runtime concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderClientVersions {
    pub codex: String,
    pub claude: String,
    pub gemini: String,
}

/// Transport-neutral provider HTTP timeout settings.
/// Concrete client construction remains in the runtime/infrastructure layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderHttpClientConfig {
    pub timeout: Duration,
    pub connect_timeout: Duration,
}

/// Transport selected for Gemini streaming requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiStreamMode {
    Sse,
    RealtimeWebSocket,
}

/// Backend protocol selected for OpenAI Responses adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiResponsesBackend {
    Api,
    ChatgptCodex,
}

/// Provider identity profile used by OpenAI-compatible adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiProfile {
    Standard,
    GithubCopilot,
}

/// Provider identity profile used by Anthropic-compatible adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicProfile {
    Standard,
    GithubCopilot,
}

/// Which credential field a provider adapter should prefer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSecretSelector {
    AccessOrApiKey,
    RefreshOrAccess,
}

/// Provider credential refresh policy selected by runtime composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthRefreshStrategy {
    None,
    ReloadFromStore,
    OpenAiOAuth,
    GitlabOAuth { instance_url: String },
}

/// SAP AI Core service-key payload supplied to provider authentication.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SapAiCoreServiceKey {
    pub clientid: String,
    pub clientsecret: String,
    pub url: String,
    pub serviceurls: SapAiCoreServiceUrls,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SapAiCoreServiceUrls {
    #[serde(rename = "AI_API_URL")]
    pub ai_api_url: String,
}

/// Parsed OAuth redirect result returned by provider authorization flows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallback {
    pub code: String,
    pub state: String,
}

/// Provider-facing GitLab routing and feature configuration.
#[derive(Debug, Clone)]
pub struct GitlabProviderConfig {
    pub instance_url: String,
    pub ai_gateway_url: String,
    pub default_model: String,
    pub ai_gateway_headers: HashMap<String, String>,
    pub feature_flags: HashMap<String, bool>,
}

impl Default for ProviderHttpClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            connect_timeout: Duration::from_secs(15),
        }
    }
}

impl Default for ProviderClientVersions {
    fn default() -> Self {
        Self {
            codex: "0.144.4".to_owned(),
            claude: "2.1.209".to_owned(),
            gemini: "0.50.0".to_owned(),
        }
    }
}

/// Browser OAuth authorization data returned by a concrete provider adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthAuthorizeStart {
    pub authorize_url: String,
    pub state: String,
    pub pkce_verifier: String,
}

/// Device-code authorization data returned by a concrete provider adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeStart {
    pub verification_url: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_seconds: u64,
}

/// Provider-normalized reason for a completed generation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "raw", rename_all = "snake_case")]
pub enum CompletionFinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other(String),
}

/// The provider protocol family used for capability and model-mode resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFamily {
    OpenAi,
    OpenAiCompatible,
    Anthropic,
    Gemini,
    Bedrock,
    Gitlab,
}

/// Provider-facing capability lookup contract. Concrete rule tables and
/// runtime/configuration composition remain outside this contract crate.
pub trait CapabilityResolver: Send + Sync {
    fn capabilities_for_family(
        &self,
        family: CapabilityFamily,
        model: &str,
    ) -> agena_domain::ModelCapabilities;
}

/// Provider-facing model-thinking-mode lookup contract.
pub trait ModelModeResolver: Send + Sync {
    fn thinking_modes_for_family(
        &self,
        family: CapabilityFamily,
        adapter_id: Option<&AdapterId>,
        model: &str,
        metadata: &ModelMetadata,
    ) -> Vec<ModelThinkingMode>;
}

/// Provider-facing capability dimensions used by model catalog/configuration values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapabilityFeature {
    ToolCalling,
    Streaming,
    Reasoning,
    StructuredOutput,
    #[serde(rename = "temperature")]
    Temperature,
}

impl AsRef<str> for ModelCapabilityFeature {
    fn as_ref(&self) -> &str {
        match self {
            Self::ToolCalling => "tool_calling",
            Self::Streaming => "streaming",
            Self::Reasoning => "reasoning",
            Self::StructuredOutput => "structured_output",
            Self::Temperature => "temperature",
        }
    }
}

impl std::fmt::Display for ModelCapabilityFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct CapabilitySelectionPatchBody<T> {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported: Vec<T>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<T>,
}

impl<T> CapabilitySelectionPatchBody<T> {
    pub fn is_empty(&self) -> bool {
        self.supported.is_empty() && self.unsupported.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub enum CapabilitySelectionPatch<T> {
    Supported(Vec<T>),
    Patch(CapabilitySelectionPatchBody<T>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCapabilityPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<CapabilitySelectionPatch<agena_domain::ModelInputModality>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<CapabilitySelectionPatch<ModelCapabilityFeature>>,
}

impl ModelCapabilityPatch {
    pub fn is_empty(&self) -> bool {
        self.input
            .as_ref()
            .is_none_or(CapabilitySelectionPatch::is_empty)
            && self
                .features
                .as_ref()
                .is_none_or(CapabilitySelectionPatch::is_empty)
    }

    pub fn input_support(&self, modality: ModelInputModality) -> Option<CapabilitySupport> {
        self.input.as_ref().and_then(|selection| {
            if selection.supported().contains(&modality) {
                Some(CapabilitySupport::Supported)
            } else if selection.unsupported().contains(&modality) {
                Some(CapabilitySupport::Unsupported)
            } else {
                None
            }
        })
    }

    pub fn feature_support(&self, feature: ModelCapabilityFeature) -> Option<CapabilitySupport> {
        self.features.as_ref().and_then(|selection| {
            if selection.supported().contains(&feature) {
                Some(CapabilitySupport::Supported)
            } else if selection.unsupported().contains(&feature) {
                Some(CapabilitySupport::Unsupported)
            } else {
                None
            }
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_selection(self.input.as_ref(), |value| match value {
            ModelInputModality::Text => "text",
            ModelInputModality::Image => "image",
            ModelInputModality::Document => "document",
            ModelInputModality::Audio => "audio",
            ModelInputModality::Video => "video",
            ModelInputModality::File => "file",
        })?;
        validate_selection(self.features.as_ref(), |value| match value {
            ModelCapabilityFeature::ToolCalling => "tool_calling",
            ModelCapabilityFeature::Streaming => "streaming",
            ModelCapabilityFeature::Reasoning => "reasoning",
            ModelCapabilityFeature::StructuredOutput => "structured_output",
            ModelCapabilityFeature::Temperature => "temperature",
        })
    }

    pub fn normalize_compact_patch(&mut self) {
        *self = self.normalized_resolved_patch();
    }

    pub fn normalized_resolved_patch(&self) -> Self {
        let inputs = [
            ModelInputModality::Text,
            ModelInputModality::Image,
            ModelInputModality::Document,
            ModelInputModality::Audio,
            ModelInputModality::Video,
            ModelInputModality::File,
        ];
        let features = [
            ModelCapabilityFeature::ToolCalling,
            ModelCapabilityFeature::Streaming,
            ModelCapabilityFeature::Reasoning,
            ModelCapabilityFeature::StructuredOutput,
            ModelCapabilityFeature::Temperature,
        ];
        Self {
            input: compact_selection(
                inputs
                    .into_iter()
                    .map(|value| (value, self.input_support(value))),
            ),
            features: compact_selection(
                features
                    .into_iter()
                    .map(|value| (value, self.feature_support(value))),
            ),
        }
    }

    pub fn apply_to(&self, mut capabilities: ModelCapabilities) -> ModelCapabilities {
        if let Some(selection) = &self.input {
            for value in selection.supported() {
                set_input(&mut capabilities, *value, CapabilitySupport::Supported);
            }
            for value in selection.unsupported() {
                set_input(&mut capabilities, *value, CapabilitySupport::Unsupported);
            }
        }
        if let Some(selection) = &self.features {
            for value in selection.supported() {
                set_feature(&mut capabilities, *value, CapabilitySupport::Supported);
            }
            for value in selection.unsupported() {
                set_feature(&mut capabilities, *value, CapabilitySupport::Unsupported);
            }
        }
        capabilities
    }
}

fn validate_selection<T>(
    selection: Option<&CapabilitySelectionPatch<T>>,
    name: impl Fn(&T) -> &'static str,
) -> Result<(), String> {
    let Some(selection) = selection else {
        return Ok(());
    };
    let supported = selection
        .supported()
        .iter()
        .map(&name)
        .collect::<std::collections::BTreeSet<_>>();
    if supported.len() != selection.supported().len() {
        return Err("capability listed more than once".to_owned());
    }
    let unsupported = selection
        .unsupported()
        .iter()
        .map(name)
        .collect::<std::collections::BTreeSet<_>>();
    if unsupported.len() != selection.unsupported().len() {
        return Err("capability listed more than once".to_owned());
    }
    if supported.intersection(&unsupported).next().is_some() {
        return Err("capability cannot be both supported and unsupported".to_owned());
    }
    Ok(())
}

fn compact_selection<T>(
    values: impl Iterator<Item = (T, Option<CapabilitySupport>)>,
) -> Option<CapabilitySelectionPatch<T>> {
    let mut supported = Vec::new();
    let mut unsupported = Vec::new();
    for (value, support) in values {
        match support {
            Some(CapabilitySupport::Supported) => supported.push(value),
            Some(CapabilitySupport::Unsupported) => unsupported.push(value),
            _ => {}
        }
    }
    CapabilitySelectionPatch::optional_from_supported_unsupported(supported, unsupported)
}

fn set_input(
    capabilities: &mut ModelCapabilities,
    modality: ModelInputModality,
    support: CapabilitySupport,
) {
    match modality {
        ModelInputModality::Text => capabilities.text_input = support,
        ModelInputModality::Image => capabilities.image_input = support,
        ModelInputModality::Document => capabilities.document_input = support,
        ModelInputModality::Audio => capabilities.audio_input = support,
        ModelInputModality::Video => capabilities.video_input = support,
        ModelInputModality::File => capabilities.file_input = support,
    }
}

fn set_feature(
    capabilities: &mut ModelCapabilities,
    feature: ModelCapabilityFeature,
    support: CapabilitySupport,
) {
    match feature {
        ModelCapabilityFeature::ToolCalling => capabilities.tool_calling = support,
        ModelCapabilityFeature::Streaming => capabilities.streaming = support,
        ModelCapabilityFeature::Reasoning => capabilities.reasoning = support,
        ModelCapabilityFeature::StructuredOutput => capabilities.structured_output = support,
        ModelCapabilityFeature::Temperature => capabilities.temperature_supported = support,
    }
}

impl<T> CapabilitySelectionPatch<T> {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Supported(values) => values.is_empty(),
            Self::Patch(values) => values.is_empty(),
        }
    }

    pub fn supported(&self) -> &[T] {
        match self {
            Self::Supported(values) => values,
            Self::Patch(values) => &values.supported,
        }
    }

    pub fn unsupported(&self) -> &[T] {
        match self {
            Self::Supported(_) => &[],
            Self::Patch(values) => &values.unsupported,
        }
    }

    pub fn from_supported_unsupported(supported: Vec<T>, unsupported: Vec<T>) -> Self {
        if unsupported.is_empty() {
            Self::Supported(supported)
        } else {
            Self::Patch(CapabilitySelectionPatchBody {
                supported,
                unsupported,
            })
        }
    }

    pub fn optional_from_supported_unsupported(
        supported: Vec<T>,
        unsupported: Vec<T>,
    ) -> Option<Self> {
        (!supported.is_empty() || !unsupported.is_empty())
            .then(|| Self::from_supported_unsupported(supported, unsupported))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConfiguredModeDefault {
    #[default]
    Inherit,
    Clear,
    Mode(String),
}

impl ConfiguredModeDefault {
    fn is_inherit(&self) -> bool {
        matches!(self, Self::Inherit)
    }

    pub fn mode(&self) -> Option<&str> {
        match self {
            Self::Mode(value) => Some(value),
            _ => None,
        }
    }
}

fn deserialize_mode_default<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ConfiguredModeDefault, D::Error> {
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(value) => ConfiguredModeDefault::Mode(value),
        None => ConfiguredModeDefault::Clear,
    })
}

fn serialize_mode_default<S: serde::Serializer>(
    value: &ConfiguredModeDefault,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        ConfiguredModeDefault::Inherit | ConfiguredModeDefault::Clear => {
            serializer.serialize_none()
        }
        ConfiguredModeDefault::Mode(value) => serializer.serialize_str(value),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct ConfiguredModelModeMap<T> {
    #[serde(
        default,
        deserialize_with = "deserialize_mode_default",
        serialize_with = "serialize_mode_default",
        skip_serializing_if = "ConfiguredModeDefault::is_inherit"
    )]
    pub default: ConfiguredModeDefault,
    #[serde(flatten)]
    pub modes: BTreeMap<String, T>,
}

impl<T> ConfiguredModelModeMap<T> {
    pub fn is_empty(&self) -> bool {
        self.default.is_inherit() && self.modes.is_empty()
    }
}

impl<T> std::ops::Deref for ConfiguredModelModeMap<T> {
    type Target = BTreeMap<String, T>;
    fn deref(&self) -> &Self::Target {
        &self.modes
    }
}

impl<T> std::ops::DerefMut for ConfiguredModelModeMap<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.modes
    }
}

impl<T> From<BTreeMap<String, T>> for ConfiguredModelModeMap<T> {
    fn from(modes: BTreeMap<String, T>) -> Self {
        Self {
            default: ConfiguredModeDefault::Inherit,
            modes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredModelSpeedMode {
    #[serde(skip)]
    pub is_default: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty"
    )]
    pub request_override: ModelSpeedModeRequestOverride,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ModelSpeedModeRequestOverride>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredModelThinkingMode {
    #[serde(skip)]
    pub is_default: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip)]
    pub preset: Option<String>,
    #[serde(skip)]
    pub thinking: Option<ThinkingRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<ConfiguredThinkingStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ThinkingDisplay>,
    #[serde(
        default,
        skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty"
    )]
    pub request_override: ModelSpeedModeRequestOverride,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ModelSpeedModeRequestOverride>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConfiguredModelDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<agena_domain::ModelLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_temperature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_p: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_interleaved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<agena_domain::ModelPricing>,
    #[serde(default, skip_serializing_if = "ConfiguredModelModeMap::is_empty")]
    pub thinking_modes: ConfiguredModelModeMap<ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "ConfiguredModelModeMap::is_empty")]
    pub speed_modes: ConfiguredModelModeMap<ConfiguredModelSpeedMode>,
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatch,
}

impl ConfiguredModelDefinition {
    pub fn is_empty(&self) -> bool {
        self.lifecycle.is_none()
            && self.context_window_tokens.is_none()
            && self.max_input_tokens.is_none()
            && self.max_output_tokens.is_none()
            && self.display_name.is_none()
            && self.description.is_none()
            && self.knowledge_cutoff.is_none()
            && self.release_date.is_none()
            && self.last_updated.is_none()
            && self.open_weights.is_none()
            && self.supports_parallel_tool_calls.is_none()
            && self.supports_verbosity.is_none()
            && self.default_verbosity.is_none()
            && self.default_temperature.is_none()
            && self.default_top_p.is_none()
            && self.default_top_k.is_none()
            && self.assistant_reasoning_interleaved.is_none()
            && self.assistant_reasoning_field.is_none()
            && self.output_modalities.is_empty()
            && self.pricing.is_none()
            && self.thinking_modes.is_empty()
            && self.speed_modes.is_empty()
            && self.capabilities.is_empty()
    }

    pub fn metadata(&self) -> ModelMetadata {
        ModelMetadata {
            lifecycle: self.lifecycle,
            limits: agena_domain::ModelTokenLimits {
                context_window_tokens: self.context_window_tokens,
                max_input_tokens: self.max_input_tokens,
                max_output_tokens: self.max_output_tokens,
            },
            description: self.description.clone(),
            knowledge_cutoff: self.knowledge_cutoff.clone(),
            release_date: self.release_date.clone(),
            last_updated: self.last_updated.clone(),
            open_weights: self.open_weights,
            supports_parallel_tool_calls: self.supports_parallel_tool_calls,
            supports_verbosity: self.supports_verbosity,
            default_verbosity: self.default_verbosity.clone(),
            default_temperature: self.default_temperature.clone(),
            default_top_p: self.default_top_p.clone(),
            default_top_k: self.default_top_k,
            assistant_reasoning_interleaved: self.assistant_reasoning_interleaved,
            assistant_reasoning_field: self.assistant_reasoning_field.clone(),
            output_modalities: self.output_modalities.clone(),
            pricing: self.pricing.clone(),
        }
    }

    pub fn apply_to_model(
        &self,
        mut model: Model,
        capability_fallback: &ModelCapabilities,
        metadata_fallback: &ModelMetadata,
    ) -> Model {
        if let Some(display_name) = &self.display_name {
            model.display_name = Some(display_name.clone());
        }
        model.capabilities = self.capabilities.apply_to(
            model
                .capabilities
                .clone()
                .merged_with_fallbacks_from(capability_fallback),
        );
        model.metadata = self.metadata().merged_with_fallbacks_from(
            &model
                .metadata
                .clone()
                .merged_with_fallbacks_from(metadata_fallback),
        );
        model.thinking_modes = configured_models::apply_configured_thinking_modes(
            model.thinking_modes,
            &self.thinking_modes,
        );
        model.speed_modes = apply_configured_speed_modes(model.speed_modes, &self.speed_modes);
        model
    }
}

fn apply_configured_speed_modes(
    mut modes: BTreeMap<String, ModelSpeedMode>,
    configured: &ConfiguredModelModeMap<ConfiguredModelSpeedMode>,
) -> BTreeMap<String, ModelSpeedMode> {
    for (name, patch) in configured.iter() {
        match patch.apply_to_mode(modes.get(name)) {
            Some(mode) => {
                modes.insert(name.clone(), mode);
            }
            None => {
                modes.remove(name);
            }
        }
    }
    if let Some(default_name) = configured.default.mode() {
        for (name, mode) in &mut modes {
            mode.is_default = name == default_name;
        }
    }
    modes
}

impl ConfiguredModelThinkingMode {
    pub fn is_empty(&self) -> bool {
        self.is_default.is_none()
            && self.display_name.is_none()
            && self.description.is_none()
            && self.preset.is_none()
            && self.thinking.is_none()
            && self.strategy.is_none()
            && self.effort.is_none()
            && self.budget_tokens.is_none()
            && self.display.is_none()
            && self.request_override.is_empty()
            && self.adapter_overrides.is_empty()
            && !self.disabled
    }

    pub fn apply_to_mode(&self, base: Option<&ModelThinkingMode>) -> Option<ModelThinkingMode> {
        if self.disabled {
            return None;
        }
        let mut mode = base.cloned().unwrap_or_default();
        if let Some(value) = self.is_default {
            mode.is_default = value;
        }
        if let Some(value) = &self.display_name {
            mode.display_name = Some(value.clone());
        }
        if let Some(value) = &self.description {
            mode.description = Some(value.clone());
        }
        if let Some(value) = &self.preset {
            mode.preset = Some(value.clone());
        }
        if let Some(value) = &self.thinking {
            mode.thinking = Some(value.clone());
        }
        mode.request_override = mode.request_override.merged_with(&self.request_override);
        for (adapter, value) in &self.adapter_overrides {
            let merged = mode
                .adapter_overrides
                .get(adapter)
                .cloned()
                .unwrap_or_default()
                .merged_with(value);
            mode.adapter_overrides.insert(adapter.clone(), merged);
        }
        Some(mode)
    }
}

pub fn configured_thinking_payload_selector(mode: &ConfiguredModelThinkingMode) -> Option<String> {
    ModelThinkingMode {
        preset: mode.preset.clone(),
        thinking: mode.thinking.clone(),
        ..Default::default()
    }
    .selector()
    .map(|value| value.into_owned())
}

impl From<Vec<ConfiguredModelThinkingMode>>
    for ConfiguredModelModeMap<ConfiguredModelThinkingMode>
{
    fn from(values: Vec<ConfiguredModelThinkingMode>) -> Self {
        let mut result = Self::default();
        for mut mode in values {
            if let Some(name) = configured_thinking_payload_selector(&mode) {
                if mode.is_default == Some(true) {
                    result.default = ConfiguredModeDefault::Mode(name.clone());
                }
                match mode.thinking.take() {
                    Some(ThinkingRequest::Disabled) => {
                        mode.strategy = Some(ConfiguredThinkingStrategy::Disabled);
                    }
                    Some(ThinkingRequest::Effort { effort }) => {
                        mode.strategy = Some(ConfiguredThinkingStrategy::Effort);
                        mode.effort = Some(effort);
                    }
                    Some(ThinkingRequest::Budget { budget_tokens }) => {
                        mode.strategy = Some(ConfiguredThinkingStrategy::Budget);
                        mode.budget_tokens = Some(budget_tokens);
                    }
                    Some(ThinkingRequest::Adaptive { effort, display }) => {
                        mode.strategy = Some(ConfiguredThinkingStrategy::Adaptive);
                        mode.effort = effort;
                        mode.display = display;
                    }
                    None => {
                        mode.strategy
                            .get_or_insert(ConfiguredThinkingStrategy::RequestOnly);
                    }
                }
                mode.preset = None;
                result.modes.insert(name, mode);
            }
        }
        result
    }
}

impl ConfiguredModelSpeedMode {
    pub fn is_empty(&self) -> bool {
        self.is_default.is_none()
            && self.display_name.is_none()
            && self.description.is_none()
            && self.request_override.is_empty()
            && self.adapter_overrides.is_empty()
            && !self.disabled
    }
    pub fn apply_to_mode(&self, base: Option<&ModelSpeedMode>) -> Option<ModelSpeedMode> {
        if self.disabled {
            return None;
        }
        let mut mode = base.cloned().unwrap_or_default();
        if let Some(value) = self.is_default {
            mode.is_default = value;
        }
        if let Some(value) = &self.display_name {
            mode.display_name = Some(value.clone());
        }
        if let Some(value) = &self.description {
            mode.description = Some(value.clone());
        }
        mode.request_override = mode.request_override.merged_with(&self.request_override);
        for (adapter, value) in &self.adapter_overrides {
            let merged = mode
                .adapter_overrides
                .get(adapter)
                .cloned()
                .unwrap_or_default()
                .merged_with(value);
            mode.adapter_overrides.insert(adapter.clone(), merged);
        }
        Some(mode)
    }
}

/// Provider-facing strategy used to interpret a configured thinking mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfiguredThinkingStrategy {
    Disabled,
    Effort,
    Budget,
    Adaptive,
    RequestOnly,
}

impl CompletionFinishReason {
    pub fn from_provider(value: Option<impl AsRef<str>>) -> Option<Self> {
        let value = value?;
        let raw = value.as_ref().trim();
        if raw.is_empty() {
            return None;
        }

        Some(match raw.to_ascii_lowercase().replace('-', "_").as_str() {
            "stop" | "end_turn" | "message_stop" | "completed" => Self::Stop,
            "length" | "max_tokens" | "max_output_tokens" => Self::Length,
            "tool_calls" | "tool_use" | "function_call" => Self::ToolCalls,
            "content_filter" => Self::ContentFilter,
            _ => Self::Other(raw.to_owned()),
        })
    }
}

/// A provider-returned function call, independent from local execution tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionToolCall {
    Function {
        id: String,
        name: String,
        #[serde(default)]
        arguments_json: String,
    },
}

/// Provider-facing declaration of one fixed Agena Tool API function.
///
/// This deliberately contains no local registry handle. `handler_key` and
/// `plugin_name` are stable binding references that let session processing
/// associate a returned function call with its local tool contract; they do
/// not grant execution capability.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolApiDefinition {
    pub handler_key: String,
    pub plugin_name: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub strict: bool,
    pub definition_identity: String,
}

/// Normalized token and cost accounting for a provider completion.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CompletionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
}

impl CompletionUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.reasoning_tokens)
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn saturating_sub(&self, earlier: &Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_sub(earlier.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(earlier.output_tokens),
            reasoning_tokens: self
                .reasoning_tokens
                .saturating_sub(earlier.reasoning_tokens),
            cache_write_tokens: self
                .cache_write_tokens
                .saturating_sub(earlier.cache_write_tokens),
            cache_read_tokens: self
                .cache_read_tokens
                .saturating_sub(earlier.cache_read_tokens),
            total_cost: (self.total_cost - earlier.total_cost).max(0.0),
        }
    }
}

/// A provider-native artifact made available while a completion is streaming.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderNativeToolArtifact {
    pub uri: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// One normalized search result emitted by a provider-native tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderNativeToolSearchResult {
    pub title: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

/// Presentation-safe output emitted by a provider-native tool.
///
/// This intentionally represents provider output rather than core message
/// parts. Session orchestration maps it into its persisted presentation model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderNativeToolOutputBlock {
    Text {
        text: String,
    },
    SearchResults {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<ProviderNativeToolSearchResult>,
    },
    Media {
        mime_type: String,
        artifact: ProviderNativeToolArtifact,
    },
}

/// A normalized event in a streaming provider completion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionStreamEvent {
    TextDelta {
        provider_id: ProviderId,
        model: agena_domain::ModelId,
        delta: String,
    },
    ThinkingDelta {
        provider_id: ProviderId,
        model: agena_domain::ModelId,
        delta: String,
    },
    ToolCallDelta {
        provider_id: ProviderId,
        model: agena_domain::ModelId,
        stream_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default)]
        arguments_delta: String,
    },
    ToolCallSnapshot {
        provider_id: ProviderId,
        model: agena_domain::ModelId,
        stream_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default)]
        arguments_json: String,
    },
    ProviderNativeToolCallStarted {
        provider_id: ProviderId,
        model: agena_domain::ModelId,
        stream_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        invocation: agena_domain::ToolInvocation,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<serde_json::Value>,
    },
    ProviderNativeToolCallCompleted {
        provider_id: ProviderId,
        model: agena_domain::ModelId,
        stream_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        invocation: agena_domain::ToolInvocation,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        output_text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<ProviderNativeToolOutputBlock>,
        #[serde(default, skip_serializing_if = "agena_domain::ToolOutput::is_empty")]
        details: agena_domain::ToolOutput,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<serde_json::Value>,
    },
    Completed {
        provider_id: ProviderId,
        model: agena_domain::ModelId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish_reason: Option<CompletionFinishReason>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<CompletionUsage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<serde_json::Value>,
    },
}

/// Provider-ready attachment category, independent of core message storage.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompletionInputAttachmentKind {
    Image,
    Audio,
    Video,
    Pdf,
    File,
}

/// Provider-ready source for an input attachment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionInputAttachmentSource {
    Url { url: String },
    DataUrl { url: String },
    Base64 { data: String },
    FileId { id: String },
    LocalPath { path: String },
}

/// An attachment projected for a provider completion request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletionInputAttachment {
    pub kind: CompletionInputAttachmentKind,
    pub mime: String,
    pub source: CompletionInputAttachmentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
}

/// A flat provider-ready part of a conversation input message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionInputPart {
    Text {
        text: String,
    },
    /// Assistant reasoning preserved for providers that replay a dedicated
    /// reasoning field alongside visible content.
    Reasoning {
        text: String,
    },
    Attachment {
        attachment: CompletionInputAttachment,
    },
    ToolCall {
        id: String,
        function: agena_domain::ToolApiFunction,
        arguments_json: String,
    },
    ToolResult {
        tool_call_id: String,
        function: agena_domain::ToolApiFunction,
        #[serde(default)]
        arguments_json: String,
        #[serde(default)]
        status: CompletionInputToolResultStatus,
        output_json: String,
    },
}

/// Terminal state of a Tool API result replayed into a provider request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompletionInputToolResultStatus {
    #[default]
    Completed,
    Failed,
    Cancelled,
}

/// Provider-specific replay state associated with one prior assistant message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CompletionInputProviderState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<agena_domain::AssistantReasoningField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub gemini_thought_signatures: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anthropic_thinking_blocks: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openai_reasoning_items: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_chat_reasoning_details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_reasoning_opaque: Option<String>,
}

impl CompletionInputProviderState {
    pub fn is_empty(&self) -> bool {
        self.assistant_reasoning_field.is_none()
            && self.response_id.is_none()
            && self.gemini_thought_signatures.is_empty()
            && self.anthropic_thinking_blocks.is_empty()
            && self.openai_reasoning_items.is_empty()
            && self.openai_chat_reasoning_details.is_none()
            && self.copilot_reasoning_opaque.is_none()
    }
}

/// A single provider-ready conversation message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionInputMessage {
    pub role: agena_domain::Role,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<CompletionInputPart>,
    #[serde(
        default,
        skip_serializing_if = "CompletionInputProviderState::is_empty"
    )]
    pub provider_state: CompletionInputProviderState,
}

impl CompletionInputMessage {
    /// Best-effort plain-text rendering for providers that accept only text.
    /// This is intentionally defined on the provider contract so adapters do
    /// not need to reach back into core message storage for their fallback.
    pub fn as_text_lossy(&self) -> String {
        self.parts
            .iter()
            .map(|part| match part {
                CompletionInputPart::Text { text } => text.clone(),
                CompletionInputPart::Reasoning { text } => text.clone(),
                CompletionInputPart::Attachment { attachment } => attachment_text_hint(attachment),
                CompletionInputPart::ToolCall { id, function, .. } => {
                    format!("[tool_call:{}:{id}]", function.function_name())
                }
                CompletionInputPart::ToolResult { tool_call_id, .. } => {
                    format!("[tool_result:{tool_call_id}]")
                }
            })
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Complete provider execution request. Persisted core messages are projected
/// into [`CompletionInputMessage`] at the session boundary before this value
/// is constructed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionRequest {
    pub model: ModelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<CompletionInputMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "tools")]
    pub tool_api_functions: Vec<ToolApiDefinition>,
    #[serde(default, skip_serializing_if = "ProviderNativeToolsConfig::is_empty")]
    pub provider_native_tools: ProviderNativeToolsConfig,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disable_tools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_window_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_compaction: Option<ProviderCompactionContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responses_api_metadata: Option<ResponsesApiRequestMetadata>,
    #[serde(
        default,
        skip_serializing_if = "ModelSpeedModeRequestOverride::is_empty"
    )]
    pub request_override: ModelSpeedModeRequestOverride,
}

fn attachment_text_hint(attachment: &CompletionInputAttachment) -> String {
    let label = attachment
        .filename
        .as_deref()
        .or(attachment.title.as_deref())
        .or(match &attachment.source {
            CompletionInputAttachmentSource::Url { url }
            | CompletionInputAttachmentSource::DataUrl { url } => Some(url.as_str()),
            CompletionInputAttachmentSource::Base64 { .. } => Some("base64"),
            CompletionInputAttachmentSource::FileId { id } => Some(id.as_str()),
            CompletionInputAttachmentSource::LocalPath { path } => Some(path.as_str()),
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(match attachment.kind {
            CompletionInputAttachmentKind::Image => "image",
            CompletionInputAttachmentKind::Audio => "audio",
            CompletionInputAttachmentKind::Video => "video",
            CompletionInputAttachmentKind::Pdf => "document",
            CompletionInputAttachmentKind::File => "file",
        });
    let prefix = match attachment.kind {
        CompletionInputAttachmentKind::Image => "image",
        CompletionInputAttachmentKind::Audio => "audio",
        CompletionInputAttachmentKind::Video => "video",
        CompletionInputAttachmentKind::Pdf => "document",
        CompletionInputAttachmentKind::File => "file",
    };
    format!("[{prefix}:{label}]")
}

/// A completed provider generation before session-specific processing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionResponse {
    pub provider_id: ProviderId,
    pub model: agena_domain::ModelId,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<CompletionFinishReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<CompletionToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<CompletionUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<serde_json::Value>,
}

/// Instructs a provider to produce output in a specific portable format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        #[serde(default)]
        strict: bool,
    },
}

/// Cross-provider metadata used by the OpenAI Responses protocol family.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponsesApiRequestMetadata {
    pub installation_id: String,
    pub session_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub window_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_started_at_unix_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

impl ResponsesApiRequestMetadata {
    pub fn client_metadata(&self) -> HashMap<String, String> {
        let mut metadata = HashMap::from([
            (
                "x-codex-installation-id".to_owned(),
                self.installation_id.clone(),
            ),
            ("session_id".to_owned(), self.session_id.clone()),
            ("thread_id".to_owned(), self.thread_id.clone()),
            ("turn_id".to_owned(), self.turn_id.clone()),
            ("x-codex-window-id".to_owned(), self.window_id.clone()),
            (
                "x-codex-turn-metadata".to_owned(),
                self.turn_metadata_json(),
            ),
        ]);
        if let Some(value) = self.subagent_header.as_ref() {
            metadata.insert("x-openai-subagent".to_owned(), value.clone());
        }
        if let Some(value) = self.parent_thread_id.as_ref() {
            metadata.insert("x-codex-parent-thread-id".to_owned(), value.clone());
        }
        metadata
    }

    pub fn compatibility_headers(&self) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::from([
            ("x-codex-window-id".to_owned(), self.window_id.clone()),
            (
                "x-codex-turn-metadata".to_owned(),
                self.turn_metadata_json(),
            ),
        ]);
        if let Some(value) = self.subagent_header.as_ref() {
            headers.insert("x-openai-subagent".to_owned(), value.clone());
        }
        if let Some(value) = self.parent_thread_id.as_ref() {
            headers.insert("x-codex-parent-thread-id".to_owned(), value.clone());
        }
        headers
    }

    pub fn session_headers(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("session-id".to_owned(), self.session_id.clone()),
            ("thread-id".to_owned(), self.thread_id.clone()),
        ])
    }

    pub fn turn_metadata_json(&self) -> String {
        let mut value = serde_json::Map::from_iter([
            (
                "installation_id".to_owned(),
                serde_json::Value::String(self.installation_id.clone()),
            ),
            (
                "session_id".to_owned(),
                serde_json::Value::String(self.session_id.clone()),
            ),
            (
                "thread_id".to_owned(),
                serde_json::Value::String(self.thread_id.clone()),
            ),
            (
                "turn_id".to_owned(),
                serde_json::Value::String(self.turn_id.clone()),
            ),
            (
                "window_id".to_owned(),
                serde_json::Value::String(self.window_id.clone()),
            ),
        ]);
        for (name, value_to_insert) in [
            ("parent_thread_id", self.parent_thread_id.as_ref()),
            ("subagent_kind", self.subagent_kind.as_ref()),
            ("request_kind", self.request_kind.as_ref()),
        ] {
            if let Some(value_to_insert) = value_to_insert {
                value.insert(
                    name.to_owned(),
                    serde_json::Value::String(value_to_insert.clone()),
                );
            }
        }
        if let Some(value_to_insert) = self.turn_started_at_unix_ms {
            value.insert(
                "turn_started_at_unix_ms".to_owned(),
                serde_json::Value::from(value_to_insert),
            );
        }
        for (key, field_value) in &self.extra {
            if !key.trim().is_empty()
                && !field_value.trim().is_empty()
                && !reserved_responses_metadata_key(key)
            {
                value.insert(key.clone(), serde_json::Value::String(field_value.clone()));
            }
        }
        serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned())
    }
}

fn reserved_responses_metadata_key(key: &str) -> bool {
    matches!(
        key,
        "installation_id"
            | "x-codex-installation-id"
            | "session_id"
            | "session-id"
            | "thread_id"
            | "thread-id"
            | "turn_id"
            | "window_id"
            | "x-codex-window-id"
            | "x-codex-turn-metadata"
            | "x-codex-parent-thread-id"
            | "x-openai-subagent"
            | "request_kind"
            | "turn_started_at_unix_ms"
            | "parent_thread_id"
            | "subagent_kind"
    )
}

/// Opaque provider-native compacted output, persisted separately from messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderCompactionOutput {
    OpenAiResponses { items: Vec<serde_json::Value> },
}

impl ProviderCompactionOutput {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::OpenAiResponses { items } => items.is_empty(),
        }
    }
}

/// Opaque provider-native compacted input replayed on a later completion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderCompactionContext {
    OpenAiResponses { items: Vec<serde_json::Value> },
}

/// Whether a provider stream may be safely resumed by replaying its prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamResumePolicy {
    Disabled,
    ReplaySafePrefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNativeToolKind {
    WebSearch,
    FileSearch,
    CodeExecution,
    ImageGeneration,
    Computer,
    Bash,
    TextEditor,
    UrlContext,
    RemoteMcp,
}

/// The category of a process-local harness selected for a provider-native
/// tool. The harness implementation itself belongs to runtime configuration;
/// this value only identifies the provider-facing binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNativeToolHarnessKind {
    Browser,
    Shell,
    Editor,
}

/// A stable reference from a provider-native tool to a configured local
/// harness. It intentionally carries no runtime handle or harness settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderNativeToolHarnessRef {
    pub kind: ProviderNativeToolHarnessKind,
    pub name: String,
}

/// Provider-native tool to local-harness bindings for one model route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolHarnessBindings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<ProviderNativeToolHarnessRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bash: Option<ProviderNativeToolHarnessRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_editor: Option<ProviderNativeToolHarnessRef>,
}

/// Provider-hosted URL-context settings for one model route. These values are
/// sent to provider APIs and contain no process-local harness configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedUrlContextConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_urls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedUrlContextConfig {
    pub fn is_empty(&self) -> bool {
        self.max_urls.is_none() && self.provider_options.is_none()
    }
}

/// Provider-hosted web-search settings for one model route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedWebSearchConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_domains: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness: Option<ProviderNativeToolFreshness>,
    #[serde(
        default,
        skip_serializing_if = "ProviderNativeToolUserLocationConfig::is_empty"
    )]
    pub user_location: ProviderNativeToolUserLocationConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedWebSearchConfig {
    pub fn is_empty(&self) -> bool {
        self.allowed_domains.is_empty()
            && self.blocked_domains.is_empty()
            && self.freshness.is_none()
            && self.user_location.is_empty()
            && self.max_results.is_none()
            && self.search_context_size.is_none()
            && self.provider_options.is_none()
    }
}

/// Provider-hosted file-search settings for one model route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedFileSearchConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vector_store_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_results: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedFileSearchConfig {
    pub fn is_empty(&self) -> bool {
        self.vector_store_ids.is_empty()
            && self.max_results.is_none()
            && self.include_results.is_none()
            && self.provider_options.is_none()
    }
}

/// Provider-hosted code-execution container selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HostedCodeExecutionContainerConfig {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_ids: Vec<String>,
}

impl HostedCodeExecutionContainerConfig {
    pub fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.id.is_none()
            && self.memory_limit.is_none()
            && self.file_ids.is_empty()
    }
}

/// Provider-hosted code-execution settings for one model route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedCodeExecutionConfig {
    #[serde(
        default,
        skip_serializing_if = "HostedCodeExecutionContainerConfig::is_empty"
    )]
    pub container: HostedCodeExecutionContainerConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedCodeExecutionConfig {
    pub fn is_empty(&self) -> bool {
        self.container.is_empty() && self.provider_options.is_none()
    }
}

/// Provider-hosted image-generation settings for one model route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedImageGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedImageGenerationConfig {
    pub fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.size.is_none()
            && self.quality.is_none()
            && self.moderation.is_none()
            && self.provider_options.is_none()
    }
}

impl ProviderNativeToolHarnessBindings {
    pub const fn is_empty(&self) -> bool {
        self.computer.is_none() && self.bash.is_none() && self.text_editor.is_none()
    }

    pub fn binding_for(
        &self,
        tool: ProviderNativeToolKind,
    ) -> Option<&ProviderNativeToolHarnessRef> {
        match tool {
            ProviderNativeToolKind::Computer => self.computer.as_ref(),
            ProviderNativeToolKind::Bash => self.bash.as_ref(),
            ProviderNativeToolKind::TextEditor => self.text_editor.as_ref(),
            _ => None,
        }
    }
}

impl ProviderNativeToolKind {
    pub const ALL: [Self; 9] = [
        Self::WebSearch,
        Self::FileSearch,
        Self::CodeExecution,
        Self::ImageGeneration,
        Self::Computer,
        Self::Bash,
        Self::TextEditor,
        Self::UrlContext,
        Self::RemoteMcp,
    ];
    pub const fn config_key(self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
            Self::FileSearch => "file_search",
            Self::CodeExecution => "code_execution",
            Self::ImageGeneration => "image_generation",
            Self::Computer => "computer",
            Self::Bash => "bash",
            Self::TextEditor => "text_editor",
            Self::UrlContext => "url_context",
            Self::RemoteMcp => "remote_mcp",
        }
    }
    pub const fn supports_route(self, route: ProviderNativeToolRoute) -> bool {
        match self {
            Self::WebSearch => matches!(
                route,
                ProviderNativeToolRoute::Disabled
                    | ProviderNativeToolRoute::Plugin
                    | ProviderNativeToolRoute::ProviderHosted
            ),
            Self::FileSearch | Self::CodeExecution | Self::ImageGeneration | Self::UrlContext => {
                matches!(
                    route,
                    ProviderNativeToolRoute::Disabled | ProviderNativeToolRoute::ProviderHosted
                )
            }
            Self::Computer | Self::Bash | Self::TextEditor => matches!(
                route,
                ProviderNativeToolRoute::Disabled | ProviderNativeToolRoute::ProviderHarness
            ),
            Self::RemoteMcp => matches!(
                route,
                ProviderNativeToolRoute::Disabled | ProviderNativeToolRoute::ProviderConnector
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNativeToolRoute {
    Disabled,
    Plugin,
    ProviderHosted,
    ProviderHarness,
    ProviderConnector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNativeToolFreshness {
    Auto,
    Cached,
    Live,
}

/// Provider-native tool routing for one model configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolRoutesConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_search: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_execution: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_generation: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bash: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_editor: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_context: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_mcp: Option<ProviderNativeToolRoute>,
}

impl ProviderNativeToolRoutesConfig {
    pub const fn is_empty(&self) -> bool {
        self.web_search.is_none()
            && self.file_search.is_none()
            && self.code_execution.is_none()
            && self.image_generation.is_none()
            && self.computer.is_none()
            && self.bash.is_none()
            && self.text_editor.is_none()
            && self.url_context.is_none()
            && self.remote_mcp.is_none()
    }

    pub const fn route_for(&self, tool: ProviderNativeToolKind) -> Option<ProviderNativeToolRoute> {
        match tool {
            ProviderNativeToolKind::WebSearch => self.web_search,
            ProviderNativeToolKind::FileSearch => self.file_search,
            ProviderNativeToolKind::CodeExecution => self.code_execution,
            ProviderNativeToolKind::ImageGeneration => self.image_generation,
            ProviderNativeToolKind::Computer => self.computer,
            ProviderNativeToolKind::Bash => self.bash,
            ProviderNativeToolKind::TextEditor => self.text_editor,
            ProviderNativeToolKind::UrlContext => self.url_context,
            ProviderNativeToolKind::RemoteMcp => self.remote_mcp,
        }
    }
}

/// Optional geographical context passed to a provider-hosted tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolUserLocationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl ProviderNativeToolUserLocationConfig {
    pub const fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.region.is_none()
            && self.city.is_none()
            && self.timezone.is_none()
    }
}

/// A named remote connector available to a provider-native tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolConnectorConfig {
    pub server: String,
    pub require_approval: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_filter: Vec<String>,
}

impl Default for ProviderNativeToolConnectorConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            require_approval: true,
            tool_filter: Vec::new(),
        }
    }
}

/// Provider-hosted tool settings for one configured model route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedToolConfigs {
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedWebSearchConfig::is_empty"
    )]
    pub web_search: ProviderHostedWebSearchConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedFileSearchConfig::is_empty"
    )]
    pub file_search: ProviderHostedFileSearchConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedCodeExecutionConfig::is_empty"
    )]
    pub code_execution: ProviderHostedCodeExecutionConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedImageGenerationConfig::is_empty"
    )]
    pub image_generation: ProviderHostedImageGenerationConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedUrlContextConfig::is_empty"
    )]
    pub url_context: ProviderHostedUrlContextConfig,
}

impl ProviderHostedToolConfigs {
    pub fn is_empty(&self) -> bool {
        self.web_search.is_empty()
            && self.file_search.is_empty()
            && self.code_execution.is_empty()
            && self.image_generation.is_empty()
            && self.url_context.is_empty()
    }
}

/// Complete provider-native tool configuration for one model route.
/// Concrete harness settings remain runtime configuration; this value carries
/// only provider-facing routes, hosted options, stable harness references, and
/// connector declarations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolsConfig {
    #[serde(
        default,
        skip_serializing_if = "ProviderNativeToolRoutesConfig::is_empty"
    )]
    pub routes: ProviderNativeToolRoutesConfig,
    #[serde(default, skip_serializing_if = "ProviderHostedToolConfigs::is_empty")]
    pub hosted: ProviderHostedToolConfigs,
    #[serde(
        default,
        skip_serializing_if = "ProviderNativeToolHarnessBindings::is_empty"
    )]
    pub harness: ProviderNativeToolHarnessBindings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub connectors: BTreeMap<String, ProviderNativeToolConnectorConfig>,
}

impl ProviderNativeToolsConfig {
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
            && self.hosted.is_empty()
            && self.harness.is_empty()
            && self.connectors.is_empty()
    }

    pub fn bindings(&self) -> Vec<ProviderNativeToolBinding> {
        ProviderNativeToolKind::ALL
            .into_iter()
            .filter_map(|tool| {
                let route = self.routes.route_for(tool)?;
                if route == ProviderNativeToolRoute::Disabled {
                    return None;
                }
                if tool == ProviderNativeToolKind::FileSearch
                    && route == ProviderNativeToolRoute::ProviderHosted
                    && self.hosted.file_search.vector_store_ids.is_empty()
                {
                    return None;
                }
                Some(ProviderNativeToolBinding {
                    tool,
                    route,
                    harness: self.harness.binding_for(tool).cloned(),
                    connector_names: if tool == ProviderNativeToolKind::RemoteMcp {
                        self.connectors.keys().cloned().collect()
                    } else {
                        Vec::new()
                    },
                })
            })
            .collect()
    }
}

/// A resolved provider-native tool route with its stable binding references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderNativeToolBinding {
    pub tool: ProviderNativeToolKind,
    pub route: ProviderNativeToolRoute,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<ProviderNativeToolHarnessRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connector_names: Vec<String>,
}

/// Presentation-neutral defaults for one configured provider route.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderDefaults {
    pub adapter: Option<String>,
    pub model: String,
    pub thinking_mode: Option<String>,
    pub speed_mode: Option<String>,
    pub verbosity: Option<String>,
    pub parallel_tool_calls: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAdapterSummary {
    pub adapter_id: String,
    pub enabled: bool,
    pub configured_model_count: usize,
}

/// Complete configured adapter/model routing summary needed by presentation
/// editors. This intentionally exposes only stable route values, not Core's
/// resolved provider schema or authentication implementation details.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderConfiguredAdapterModels {
    pub adapter_id: String,
    pub enabled: bool,
    pub model_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfiguredRouting {
    pub provider_id: ProviderId,
    pub adapters: Vec<ProviderConfiguredAdapterModels>,
}

/// Complete, presentation-neutral editable configuration for one saved
/// provider. The value intentionally contains stable auth/credential data
/// rather than exposing Core's resolved configuration schema to an editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfiguredEditor {
    pub provider_id: String,
    pub auth: ProviderConfiguredEditorAuth,
    pub default_adapter: Option<String>,
    pub default_model: Option<String>,
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderConfiguredEditorAuth {
    None,
    Api {
        base_url: String,
        api_key: Option<ProviderApiKeySource>,
    },
    ClineApi {
        api_key: Option<ProviderApiKeySource>,
    },
    Gitlab {
        api_key: Option<ProviderApiKeySource>,
        instance_url: Option<String>,
    },
    Credential {
        issuer: CredentialIssuer,
        credential: Option<AuthData>,
        base_url: Option<String>,
        instance_url: Option<String>,
        service_key_env: Option<String>,
    },
    BedrockSigv4 {
        base_url: String,
        region: String,
        profile: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        session_token: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderNativeToolBindingSummary {
    pub tool: String,
    pub route: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderNativeToolsSummary {
    pub active: bool,
    pub model_count: usize,
    pub bindings: Vec<ProviderNativeToolBindingSummary>,
}

/// A fully projected provider entry for catalog/listing presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCatalogEntry {
    pub provider_id: ProviderId,
    pub defaults: ProviderDefaults,
    pub adapters: Vec<ProviderAdapterSummary>,
    pub provider_native_tools: Option<ProviderNativeToolsSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderProtocolPaths {
    pub openai: String,
    pub anthropic: String,
    pub gemini: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderApiKeySource {
    Inline(String),
    Environment(String),
}

/// Stable request for live discovery against a not-yet-saved provider draft.
///
/// The request deliberately carries the draft authentication shape rather
/// than a Core configuration target, so presentation callers do not need to
/// construct or inspect the concrete configuration schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftProviderAdapterModelsRequest {
    Http(DraftHttpProviderAdapterModelsRequest),
    None {
        provider_id: Option<String>,
        adapter_ids: Vec<String>,
    },
    ClineApi {
        provider_id: Option<String>,
        api_key: Option<ProviderApiKeySource>,
        adapter_ids: Vec<String>,
        models_url: Option<String>,
    },
    Gitlab {
        provider_id: Option<String>,
        api_key: Option<ProviderApiKeySource>,
        adapter_ids: Vec<String>,
    },
    Credential {
        provider_id: Option<String>,
        issuer: CredentialIssuer,
        credential: Option<Box<AuthData>>,
        base_url: Option<String>,
        protocol_paths: ProviderProtocolPaths,
        service_key_env: Option<String>,
        instance_url: Option<String>,
        adapter_ids: Vec<String>,
    },
    BedrockSigv4 {
        provider_id: Option<String>,
        base_url: Option<String>,
        region: Option<String>,
        profile: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        session_token: Option<String>,
        adapter_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftHttpProviderAdapterModelsRequest {
    pub provider_id: Option<String>,
    pub base_url: String,
    pub protocol_paths: ProviderProtocolPaths,
    pub api_key: Option<ProviderApiKeySource>,
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAdapterModelsEntry {
    pub adapter_id: String,
    pub enabled: bool,
    pub resolved_base_url: Option<String>,
    pub models: Vec<Model>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAdapterModelsListing {
    pub provider_id: String,
    pub adapters: Vec<ProviderAdapterModelsEntry>,
}

/// Provider-derived values required to validate and materialize run options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModelExecutionOptions {
    pub default_adapter: Option<AdapterId>,
    pub capabilities: ModelCapabilities,
    pub thinking_modes: Vec<ModelThinkingMode>,
    pub speed_modes: BTreeMap<String, ModelSpeedMode>,
    pub metadata: ModelMetadata,
}

/// Read-only provider catalog required by application-facing provider queries.
///
/// Implementations must resolve the catalog against their current runtime
/// snapshot, so reloads are observed without rebuilding application services.
#[async_trait]
pub trait ProviderCatalog: Send + Sync {
    fn list_providers(&self) -> Vec<ProviderCatalogEntry>;

    fn contains_provider(&self, provider_id: &ProviderId) -> bool;

    fn configured_routing(&self, provider_id: &ProviderId) -> Option<ProviderConfiguredRouting>;

    fn configured_editor(&self, provider_id: &ProviderId) -> Option<ProviderConfiguredEditor>;

    /// Synchronous model choices implied by the saved configuration only.
    /// This must not perform remote provider discovery.
    fn configured_local_models(
        &self,
        provider_id: &ProviderId,
    ) -> Result<Vec<Model>, ProviderCatalogError>;

    fn default_model(&self) -> Result<Option<ModelRef>, ProviderCatalogError>;

    /// Resolve a CLI/application model target against the current configured
    /// provider catalog. `target` may be a provider or a fully qualified
    /// model target; implementations observe runtime reloads.
    fn resolve_model_target(
        &self,
        target: &str,
        model: Option<&str>,
    ) -> Result<ModelRef, ProviderCatalogError>;

    fn model_execution_options(
        &self,
        model: &ModelRef,
    ) -> Result<ProviderModelExecutionOptions, ProviderCatalogError>;

    async fn list_models(
        &self,
        provider_id: &ProviderId,
    ) -> Result<Vec<Model>, ProviderCatalogError>;

    async fn list_draft_adapter_models(
        &self,
        request: DraftProviderAdapterModelsRequest,
    ) -> Result<ProviderAdapterModelsListing, ProviderCatalogError>;

    async fn list_saved_adapter_models(
        &self,
        provider_id: &ProviderId,
        adapter_ids: Vec<String>,
    ) -> Result<ProviderAdapterModelsListing, ProviderCatalogError>;
}

/// Narrow provider-model source used while composing a model catalog.
///
/// This excludes persistence, source ranking, refresh policy, and execution
/// configuration so catalog composition can consume an adapter without
/// depending on a concrete provider registry.
#[async_trait]
pub trait ProviderModelSource: Send + Sync {
    fn provider_ids(&self) -> Vec<ProviderId>;

    async fn list_models(
        &self,
        provider_id: &ProviderId,
    ) -> Result<Vec<Model>, ProviderCatalogError>;
}

#[cfg(test)]
mod tests {
    use super::{
        AgenaToolMode, AgenaToolsConfig, CatalogModelRecord, ModelCapabilityPatch,
        ModelCatalogSnapshotSourceKind, OAuthCallback, ProviderClientVersions,
        ProviderHttpClientConfig, ProviderModelPriorities, SapAiCoreServiceKey,
    };

    #[test]
    fn catalog_model_record_round_trips_as_provider_contract() {
        let record = CatalogModelRecord {
            model_id: "provider/model".to_owned(),
            display_name: Some("Model".to_owned()),
            capabilities: ModelCapabilityPatch::default(),
            ..Default::default()
        };
        let encoded = serde_json::to_value(&record).expect("serialize catalog record");
        assert_eq!(encoded["model_id"], "provider/model");
        let decoded: CatalogModelRecord =
            serde_json::from_value(encoded).expect("deserialize catalog record");
        assert_eq!(decoded, record);
    }

    #[test]
    fn provider_client_versions_have_stable_defaults() {
        let versions = ProviderClientVersions::default();
        assert_eq!(versions.codex, "0.144.4");
        assert_eq!(versions.claude, "2.1.209");
        assert_eq!(versions.gemini, "0.50.0");
    }

    #[test]
    fn provider_http_client_config_has_stable_timeouts() {
        let config = ProviderHttpClientConfig::default();
        assert_eq!(config.timeout, std::time::Duration::from_secs(120));
        assert_eq!(config.connect_timeout, std::time::Duration::from_secs(15));
    }

    #[test]
    fn provider_model_priorities_are_value_owned_and_default_missing_entries() {
        let priorities =
            ProviderModelPriorities::new([("openai".to_owned(), 450)].into_iter().collect());
        assert_eq!(priorities.get("openai"), 450);
        assert_eq!(priorities.get("missing"), 0);
        assert!(!priorities.is_empty());
    }

    #[test]
    fn tool_config_has_stable_default_and_wire_shape() {
        let config = AgenaToolsConfig::default();
        assert_eq!(config.mode, AgenaToolMode::Disabled);
        assert!(config.provider_native.is_empty());
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::json!({"mode": "disabled"})
        );
    }

    #[test]
    fn sap_ai_core_service_key_keeps_wire_field_names() {
        let value: SapAiCoreServiceKey = serde_json::from_value(serde_json::json!({
            "clientid": "id",
            "clientsecret": "secret",
            "url": "https://example.invalid",
            "serviceurls": {"AI_API_URL": "https://api.example.invalid"}
        }))
        .unwrap();
        assert_eq!(value.serviceurls.ai_api_url, "https://api.example.invalid");
    }

    #[test]
    fn oauth_callback_is_a_stable_code_state_value() {
        let callback = OAuthCallback {
            code: "code".to_owned(),
            state: "state".to_owned(),
        };
        assert_eq!(callback.code, "code");
        assert_eq!(callback.state, "state");
    }

    #[test]
    fn catalog_snapshot_source_kind_keeps_persistent_tags() {
        assert_eq!(
            serde_json::to_string(&ModelCatalogSnapshotSourceKind::Generated).unwrap(),
            "\"generated\""
        );
        assert_eq!(
            serde_json::to_string(&ModelCatalogSnapshotSourceKind::Cache).unwrap(),
            "\"cache\""
        );
    }

    use super::{
        CompletionFinishReason, CompletionInputAttachment, CompletionInputAttachmentKind,
        CompletionInputAttachmentSource, CompletionInputMessage, CompletionInputPart,
        CompletionResponse, CompletionStreamEvent, ProviderHostedCodeExecutionConfig,
        ProviderHostedFileSearchConfig, ProviderHostedImageGenerationConfig,
        ProviderHostedToolConfigs, ProviderHostedUrlContextConfig, ProviderHostedWebSearchConfig,
        ProviderNativeToolHarnessBindings, ProviderNativeToolHarnessKind,
        ProviderNativeToolHarnessRef, ProviderNativeToolKind, ProviderNativeToolOutputBlock,
        ProviderNativeToolRoute, ProviderNativeToolRoutesConfig, ProviderNativeToolSearchResult,
        ProviderNativeToolsConfig, ToolApiDefinition,
    };
    use agena_domain::{ModelId, ProviderId};

    #[test]
    fn normalizes_common_provider_finish_reasons() {
        assert_eq!(
            CompletionFinishReason::from_provider(Some("max_output_tokens")),
            Some(CompletionFinishReason::Length)
        );
        assert_eq!(
            CompletionFinishReason::from_provider(Some("content_filter")),
            Some(CompletionFinishReason::ContentFilter)
        );
    }

    #[test]
    fn tool_api_definition_is_a_registry_free_serializable_contract() {
        let definition = ToolApiDefinition {
            handler_key: "agena.tools.help".to_owned(),
            plugin_name: "tools".to_owned(),
            name: "tools_help".to_owned(),
            description: "Describe an execution tool.".to_owned(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            strict: true,
            definition_identity: "tools-help-v1".to_owned(),
        };

        let encoded = serde_json::to_value(&definition).expect("serialize provider declaration");
        assert_eq!(encoded["name"], "tools_help");
        assert_eq!(encoded["handler_key"], "agena.tools.help");
        assert_eq!(encoded["plugin_name"], "tools");
        assert_eq!(
            serde_json::from_value::<ToolApiDefinition>(encoded)
                .expect("deserialize provider declaration"),
            definition
        );
    }

    #[test]
    fn completion_response_uses_only_contract_and_domain_values() {
        let response = CompletionResponse {
            provider_id: ProviderId::new("test"),
            model: ModelId::new("test-model"),
            text: "done".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: None,
            provider_metadata: None,
        };

        let encoded = serde_json::to_value(&response).expect("serialize completion response");
        assert_eq!(encoded["provider_id"], "test");
        assert_eq!(encoded["model"], "test-model");
        assert_eq!(encoded["finish_reason"]["type"], "stop");
        assert_eq!(
            serde_json::from_value::<CompletionResponse>(encoded)
                .expect("deserialize completion response"),
            response
        );
    }

    #[test]
    fn provider_native_harness_bindings_are_contract_values_without_runtime_handles() {
        let bindings = ProviderNativeToolHarnessBindings {
            computer: Some(ProviderNativeToolHarnessRef {
                kind: ProviderNativeToolHarnessKind::Browser,
                name: "browser-default".to_owned(),
            }),
            ..Default::default()
        };

        assert_eq!(
            bindings
                .binding_for(ProviderNativeToolKind::Computer)
                .map(|binding| binding.name.as_str()),
            Some("browser-default")
        );
        assert!(
            bindings
                .binding_for(ProviderNativeToolKind::WebSearch)
                .is_none()
        );
        assert_eq!(
            serde_json::from_value::<ProviderNativeToolHarnessBindings>(
                serde_json::to_value(&bindings).expect("serialize bindings"),
            )
            .expect("deserialize bindings"),
            bindings
        );
    }

    #[test]
    fn hosted_url_context_is_a_provider_contract_value() {
        let config = ProviderHostedUrlContextConfig {
            max_urls: Some(12),
            provider_options: Some(serde_json::json!({"vendor_mode": "compact"})),
        };
        assert!(!config.is_empty());
        assert_eq!(
            serde_json::from_value::<ProviderHostedUrlContextConfig>(
                serde_json::to_value(&config).expect("serialize URL context"),
            )
            .expect("deserialize URL context"),
            config
        );
    }

    #[test]
    fn hosted_tool_configuration_values_are_serializable_and_empty_by_default() {
        assert!(ProviderHostedWebSearchConfig::default().is_empty());
        assert!(ProviderHostedFileSearchConfig::default().is_empty());
        assert!(ProviderHostedCodeExecutionConfig::default().is_empty());
        assert!(ProviderHostedImageGenerationConfig::default().is_empty());

        let web_search = ProviderHostedWebSearchConfig {
            allowed_domains: vec!["example.com".to_owned()],
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_value::<ProviderHostedWebSearchConfig>(
                serde_json::to_value(&web_search).expect("serialize web search config"),
            )
            .expect("deserialize web search config"),
            web_search
        );
    }

    #[test]
    fn complete_native_tool_configuration_is_a_provider_contract_value() {
        let config = ProviderNativeToolsConfig {
            routes: ProviderNativeToolRoutesConfig {
                web_search: Some(ProviderNativeToolRoute::ProviderHosted),
                ..Default::default()
            },
            hosted: ProviderHostedToolConfigs {
                web_search: ProviderHostedWebSearchConfig {
                    allowed_domains: vec!["example.com".to_owned()],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(config.bindings().len(), 1);
        assert_eq!(config.bindings()[0].tool, ProviderNativeToolKind::WebSearch);
        assert_eq!(
            serde_json::from_value::<ProviderNativeToolsConfig>(
                serde_json::to_value(&config).expect("serialize native tool configuration"),
            )
            .expect("deserialize native tool configuration"),
            config
        );
    }

    #[test]
    fn native_tool_completion_stream_event_uses_provider_and_domain_values() {
        let event = CompletionStreamEvent::ProviderNativeToolCallCompleted {
            provider_id: ProviderId::new("test"),
            model: agena_domain::ModelId::new("test-model"),
            stream_key: "native:1".to_owned(),
            id: Some("call_1".to_owned()),
            invocation: agena_domain::ToolInvocation::new(
                "web.run",
                agena_domain::StructuredObject::default(),
            ),
            title: "web search".to_owned(),
            output_text: "one result".to_owned(),
            blocks: vec![ProviderNativeToolOutputBlock::SearchResults {
                query: Some("Agena".to_owned()),
                results: vec![ProviderNativeToolSearchResult {
                    title: "Agena".to_owned(),
                    uri: "https://example.com".to_owned(),
                    snippet: None,
                    score: Some(1.0),
                }],
            }],
            details: agena_domain::ToolOutput::default(),
            raw: None,
        };

        let encoded = serde_json::to_value(&event).expect("serialize stream event");
        assert_eq!(encoded["type"], "provider_native_tool_call_completed");
        assert_eq!(encoded["blocks"][0]["type"], "search_results");
        assert_eq!(
            serde_json::from_value::<CompletionStreamEvent>(encoded)
                .expect("deserialize stream event"),
            event
        );
    }

    #[test]
    fn completion_input_message_has_a_contract_owned_text_fallback() {
        let message = CompletionInputMessage {
            role: agena_domain::Role::User,
            parts: vec![
                CompletionInputPart::Text {
                    text: "inspect ".to_owned(),
                },
                CompletionInputPart::Attachment {
                    attachment: CompletionInputAttachment {
                        kind: CompletionInputAttachmentKind::Pdf,
                        mime: "application/pdf".to_owned(),
                        source: CompletionInputAttachmentSource::FileId {
                            id: "file_123".to_owned(),
                        },
                        filename: Some("report.pdf".to_owned()),
                        title: None,
                        size_bytes: None,
                        sha256: None,
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: Some(2),
                    },
                },
            ],
            provider_state: Default::default(),
        };

        assert_eq!(message.as_text_lossy(), "inspect [document:report.pdf]");
    }
}
