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
    config::{NativeToolFreshness, ProviderNativeToolKind, ProviderNativeToolRoute},
    error::AppError,
    message::{
        ArtifactRef, AttachmentItem, AttachmentKind, AttachmentSource, Message, MessageUsage,
        OperationBlock, SearchResultItem, StructuredObject, ToolInvocation, ToolOutput,
    },
    model::{
        CapabilitySupport, ModelCapabilities, ModelId, ModelInputModality, ModelMetadata,
        ModelThinkingMode, ModelTokenLimits, ProviderId,
    },
    provider::{
        CapabilityFamily, CompletionFinishReason, CompletionRequest, CompletionResponse,
        CompletionStreamEvent, CompletionToolCall, CompletionUsage, ManagedCredential,
        ModelRuntime, ProviderModel, StreamResumePolicy,
        auth::AuthData,
        chat_wire::{self, ChatCompletionRequest, ChatCompletionResponse, ChatStreamOptions},
        prompt_cache, sse, utils, wire_message,
    },
    role::Role,
};

mod openai_models;
mod openai_native_tools;
mod openai_requests;
mod openai_response_builders;
mod openai_response_types;
mod openai_runtime;
mod openai_setup;
mod openai_wire;

use self::openai_models::*;
use self::openai_native_tools::*;
use self::openai_response_types::*;
use self::openai_wire::*;

const CHATGPT_CODEX_ORIGINATOR: &str = crate::provider::CODEX_ORIGINATOR;
const DEFAULT_COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
const ADAPTER_KIND: &str = "openai";

#[derive(Clone)]
pub struct OpenAiAdapter {
    id: String,
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    backend: OpenAiBackend,
    auth_data: Option<Arc<Mutex<AuthData>>>,
    api_mode: OpenAiApiMode,
    api_mode_explicit: bool,
    profile: OpenAiProfile,
    models_url: Option<String>,
    auth_header: String,
    auth_scheme: Option<String>,
    capability_family: CapabilityFamily,
    extra_headers: HashMap<String, String>,
    stream_mode: OpenAiStreamMode,
    realtime_ws_url: Option<String>,
    top_level_prompt_cache_override: Option<bool>,
}

#[derive(Clone)]
pub struct OpenAiAdapterOptions {
    pub backend: OpenAiBackend,
    pub auth_data: Option<Arc<Mutex<AuthData>>>,
    pub api_mode: OpenAiApiMode,
    pub api_mode_explicit: bool,
    pub profile: OpenAiProfile,
    pub models_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub capability_family: CapabilityFamily,
    pub extra_headers: HashMap<String, String>,
    pub stream_mode: OpenAiStreamMode,
    pub realtime_ws_url: Option<String>,
    pub top_level_prompt_cache_override: Option<bool>,
}

impl Default for OpenAiAdapterOptions {
    fn default() -> Self {
        Self {
            backend: OpenAiBackend::Api,
            auth_data: None,
            api_mode: OpenAiApiMode::Responses,
            api_mode_explicit: false,
            profile: OpenAiProfile::Standard,
            models_url: None,
            auth_header: "authorization".to_owned(),
            auth_scheme: Some("Bearer".to_owned()),
            capability_family: CapabilityFamily::OpenAi,
            extra_headers: HashMap::new(),
            stream_mode: OpenAiStreamMode::Sse,
            realtime_ws_url: None,
            top_level_prompt_cache_override: None,
        }
    }
}

#[derive(Debug)]
struct OpenAiResponsesToolPlan {
    tools: Vec<serde_json::Value>,
    include: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiApiMode {
    Responses,
    Chat,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiBackend {
    Api,
    ChatgptCodex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiProfile {
    Standard,
    GithubCopilot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiStreamMode {
    Sse,
    RealtimeWebSocket,
}

fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    utils::response_id_metadata(response_id)
}

#[derive(Clone, Copy, Default)]
struct RequestHeaderContext<'a> {
    responses_api_metadata: Option<&'a crate::provider::ResponsesApiRequestMetadata>,
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
            initiator: Some(OpenAiAdapter::initiator(request)),
            vision_request: OpenAiAdapter::is_vision_request(request),
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
    use super::{ToolStreamInputKind, chat_tool_stream_input};
    use crate::provider::chat_wire::{ChatFunctionCallWire, ChatToolCallWire};

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
}
