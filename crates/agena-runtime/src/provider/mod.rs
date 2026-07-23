pub(crate) mod auth;
mod credential;

mod amazon_bedrock;
mod anthropic;
mod cataloged_models;
mod chat_wire;
mod core;
mod gemini;
mod gitlab;
mod multi_adapter;
mod ollama;
mod openai;
mod prompt_tool_transport;
mod registry;
mod tool_mode;
mod utils;
mod wire_message;

use agena_provider::CompletionResponse;
pub(crate) use agena_provider::{
    self as copilot_models, self as prompt_cache, self as protocol_ids, self as tool_stream,
};
pub(crate) use agena_runtime as sse;
pub(crate) use utils::with_request_cancellation;

pub(crate) use crate::model_catalog::catalog_decoration_source;
pub(crate) use agena_domain::{Model, ModelId, ModelSpeedMode, ModelThinkingMode};
pub(crate) use amazon_bedrock::AmazonBedrockAdapter;
pub(crate) use anthropic::{AnthropicAdapter, AnthropicAdapterOptions};
pub(crate) use cataloged_models::CatalogedModelsProvider;
pub(crate) use core::ModelRuntime;
pub(crate) use credential::{
    ManagedCredential, parse_sap_ai_core_service_key, should_retry_credential,
};
pub(crate) use gemini::{GeminiAdapter, GeminiAdapterOptions};
pub(crate) use gitlab::GitlabProvider;
pub(crate) use gitlab::{
    default_ai_gateway_headers as default_gitlab_ai_gateway_headers,
    default_feature_flags as default_gitlab_feature_flags,
};
pub(crate) use multi_adapter::MultiAdapterProvider;
pub(crate) use multi_adapter::ProviderModelRoute;
pub(crate) use ollama::OllamaAdapter;
pub(crate) use openai::{
    OpenAiChatCompletionsAdapter, OpenAiChatCompletionsAdapterOptions, OpenAiRealtimeAdapter,
    OpenAiRealtimeAdapterOptions, OpenAiResponsesAdapter, OpenAiResponsesAdapterOptions,
};
pub(crate) use registry::ProviderRegistry;
pub(crate) use wire_message::{
    WirePart as ProjectedSessionPart, project_completion_input,
    project_operation_output as project_session_tool_result_output,
    project_persisted as project_session_parts,
    project_persisted_text_lossy as project_session_text_lossy,
};
