use agena_domain::*;
use agena_domain::{ModelCapabilities, Role};
pub(super) use agena_domain::{ModelMetadata, ModelTokenLimits};
use agena_provider::{
    AuthData, CapabilityFamily, CompletionFinishReason, CompletionStreamEvent, CompletionToolCall,
    CompletionUsage, OpenAiProfile, OpenAiResponsesBackend, ProviderImageCapabilities,
    ProviderImageOperation, ProviderImageRequest, ProviderImageResponse,
    ProviderNativeToolArtifact, ProviderNativeToolKind, ResponsesApiRequestMetadata,
};
use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};
use tokio::sync::Mutex;

use super::copilot_models::CopilotModelExtension;
use super::protocol_ids;
use super::tool_stream::{
    ToolStreamAccumulator, ToolStreamInput, ToolStreamInputKind, ToolStreamUpdate,
};

use crate::{
    ProviderError,
    provider::{
        CompletionResponse, ManagedCredential, ModelRuntime,
        chat_wire::{self, ChatCompletionRequest, ChatCompletionResponse, ChatStreamOptions},
        prompt_cache, sse, utils, wire_message,
    },
};
use agena_domain::{CapabilitySupport, ModelInputModality, StructuredObject};
use agena_provider::CompletionRequest;
use agena_runtime_contracts::part::{AttachmentItem, AttachmentKind, AttachmentSource};

mod openai_models;
mod openai_provider_native_tools;
mod openai_requests;
mod openai_response_builders;
mod openai_response_types;
mod openai_runtime;
mod openai_setup;
mod openai_wire;

use self::openai_models::*;
use self::openai_provider_native_tools::*;
use self::openai_response_types::*;
use self::openai_wire::*;

// Re-export the tool-stream helpers so sibling adapters (e.g. amazon-bedrock's
// chat-compat path) can route tool deltas through the shared accumulator.
pub(crate) use self::openai_wire::{
    chat_tool_stream_input, completion_event_from_tool_stream_update,
};

const CHATGPT_CODEX_ORIGINATOR: &str = crate::RUNTIME_CODEX_ORIGINATOR;
const DEFAULT_COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
const RESPONSES_ADAPTER_KIND: &str = "openai_responses";
const CHAT_COMPLETIONS_ADAPTER_KIND: &str = "openai_chat_completions";

#[derive(Clone)]
#[doc(hidden)]
/// Transport for OpenAI-compatible providers.
pub struct OpenAiTransport {
    client_identity: crate::ProviderClientIdentity,
    id: String,
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    backend: OpenAiResponsesBackend,
    auth_data: Option<Arc<Mutex<AuthData>>>,
    profile: OpenAiProfile,
    models_url: Option<String>,
    auth_header: String,
    auth_scheme: Option<String>,
    capability_family: CapabilityFamily,
    extra_headers: HashMap<String, String>,
    top_level_prompt_cache_override: Option<bool>,
}

#[derive(Clone)]
/// Adapter for the OpenAI Responses API.
pub struct OpenAiResponsesAdapter {
    transport: OpenAiTransport,
}

#[derive(Clone)]
/// Adapter for the OpenAI Chat Completions API.
pub struct OpenAiChatCompletionsAdapter {
    transport: OpenAiTransport,
}

#[derive(Clone)]
struct OpenAiTransportOptions {
    client_identity: crate::ProviderClientIdentity,
    backend: OpenAiResponsesBackend,
    auth_data: Option<Arc<Mutex<AuthData>>>,
    profile: OpenAiProfile,
    models_url: Option<String>,
    auth_header: String,
    auth_scheme: Option<String>,
    capability_family: CapabilityFamily,
    extra_headers: HashMap<String, String>,
    top_level_prompt_cache_override: Option<bool>,
}

#[derive(Clone)]
/// Options of the OpenAI Responses adapter.
pub struct OpenAiResponsesAdapterOptions {
    pub client_identity: crate::ProviderClientIdentity,
    pub backend: OpenAiResponsesBackend,
    pub auth_data: Option<Arc<Mutex<AuthData>>>,
    pub profile: OpenAiProfile,
    pub models_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub capability_family: CapabilityFamily,
    pub extra_headers: HashMap<String, String>,
    pub top_level_prompt_cache_override: Option<bool>,
}

impl Default for OpenAiResponsesAdapterOptions {
    fn default() -> Self {
        Self {
            client_identity: Default::default(),
            backend: OpenAiResponsesBackend::Api,
            auth_data: None,
            profile: OpenAiProfile::Standard,
            models_url: None,
            auth_header: "authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            capability_family: CapabilityFamily::OpenAi,
            extra_headers: HashMap::new(),
            top_level_prompt_cache_override: None,
        }
    }
}

#[derive(Clone)]
/// Options of the OpenAI Chat Completions adapter.
pub struct OpenAiChatCompletionsAdapterOptions {
    pub client_identity: crate::ProviderClientIdentity,
    pub auth_data: Option<Arc<Mutex<AuthData>>>,
    pub profile: OpenAiProfile,
    pub models_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub capability_family: CapabilityFamily,
    pub extra_headers: HashMap<String, String>,
    pub top_level_prompt_cache_override: Option<bool>,
}

impl Default for OpenAiChatCompletionsAdapterOptions {
    fn default() -> Self {
        Self {
            client_identity: Default::default(),
            auth_data: None,
            profile: OpenAiProfile::Standard,
            models_url: None,
            auth_header: "authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            capability_family: CapabilityFamily::OpenAi,
            extra_headers: HashMap::new(),
            top_level_prompt_cache_override: None,
        }
    }
}

impl std::ops::Deref for OpenAiResponsesAdapter {
    type Target = OpenAiTransport;

    fn deref(&self) -> &Self::Target {
        &self.transport
    }
}

impl std::ops::Deref for OpenAiChatCompletionsAdapter {
    type Target = OpenAiTransport;

    fn deref(&self) -> &Self::Target {
        &self.transport
    }
}

#[derive(Debug)]
struct OpenAiResponsesToolPlan {
    tools: Vec<serde_json::Value>,
    include: Vec<String>,
}

fn openai_reasoning_items_from_output(
    items: Option<&[OpenAiOutputItem]>,
) -> Vec<serde_json::Value> {
    items
        .into_iter()
        .flatten()
        .filter_map(|item| {
            if item.kind.as_deref() != Some("reasoning") {
                return None;
            }
            let encrypted_content = item
                .encrypted_content
                .as_deref()
                .filter(|content| !content.trim().is_empty())
                .map(ToOwned::to_owned);
            let summary = item
                .summary
                .iter()
                .flatten()
                .filter_map(|part| part.text.as_deref())
                .filter(|text| !text.trim().is_empty())
                .map(|text| {
                    serde_json::json!({
                        "type": "summary_text",
                        "text": text,
                    })
                })
                .collect::<Vec<_>>();
            let mut content = item
                .content
                .iter()
                .flatten()
                .filter_map(|part| {
                    part.text
                        .as_deref()
                        .filter(|text| !text.trim().is_empty())
                        .map(|text| {
                            serde_json::json!({
                                "type": part.kind.as_deref().unwrap_or("reasoning_text"),
                                "text": text,
                            })
                        })
                })
                .collect::<Vec<_>>();
            // A `reasoning_content` model (e.g. deepseek) returns reasoning as
            // a `summary` (or streaming deltas) without a `content` array or
            // `encrypted_content`. Fall back to the summary text so the item
            // survives into the replay carrier and can be passed back.
            if content.is_empty() && !summary.is_empty() {
                content = summary
                    .iter()
                    .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
                    .map(|text| {
                        serde_json::json!({
                            "type": "reasoning_text",
                            "text": text,
                        })
                    })
                    .collect();
            }
            // Preserve a reasoning item when it carries either encrypted
            // content (OpenAI-style) or plain text content (chat-style
            // `reasoning_content` models that must replay their reasoning).
            if encrypted_content.is_none() && content.is_empty() {
                return None;
            }
            let mut normalized = serde_json::json!({
                "type": "reasoning",
                "summary": summary,
            });
            if let Some(encrypted_content) = encrypted_content {
                normalized["encrypted_content"] = serde_json::Value::String(encrypted_content);
            }
            if !content.is_empty() {
                normalized["content"] = serde_json::Value::Array(content);
            }
            Some(normalized)
        })
        .collect()
}

fn openai_reasoning_item_from_event(
    event: &serde_json::Value,
) -> Option<(String, serde_json::Value)> {
    let event_type = event.get("type").and_then(serde_json::Value::as_str)?;
    if !matches!(
        event_type,
        "response.output_item.added" | "response.output_item.done"
    ) {
        return None;
    }
    let item = event.get("item")?;
    if item.get("type").and_then(serde_json::Value::as_str) != Some("reasoning") {
        return None;
    }
    let encrypted_content = item
        .get("encrypted_content")
        .and_then(serde_json::Value::as_str)
        .filter(|content| !content.trim().is_empty())
        .map(ToOwned::to_owned);
    let summary: Vec<serde_json::Value> = item
        .get("summary")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("text")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty())
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mut content = item
        .get("content")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("text")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty())
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter(|content| !content.is_empty());
    // Fall back to the summary text (chat-style `reasoning_content` models)
    // when no explicit content array is present, so the reasoning survives
    // into the replay carrier.
    if content.as_ref().is_none_or(|content| content.is_empty()) {
        let summary_text = summary
            .iter()
            .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
            .map(|text| {
                serde_json::json!({
                    "type": "reasoning_text",
                    "text": text,
                })
            })
            .collect::<Vec<_>>();
        if !summary_text.is_empty() {
            content = Some(summary_text);
        }
    }
    // Preserve a reasoning item when it carries either encrypted content or
    // plain text content (chat-style `reasoning_content` models that must
    // replay their reasoning back to the API).
    if encrypted_content.is_none() && content.as_ref().is_none_or(|content| content.is_empty()) {
        return None;
    }
    let mut normalized = serde_json::json!({
        "type": "reasoning",
        "summary": summary,
    });
    if let Some(encrypted_content) = encrypted_content {
        normalized["encrypted_content"] = serde_json::Value::String(encrypted_content);
    }
    if let Some(content) = content {
        normalized["content"] = serde_json::Value::Array(content);
    }
    // The output index is stable across `added` -> `done` even when the
    // provider item id appears only on the later event. Prefer it whenever it
    // exists so one reasoning item cannot split into two replay slots when its
    // transport id arrives late.
    let key = event
        .get("output_index")
        .and_then(serde_json::Value::as_u64)
        .map(|index| format!("output-index:{index}"))
        .or_else(|| {
            item.get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| utils::request_shape_fingerprint(&normalized));
    Some((key, normalized))
}

fn merge_openai_reasoning_item(current: &mut serde_json::Value, update: serde_json::Value) {
    if let Some(merged) = utils::merge_provider_metadata(Some(current.clone()), Some(update)) {
        *current = merged;
    }
}

fn openai_responses_metadata(
    response_id: Option<String>,
    reasoning_items: impl IntoIterator<Item = serde_json::Value>,
) -> Option<serde_json::Value> {
    let items = reasoning_items.into_iter().collect::<Vec<_>>();
    let mut metadata = serde_json::Map::new();
    if let Some(response_id) = utils::normalize_optional_text(response_id) {
        metadata.insert(
            "response_id".to_owned(),
            serde_json::Value::String(response_id),
        );
    }
    if !items.is_empty() {
        metadata.insert(
            "openai_reasoning_items".to_owned(),
            serde_json::Value::Array(items),
        );
    }
    (!metadata.is_empty()).then_some(serde_json::Value::Object(metadata))
}

#[derive(Clone, Copy, Default)]
struct RequestHeaderContext<'a> {
    responses_api_metadata: Option<&'a ResponsesApiRequestMetadata>,
    prompt_cache_key: Option<&'a str>,
    session_affinity: Option<&'a str>,
    prompt_window_generation: Option<u64>,
    initiator: Option<&'a str>,
    vision_request: bool,
    request_headers: Option<&'a std::collections::BTreeMap<String, String>>,
}

impl<'a> RequestHeaderContext<'a> {
    fn from_request(request: &'a CompletionRequest) -> Self {
        Self {
            responses_api_metadata: request.responses_api_metadata.as_ref(),
            prompt_cache_key: request.prompt_cache_key.as_deref(),
            session_affinity: None,
            prompt_window_generation: request.prompt_window_generation,
            initiator: Some(OpenAiTransport::initiator(request)),
            vision_request: OpenAiTransport::is_vision_request(request),
            request_headers: Some(&request.request_override.headers),
        }
    }

    fn from_chat_request(
        request: &'a CompletionRequest,
        session_affinity: Option<&'a str>,
    ) -> Self {
        Self {
            session_affinity,
            ..Self::from_request(request)
        }
    }

    fn none() -> Self {
        Self::default()
    }

    fn window_id_header(&self) -> Option<String> {
        self.responses_api_metadata
            .map(|metadata| metadata.window_id.clone())
            .or_else(|| {
                self.prompt_cache_key.map(|prompt_cache_key| {
                    format!(
                        "{}:{}",
                        prompt_cache_key,
                        self.prompt_window_generation.unwrap_or_default()
                    )
                })
            })
    }

    fn session_affinity_header(&self) -> Option<&str> {
        self.session_affinity
            .filter(|value| !value.trim().is_empty())
    }

    fn initiator_header(&self) -> &str {
        self.initiator.unwrap_or("agent")
    }
}
