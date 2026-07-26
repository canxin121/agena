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
use agena_runtime_contracts::message::{AttachmentItem, AttachmentKind, AttachmentSource};

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

const CHATGPT_CODEX_ORIGINATOR: &str = crate::RUNTIME_CODEX_ORIGINATOR;
const DEFAULT_COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
const RESPONSES_ADAPTER_KIND: &str = "openai_responses";
const CHAT_COMPLETIONS_ADAPTER_KIND: &str = "openai_chat_completions";
const REALTIME_ADAPTER_KIND: &str = "openai_realtime";

#[derive(Clone)]
#[doc(hidden)]
pub struct OpenAiTransport {
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
pub struct OpenAiResponsesAdapter {
    transport: OpenAiTransport,
}

#[derive(Clone)]
pub struct OpenAiChatCompletionsAdapter {
    transport: OpenAiTransport,
}

#[derive(Clone)]
pub struct OpenAiRealtimeAdapter {
    transport: OpenAiTransport,
    realtime_ws_url: Option<String>,
}

#[derive(Clone)]
struct OpenAiTransportOptions {
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
pub struct OpenAiResponsesAdapterOptions {
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
pub struct OpenAiChatCompletionsAdapterOptions {
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
pub struct OpenAiRealtimeAdapterOptions {
    pub auth_data: Option<Arc<Mutex<AuthData>>>,
    pub models_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub capability_family: CapabilityFamily,
    pub extra_headers: HashMap<String, String>,
    pub realtime_ws_url: Option<String>,
}

impl Default for OpenAiRealtimeAdapterOptions {
    fn default() -> Self {
        Self {
            auth_data: None,
            models_url: None,
            auth_header: "authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            capability_family: CapabilityFamily::OpenAi,
            extra_headers: HashMap::new(),
            realtime_ws_url: None,
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

impl std::ops::Deref for OpenAiRealtimeAdapter {
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

fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    utils::response_id_metadata(response_id)
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
                .filter(|content| !content.is_empty())?;
            let summary = item
                .summary
                .iter()
                .flatten()
                .filter_map(|part| part.text.as_deref())
                .map(|text| {
                    serde_json::json!({
                        "type": "summary_text",
                        "text": text,
                    })
                })
                .collect::<Vec<_>>();
            let content = item
                .content
                .iter()
                .flatten()
                .filter_map(|part| {
                    part.text.as_deref().map(|text| {
                        serde_json::json!({
                            "type": part.kind.as_deref().unwrap_or("reasoning_text"),
                            "text": text,
                        })
                    })
                })
                .collect::<Vec<_>>();
            let mut normalized = serde_json::json!({
                "type": "reasoning",
                "summary": summary,
                "encrypted_content": encrypted_content,
            });
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
        .filter(|content| !content.is_empty())?;
    let summary = item
        .get("summary")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let content = item
        .get("content")
        .and_then(serde_json::Value::as_array)
        .filter(|content| !content.is_empty())
        .cloned();
    let mut normalized = serde_json::json!({
        "type": "reasoning",
        "summary": summary,
        "encrypted_content": encrypted_content,
    });
    if let Some(content) = content {
        normalized["content"] = serde_json::Value::Array(content);
    }
    let key = item
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| utils::request_shape_fingerprint(&normalized));
    Some((key, normalized))
}

fn openai_responses_metadata(
    response_id: Option<String>,
    reasoning_items: impl IntoIterator<Item = serde_json::Value>,
) -> Option<serde_json::Value> {
    let items = reasoning_items.into_iter().collect::<Vec<_>>();
    let mut metadata = serde_json::Map::new();
    if let Some(response_id) = response_id.filter(|value| !value.is_empty()) {
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

#[cfg(test)]
mod tests {
    use super::{
        CapabilityFamily, ManagedCredential, OpenAiChatCompletionsAdapter,
        OpenAiChatCompletionsAdapterOptions, OpenAiResponsesAdapter, OpenAiResponsesAdapterOptions,
        OpenAiResponsesResponse, OpenAiTransport, OpenAiUsage, ProviderImageOperation,
        ProviderImageRequest, RequestHeaderContext, ToolStreamAccumulator, ToolStreamInputKind,
        ToolStreamUpdate, chat_tool_stream_input, openai_reasoning_item_from_event,
        openai_reasoning_items_from_output, openai_responses_metadata, responses_tool_stream_input,
    };
    use crate::provider::{
        ModelRuntime,
        chat_wire::{ChatFunctionCallWire, ChatToolCallWire},
        utils,
    };

    async fn read_http_request_body(stream: &mut tokio::net::TcpStream) -> serde_json::Value {
        use tokio::io::AsyncReadExt as _;

        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        let body_start;
        loop {
            let read = stream.read(&mut buffer).await.expect("read request");
            assert!(read > 0, "request closed before headers");
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                body_start = index + 4;
                break;
            }
        }
        let headers = String::from_utf8_lossy(&bytes[..body_start]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
            })
            .expect("content-length header");
        while bytes.len() - body_start < content_length {
            let read = stream.read(&mut buffer).await.expect("read body");
            assert!(read > 0, "request closed before body");
            bytes.extend_from_slice(&buffer[..read]);
        }
        serde_json::from_slice(&bytes[body_start..body_start + content_length])
            .expect("request JSON")
    }

    fn chat_transport(id: &str, base_url: &str) -> OpenAiTransport {
        OpenAiChatCompletionsAdapter::new_managed_with_options(
            id,
            reqwest::Client::new(),
            ManagedCredential::static_value("test key", "secret"),
            base_url,
            "test-model",
            OpenAiChatCompletionsAdapterOptions {
                capability_family: CapabilityFamily::OpenAiCompatible,
                ..OpenAiChatCompletionsAdapterOptions::default()
            },
        )
        .transport
    }

    #[test]
    fn xai_chat_uses_the_documented_affinity_header_and_usage_extension() {
        let transport = chat_transport("jiuuij", "https://api.x.ai/v1");

        assert!(transport.is_xai_endpoint());
        assert!(transport.supports_chat_stream_usage());
        assert!(!transport.supports_chat_prompt_cache_key());
        let headers = transport.resolved_headers(RequestHeaderContext {
            session_affinity: Some("conversation-123"),
            ..RequestHeaderContext::default()
        });
        assert_eq!(
            headers.get("x-grok-conv-id").map(String::as_str),
            Some("conversation-123")
        );
        assert!(!headers.contains_key("x-session-affinity"));
    }

    #[test]
    fn unknown_chat_compatible_endpoints_omit_unportable_body_extensions() {
        let transport = chat_transport("custom", "https://llm.example.test/v1");

        assert!(!transport.is_xai_endpoint());
        assert!(!transport.supports_chat_stream_usage());
        assert!(!transport.supports_chat_prompt_cache_key());
    }

    #[test]
    fn chat_tool_stream_input_keeps_id_and_index_as_aliases() {
        let input = chat_tool_stream_input(
            "cline",
            ChatToolCallWire {
                index: Some(6),
                id: Some("call_shared".to_string()),
                function: Some(ChatFunctionCallWire {
                    name: Some("tools_call".to_string()),
                    arguments: Some(r#"{"tool":"skills.list","input":{}}"#.to_string()),
                }),
            },
        )
        .expect("valid chat tool chunk");

        let keys: Vec<&str> = input
            .stream_key_candidates
            .iter()
            .map(AsRef::as_ref)
            .collect();
        assert_eq!(keys, vec!["id:call_shared", "idx:6"]);
        assert_eq!(
            input.model_call_id.as_ref().map(AsRef::as_ref),
            Some("call_shared")
        );
        assert_eq!(input.name.as_deref(), Some("tools_call"));
        assert_eq!(
            input.arguments.as_deref(),
            Some(r#"{"tool":"skills.list","input":{}}"#)
        );
        assert_eq!(input.kind, ToolStreamInputKind::Delta);
    }

    #[test]
    fn responses_item_id_never_becomes_the_model_call_id() {
        let events = [
            serde_json::json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "tools_list",
                    "arguments": ""
                }
            }),
            serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "output_index": 0,
                "item_id": "fc_1",
                "delta": "{\"limit\":"
            }),
            serde_json::json!({
                "type": "response.function_call_arguments.done",
                "output_index": 0,
                "item_id": "fc_1",
                "arguments": "{\"limit\":100}"
            }),
            serde_json::json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "tools_list",
                    "arguments": "{\"limit\":100}"
                }
            }),
        ];
        let mut accumulator = ToolStreamAccumulator::new();
        let mut updates = Vec::new();

        for event in events {
            let event = utils::responses_tool_event("openai", &event)
                .expect("valid Responses event")
                .expect("tool event");
            let input = responses_tool_stream_input("openai", event).expect("stream input");
            updates.extend(
                accumulator
                    .ingest("openai", input)
                    .expect("accumulate tool event"),
            );
        }

        assert_eq!(updates.len(), 3);
        for update in &updates {
            match update {
                ToolStreamUpdate::Registered { stream_key, id, .. }
                | ToolStreamUpdate::ArgumentsDelta { stream_key, id, .. }
                | ToolStreamUpdate::ArgumentsSnapshot { stream_key, id, .. } => {
                    assert_eq!(stream_key, "call:call_1");
                    assert_eq!(id.as_deref(), Some("call_1"));
                }
            }
        }
        assert!(updates.iter().all(|update| match update {
            ToolStreamUpdate::Registered { id, .. }
            | ToolStreamUpdate::ArgumentsDelta { id, .. }
            | ToolStreamUpdate::ArgumentsSnapshot { id, .. } => id.as_deref() != Some("fc_1"),
        }));
    }

    #[test]
    fn responses_nonstream_function_call_requires_call_id() {
        let response: OpenAiResponsesResponse = serde_json::from_value(serde_json::json!({
            "output": [{
                "type": "function_call",
                "id": "fc_1",
                "name": "tools_list",
                "arguments": "{}"
            }]
        }))
        .expect("responses payload");

        let error = OpenAiTransport::parse_responses_tool_calls(response.output.as_ref())
            .expect_err("an output item id cannot substitute for call_id");
        assert!(error.to_string().contains("without call_id"));
    }

    #[test]
    fn responses_metadata_preserves_response_id_and_encrypted_reasoning() {
        let response: OpenAiResponsesResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_123",
            "output": [{
                "id": "rs_123",
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": "short summary" }],
                "content": [{ "type": "reasoning_text", "text": "private reasoning" }],
                "encrypted_content": "encrypted-state"
            }]
        }))
        .expect("deserialize Responses payload");
        let metadata = openai_responses_metadata(
            response.id,
            openai_reasoning_items_from_output(response.output.as_deref()),
        )
        .expect("provider metadata");

        assert_eq!(metadata["response_id"], "resp_123");
        assert_eq!(
            metadata["openai_reasoning_items"][0],
            serde_json::json!({
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": "short summary" }],
                "content": [{ "type": "reasoning_text", "text": "private reasoning" }],
                "encrypted_content": "encrypted-state"
            })
        );
    }

    #[test]
    fn responses_stream_reasoning_items_drop_transport_ids_before_replay() {
        let event = serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "id": "rs_transport_123",
                "type": "reasoning",
                "summary": [],
                "content": [{ "type": "reasoning_text", "text": "private reasoning" }],
                "encrypted_content": "encrypted-state"
            }
        });
        let (key, item) = openai_reasoning_item_from_event(&event).expect("reasoning item");

        assert_eq!(key, "rs_transport_123");
        assert!(item.get("id").is_none());
        assert_eq!(item["type"], "reasoning");
        assert_eq!(item["content"][0]["text"], "private reasoning");
        assert_eq!(item["encrypted_content"], "encrypted-state");
    }

    #[test]
    fn codex_compatible_responses_always_request_encrypted_reasoning() {
        let include = OpenAiTransport::responses_include(Vec::new(), None, true)
            .expect("Codex-compatible include list");
        assert_eq!(include, vec!["reasoning.encrypted_content"]);
    }

    #[test]
    fn responses_usage_uses_total_to_disambiguate_separate_reasoning() {
        let raw: OpenAiUsage = serde_json::from_value(serde_json::json!({
            "input_tokens": 100,
            "output_tokens": 20,
            "output_tokens_details": { "reasoning_tokens": 15 },
            "total_tokens": 135,
            "cost_in_usd_ticks": 37_756_000
        }))
        .expect("deserialize compatible Responses usage");

        let usage = OpenAiTransport::map_usage(Some(raw)).expect("mapped usage");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.reasoning_tokens, 15);
        assert!((usage.total_cost - 0.0037756).abs() < 1e-12);
    }

    #[tokio::test]
    async fn direct_image_request_forces_hosted_tool_and_returns_terminal_artifact() {
        use tokio::io::AsyncWriteExt as _;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let (body_tx, body_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let body = read_http_request_body(&mut stream).await;
            body_tx.send(body).expect("record body");
            let payload = serde_json::json!({
                "id": "resp_image_1",
                "model": "gpt-image-route",
                "status": "completed",
                "output": [{
                    "type": "image_generation_call",
                    "result": "iVBORw0KGgo=",
                    "mime_type": "image/png",
                    "revised_prompt": "a revised fixture prompt"
                }],
                "usage": { "input_tokens": 7, "output_tokens": 3, "total_tokens": 10 }
            });
            let response_body = serde_json::to_vec(&payload).expect("response JSON");
            let headers = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                response_body.len()
            );
            stream
                .write_all(headers.as_bytes())
                .await
                .expect("write headers");
            stream
                .write_all(response_body.as_slice())
                .await
                .expect("write body");
        });
        let adapter = OpenAiResponsesAdapter::new_managed_with_options(
            "openai-fixture",
            reqwest::Client::new(),
            ManagedCredential::static_value("test key", "secret"),
            format!("http://{address}/v1"),
            "gpt-image-route",
            OpenAiResponsesAdapterOptions {
                capability_family: CapabilityFamily::OpenAiCompatible,
                ..OpenAiResponsesAdapterOptions::default()
            },
        );
        let response = ModelRuntime::execute_image(
            &adapter,
            &agena_domain::ModelId::new("gpt-image-route"),
            ProviderImageRequest {
                operation: ProviderImageOperation::Generate,
                prompt: "fixture image".to_owned(),
                inputs: Vec::new(),
                options: agena_provider::ProviderHostedImageGenerationConfig {
                    size: Some("1024x1024".to_owned()),
                    ..Default::default()
                },
            },
        )
        .await
        .expect("direct image response");
        server.await.expect("fixture server");
        let body = body_rx.await.expect("captured body");

        assert_eq!(body["tool_choice"], "required");
        assert_eq!(body["tools"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["tools"][0]["type"], "image_generation");
        assert_eq!(body["tools"][0]["size"], "1024x1024");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(response.artifacts.len(), 1);
        assert!(
            response.artifacts[0]
                .uri
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(
            response.revised_prompt.as_deref(),
            Some("a revised fixture prompt")
        );
    }
}
