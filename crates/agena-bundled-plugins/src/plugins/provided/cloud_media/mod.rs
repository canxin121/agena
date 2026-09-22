//! Provider-owned multimodal inference and file resources. File reads and cloud
//! requests are separate declared effects. No clipboard access, implicit local
//! executor, credential forwarding, or cross-provider file-ID substitution.
mod files;
use super::official_service::{self, ProviderHttpResponse, ProviderUsageKind};
use agena_macros::ToolInput;
use agena_plugin_host::{
    PluginError,
    sdk::{
        PathRequest, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, host_api::HostClient,
    },
};
use agena_runtime_tools::media_input::{self, PreparedMedia};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum MediaSource {
    Local {
        path: String,
        #[serde(default)]
        expected_sha256: Option<String>,
    },
    Cloud {
        handle: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(non_empty("prompt"), max_chars("prompt", 64000))]
pub struct AnalyzeInput {
    pub inputs: Vec<MediaSource>,
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub detail: ImageDetail,
    #[serde(default = "default_tokens")]
    pub max_output_tokens: u32,
}
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageDetail {
    #[default]
    Auto,
    Low,
    High,
}
const fn default_tokens() -> u32 {
    4096
}
impl AnalyzeInput {
    pub fn paths(&self, root: &Path) -> Vec<PathRequest> {
        self.inputs
            .iter()
            .filter_map(|source| match source {
                MediaSource::Local { path, .. } => Some(PathRequest::read(path)),
                _ => None,
            })
            .chain([
                PathRequest::read(
                    root.join(".agena/artifacts/provider-tools/media")
                        .display()
                        .to_string(),
                ),
                PathRequest::write(
                    root.join(".agena/artifacts/provider-tools")
                        .display()
                        .to_string(),
                ),
            ])
            .collect()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(non_empty("path"))]
pub struct UploadInput {
    pub path: String,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    /// Optional provider expiry. OpenAI/Anthropic default to one day.
    /// Google uses its own lifecycle and rejects custom expiry.
    #[serde(default)]
    pub expires_in_seconds: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(non_empty("handle"))]
pub struct FileInput {
    pub handle: String,
}

pub struct Service<'a> {
    pub provider: &'static str,
    pub root: &'a Path,
    pub host: &'a Arc<dyn HostClient>,
    pub base_url: &'a str,
    pub key_env: &'a str,
    pub timeout_secs: u64,
    pub anthropic_version: &'a str,
}
impl Service<'_> {
    fn base(&self) -> SdkResult<url::Url> {
        let base = url::Url::parse(self.base_url).map_err(|e| {
            PluginError::invalid_params(format!("invalid cloud media endpoint: {e}"))
        })?;
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(PluginError::invalid_params(
                "media endpoint must not contain userinfo, query credentials or a fragment",
            ));
        }
        let local = base.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if base.scheme() != "https" && !(base.scheme() == "http" && local) {
            return Err(PluginError::invalid_params(
                "cloud media requires HTTPS, except an explicitly configured loopback test endpoint",
            ));
        }
        Ok(base)
    }
    fn endpoint(&self, suffix: &str) -> SdkResult<String> {
        self.base()?;
        official_service::endpoint(self.base_url, suffix)
    }
    fn authenticated(
        &self,
        request: reqwest::RequestBuilder,
        key: &str,
    ) -> reqwest::RequestBuilder {
        let request = request.timeout(std::time::Duration::from_secs(
            self.timeout_secs.clamp(1, 300),
        ));
        match self.provider {
            "chatgpt" => request.bearer_auth(key),
            "claude" => request
                .header("x-api-key", key)
                .header("anthropic-version", self.anthropic_version),
            _ => request.header("x-goog-api-key", key),
        }
    }
    async fn response(
        &self,
        request: reqwest::RequestBuilder,
        operation: &str,
    ) -> SdkResult<ProviderHttpResponse> {
        let response=request.send().await.map_err(|error|PluginError::internal(format!("cloud {operation} transport failed; remote acceptance is unknown, do not automatically repeat: {}",error.without_url())))?;
        let (status, request_id, value) =
            official_service::read_json_response_bounded(response, self.provider, operation)
                .await?;
        if !status.is_success() {
            return Err(PluginError::internal(format!(
                "cloud {operation} returned HTTP {status}; inspect status before retrying"
            )));
        }
        Ok(ProviderHttpResponse { value, request_id })
    }
    pub async fn analyze(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
        model: String,
        images_only: bool,
    ) -> SdkResult<ToolInvokeOutput> {
        self.base()?;
        if input.inputs.is_empty() || input.inputs.len() > media_input::MAX_MEDIA_INPUTS {
            return Err(PluginError::invalid_params(
                "provide 1..8 explicit media inputs",
            ));
        }
        if input.prompt.trim().is_empty()
            || input.max_output_tokens == 0
            || input.max_output_tokens > 32768
        {
            return Err(PluginError::invalid_params(
                "prompt must be nonempty and max_output_tokens must be 1..32768",
            ));
        }
        if model.is_empty()
            || model.len() > 200
            || !model
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b':'))
        {
            return Err(PluginError::invalid_params("invalid model identifier"));
        }
        if self.provider != "chatgpt" && input.detail != ImageDetail::Auto {
            return Err(PluginError::invalid_params(
                "this provider does not accept OpenAI's image detail setting; use detail=auto",
            ));
        }
        let key = official_service::env_secret(self.key_env, self.provider)?;
        let mut parts = Vec::new();
        let mut receipts = Vec::new();
        let mut total = 0usize;
        for source in &input.inputs {
            let (mime, kind, filename, sha, size, inline, remote) = match source {
                MediaSource::Local {
                    path,
                    expected_sha256,
                } => {
                    let root = self.root.to_owned();
                    let path = path.clone();
                    let expected = expected_sha256.clone();
                    let prepared = tokio::task::spawn_blocking(move || {
                        media_input::read_local(&root, &path, expected.as_deref())
                    })
                    .await
                    .map_err(|e| {
                        PluginError::internal(format!("media preparation worker failed: {e}"))
                    })?
                    .map_err(PluginError::invalid_params)?;
                    total = total.saturating_add(prepared.bytes.len());
                    if total > 12 * 1024 * 1024 {
                        return Err(PluginError::invalid_params(
                            "inline analysis inputs exceed 12 MiB; use this provider's cloud_file_upload and a cloud handle",
                        ));
                    }
                    if self.provider == "claude"
                        && prepared.kind == agena_domain::AttachmentKind::Image
                        && prepared.size_bytes > 5 * 1024 * 1024
                    {
                        return Err(PluginError::invalid_params(
                            "Anthropic inline images must not exceed 5 MiB; resize or use an allowed supported input",
                        ));
                    }
                    (
                        prepared.mime,
                        prepared.kind,
                        prepared.filename,
                        prepared.sha256,
                        prepared.size_bytes,
                        Some(prepared.bytes),
                        None,
                    )
                }
                MediaSource::Cloud { handle } => {
                    let record = self.load_record(context, handle, &key).await?;
                    if record.is_expired() {
                        return Err(PluginError::invalid_params(
                            "cloud file expired; upload an explicitly selected new input instead",
                        ));
                    }
                    if record.state != "ready" {
                        return Err(PluginError::invalid_params(format!(
                            "cloud file is {}; query cloud_file_status before using it",
                            record.state
                        )));
                    }
                    (
                        record.mime,
                        record.kind,
                        record.filename,
                        record.sha256,
                        record.size_bytes,
                        None,
                        Some(record.reference),
                    )
                }
            };
            if images_only && kind != agena_domain::AttachmentKind::Image {
                return Err(PluginError::invalid_params(
                    "cloud_image_understanding accepts images only",
                ));
            }
            if !images_only
                && !matches!(
                    kind,
                    agena_domain::AttachmentKind::Pdf | agena_domain::AttachmentKind::File
                )
            {
                return Err(PluginError::invalid_params(
                    "cloud_document_understanding accepts PDF or UTF-8 text documents",
                ));
            }
            let block = match (self.provider, kind, inline.as_deref(), remote.as_deref()) {
                ("chatgpt", agena_domain::AttachmentKind::Image, bytes, remote) => {
                    let mut block = json!({"type":"input_image","detail":match input.detail{ImageDetail::Auto=>"auto",ImageDetail::Low=>"low",ImageDetail::High=>"high"}});
                    if let Some(bytes) = bytes {
                        block["image_url"] =
                            json!(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
                    } else {
                        block["file_id"] = json!(remote.unwrap());
                    }
                    block
                }
                ("chatgpt", agena_domain::AttachmentKind::File, Some(bytes), _) => {
                    json!({"type":"input_text","text":std::str::from_utf8(bytes).map_err(|_|PluginError::invalid_params("text input is not UTF-8"))?})
                }
                ("chatgpt", _, bytes, remote) => {
                    let mut block = json!({"type":"input_file"});
                    if let Some(bytes) = bytes {
                        block["file_data"] =
                            json!(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
                        block["filename"] = json!(filename);
                    } else {
                        block["file_id"] = json!(remote.unwrap());
                    }
                    block
                }
                ("claude", agena_domain::AttachmentKind::File, Some(bytes), _) => {
                    json!({"type":"text","text":std::str::from_utf8(bytes).map_err(|_|PluginError::invalid_params("text input is not UTF-8"))?})
                }
                ("claude", kind, bytes, remote) => {
                    json!({"type":if kind==agena_domain::AttachmentKind::Image{"image"}else{"document"},"source":if let Some(bytes)=bytes{json!({"type":"base64","media_type":mime,"data":STANDARD.encode(bytes)})}else{json!({"type":"file","file_id":remote.unwrap()})}})
                }
                ("gemini", agena_domain::AttachmentKind::File, Some(bytes), _) => {
                    json!({"text":std::str::from_utf8(bytes).map_err(|_|PluginError::invalid_params("text input is not UTF-8"))?})
                }
                ("gemini", _, Some(bytes), _) => {
                    json!({"inlineData":{"mimeType":mime,"data":STANDARD.encode(bytes)}})
                }
                ("gemini", _, _, Some(reference)) => {
                    json!({"fileData":{"mimeType":mime,"fileUri":reference}})
                }
                _ => {
                    return Err(PluginError::invalid_params(
                        "unsupported provider media input",
                    ));
                }
            };
            receipts.push(json!({"filename":filename,"mime":mime,"sha256":sha,"size_bytes":size,"transport":if remote.is_some(){"provider_file"}else{"inline"}}));
            parts.push(block);
        }
        let (url, body, usage) = match self.provider {
            "chatgpt" => {
                parts.push(json!({"type":"input_text","text":input.prompt}));
                (
                    self.endpoint("responses")?,
                    json!({"model":model,"input":[{"role":"user","content":parts}],"max_output_tokens":input.max_output_tokens,"store":false,"stream":false}),
                    ProviderUsageKind::OpenAiResponses,
                )
            }
            "claude" => {
                parts.push(json!({"type":"text","text":input.prompt}));
                (
                    self.endpoint("v1/messages")?,
                    json!({"model":model,"messages":[{"role":"user","content":parts}],"max_tokens":input.max_output_tokens,"stream":false}),
                    ProviderUsageKind::AnthropicMessages,
                )
            }
            _ => {
                parts.push(json!({"text":input.prompt}));
                (
                    self.endpoint(&format!("models/{model}:generateContent"))?,
                    json!({"contents":[{"role":"user","parts":parts}],"generationConfig":{"maxOutputTokens":input.max_output_tokens}}),
                    ProviderUsageKind::GeminiGenerateContent,
                )
            }
        };
        let response = self
            .response(
                self.authenticated(crate::PROVIDER_HTTP_CLIENT.post(url).json(&body), &key),
                "media analysis",
            )
            .await?;
        let operation = if images_only {
            "image_understanding"
        } else {
            "document_understanding"
        };
        let mut result = official_service::provider_output(
            self.host,
            self.root,
            self.provider,
            operation,
            &model,
            "Cloud media analysis",
            usage,
            response,
        )
        .await?;
        if let Some(payload) = result.payload.as_mut() {
            payload["media_inputs"] = json!(receipts);
            payload["input_sent"] = json!(true);
            payload["remote_file_created_by_analysis"] = json!(false);
        }
        result.output_text.push_str(&format!("\n[{} explicit input(s) processed using {}/{}. No automatic workspace upload; local inputs were sent inline, existing cloud handles were reused.]",receipts.len(),self.provider,model));
        Ok(result)
    }
}
