pub(super) use agena_domain::ModelMetadata;
use agena_domain::Role;
use agena_domain::*;
use agena_provider::{
    CompletionFinishReason, CompletionToolCall, CompletionUsage, StreamResumePolicy,
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
    body: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
    body_debug: Option<&'a Value>,
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
    response_id.map(|response_id| serde_json::json!({ "response_id": response_id }))
}

fn bedrock_anthropic_metadata(
    response_id: Option<String>,
    blocks: &[BedrockAnthropicTextBlock],
) -> Option<serde_json::Value> {
    let thinking_blocks = blocks
        .iter()
        .filter_map(|block| match block.kind.as_str() {
            "thinking"
                if block
                    .signature
                    .as_deref()
                    .is_some_and(|signature| !signature.is_empty()) =>
            {
                Some(serde_json::json!({
                    "type": "thinking",
                    "thinking": block.thinking.as_deref().unwrap_or_default(),
                    "signature": block.signature.as_deref().unwrap_or_default(),
                }))
            }
            "redacted_thinking" if block.data.as_deref().is_some_and(|data| !data.is_empty()) => {
                Some(serde_json::json!({
                    "type": "redacted_thinking",
                    "data": block.data.as_deref().unwrap_or_default(),
                }))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut metadata = serde_json::Map::new();
    if let Some(response_id) = response_id.filter(|value| !value.is_empty()) {
        metadata.insert("response_id".to_owned(), Value::String(response_id));
    }
    if !thinking_blocks.is_empty() {
        metadata.insert(
            "anthropic_thinking_blocks".to_owned(),
            Value::Array(thinking_blocks),
        );
    }
    (!metadata.is_empty()).then_some(Value::Object(metadata))
}

fn parse_json_or_object(raw: String) -> Value {
    match serde_json::from_str::<Value>(&raw) {
        Ok(value @ Value::Object(_)) => value,
        _ => Value::Object(Default::default()),
    }
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn bedrock_wire_tool_name(name: &str) -> String {
    name.to_owned()
}

fn map_bedrock_anthropic_usage(usage: BedrockAnthropicUsage) -> CompletionUsage {
    let cache_write_tokens = usage.cache_creation_input_tokens.unwrap_or_else(|| {
        usage
            .cache_creation
            .as_ref()
            .map(BedrockAnthropicCacheCreationUsage::total_input_tokens)
            .unwrap_or_default()
    });

    let reasoning_tokens = usage
        .output_tokens_details
        .as_ref()
        .and_then(|details| details.thinking_tokens)
        .unwrap_or_default();
    let output_tokens = usage.output_tokens.unwrap_or_default();

    let cache_write_5m_tokens = usage
        .cache_creation
        .as_ref()
        .and_then(|value| value.ephemeral_5m_input_tokens)
        .unwrap_or_default();
    let cache_write_1h_tokens = usage
        .cache_creation
        .as_ref()
        .and_then(|value| value.ephemeral_1h_input_tokens)
        .unwrap_or_default();
    CompletionUsage {
        requests: 1,
        input_tokens: usage.input_tokens.unwrap_or_default(),
        output_tokens: output_tokens.saturating_sub(reasoning_tokens),
        reasoning_tokens,
        cache_write_tokens,
        cache_write_5m_tokens,
        cache_write_1h_tokens,
        cache_read_tokens: usage.cache_read_input_tokens.unwrap_or_default(),
        ..CompletionUsage::default()
    }
}

fn merge_bedrock_anthropic_usage(
    current: Option<BedrockAnthropicUsage>,
    update: BedrockAnthropicUsage,
) -> BedrockAnthropicUsage {
    let Some(current) = current else {
        return update;
    };

    BedrockAnthropicUsage {
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
        cache_creation: merge_bedrock_anthropic_cache_creation_usage(
            current.cache_creation,
            update.cache_creation,
        ),
    }
}

fn merge_bedrock_anthropic_cache_creation_usage(
    current: Option<BedrockAnthropicCacheCreationUsage>,
    update: Option<BedrockAnthropicCacheCreationUsage>,
) -> Option<BedrockAnthropicCacheCreationUsage> {
    match (current, update) {
        (Some(current), Some(update)) => Some(BedrockAnthropicCacheCreationUsage {
            ephemeral_1h_input_tokens: update
                .ephemeral_1h_input_tokens
                .or(current.ephemeral_1h_input_tokens),
            ephemeral_5m_input_tokens: update
                .ephemeral_5m_input_tokens
                .or(current.ephemeral_5m_input_tokens),
        }),
        (None, Some(update)) => Some(update),
        (Some(current), None) => Some(current),
        (None, None) => None,
    }
}

fn bedrock_anthropic_budget_for_effort(effort: agena_domain::ReasoningEffort) -> u32 {
    match effort {
        agena_domain::ReasoningEffort::Minimal => 1_024,
        agena_domain::ReasoningEffort::Low => 4_000,
        agena_domain::ReasoningEffort::Medium => 10_000,
        agena_domain::ReasoningEffort::High => 16_000,
        agena_domain::ReasoningEffort::Xhigh | agena_domain::ReasoningEffort::Max => 31_999,
    }
}

fn bedrock_anthropic_effort_for_budget(
    model: &str,
    budget_tokens: u32,
) -> Option<agena_domain::ReasoningEffort> {
    if !bedrock_anthropic_model_requires_adaptive_thinking(model) {
        return None;
    }

    Some(if budget_tokens <= 4_000 {
        agena_domain::ReasoningEffort::Low
    } else if budget_tokens <= 10_000 {
        agena_domain::ReasoningEffort::Medium
    } else if budget_tokens <= 16_000 {
        agena_domain::ReasoningEffort::High
    } else if budget_tokens < 31_999 {
        agena_domain::ReasoningEffort::Xhigh
    } else {
        agena_domain::ReasoningEffort::Max
    })
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
    const MIN_THINKING_BUDGET: u32 = 1_024;
    if max_output_tokens <= MIN_THINKING_BUDGET {
        return None;
    }
    Some(BedrockAnthropicThinkingConfig::Enabled {
        budget_tokens: budget_tokens
            .max(MIN_THINKING_BUDGET)
            .min(max_output_tokens - 1),
    })
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
                    bedrock_anthropic_budget_for_effort(
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
                    bedrock_anthropic_budget_for_effort(*effort),
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
    system: Option<Vec<BedrockAnthropicTextBlock>>,
    messages: Vec<BedrockAnthropicMessage>,
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

#[derive(Debug, Serialize)]
struct BedrockAnthropicMessage {
    role: String,
    content: Vec<BedrockAnthropicTextBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BedrockAnthropicTextBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<BedrockAnthropicBinarySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_control: Option<prompt_cache::PromptCacheControl>,
}

impl BedrockAnthropicTextBlock {
    fn text(text: impl Into<String>) -> Self {
        Self {
            kind: "text".to_owned(),
            text: Some(text.into()),
            thinking: None,
            signature: None,
            data: None,
            source: None,
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    fn image(source: BedrockAnthropicBinarySource) -> Self {
        Self {
            kind: "image".to_owned(),
            text: None,
            thinking: None,
            signature: None,
            data: None,
            source: Some(source),
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    fn document(source: BedrockAnthropicBinarySource) -> Self {
        Self {
            kind: "document".to_owned(),
            text: None,
            thinking: None,
            signature: None,
            data: None,
            source: Some(source),
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input_json: impl Into<String>,
    ) -> Self {
        Self {
            kind: "tool_use".to_owned(),
            text: None,
            thinking: None,
            signature: None,
            data: None,
            source: None,
            id: Some(id.into()),
            name: Some(name.into()),
            input: Some(parse_json_or_object(input_json.into())),
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            kind: "tool_result".to_owned(),
            text: None,
            thinking: None,
            signature: None,
            data: None,
            source: None,
            id: None,
            name: None,
            input: None,
            tool_use_id: Some(tool_use_id.into()),
            content: Some(Value::String(content.into())),
            cache_control: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BedrockAnthropicBinarySource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

impl BedrockAnthropicBinarySource {
    fn base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            kind: "base64".to_owned(),
            media_type: media_type.into(),
            data: data.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct BedrockAnthropicMessagesResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    content: Vec<BedrockAnthropicTextBlock>,
    #[serde(default)]
    usage: Option<BedrockAnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct BedrockAnthropicUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    output_tokens_details: Option<BedrockAnthropicOutputTokensDetails>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation: Option<BedrockAnthropicCacheCreationUsage>,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicOutputTokensDetails {
    #[serde(default)]
    thinking_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct BedrockAnthropicCacheCreationUsage {
    #[serde(default)]
    ephemeral_1h_input_tokens: Option<u64>,
    #[serde(default)]
    ephemeral_5m_input_tokens: Option<u64>,
}

impl BedrockAnthropicCacheCreationUsage {
    fn total_input_tokens(&self) -> u64 {
        self.ephemeral_1h_input_tokens.unwrap_or_default()
            + self.ephemeral_5m_input_tokens.unwrap_or_default()
    }
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
        usage: Option<BedrockAnthropicUsage>,
        #[serde(default)]
        message: Option<BedrockAnthropicStreamMessage>,
    },
    MessageStop {
        #[serde(default)]
        usage: Option<BedrockAnthropicUsage>,
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
    usage: Option<BedrockAnthropicUsage>,
}

#[derive(Debug, Default)]
struct BedrockAnthropicToolCallState {
    id: String,
    name: String,
}

#[derive(Debug, Default)]
struct BedrockAnthropicThinkingBlockState {
    kind: String,
    thinking: String,
    signature: Option<String>,
    data: Option<String>,
}

impl BedrockAnthropicThinkingBlockState {
    fn into_value(self) -> Option<Value> {
        match self.kind.as_str() {
            "thinking" => self
                .signature
                .filter(|signature| !signature.is_empty())
                .map(|signature| {
                    serde_json::json!({
                        "type": "thinking",
                        "thinking": self.thinking,
                        "signature": signature,
                    })
                }),
            "redacted_thinking" => self.data.filter(|data| !data.is_empty()).map(|data| {
                serde_json::json!({
                    "type": "redacted_thinking",
                    "data": data,
                })
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_thinking_parts(
        parts: BedrockAnthropicThinkingParts,
    ) -> BedrockAnthropicMessagesRequest {
        BedrockAnthropicMessagesRequest {
            anthropic_version: BEDROCK_ANTHROPIC_VERSION.to_owned(),
            anthropic_beta: parts.anthropic_beta,
            max_tokens: 32_000,
            system: None,
            messages: Vec::new(),
            tools: None,
            thinking: parts.thinking,
            output_config: parts.output_config,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: Vec::new(),
        }
    }

    #[test]
    fn bedrock_adaptive_effort_is_serialized_in_output_config() {
        let parts = bedrock_anthropic_thinking_parts(
            "anthropic.claude-opus-4-6-v1:0",
            Some(&ThinkingRequest::Effort {
                effort: agena_domain::ReasoningEffort::Max,
            }),
            32_000,
        );
        let value = serde_json::to_value(request_with_thinking_parts(parts))
            .expect("serialize Bedrock Anthropic request");

        assert_eq!(value["thinking"], serde_json::json!({ "type": "adaptive" }));
        assert_eq!(
            value["output_config"],
            serde_json::json!({ "effort": "max" })
        );
        assert!(value["thinking"].get("effort").is_none());
    }

    #[test]
    fn bedrock_opus_45_effort_uses_manual_thinking_and_beta_body_field() {
        let parts = bedrock_anthropic_thinking_parts(
            "anthropic.claude-opus-4-5-v1:0",
            Some(&ThinkingRequest::Effort {
                effort: agena_domain::ReasoningEffort::High,
            }),
            32_000,
        );
        let value = serde_json::to_value(request_with_thinking_parts(parts))
            .expect("serialize Bedrock Anthropic request");

        assert_eq!(value["thinking"]["type"], "enabled");
        assert_eq!(value["thinking"]["budget_tokens"], 16_000);
        assert_eq!(value["output_config"]["effort"], "high");
        assert_eq!(
            value["anthropic_beta"],
            serde_json::json!(["effort-2025-11-24"])
        );
    }

    #[test]
    fn bedrock_anthropic_blocks_omit_nulls_and_use_official_tool_shapes() {
        let text = serde_json::to_value(BedrockAnthropicTextBlock::text("hello"))
            .expect("serialize text block");
        assert_eq!(text, serde_json::json!({ "type": "text", "text": "hello" }));

        let call = serde_json::to_value(BedrockAnthropicTextBlock::tool_use(
            "toolu_1",
            "lookup",
            r#"["not", "an", "object"]"#,
        ))
        .expect("serialize tool use");
        assert_eq!(call["input"], serde_json::json!({}));

        let result = serde_json::to_value(BedrockAnthropicTextBlock::tool_result(
            "toolu_1",
            r#"{"ok":true}"#,
        ))
        .expect("serialize tool result");
        assert_eq!(result["content"], serde_json::json!(r#"{"ok":true}"#));
    }

    #[test]
    fn bedrock_anthropic_usage_separates_visible_output_from_thinking() {
        let usage = map_bedrock_anthropic_usage(BedrockAnthropicUsage {
            input_tokens: Some(20),
            output_tokens: Some(100),
            output_tokens_details: Some(BedrockAnthropicOutputTokensDetails {
                thinking_tokens: Some(60),
            }),
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation: None,
        });

        assert_eq!(usage.output_tokens, 40);
        assert_eq!(usage.reasoning_tokens, 60);
    }
}
