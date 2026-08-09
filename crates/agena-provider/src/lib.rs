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
    AdapterId, CapabilitySupport, Model, ModelCapabilities, ModelId, ModelInputModality,
    ModelLifecycle, ModelMetadata, ModelPricing, ModelRef, ModelSpeedMode,
    ModelSpeedModeRequestOverride, ModelThinkingMode, ProviderId, ReasoningEffort, ThinkingDisplay,
    ThinkingRequest,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

mod auth_values;
mod catalog_contract;
pub use catalog_contract::*;
mod contract;
pub use auth_values::{
    AuthData, CopilotDeployment, CredentialIssuer, OAuthTokenResponse, OAuthUserInfo,
};
pub use contract::*;
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
    completion_usage_own_cost_contribution, estimate_completion_usage_cost_usd,
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
            issuer: None,
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
        CompletionInputAttachmentSource, CompletionInputRun, CompletionInputPart,
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
            serde_json::from_value::<ToolApiDefinition>(encoded.clone())
                .expect("deserialize provider declaration"),
            definition
        );
        let mut removed_direct_shape = encoded;
        removed_direct_shape["execution_tool"] = serde_json::json!("fs.read");
        let error = serde_json::from_value::<ToolApiDefinition>(removed_direct_shape)
            .expect_err("removed direct execution-tool binding must not deserialize");
        assert!(error.to_string().contains("execution_tool"));
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
            summary: "1 result".to_owned(),
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
    fn completion_input_run_has_a_contract_owned_text_fallback() {
        let run = CompletionInputRun {
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

        assert_eq!(run.as_text_lossy(), "inspect [document:report.pdf]");
    }
}
