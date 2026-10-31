//! # agena-provider
//!
//! Provider-facing ports shared by application services and concrete adapters.
//!
//! This crate deliberately owns contracts only: provider client traits, wire
//! types, auth values, catalog contracts, usage/cost models, and
//! protocol-specific data shapes (Anthropic, Gemini, Ollama, Bedrock). SDK
//! clients, runtime composition, configuration parsing, and catalog
//! decoration live in their respective concrete layers.
//!
//! ## Key items
//!
//! - [`CredentialIssuer`] — OAuth/credential issuing flow used by login.
//! - [`AuthData`] — provider credential material.
//! - [`ProviderOverlay`] / [`ProviderAdapterOverlay`] — configuration
//!   overlays applied on top of provider defaults.
//! - [`CatalogModelDefinition`] / [`ProviderCapabilityFamilyConfig`] —
//!   catalog entries and capability families.
//! - [`BedrockSigv4AuthConfig`] — AWS SigV4 auth configuration.
//! - [`CopilotModelExtension`] — GitHub Copilot model metadata extension.
//!
//! Wire-type modules (`anthropic_*`, `gemini_*`, `ollama_*`, `prompt_cache_*`,
//! `usage_cost`, `http_utils`, ...) are re-exported from the crate root.

use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use agena_domain::{
    CapabilitySupport, Model, ModelCapabilities, ModelId, ModelInputModality, ModelLifecycle,
    ModelMetadata, ModelPricing, ModelRef, ModelSpeedMode, ModelSpeedModeRequestOverride,
    ModelThinkingMode, ProviderId, ReasoningEffort, ThinkingDisplay, ThinkingRequest,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

mod auth_values;
mod catalog_contract;
pub mod draft;
pub use catalog_contract::*;
mod model_discovery;
pub use model_discovery::*;
mod contract;
pub use auth_values::{
    AuthData, CopilotDeployment, CredentialIssuer, OAuthTokenResponse, OAuthUserInfo,
};
pub use contract::*;
mod config_patch_values;
pub use config_patch_values::{
    HostedCodeExecutionContainerOverlay, OpenAiResponsesBackendConfig, ProviderAdapterOverlay,
    ProviderApiSubtype, ProviderAuthMode, ProviderAuthOverlay, ProviderCapabilityFamilyConfig,
    ProviderGitlabApiAccessOverlay, ProviderHostedCodeExecutionOverlay,
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
    merge_json_object_patch_map, merge_provider_metadata, merged_request_headers,
    normalize_base_url, normalize_optional_text, optional_non_empty, prompt_cache_header_entries,
    prompt_cache_ignores_header, provider_metadata_value_is_meaningful, request_shape_fingerprint,
};
mod usage_cost;
pub use usage_cost::{
    CompletionUsageCostContribution, completion_usage_cost_contribution,
    completion_usage_own_cost_contribution, estimate_completion_usage_cost_usd,
};
mod anthropic_wire_text;
pub use anthropic_wire_text::{AnthropicBinarySource, AnthropicTextBlock};
mod anthropic_wire;
pub use anthropic_wire::{
    AnthropicCacheCreationUsage, AnthropicMessage, AnthropicMessagesRequest,
    AnthropicMessagesResponse, AnthropicModel, AnthropicModelListResponse, AnthropicOutputConfig,
    AnthropicOutputTokensDetails, AnthropicSseDelta, AnthropicSseEvent, AnthropicSseMessage,
    AnthropicSseMessageDelta, AnthropicToolCallState, AnthropicUsage, normalize_anthropic_model_id,
};
mod anthropic_thinking;
pub use anthropic_thinking::{
    AnthropicThinkingBlockState, AnthropicThinkingParts, anthropic_adaptive_parts,
    anthropic_budget_effort, anthropic_budget_for_effort, anthropic_default_display,
    anthropic_effort_for_budget, anthropic_enabled_parts,
    anthropic_model_defaults_to_omitted_thinking, anthropic_model_rejects_disabled_thinking,
    anthropic_model_rejects_sampling, anthropic_model_requires_adaptive_thinking,
    anthropic_model_supports_adaptive_thinking, anthropic_model_supports_effort,
    anthropic_model_supports_max_effort, anthropic_model_supports_xhigh_effort,
    anthropic_thinking_budget, anthropic_thinking_metadata, anthropic_thinking_parts,
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
    GeminiContent, GeminiFunctionCall, GeminiFunctionResponse, GeminiFunctionResponsePart,
    GeminiInlineData, GeminiPart,
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
    project_disabled_completion_input_history, strip_provider_tool_body_fields,
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
mod configured_models;
pub use configured_models::{
    apply_configured_modes, apply_configured_thinking_modes, configured_thinking_mode_to_model,
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
mod catalog_collector;
pub use catalog_collector::collect_live_provider_models;
mod catalog_decoration;
pub use catalog_decoration::{
    apply_catalog_definition_as_authority, apply_catalog_definition_as_baseline,
    apply_configured_definition_as_authority, apply_configured_definition_as_baseline,
    catalog_definition_to_provider_definition, merge_catalog_baseline_speed_modes,
    merge_catalog_baseline_thinking_modes,
};
mod catalog_model_decoration;
pub use catalog_model_decoration::{CatalogModelDecorationSource, decorate_provider_models};
