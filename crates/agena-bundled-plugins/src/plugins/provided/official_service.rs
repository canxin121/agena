//! Shared implementation for provider-backed tools that remain ordinary Agena tools.
//!
//! The outer model only sees Agena's five Tool API gateway functions.  These
//! helpers let an ordinary execution tool call an official provider endpoint,
//! preserve pending client-action calls for a later continuation, and persist
//! image payloads as permission-checked Agena attachments.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::attachment::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_plugin_host::sdk::host_api::{
    HostClient, HostNetworkPermissionCheckRequest, HostPathPermissionCheckRequest,
};
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeOutput};
use base64::Engine as _;
use sha2::{Digest, Sha256};

const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_PENDING_CALLS: usize = 128;

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
            PluginError::invalid_params(format!(
                "{tool_name} requires input.model, plugin model configuration, or one of: {}",
                env_names.join(", ")
            ))
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
            PluginError::new(format!(
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

pub(crate) async fn authorize_network(host: &Arc<dyn HostClient>, url: &str) -> SdkResult<()> {
    host.ensure_network_permission(HostNetworkPermissionCheckRequest::connect(url.to_owned()))
        .await
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
        .ok_or_else(|| PluginError::new(format!("{scope} base value must be an object")))?;
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
) -> SdkResult<serde_json::Value> {
    authorize_network(host, url).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
        .map_err(|error| {
            PluginError::new(format!("cannot create {provider} HTTP client: {error}"))
        })?;
    let mut request = client.post(url).json(body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|error| {
        PluginError::new(format!("{provider} {operation} request failed: {error}"))
    })?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|error| {
        PluginError::new(format!(
            "cannot read {provider} {operation} response: {error}"
        ))
    })?;
    let value = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| {
        let preview = String::from_utf8_lossy(&bytes);
        PluginError::new(format!(
            "{provider} {operation} returned invalid JSON: {error}; body={}",
            truncate_text(preview.as_ref(), 2048)
        ))
    })?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .or_else(|| value.get("message").and_then(serde_json::Value::as_str))
            .unwrap_or("provider request failed");
        return Err(PluginError::new(format!(
            "{provider} {operation} failed: {message} (HTTP {status})"
        )));
    }
    Ok(value)
}

pub(crate) async fn provider_output(
    host: &Arc<dyn HostClient>,
    workspace_root: &Path,
    provider: &str,
    tool: &str,
    model: &str,
    title: &str,
    response: serde_json::Value,
) -> SdkResult<ToolInvokeOutput> {
    let attachments = persist_images(host, workspace_root, provider, title, &response).await?;
    let output_text = extract_response_text(&response)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| format!("{provider}.{tool} completed without a text summary."));
    let pending_calls = pending_calls(&response);
    let continuation_required = !pending_calls.is_empty();
    let response_id = response
        .get("id")
        .or_else(|| response.get("interaction_id"))
        .cloned();
    let mut safe_response = response;
    redact_binary_payloads(&mut safe_response);
    let payload = serde_json::json!({
        "provider": provider,
        "tool": tool,
        "model": model,
        "response_id": response_id,
        "pending_calls": pending_calls,
        "response": safe_response,
        "continuation_required": continuation_required,
    });
    Ok(ToolInvokeOutput::from_parts(
        title,
        truncate_text(output_text.as_str(), MAX_TEXT_BYTES),
        Some(payload),
        BTreeMap::from([
            ("provider".to_owned(), provider.to_owned()),
            ("tool".to_owned(), tool.to_owned()),
            ("model".to_owned(), model.to_owned()),
        ]),
        attachments,
    ))
}

pub(crate) fn append_prompt_to_items(
    mut items: Vec<serde_json::Value>,
    prompt: String,
    openai_shape: bool,
) -> serde_json::Value {
    if items.is_empty() {
        return serde_json::Value::String(prompt);
    }
    if !prompt.trim().is_empty() {
        items.push(if openai_shape {
            serde_json::json!({"role":"user","content":prompt})
        } else {
            serde_json::json!({"type":"message","role":"user","content":prompt})
        });
    }
    serde_json::Value::Array(items)
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
    host: &Arc<dyn HostClient>,
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
        host.ensure_path_permission(HostPathPermissionCheckRequest::write(
            path.to_string_lossy().to_string(),
        ))
        .await?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                PluginError::new(format!(
                    "cannot create provider artifact directory: {error}"
                ))
            })?;
        }
        tokio::fs::write(&path, bytes.as_slice())
            .await
            .map_err(|error| {
                PluginError::new(format!(
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
