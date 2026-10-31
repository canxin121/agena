pub(super) use agena_domain::ModelMetadata;
use agena_domain::Role;
use agena_domain::*;
use agena_provider::{
    AnthropicBinarySource, AnthropicMessage, AnthropicTextBlock, AnthropicThinkingBlockState,
    AnthropicUsage, CompletionFinishReason, CompletionToolCall, CompletionUsage,
    StreamResumePolicy, anthropic_budget_effort, anthropic_budget_for_effort,
    anthropic_thinking_budget, anthropic_thinking_metadata, anthropic_wire_tool_name,
    json_value_to_string, map_anthropic_usage, merge_anthropic_cache_creation_usage,
    merge_provider_metadata,
};
use agena_provider_bedrock_auth::AwsCredentials as Credentials;
use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use crate::{
    ProviderError,
    provider::{
        CompletionResponse, ModelRuntime,
        chat_wire::{
            ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ChatStreamOptions,
            ChatToolCallWire, ChatUsage,
        },
        prompt_cache, sse, utils, wire_message,
    },
};
use agena_domain::ThinkingRequest;
use agena_provider::CompletionRequest;
use agena_provider::CompletionStreamEvent;
use agena_runtime_contracts::part::{AttachmentItem, AttachmentKind};

mod bedrock_adapter;

const PROVIDER_ID: &str = "amazon-bedrock";
const BEDROCK_ANTHROPIC_VERSION: &str = "bedrock-2023-05-31";
const JSON_CONTENT_TYPE: &str = "application/json";
const EVENTSTREAM_CONTENT_TYPE: &str = "application/vnd.amazon.eventstream";
const ADAPTER_KIND: &str = "amazon_bedrock";

const CROSS_REGION_PREFIXES: &[&str] = &["global.", "us.", "eu.", "jp.", "apac.", "au."];
const US_MODELS: &[&str] = &[
    "nova-micro",
    "nova-lite",
    "nova-pro",
    "nova-premier",
    "nova-2",
    "claude",
    "deepseek",
];
const EU_REGIONS: &[&str] = &[
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-north-1",
    "eu-central-1",
    "eu-south-1",
    "eu-south-2",
];
const EU_MODELS: &[&str] = &[
    "claude",
    "nova-lite",
    "nova-micro",
    "nova-pro",
    "llama3",
    "pixtral",
];
const AP_MODELS: &[&str] = &["claude", "nova-lite", "nova-micro", "nova-pro"];
const AU_MODELS: &[&str] = &["anthropic.claude-sonnet-4-5", "anthropic.claude-haiku"];

#[derive(Clone)]
enum BedrockAuthMode {
    SigV4 {
        profile: Option<String>,
        static_credentials: Option<Credentials>,
    },
}

struct Sigv4Request<'a> {
    method: reqwest::Method,
    url: String,
    body: Option<bytes::Bytes>,
    headers: Vec<(String, String)>,
    body_debug: Option<&'a utils::PreparedJsonBody>,
}

#[derive(Clone)]
/// Adapter for Amazon Bedrock.
pub struct AmazonBedrockAdapter {
    client: reqwest::Client,
    base_url: String,
    default_model: ModelId,
    region: String,
    auth_mode: BedrockAuthMode,
    resolved_sigv4_shape: Arc<Mutex<Option<agena_provider::PromptCacheShape>>>,
}

#[async_trait]
impl ModelRuntime for AmazonBedrockAdapter {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<agena_provider::CapabilityFamily> {
        Some(agena_provider::CapabilityFamily::Bedrock)
    }

    fn validate_media_inputs_for_adapter(
        &self,
        _: Option<&agena_domain::AdapterId>,
        model: &ModelId,
        attachments: &[AttachmentItem],
    ) -> Result<(), ProviderError> {
        use agena_runtime_provider::provider::media_input::{MediaProtocol, validate};
        validate(
            if Self::is_native_anthropic_model(model.as_ref()) {
                MediaProtocol::Anthropic
            } else {
                MediaProtocol::OpenAiChat
            },
            attachments,
        )
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        StreamResumePolicy::ReplaySafePrefix
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        let mut shape = agena_provider::PromptCacheShape::from_fields(
            PROVIDER_ID,
            [
                ("base_url", self.base_url.clone()),
                ("region", self.region.clone()),
                (
                    "native_anthropic_transport",
                    (matches!(&self.auth_mode, BedrockAuthMode::SigV4 { .. })
                        && Self::is_native_anthropic_model(model.as_ref()))
                    .to_string(),
                ),
            ],
        );

        match &self.auth_mode {
            BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            } => {
                shape.insert_string("auth_mode", "sigv4");
                if let Some(profile) = Self::prompt_cache_sigv4_profile(profile.as_deref()) {
                    shape.insert_string("sigv4.profile", profile);
                }
                if let Some(static_credentials) = static_credentials.as_ref() {
                    shape.extend_prefixed(
                        "sigv4.static",
                        &Self::prompt_cache_sigv4_static_credentials_shape(static_credentials),
                    );
                } else if let Some(env_shape) = Self::prompt_cache_sigv4_env_shape() {
                    shape.extend_prefixed("sigv4.env", &env_shape);
                }
                if let Some(runtime_shape) = self
                    .resolved_sigv4_shape
                    .lock()
                    .ok()
                    .and_then(|shape| shape.clone())
                {
                    shape.extend_prefixed("sigv4.runtime", &runtime_shape);
                }
            }
        }

        Some(shape)
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        match &self.auth_mode {
            BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            } => {
                self.list_models_sigv4(profile.as_deref(), static_credentials.as_ref())
                    .await
            }
        }
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let model = self.resolve_model(request.model.as_ref());
        let request = CompletionRequest {
            model: ModelId::new(model),
            ..request
        };

        match &self.auth_mode {
            BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            } => {
                self.complete_sigv4(profile.as_deref(), static_credentials.as_ref(), request)
                    .await
            }
        }
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, ProviderError>> + Send>>,
        ProviderError,
    > {
        let model = self.resolve_model(request.model.as_ref());
        let request = CompletionRequest {
            model: ModelId::new(model),
            ..request
        };

        match &self.auth_mode {
            BedrockAuthMode::SigV4 {
                profile,
                static_credentials,
            } => {
                self.complete_stream_sigv4(profile.as_deref(), static_credentials.as_ref(), request)
                    .await
            }
        }
    }
}

fn strip_cross_region_prefix(model: &str) -> &str {
    CROSS_REGION_PREFIXES
        .iter()
        .find_map(|prefix| model.strip_prefix(prefix))
        .unwrap_or(model)
}

fn prefix_bedrock_model(region: &str, model: &str) -> String {
    if model.is_empty() || has_cross_region_prefix(model) || is_bedrock_resource_name(model) {
        return model.to_owned();
    }

    let region = region.to_ascii_lowercase();
    let normalized_model = model.to_ascii_lowercase();

    if region.starts_with("us-")
        && !region.starts_with("us-gov")
        && contains_any(normalized_model.as_str(), US_MODELS)
    {
        return format!("us.{model}");
    }

    if EU_REGIONS.contains(&region.as_str()) && contains_any(normalized_model.as_str(), EU_MODELS) {
        return format!("eu.{model}");
    }

    if region.starts_with("ap-") {
        let is_au_region = region == "ap-southeast-2" || region == "ap-southeast-4";
        if is_au_region && contains_any(normalized_model.as_str(), AU_MODELS) {
            return format!("au.{model}");
        }

        if region == "ap-northeast-1" && contains_any(normalized_model.as_str(), AP_MODELS) {
            return format!("jp.{model}");
        }

        if contains_any(normalized_model.as_str(), AP_MODELS) {
            return format!("apac.{model}");
        }
    }

    model.to_owned()
}

fn has_cross_region_prefix(model: &str) -> bool {
    CROSS_REGION_PREFIXES
        .iter()
        .any(|prefix| model.starts_with(prefix))
}

fn is_bedrock_resource_name(model: &str) -> bool {
    model.starts_with("arn:")
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

fn response_id_metadata(response_id: Option<String>) -> Option<serde_json::Value> {
    utils::response_id_metadata(response_id)
}

fn bedrock_anthropic_metadata(
    response_id: Option<String>,
    blocks: &[AnthropicTextBlock],
) -> Option<serde_json::Value> {
    merge_provider_metadata(
        response_id_metadata(response_id),
        anthropic_thinking_metadata(blocks),
    )
}

// Bedrock treats an explicitly supplied output_tokens_details object as a
// snapshot (including an empty object); Anthropic SSE merges its inner fields.
fn merge_bedrock_anthropic_usage(
    current: Option<AnthropicUsage>,
    update: AnthropicUsage,
) -> AnthropicUsage {
    let Some(current) = current else {
        return update;
    };

    AnthropicUsage {
        input_tokens: update.input_tokens.or(current.input_tokens),
        output_tokens: update.output_tokens.or(current.output_tokens),
        output_tokens_details: update
            .output_tokens_details
            .or(current.output_tokens_details),
        cache_creation_input_tokens: update
            .cache_creation_input_tokens
            .or(current.cache_creation_input_tokens),
        cache_read_input_tokens: update
            .cache_read_input_tokens
            .or(current.cache_read_input_tokens),
        cache_creation: merge_anthropic_cache_creation_usage(
            current.cache_creation,
            update.cache_creation,
        ),
    }
}

fn bedrock_anthropic_effort_for_budget(
    model: &str,
    budget_tokens: u32,
) -> Option<agena_domain::ReasoningEffort> {
    bedrock_anthropic_model_requires_adaptive_thinking(model)
        .then(|| anthropic_budget_effort(budget_tokens))
}

fn bedrock_anthropic_display(
    model: &str,
    explicit: Option<agena_domain::ThinkingDisplay>,
) -> Option<&'static str> {
    match explicit {
        Some(agena_domain::ThinkingDisplay::Summarized) => Some("summarized"),
        Some(agena_domain::ThinkingDisplay::Omitted) => Some("omitted"),
        None => bedrock_anthropic_model_defaults_to_omitted_thinking(model).then_some("summarized"),
    }
}

fn bedrock_anthropic_model_requires_adaptive_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-fable-5")
        || normalized.contains("claude-mythos-5")
        || normalized.contains("claude-mythos-preview")
        || normalized.contains("claude-opus-4-7")
        || normalized.contains("claude-opus-4.7")
}

fn bedrock_anthropic_model_defaults_to_omitted_thinking(model: &str) -> bool {
    bedrock_anthropic_model_requires_adaptive_thinking(model)
}

fn bedrock_anthropic_model_supports_effort(model: &str) -> bool {
    AmazonBedrockAdapter::anthropic_model_uses_adaptive_thinking(model)
        || model.to_ascii_lowercase().contains("claude-opus-4-5")
        || model.to_ascii_lowercase().contains("claude-opus-4.5")
}

fn bedrock_anthropic_model_supports_max_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-opus-4-6") || normalized.contains("claude-opus-4.6")
}

fn bedrock_anthropic_wire_effort(
    model: &str,
    effort: agena_domain::ReasoningEffort,
) -> &'static str {
    match effort {
        agena_domain::ReasoningEffort::Minimal | agena_domain::ReasoningEffort::Low => "low",
        agena_domain::ReasoningEffort::Medium => "medium",
        agena_domain::ReasoningEffort::High => "high",
        agena_domain::ReasoningEffort::Xhigh | agena_domain::ReasoningEffort::Max
            if bedrock_anthropic_model_supports_max_effort(model) =>
        {
            "max"
        }
        agena_domain::ReasoningEffort::Xhigh | agena_domain::ReasoningEffort::Max => "high",
    }
}

fn bedrock_anthropic_enabled_thinking(
    budget_tokens: u32,
    max_output_tokens: u32,
) -> Option<BedrockAnthropicThinkingConfig> {
    anthropic_thinking_budget(budget_tokens, max_output_tokens)
        .map(|budget_tokens| BedrockAnthropicThinkingConfig::Enabled { budget_tokens })
}

#[derive(Debug, Default)]
struct BedrockAnthropicThinkingParts {
    thinking: Option<BedrockAnthropicThinkingConfig>,
    output_config: Option<BedrockAnthropicOutputConfig>,
    anthropic_beta: Option<Vec<&'static str>>,
}

impl BedrockAnthropicThinkingParts {
    fn include_thinking(&self) -> bool {
        self.thinking
            .as_ref()
            .is_some_and(|thinking| !matches!(thinking, BedrockAnthropicThinkingConfig::Disabled))
    }

    fn set_effort(&mut self, model: &str, effort: Option<agena_domain::ReasoningEffort>) {
        let Some(effort) = effort.filter(|_| bedrock_anthropic_model_supports_effort(model)) else {
            return;
        };
        self.output_config = Some(BedrockAnthropicOutputConfig {
            effort: bedrock_anthropic_wire_effort(model, effort),
        });
        let normalized = model.to_ascii_lowercase();
        if normalized.contains("claude-opus-4-5") || normalized.contains("claude-opus-4.5") {
            self.anthropic_beta = Some(vec!["effort-2025-11-24"]);
        }
    }
}

fn bedrock_anthropic_thinking_parts(
    model: &str,
    thinking: Option<&ThinkingRequest>,
    max_output_tokens: u32,
) -> BedrockAnthropicThinkingParts {
    let Some(thinking) = thinking else {
        return BedrockAnthropicThinkingParts::default();
    };
    match thinking {
        ThinkingRequest::Disabled if bedrock_anthropic_model_requires_adaptive_thinking(model) => {
            BedrockAnthropicThinkingParts::default()
        }
        ThinkingRequest::Disabled => BedrockAnthropicThinkingParts {
            thinking: Some(BedrockAnthropicThinkingConfig::Disabled),
            ..BedrockAnthropicThinkingParts::default()
        },
        ThinkingRequest::Budget { budget_tokens } => {
            if let Some(effort) = bedrock_anthropic_effort_for_budget(model, *budget_tokens) {
                let mut parts = BedrockAnthropicThinkingParts {
                    thinking: Some(BedrockAnthropicThinkingConfig::Adaptive {
                        display: bedrock_anthropic_display(model, None),
                    }),
                    ..BedrockAnthropicThinkingParts::default()
                };
                parts.set_effort(model, Some(effort));
                parts
            } else {
                BedrockAnthropicThinkingParts {
                    thinking: bedrock_anthropic_enabled_thinking(*budget_tokens, max_output_tokens),
                    ..BedrockAnthropicThinkingParts::default()
                }
            }
        }
        ThinkingRequest::Adaptive { effort, display }
            if AmazonBedrockAdapter::anthropic_model_uses_adaptive_thinking(model) =>
        {
            let mut parts = BedrockAnthropicThinkingParts {
                thinking: Some(BedrockAnthropicThinkingConfig::Adaptive {
                    display: bedrock_anthropic_display(model, *display),
                }),
                ..BedrockAnthropicThinkingParts::default()
            };
            parts.set_effort(model, *effort);
            parts
        }
        ThinkingRequest::Adaptive { effort, .. } => {
            let mut parts = BedrockAnthropicThinkingParts {
                thinking: bedrock_anthropic_enabled_thinking(
                    anthropic_budget_for_effort(
                        effort.unwrap_or(agena_domain::ReasoningEffort::High),
                    ),
                    max_output_tokens,
                ),
                ..BedrockAnthropicThinkingParts::default()
            };
            parts.set_effort(model, *effort);
            parts
        }
        ThinkingRequest::Effort { effort }
            if AmazonBedrockAdapter::anthropic_model_uses_adaptive_thinking(model) =>
        {
            let mut parts = BedrockAnthropicThinkingParts {
                thinking: Some(BedrockAnthropicThinkingConfig::Adaptive {
                    display: bedrock_anthropic_display(model, None),
                }),
                ..BedrockAnthropicThinkingParts::default()
            };
            parts.set_effort(model, Some(*effort));
            parts
        }
        ThinkingRequest::Effort { effort } => {
            let mut parts = BedrockAnthropicThinkingParts {
                thinking: bedrock_anthropic_enabled_thinking(
                    anthropic_budget_for_effort(*effort),
                    max_output_tokens,
                ),
                ..BedrockAnthropicThinkingParts::default()
            };
            parts.set_effort(model, Some(*effort));
            parts
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAiCompatibleModelList {
    Object {
        #[serde(default)]
        data: Vec<OpenAiCompatibleModel>,
    },
    Array(Vec<OpenAiCompatibleModel>),
}

#[derive(Debug, Deserialize)]
struct OpenAiCompatibleModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct BedrockAnthropicMessagesRequest {
    anthropic_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    anthropic_beta: Option<Vec<&'static str>>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<AnthropicTextBlock>>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<BedrockAnthropicToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<BedrockAnthropicThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<BedrockAnthropicOutputConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    stop_sequences: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BedrockAnthropicOutputConfig {
    effort: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BedrockAnthropicThinkingConfig {
    Disabled,
    Enabled {
        budget_tokens: u32,
    },
    Adaptive {
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<&'static str>,
    },
}

#[derive(Debug, Serialize)]
struct BedrockAnthropicToolDefinition {
    name: String,
    description: String,
    input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_control: Option<prompt_cache::PromptCacheControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    eager_input_streaming: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct BedrockAnthropicMessagesResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bedrock_content")]
    content: Vec<AnthropicTextBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

// A complete Bedrock response requires a block type. AnthropicTextBlock also
// represents partial SSE starts, where that field may be absent. Keep this
// validation at Bedrock's response/persisted-input boundary while sharing every
// content field, constructor, and serialized representation.
#[derive(Deserialize)]
struct BedrockResponseContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    block: AnthropicTextBlock,
}

impl BedrockResponseContentBlock {
    fn into_block(self) -> AnthropicTextBlock {
        AnthropicTextBlock {
            kind: self.kind,
            ..self.block
        }
    }
}

fn deserialize_bedrock_content<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<AnthropicTextBlock>, D::Error> {
    Vec::<BedrockResponseContentBlock>::deserialize(deserializer).map(|blocks| {
        blocks
            .into_iter()
            .map(BedrockResponseContentBlock::into_block)
            .collect()
    })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BedrockAnthropicStreamEvent {
    MessageStart {
        #[serde(default)]
        message: BedrockAnthropicStreamMessage,
    },
    ContentBlockStart {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        content_block: BedrockAnthropicStreamContentBlock,
    },
    ContentBlockDelta {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        delta: BedrockAnthropicStreamDelta,
    },
    ContentBlockStop {
        #[serde(default)]
        index: Option<usize>,
    },
    MessageDelta {
        #[serde(default)]
        delta: BedrockAnthropicStreamMessageDelta,
        #[serde(default)]
        usage: Option<AnthropicUsage>,
        #[serde(default)]
        message: Option<BedrockAnthropicStreamMessage>,
    },
    MessageStop {
        #[serde(default)]
        usage: Option<AnthropicUsage>,
        #[serde(default)]
        message: Option<BedrockAnthropicStreamMessage>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicStreamContentBlock {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<Value>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    data: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicStreamDelta {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicStreamMessageDelta {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicStreamMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Default)]
struct BedrockAnthropicToolCallState {
    id: String,
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bedrock_request_keeps_wrapper_and_shared_content_wire_shapes() {
        let request = BedrockAnthropicMessagesRequest {
            anthropic_version: BEDROCK_ANTHROPIC_VERSION.into(),
            anthropic_beta: None,
            max_tokens: 2048,
            system: None,
            tools: None,
            thinking: None,
            output_config: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: vec![],
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: vec![
                    AnthropicTextBlock::text("hello"),
                    AnthropicTextBlock::image(AnthropicBinarySource::base64("image/png", "image")),
                    AnthropicTextBlock::document(AnthropicBinarySource::base64(
                        "application/pdf",
                        "pdf",
                    )),
                    AnthropicTextBlock::tool_use("call", "tool", "[]"),
                    AnthropicTextBlock::tool_result("call", r#"{"ok":true}"#),
                ],
            }],
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({
                "anthropic_version": "bedrock-2023-05-31", "max_tokens": 2048,
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": "hello"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "image"}},
                    {"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": "pdf"}},
                    {"type": "tool_use", "id": "call", "name": "tool", "input": {}},
                    {"type": "tool_result", "tool_use_id": "call", "content": r#"{"ok":true}"#}
                ]}]
            })
        );
    }

    #[test]
    fn bedrock_content_requires_type_but_preserves_empty_type_and_optional_fields() {
        for value in [
            json!({"content": [{"text": "missing type"}]}),
            json!({"content": [{"type": null}]}),
        ] {
            assert!(serde_json::from_value::<BedrockAnthropicMessagesResponse>(value).is_err());
        }
        let response: BedrockAnthropicMessagesResponse = serde_json::from_value(json!({
            "content": [{"type": "", "text": null}, {"type": "thinking", "signature": "signed"}]
        }))
        .unwrap();
        assert_eq!(response.content[0].kind, "");
        assert_eq!(
            serde_json::to_value(&response.content[0]).unwrap(),
            json!({"type": ""})
        );
        assert_eq!(
            bedrock_anthropic_metadata(Some(" response ".into()), &response.content),
            Some(json!({
                "response_id": "response", "anthropic_thinking_blocks": [
                    {"type": "thinking", "thinking": "", "signature": "signed"}
                ]
            }))
        );
    }

    #[test]
    fn bedrock_keeps_platform_specific_thinking_eligibility_and_effort_limits() {
        let disabled = bedrock_anthropic_thinking_parts(
            "claude-opus-4-7",
            Some(&ThinkingRequest::Disabled),
            32000,
        );
        assert!(disabled.thinking.is_none());
        assert!(
            agena_provider::anthropic_thinking_parts(
                "claude-opus-4-7",
                Some(&ThinkingRequest::Disabled),
                32000
            )
            .thinking
            .is_some()
        );
        let effort = bedrock_anthropic_thinking_parts(
            "claude-opus-4-7",
            Some(&ThinkingRequest::Effort {
                effort: ReasoningEffort::Max,
            }),
            32000,
        );
        assert_eq!(
            serde_json::to_value(effort.output_config).unwrap(),
            json!({"effort": "high"})
        );
        let opus45 = bedrock_anthropic_thinking_parts(
            "claude-opus-4-5",
            Some(&ThinkingRequest::Effort {
                effort: ReasoningEffort::High,
            }),
            8192,
        );
        assert_eq!(
            serde_json::to_value(opus45.thinking).unwrap(),
            json!({"type": "enabled", "budget_tokens": 8191})
        );
        assert_eq!(opus45.anthropic_beta, Some(vec!["effort-2025-11-24"]));
        assert!(bedrock_anthropic_enabled_thinking(4000, 1024).is_none());
        assert_eq!(
            serde_json::to_value(bedrock_anthropic_enabled_thinking(0, 1025)).unwrap(),
            json!({"type": "enabled", "budget_tokens": 1024})
        );
    }

    #[test]
    fn bedrock_usage_preserves_atomic_details_and_merges_cache_counters() {
        let current: AnthropicUsage = serde_json::from_value(json!({
            "output_tokens": 20, "output_tokens_details": {"thinking_tokens": 5},
            "cache_creation": {"ephemeral_1h_input_tokens": 9}
        }))
        .unwrap();
        let update: AnthropicUsage = serde_json::from_value(json!({
            "output_tokens_details": {}, "cache_creation": {"ephemeral_5m_input_tokens": 3}
        }))
        .unwrap();
        let usage = map_anthropic_usage(merge_bedrock_anthropic_usage(Some(current), update));
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.reasoning_tokens, 0);
        assert_eq!(usage.cache_write_tokens, 12);
        assert_eq!(usage.cache_write_1h_tokens, 9);
        assert_eq!(usage.cache_write_5m_tokens, 3);
    }
}
