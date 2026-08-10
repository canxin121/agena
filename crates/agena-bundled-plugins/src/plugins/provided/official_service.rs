//! Shared implementation for provider-backed tools that remain ordinary Agena tools.
//!
//! The outer model only sees Agena's five Tool API gateway functions. These
//! helpers call official provider endpoints, normalize provider-reported usage,
//! retain continuation state, and persist binary-redacted response receipts.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::attachment::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeOutput};
use agena_provider::{
    AttributedCompletionUsage, BillableUsageItem, CompletionUsage,
    PROVIDER_TOOL_USAGE_METADATA_KEY, estimate_completion_usage_cost_usd,
};
use base64::Engine as _;
use futures_util::StreamExt as _;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt as _;

const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024;
const MAX_IMAGE_BASE64_BYTES: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4 + 4;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 96 * 1024 * 1024;
pub(crate) const MAX_PROVIDER_IMAGE_INPUT_BYTES: u64 = 50 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 32 * 1024;
const MAX_PENDING_CALLS: usize = 128;
const MAX_SOURCES: usize = 100;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ProviderUsageKind {
    OpenAiResponses,
    OpenAiImage,
    GeminiInteractions,
    GeminiGenerateContent,
    AnthropicMessages,
}

#[derive(Debug)]
pub(crate) struct ProviderHttpResponse {
    pub value: serde_json::Value,
    pub request_id: Option<String>,
}

pub(crate) fn configured_model(
    requested: Option<String>,
    configured: Option<&str>,
    env_names: &[&str],
    tool_name: &str,
) -> SdkResult<String> {
    requested
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            configured
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            env_names.iter().find_map(|name| {
                std::env::var(name)
                    .ok()
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty())
            })
        })
        .ok_or_else(|| {
            PluginError::configuration_required(
                tool_name,
                format!(
                    "Provide input.model, configure the plugin model, or set {}.",
                    env_names.join(" or ")
                ),
            )
        })
}

pub(crate) fn env_secret(env_name: &str, provider: &str) -> SdkResult<String> {
    let env_name = env_name.trim();
    if env_name.is_empty() {
        return Err(PluginError::invalid_params(format!(
            "{provider} api_key_env must not be empty"
        )));
    }
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            PluginError::internal(format!(
                "{provider} API credential is unavailable; set environment variable {env_name}"
            ))
        })
}

pub(crate) fn endpoint(base_url: &str, path: &str) -> SdkResult<String> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(PluginError::invalid_params(
            "provider base_url must not be empty",
        ));
    }
    Ok(format!("{base}/{}", path.trim_start_matches('/')))
}

pub(crate) async fn authorize_network(_host: &Arc<dyn HostClient>, url: &str) -> SdkResult<()> {
    let parsed = url::Url::parse(url)
        .map_err(|error| PluginError::invalid_params(format!("invalid provider URL: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(PluginError::invalid_params(
            "provider URL must use HTTP or HTTPS",
        ));
    }
    Ok(())
}

/// Read one provider image without allowing a request containing several
/// paths to allocate an unbounded amount of memory. The metadata check gives
/// a fast rejection; the `take` boundary also closes the race where a file
/// grows after it was statted.
pub(crate) async fn read_image_input_bounded(
    path: &Path,
    total_bytes: &mut u64,
    provider: &str,
) -> SdkResult<Vec<u8>> {
    let metadata = tokio::fs::metadata(path).await.map_err(|error| {
        PluginError::internal(format!(
            "cannot stat {provider} image '{}': {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(PluginError::invalid_params(format!(
            "{provider} image is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() == 0 {
        return Err(PluginError::invalid_params(format!(
            "{provider} image is empty: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_PROVIDER_IMAGE_INPUT_BYTES
        || total_bytes.saturating_add(metadata.len()) > MAX_PROVIDER_IMAGE_INPUT_BYTES
    {
        return Err(PluginError::invalid_params(format!(
            "{provider} image inputs exceed the {} MiB request limit",
            MAX_PROVIDER_IMAGE_INPUT_BYTES / 1024 / 1024
        )));
    }

    let file = tokio::fs::File::open(path).await.map_err(|error| {
        PluginError::internal(format!(
            "cannot read {provider} image '{}': {error}",
            path.display()
        ))
    })?;
    let remaining = MAX_PROVIDER_IMAGE_INPUT_BYTES.saturating_sub(*total_bytes);
    let mut bytes =
        Vec::with_capacity(usize::try_from(metadata.len().min(remaining)).unwrap_or_default());
    file.take(remaining.saturating_add(1))
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| {
            PluginError::internal(format!(
                "cannot read {provider} image '{}': {error}",
                path.display()
            ))
        })?;
    if bytes.is_empty() {
        return Err(PluginError::invalid_params(format!(
            "{provider} image is empty: {}",
            path.display()
        )));
    }
    if bytes.len() as u64 > remaining {
        return Err(PluginError::invalid_params(format!(
            "{provider} image inputs exceed the {} MiB request limit",
            MAX_PROVIDER_IMAGE_INPUT_BYTES / 1024 / 1024
        )));
    }
    *total_bytes = total_bytes.saturating_add(bytes.len() as u64);
    Ok(bytes)
}

pub(crate) async fn read_json_response_bounded(
    response: reqwest::Response,
    provider: &str,
    operation: &str,
) -> SdkResult<(reqwest::StatusCode, Option<String>, serde_json::Value)> {
    let status = response.status();
    let request_id = [
        "x-request-id",
        "request-id",
        "anthropic-request-id",
        "x-goog-request-id",
    ]
    .into_iter()
    .find_map(|name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    });
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64)
    {
        return Err(PluginError::internal(format!(
            "{provider} {operation} response exceeds the {} MiB limit",
            MAX_PROVIDER_RESPONSE_BYTES / 1024 / 1024
        )));
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            PluginError::internal(format!(
                "cannot read {provider} {operation} response: {error}"
            ))
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_PROVIDER_RESPONSE_BYTES {
            return Err(PluginError::internal(format!(
                "{provider} {operation} response exceeds the {} MiB limit",
                MAX_PROVIDER_RESPONSE_BYTES / 1024 / 1024
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| {
        let preview = String::from_utf8_lossy(&bytes);
        PluginError::internal(format!(
            "{provider} {operation} returned invalid JSON: {error}; body={}",
            truncate_text(preview.as_ref(), 2048)
        ))
    })?;
    Ok((status, request_id, value))
}

pub(crate) fn merge_object_options(
    base: serde_json::Value,
    extra: &BTreeMap<String, serde_json::Value>,
    protected: &[&str],
    scope: &str,
) -> SdkResult<serde_json::Value> {
    let mut object = base
        .as_object()
        .cloned()
        .ok_or_else(|| PluginError::internal(format!("{scope} base value must be an object")))?;
    for (key, value) in extra {
        if protected.contains(&key.as_str()) {
            return Err(PluginError::invalid_params(format!(
                "{scope}.{key} is controlled by Agena and cannot be overridden"
            )));
        }
        object.insert(key.clone(), value.clone());
    }
    Ok(serde_json::Value::Object(object))
}

pub(crate) async fn post_json(
    host: &Arc<dyn HostClient>,
    url: &str,
    headers: &BTreeMap<String, String>,
    body: &serde_json::Value,
    timeout_secs: u64,
    provider: &str,
    operation: &str,
) -> SdkResult<ProviderHttpResponse> {
    authorize_network(host, url).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
        .map_err(|error| {
            PluginError::internal(format!("cannot create {provider} HTTP client: {error}"))
        })?;
    let mut request = client.post(url).json(body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|error| {
        PluginError::internal(format!("{provider} {operation} request failed: {error}"))
    })?;
    let (status, request_id, value) =
        read_json_response_bounded(response, provider, operation).await?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .or_else(|| value.get("message").and_then(serde_json::Value::as_str))
            .unwrap_or("provider request failed");
        return Err(PluginError::internal(format!(
            "{provider} {operation} failed: {message} (HTTP {status})"
        )));
    }
    Ok(ProviderHttpResponse { value, request_id })
}

pub(crate) async fn provider_output(
    host: &Arc<dyn HostClient>,
    workspace_root: &Path,
    provider: &str,
    tool: &str,
    model: &str,
    title: &str,
    usage_kind: ProviderUsageKind,
    response: ProviderHttpResponse,
) -> SdkResult<ToolInvokeOutput> {
    let attachments =
        persist_images(host, workspace_root, provider, title, &response.value).await?;
    let output_text = extract_response_text(&response.value)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| format!("{provider}.{tool} completed without a text summary."));
    let pending_calls = pending_calls(&response.value);
    let continuation_required = !pending_calls.is_empty();
    let response_id = response
        .value
        .get("id")
        .or_else(|| response.value.get("interaction_id"))
        .or_else(|| response.value.get("interactionId"))
        .cloned();
    let assistant_content = (provider == "claude")
        .then(|| response.value.get("content").cloned())
        .flatten();
    let sources = compact_sources(&response.value);
    let mut usage = parse_provider_usage(usage_kind, provider, tool, model, &response.value);
    finalize_usage_estimate(provider, model, &mut usage);
    let attributed = AttributedCompletionUsage {
        provider_id: provider.to_owned(),
        model_id: model.to_owned(),
        operation: tool.to_owned(),
        request_id: response.request_id.clone(),
        usage: Box::new(usage.clone()),
    };

    let (receipt_path, receipt_sha256) =
        persist_response_receipt(host, workspace_root, provider, tool, &response.value).await?;
    let payload = serde_json::json!({
        "provider": provider,
        "tool": tool,
        "model": model,
        "request_id": response.request_id,
        "response_id": response_id,
        "pending_calls": pending_calls,
        "assistant_content": assistant_content,
        "sources": sources,
        "usage": usage,
        "response_receipt": {
            "path": receipt_path,
            "sha256": receipt_sha256,
            "binary_payloads_redacted": true
        },
        "continuation_required": continuation_required,
    });
    let usage_metadata = serde_json::to_string(&attributed).map_err(|error| {
        PluginError::internal(format!("cannot serialize provider tool usage: {error}"))
    })?;
    let result_summary = if continuation_required {
        format!("{} tool calls pending", pending_calls.len())
    } else if !attachments.is_empty() || !sources.is_empty() {
        format!(
            "{} sources · {} attachments",
            sources.len(),
            attachments.len()
        )
    } else {
        "Response received".to_string()
    };
    Ok(ToolInvokeOutput::from_parts(
        title,
        result_summary,
        truncate_text(output_text.as_str(), MAX_TEXT_BYTES),
        Some(payload),
        BTreeMap::from([
            ("provider".to_owned(), provider.to_owned()),
            ("tool".to_owned(), tool.to_owned()),
            ("model".to_owned(), model.to_owned()),
            (PROVIDER_TOOL_USAGE_METADATA_KEY.to_owned(), usage_metadata),
        ]),
        attachments,
    ))
}

pub(crate) fn append_prompt_to_items(
    mut items: Vec<serde_json::Value>,
    prompt: Option<String>,
    openai_shape: bool,
) -> SdkResult<serde_json::Value> {
    let prompt = prompt
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if items.is_empty() {
        return prompt.map(serde_json::Value::String).ok_or_else(|| {
            PluginError::invalid_params(
                "an initial provider-tool request requires prompt or continuation items",
            )
        });
    }
    if let Some(prompt) = prompt {
        items.push(if openai_shape {
            serde_json::json!({"role":"user","content":prompt})
        } else {
            serde_json::json!({"type":"message","role":"user","content":prompt})
        });
    }
    Ok(serde_json::Value::Array(items))
}

pub(crate) fn stable_cache_key(
    namespace: &str,
    workspace_root: &Path,
    provider: &str,
    model: &str,
    tool: &str,
) -> String {
    let material = format!(
        "{}\n{}\n{}\n{}\n{}",
        namespace.trim(),
        workspace_root.display(),
        provider.trim(),
        model.trim(),
        tool.trim()
    );
    format!("agena:{}", hex::encode(Sha256::digest(material.as_bytes())))
}

fn parse_provider_usage(
    kind: ProviderUsageKind,
    provider: &str,
    tool: &str,
    model: &str,
    response: &serde_json::Value,
) -> CompletionUsage {
    match kind {
        ProviderUsageKind::OpenAiResponses => parse_openai_usage(tool, response),
        ProviderUsageKind::OpenAiImage => {
            let mut usage = parse_openai_usage(tool, response);
            usage.requests = 1;
            if !usage
                .billable_items
                .iter()
                .any(|item| item.kind.contains("image"))
            {
                usage.billable_items.push(unpriced_item(
                    "openai.image_generation",
                    1.0,
                    "image",
                    "Image price depends on model, quality, resolution, and account tier.",
                ));
                usage.cost_estimate_incomplete = true;
            }
            usage
        }
        ProviderUsageKind::GeminiInteractions => parse_gemini_interactions_usage(model, response),
        ProviderUsageKind::GeminiGenerateContent => {
            parse_gemini_generate_usage(tool, model, response)
        }
        ProviderUsageKind::AnthropicMessages => parse_anthropic_usage(response),
    }
    .with_provider_units(provider, model, tool, response)
}

trait CompletionUsageProviderUnits {
    fn with_provider_units(
        self,
        provider: &str,
        model: &str,
        tool: &str,
        response: &serde_json::Value,
    ) -> Self;
}

impl CompletionUsageProviderUnits for CompletionUsage {
    fn with_provider_units(
        mut self,
        provider: &str,
        model: &str,
        tool: &str,
        response: &serde_json::Value,
    ) -> Self {
        let provider = provider.to_ascii_lowercase();
        if provider == "chatgpt" {
            for (kind, response_type) in [
                ("openai.web_search_requests", "web_search_call"),
                ("openai.file_search_requests", "file_search_call"),
                ("openai.code_interpreter_calls", "code_interpreter_call"),
                ("openai.computer_calls", "computer_call"),
                ("openai.shell_calls", "shell_call"),
                ("openai.mcp_calls", "mcp_call"),
                ("openai.image_generation_calls", "image_generation_call"),
            ] {
                let count = count_object_type(response, response_type) as f64;
                if count > 0.0 {
                    let item = if response_type == "web_search_call" {
                        priced_item(
                            kind,
                            count,
                            "request",
                            0.01,
                            "OpenAI API pricing: web search call list price; search-content tokens may also apply.",
                        )
                    } else {
                        unpriced_item(
                            kind,
                            count,
                            "request",
                            "Price depends on tool configuration, duration, storage, model, or account tier.",
                        )
                    };
                    self.billable_items.push(item);
                }
            }
        } else if provider == "claude" {
            let usage = response.get("usage").unwrap_or(response);
            let server = usage
                .get("server_tool_use")
                .or_else(|| usage.get("serverToolUse"));
            let web_search = server
                .and_then(|value| json_u64(value, &["web_search_requests", "webSearchRequests"]))
                .unwrap_or_default();
            let web_fetch = server
                .and_then(|value| json_u64(value, &["web_fetch_requests", "webFetchRequests"]))
                .unwrap_or_default();
            let code_execution = server
                .and_then(|value| {
                    json_u64(value, &["code_execution_requests", "codeExecutionRequests"])
                })
                .unwrap_or_default();
            if web_search > 0 {
                self.billable_items.push(priced_item(
                    "anthropic.web_search_requests",
                    web_search as f64,
                    "request",
                    0.01,
                    "Anthropic API pricing: web search is $10 per 1,000 searches, plus token usage.",
                ));
            }
            if web_fetch > 0 {
                self.billable_items.push(priced_item(
                    "anthropic.web_fetch_requests",
                    web_fetch as f64,
                    "request",
                    0.0,
                    "Anthropic API pricing: web fetch currently has no additional request fee; token usage remains billable.",
                ));
            }
            if code_execution > 0 {
                self.billable_items.push(unpriced_item(
                    "anthropic.code_execution_requests",
                    code_execution as f64,
                    "request",
                    "Anthropic code execution is billed by container runtime after account free allowance; request count alone cannot determine runtime cost.",
                ));
                self.cost_estimate_incomplete = true;
            }
        }
        append_pricing_context(&mut self, provider.as_str(), model, response);
        if tool.contains("computer") && self.billable_items.is_empty() {
            self.billable_items.push(unpriced_item(
                format!("{provider}.computer_runtime"),
                1.0,
                "interaction",
                "Computer-use billing may include model tokens and environment/runtime charges not reported in this response.",
            ));
            self.cost_estimate_incomplete = true;
        }
        self
    }
}

fn parse_openai_usage(tool: &str, response: &serde_json::Value) -> CompletionUsage {
    let usage = response.get("usage").unwrap_or(response);
    let input_inclusive = json_u64(usage, &["input_tokens", "prompt_tokens"]).unwrap_or_default();
    let output_inclusive =
        json_u64(usage, &["output_tokens", "completion_tokens"]).unwrap_or_default();
    let total = json_u64(usage, &["total_tokens"]);
    let input_details = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"));
    let output_details = usage
        .get("output_tokens_details")
        .or_else(|| usage.get("completion_tokens_details"));
    let cache_read = input_details
        .and_then(|value| json_u64(value, &["cached_tokens"]))
        .unwrap_or_default();
    let cache_write = input_details
        .and_then(|value| json_u64(value, &["cache_write_tokens"]))
        .unwrap_or_default();
    let reasoning = output_details
        .and_then(|value| json_u64(value, &["reasoning_tokens"]))
        .unwrap_or_default();
    let input = input_inclusive
        .saturating_sub(cache_read)
        .saturating_sub(cache_write);
    let output = output_inclusive.saturating_sub(reasoning);
    let known = input
        .saturating_add(cache_read)
        .saturating_add(cache_write)
        .saturating_add(output)
        .saturating_add(reasoning);
    let ticks = json_u64(usage, &["cost_in_usd_ticks"]);
    let recorded = ticks.map(|value| value as f64 / 10_000_000_000.0);
    let mut normalized = CompletionUsage {
        requests: 1,
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cache_write_tokens: cache_write,
        cache_read_tokens: cache_read,
        other_tokens: total.unwrap_or_default().saturating_sub(known),
        total_cost: recorded.unwrap_or_default(),
        recorded_cost: recorded.unwrap_or_default(),
        recorded_cost_available: recorded.is_some(),
        ..CompletionUsage::default()
    };
    if tool == "file_search" {
        normalized.cost_estimate_incomplete = true;
    }
    normalized
}

fn parse_gemini_interactions_usage(model: &str, response: &serde_json::Value) -> CompletionUsage {
    let usage = response.get("usage").unwrap_or(response);
    let input_inclusive =
        json_u64(usage, &["total_input_tokens", "totalInputTokens"]).unwrap_or_default();
    let output = json_u64(usage, &["total_output_tokens", "totalOutputTokens"]).unwrap_or_default();
    let reasoning =
        json_u64(usage, &["total_thought_tokens", "totalThoughtTokens"]).unwrap_or_default();
    let cache_read =
        json_u64(usage, &["total_cached_tokens", "totalCachedTokens"]).unwrap_or_default();
    let tool_use =
        json_u64(usage, &["total_tool_use_tokens", "totalToolUseTokens"]).unwrap_or_default();
    let total = json_u64(usage, &["total_tokens", "totalTokens"]);
    let input = input_inclusive.saturating_sub(cache_read);
    let known = input
        .saturating_add(cache_read)
        .saturating_add(output)
        .saturating_add(reasoning);
    let mut normalized = CompletionUsage {
        requests: 1,
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cache_read_tokens: cache_read,
        tool_use_tokens: tool_use,
        other_tokens: total.unwrap_or_default().saturating_sub(known),
        ..CompletionUsage::default()
    };
    append_gemini_grounding_items(&mut normalized, model, usage);
    normalized
}

fn parse_gemini_generate_usage(
    tool: &str,
    model: &str,
    response: &serde_json::Value,
) -> CompletionUsage {
    let usage = response
        .get("usageMetadata")
        .or_else(|| response.get("usage_metadata"))
        .unwrap_or(response);
    let input_inclusive =
        json_u64(usage, &["promptTokenCount", "prompt_token_count"]).unwrap_or_default();
    let output =
        json_u64(usage, &["candidatesTokenCount", "candidates_token_count"]).unwrap_or_default();
    let reasoning =
        json_u64(usage, &["thoughtsTokenCount", "thoughts_token_count"]).unwrap_or_default();
    let cache_read = json_u64(
        usage,
        &["cachedContentTokenCount", "cached_content_token_count"],
    )
    .unwrap_or_default();
    let tool_use = json_u64(
        usage,
        &["toolUsePromptTokenCount", "tool_use_prompt_token_count"],
    )
    .unwrap_or_default();
    let total = json_u64(usage, &["totalTokenCount", "total_token_count"]);
    let input = input_inclusive.saturating_sub(cache_read);
    let known = input
        .saturating_add(cache_read)
        .saturating_add(output)
        .saturating_add(reasoning);
    let mut normalized = CompletionUsage {
        requests: 1,
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cache_read_tokens: cache_read,
        tool_use_tokens: tool_use,
        other_tokens: total.unwrap_or_default().saturating_sub(known),
        ..CompletionUsage::default()
    };
    append_gemini_grounding_items(&mut normalized, model, usage);
    if tool.contains("image") {
        normalized.billable_items.push(unpriced_item(
            "google.image_generation",
            1.0,
            "image",
            "Image pricing depends on model, output modality, resolution, and account tier.",
        ));
        normalized.cost_estimate_incomplete = true;
    }
    normalized
}

fn append_gemini_grounding_items(
    usage: &mut CompletionUsage,
    model: &str,
    value: &serde_json::Value,
) {
    let counts = value
        .get("grounding_tool_count")
        .or_else(|| value.get("groundingToolCount"))
        .and_then(serde_json::Value::as_array);
    let Some(counts) = counts else {
        return;
    };
    let lower_model = model.trim().to_ascii_lowercase();
    let normalized_model = lower_model
        .rsplit('/')
        .next()
        .unwrap_or(lower_model.as_str())
        .to_owned();
    for item in counts {
        let kind = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let count = json_u64(item, &["count"]).unwrap_or_default();
        if count == 0 {
            continue;
        }
        let price = if normalized_model.starts_with("gemini-3") {
            match kind {
                "google_search" | "google_maps" => Some(0.014),
                _ => None,
            }
        } else if normalized_model.starts_with("gemini-2.5") {
            match kind {
                "google_search" => Some(0.035),
                "google_maps" => Some(0.025),
                _ => None,
            }
        } else {
            None
        };
        let item = match price {
            Some(unit_price) => priced_item(
                format!("google.grounding.{kind}"),
                count as f64,
                "grounded_prompt",
                unit_price,
                "Google Gemini API list price before account-, model-, and quota-specific free allowance adjustments.",
            ),
            None => unpriced_item(
                format!("google.grounding.{kind}"),
                count as f64,
                "grounded_prompt",
                "Grounding price depends on Gemini model generation, account tier, and free quota.",
            ),
        };
        usage.billable_items.push(item);
        // Google can apply free quotas or negotiated tier pricing that is not
        // observable in the response, so list-price estimates are not invoices.
        usage.cost_estimate_incomplete = true;
    }
}

fn append_pricing_context(
    usage: &mut CompletionUsage,
    provider: &str,
    model: &str,
    response: &serde_json::Value,
) {
    let provider = provider.to_ascii_lowercase();
    if provider == "chatgpt" {
        let tier = response
            .get("service_tier")
            .or_else(|| response.get("serviceTier"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("auto");
        if !matches!(tier, "auto" | "default") {
            usage.billable_items.push(unpriced_item(
                "openai.service_tier_modifier",
                1.0,
                "request",
                format!("OpenAI service tier `{tier}` can change token pricing; the response does not provide an authoritative invoice amount."),
            ));
            usage.cost_estimate_incomplete = true;
        }
    } else if provider == "claude" {
        let usage_value = response.get("usage").unwrap_or(response);
        let tier = usage_value
            .get("service_tier")
            .or_else(|| usage_value.get("serviceTier"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("standard");
        let inference_geo = usage_value
            .get("inference_geo")
            .or_else(|| usage_value.get("inferenceGeo"))
            .and_then(serde_json::Value::as_str);
        if tier != "standard"
            || inference_geo.is_some_and(|geo| !geo.eq_ignore_ascii_case("global"))
        {
            usage.billable_items.push(unpriced_item(
                "anthropic.pricing_modifier",
                1.0,
                "request",
                format!("Anthropic service tier `{tier}` / inference geo `{}` may alter list pricing for model `{model}`.", inference_geo.unwrap_or("default")),
            ));
            usage.cost_estimate_incomplete = true;
        }
    } else if provider == "gemini" && usage.has_own_usage() {
        usage.billable_items.push(unpriced_item(
            "google.account_tier_adjustment",
            1.0,
            "request",
            "Gemini paid/free tier, batch mode, and account quota can alter list-price estimates and are not fully exposed in usage metadata.",
        ));
        usage.cost_estimate_incomplete = true;
    }
}

fn parse_anthropic_usage(response: &serde_json::Value) -> CompletionUsage {
    let usage = response.get("usage").unwrap_or(response);
    let input = json_u64(usage, &["input_tokens"]).unwrap_or_default();
    let output_inclusive = json_u64(usage, &["output_tokens"]).unwrap_or_default();
    let output_details = usage
        .get("output_tokens_details")
        .or_else(|| usage.get("outputTokensDetails"));
    let reasoning = output_details
        .and_then(|value| json_u64(value, &["thinking_tokens", "thinkingTokens"]))
        .unwrap_or_default();
    let cache_read =
        json_u64(usage, &["cache_read_input_tokens", "cacheReadInputTokens"]).unwrap_or_default();
    let cache_creation = usage
        .get("cache_creation")
        .or_else(|| usage.get("cacheCreation"));
    let cache_5m = cache_creation
        .and_then(|value| {
            json_u64(
                value,
                &["ephemeral_5m_input_tokens", "ephemeral5mInputTokens"],
            )
        })
        .unwrap_or_default();
    let cache_1h = cache_creation
        .and_then(|value| {
            json_u64(
                value,
                &["ephemeral_1h_input_tokens", "ephemeral1hInputTokens"],
            )
        })
        .unwrap_or_default();
    let cache_write = json_u64(
        usage,
        &["cache_creation_input_tokens", "cacheCreationInputTokens"],
    )
    .unwrap_or_else(|| cache_5m.saturating_add(cache_1h));
    CompletionUsage {
        requests: 1,
        input_tokens: input,
        output_tokens: output_inclusive.saturating_sub(reasoning),
        reasoning_tokens: reasoning,
        cache_write_tokens: cache_write,
        cache_write_5m_tokens: cache_5m,
        cache_write_1h_tokens: cache_1h,
        cache_read_tokens: cache_read,
        ..CompletionUsage::default()
    }
}

fn finalize_usage_estimate(provider: &str, model: &str, usage: &mut CompletionUsage) {
    if usage.recorded_cost_available {
        return;
    }
    let unit_cost = usage
        .billable_items
        .iter()
        .filter_map(|item| item.cost_usd)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .sum::<f64>();
    let token_cost = estimate_completion_usage_cost_usd(provider, model, usage);
    usage.estimated_cost = token_cost.unwrap_or_default() + unit_cost;
    usage.cost_estimate_incomplete |= usage
        .billable_items
        .iter()
        .any(|item| item.cost_usd.is_none());
    if usage.own_total_tokens() > 0 && token_cost.is_none() {
        usage.cost_estimate_incomplete = true;
    }
}

fn priced_item(
    kind: impl Into<String>,
    quantity: f64,
    unit: impl Into<String>,
    unit_price_usd: f64,
    note: impl Into<String>,
) -> BillableUsageItem {
    BillableUsageItem {
        kind: kind.into(),
        quantity,
        unit: unit.into(),
        unit_price_usd: Some(unit_price_usd),
        cost_usd: Some(quantity * unit_price_usd),
        pricing_source: Some("official-provider-pricing-snapshot-2026-07-28".to_owned()),
        note: Some(note.into()),
    }
}

fn unpriced_item(
    kind: impl Into<String>,
    quantity: f64,
    unit: impl Into<String>,
    note: impl Into<String>,
) -> BillableUsageItem {
    BillableUsageItem {
        kind: kind.into(),
        quantity,
        unit: unit.into(),
        unit_price_usd: None,
        cost_usd: None,
        pricing_source: None,
        note: Some(note.into()),
    }
}

fn json_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| {
            item.as_u64().or_else(|| {
                item.as_i64()
                    .and_then(|number| (number >= 0).then_some(number as u64))
            })
        })
    })
}

fn count_object_type(value: &serde_json::Value, expected: &str) -> u64 {
    match value {
        serde_json::Value::Object(object) => {
            u64::from(object.get("type").and_then(serde_json::Value::as_str) == Some(expected))
                + object
                    .values()
                    .map(|child| count_object_type(child, expected))
                    .sum::<u64>()
        }
        serde_json::Value::Array(items) => items
            .iter()
            .map(|child| count_object_type(child, expected))
            .sum(),
        _ => 0,
    }
}

async fn persist_response_receipt(
    _host: &Arc<dyn HostClient>,
    workspace_root: &Path,
    provider: &str,
    tool: &str,
    response: &serde_json::Value,
) -> SdkResult<(String, String)> {
    let mut safe_response = response.clone();
    redact_binary_payloads(&mut safe_response);
    let bytes = serde_json::to_vec_pretty(&safe_response).map_err(|error| {
        PluginError::internal(format!("cannot serialize provider receipt: {error}"))
    })?;
    let sha256 = hex::encode(Sha256::digest(bytes.as_slice()));
    let safe_tool = tool
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let path = workspace_root
        .join(".agena/tool-results/provider-tools")
        .join(provider)
        .join(format!(
            "{}-{}.json",
            safe_tool,
            uuid::Uuid::new_v4().simple()
        ));
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            PluginError::internal(format!("cannot create receipt directory: {error}"))
        })?;
    }
    tokio::fs::write(&path, bytes).await.map_err(|error| {
        PluginError::internal(format!("cannot write provider receipt: {error}"))
    })?;
    Ok((path.to_string_lossy().to_string(), sha256))
}

fn compact_sources(value: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut sources = Vec::new();
    collect_sources(value, &mut sources, 0);
    let mut seen = HashSet::new();
    sources.retain(|value| {
        value
            .get("url")
            .or_else(|| value.get("uri"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|url| seen.insert(url.to_owned()))
    });
    sources.truncate(MAX_SOURCES);
    sources
}

fn collect_sources(value: &serde_json::Value, output: &mut Vec<serde_json::Value>, depth: usize) {
    if depth > 18 || output.len() >= MAX_SOURCES * 4 {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            let uri = object
                .get("url")
                .or_else(|| object.get("uri"))
                .and_then(serde_json::Value::as_str);
            if let Some(uri) = uri
                && (uri.starts_with("http://") || uri.starts_with("https://"))
            {
                output.push(serde_json::json!({
                    "url": uri,
                    "title": object.get("title").and_then(serde_json::Value::as_str),
                    "snippet": object
                        .get("snippet")
                        .or_else(|| object.get("text"))
                        .and_then(serde_json::Value::as_str)
                        .map(|text| truncate_text(text, 1024)),
                }));
            }
            for child in object.values() {
                collect_sources(child, output, depth + 1);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_sources(child, output, depth + 1);
            }
        }
        _ => {}
    }
}

pub(crate) fn truncate_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}\n…[truncated]", &value[..end])
}

pub(crate) fn dedup_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let normalized = value.trim().to_owned();
            (!normalized.is_empty() && seen.insert(normalized.clone())).then_some(normalized)
        })
        .collect()
}

pub(crate) fn resolve_local_path(workspace_root: &Path, value: &str) -> SdkResult<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return Err(PluginError::invalid_params("path must not be empty"));
    }
    let path = Path::new(value);
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    })
}

fn extract_response_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.get("output_text").and_then(serde_json::Value::as_str) {
        return Some(text.to_owned());
    }
    let mut texts = Vec::new();
    collect_text(value, None, &mut texts, 0);
    let text = dedup_strings(texts).join("\n");
    (!text.is_empty()).then_some(text)
}

fn collect_text(
    value: &serde_json::Value,
    parent_key: Option<&str>,
    output: &mut Vec<String>,
    depth: usize,
) {
    if depth > 16 || output.len() > 512 {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if key == "text" || key == "output_text" {
                    if let Some(text) = child.as_str()
                        && !text.trim().is_empty()
                    {
                        output.push(text.to_owned());
                    }
                } else if !matches!(
                    key.as_str(),
                    "prompt" | "input" | "arguments" | "arguments_json"
                ) {
                    collect_text(child, Some(key.as_str()), output, depth + 1);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_text(child, parent_key, output, depth + 1);
            }
        }
        serde_json::Value::String(text)
            if matches!(parent_key, Some("content" | "result" | "output"))
                && !looks_like_base64(text)
                && !text.trim().is_empty() =>
        {
            output.push(text.to_owned());
        }
        _ => {}
    }
}

fn pending_calls(value: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut calls = Vec::new();
    collect_pending_calls(value, &mut calls, 0);
    calls.truncate(MAX_PENDING_CALLS);
    calls
}

fn collect_pending_calls(
    value: &serde_json::Value,
    output: &mut Vec<serde_json::Value>,
    depth: usize,
) {
    if depth > 16 || output.len() >= MAX_PENDING_CALLS {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            let kind = object
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let status = object
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let is_pending_kind = kind.ends_with("_call")
                || matches!(
                    kind,
                    "tool_use" | "server_tool_use" | "function_call" | "computer_call"
                )
                || kind.contains("approval_request");
            let terminal = matches!(status, "completed" | "failed" | "cancelled");
            if is_pending_kind && !terminal {
                output.push(value.clone());
            }
            for child in object.values() {
                collect_pending_calls(child, output, depth + 1);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_pending_calls(child, output, depth + 1);
            }
        }
        _ => {}
    }
}

#[derive(Clone)]
struct ImageCandidate {
    encoded: String,
    mime: String,
}

async fn persist_images(
    _host: &Arc<dyn HostClient>,
    workspace_root: &Path,
    provider: &str,
    title: &str,
    response: &serde_json::Value,
) -> SdkResult<Vec<AttachmentItem>> {
    let mut candidates = Vec::new();
    collect_image_candidates(response, &mut candidates, 0);
    let mut attachments = Vec::new();
    let mut seen = HashSet::new();
    for candidate in candidates {
        let encoded = candidate
            .encoded
            .split_once(',')
            .map(|(_, data)| data)
            .unwrap_or(candidate.encoded.as_str())
            .trim();
        if encoded.len() > MAX_IMAGE_BASE64_BYTES {
            continue;
        }
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
            continue;
        };
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            continue;
        }
        let sha256 = hex::encode(Sha256::digest(bytes.as_slice()));
        if !seen.insert(sha256.clone()) {
            continue;
        }
        let extension = extension_for_mime(candidate.mime.as_str());
        let path = workspace_root
            .join(".agena/artifacts/provider-tools")
            .join(provider)
            .join(format!("{}.{}", uuid::Uuid::new_v4().simple(), extension));
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                PluginError::internal(format!(
                    "cannot create provider artifact directory: {error}"
                ))
            })?;
        }
        tokio::fs::write(&path, bytes.as_slice())
            .await
            .map_err(|error| {
                PluginError::internal(format!(
                    "cannot persist provider image '{}': {error}",
                    path.display()
                ))
            })?;
        attachments.push(AttachmentItem {
            kind: AttachmentKind::Image,
            mime: candidate.mime.clone(),
            source: AttachmentSource::LocalPath {
                path: path.to_string_lossy().to_string(),
            },
            filename: path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_owned),
            title: Some(title.to_owned()),
            size_bytes: Some(bytes.len() as u64),
            sha256: Some(sha256),
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        });
    }
    Ok(attachments)
}

fn collect_image_candidates(
    value: &serde_json::Value,
    output: &mut Vec<ImageCandidate>,
    depth: usize,
) {
    if depth > 18 || output.len() >= 16 {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            if let Some(encoded) = object.get("b64_json").and_then(serde_json::Value::as_str) {
                output.push(ImageCandidate {
                    encoded: encoded.to_owned(),
                    mime: "image/png".to_owned(),
                });
            }
            let kind = object
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if kind == "image_generation_call"
                && let Some(encoded) = object.get("result").and_then(serde_json::Value::as_str)
            {
                output.push(ImageCandidate {
                    encoded: encoded.to_owned(),
                    mime: "image/png".to_owned(),
                });
            }
            for inline_key in ["inlineData", "inline_data"] {
                if let Some(inline) = object
                    .get(inline_key)
                    .and_then(serde_json::Value::as_object)
                {
                    let mime = inline
                        .get("mimeType")
                        .or_else(|| inline.get("mime_type"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("image/png");
                    if mime.starts_with("image/")
                        && let Some(encoded) =
                            inline.get("data").and_then(serde_json::Value::as_str)
                    {
                        output.push(ImageCandidate {
                            encoded: encoded.to_owned(),
                            mime: mime.to_owned(),
                        });
                    }
                }
            }
            if let (Some(mime), Some(encoded)) = (
                object
                    .get("mime_type")
                    .or_else(|| object.get("mimeType"))
                    .and_then(serde_json::Value::as_str),
                object.get("data").and_then(serde_json::Value::as_str),
            ) && mime.starts_with("image/")
            {
                output.push(ImageCandidate {
                    encoded: encoded.to_owned(),
                    mime: mime.to_owned(),
                });
            }
            for child in object.values() {
                collect_image_candidates(child, output, depth + 1);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_image_candidates(child, output, depth + 1);
            }
        }
        _ => {}
    }
}

fn redact_binary_payloads(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            let kind = object
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            for (key, child) in object.iter_mut() {
                let redact = matches!(key.as_str(), "b64_json")
                    || (key == "result" && kind.as_deref() == Some("image_generation_call"))
                    || (key == "data" && child.as_str().is_some_and(looks_like_base64));
                if redact {
                    if let Some(text) = child.as_str() {
                        *child = serde_json::Value::String(format!(
                            "[binary payload omitted: {} chars]",
                            text.len()
                        ));
                    }
                } else {
                    redact_binary_payloads(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                redact_binary_payloads(child);
            }
        }
        _ => {}
    }
}

fn looks_like_base64(value: &str) -> bool {
    value.len() > 1024
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'\r' | b'\n')
        })
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_PROVIDER_IMAGE_INPUT_BYTES, read_image_input_bounded};

    #[tokio::test]
    async fn oversized_sparse_image_input_is_rejected_before_full_read() {
        let workspace = tempfile::tempdir().expect("workspace");
        let path = workspace.path().join("oversized.png");
        let file = std::fs::File::create(&path).expect("image file");
        file.set_len(MAX_PROVIDER_IMAGE_INPUT_BYTES + 1)
            .expect("sparse image");
        let mut total = 0;
        let error = read_image_input_bounded(&path, &mut total, "fixture")
            .await
            .expect_err("oversized image must be rejected");
        assert!(error.to_string().contains("50 MiB"));
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn multiple_image_inputs_share_one_request_budget() {
        let workspace = tempfile::tempdir().expect("workspace");
        let first = workspace.path().join("first.png");
        let second = workspace.path().join("second.png");
        std::fs::write(&first, b"first").expect("first image");
        let file = std::fs::File::create(&second).expect("second image");
        file.set_len(MAX_PROVIDER_IMAGE_INPUT_BYTES)
            .expect("sparse second image");
        let mut total = 0;
        let first_bytes = read_image_input_bounded(&first, &mut total, "fixture")
            .await
            .expect("first image");
        assert_eq!(first_bytes, b"first");
        let error = read_image_input_bounded(&second, &mut total, "fixture")
            .await
            .expect_err("combined request budget must be enforced");
        assert!(error.to_string().contains("50 MiB"));
    }
}
