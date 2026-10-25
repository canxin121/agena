//! Provider-owned multimodal inference and file resources. File reads and cloud
//! requests are separate declared effects. No clipboard access, implicit local
//! executor, credential forwarding, or cross-provider file-ID substitution.
mod files;
use super::official_service::{self, ProviderHttpResponse, ProviderUsageKind};
use agena_macros::ToolInput;
use agena_plugin_host::{
    PluginError,
    sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, host_api::HostClient},
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
    /// Optional override; defaults to the current session model. Use a model from the invoked plugin’s AI vendor that supports this operation; pass model explicitly when the current model is unsuitable.
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
    pub connection: agena_plugin_sdk::host_api::BuiltinProviderConnection,
    pub client: reqwest::Client,
    pub anthropic_version: &'a str,
}
impl<'a> Service<'a> {
    pub fn new(
        provider: &'static str,
        root: &'a Path,
        host: &'a Arc<dyn HostClient>,
        connection: agena_plugin_sdk::host_api::BuiltinProviderConnection,
        anthropic_version: &'a str,
    ) -> SdkResult<Self> {
        let client = official_service::provider_http_client(&connection)?;
        Ok(Self {
            provider,
            root,
            host,
            connection,
            client,
            anthropic_version,
        })
    }

    fn base(&self) -> SdkResult<url::Url> {
        let base = url::Url::parse(&self.connection.base_url).map_err(|e| {
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
        if !matches!(base.scheme(), "http" | "https") {
            return Err(PluginError::invalid_params(
                "provider media endpoint must use HTTP or HTTPS",
            ));
        }
        Ok(base)
    }
    fn endpoint(&self, suffix: &str) -> SdkResult<String> {
        self.base()?;
        official_service::endpoint(&self.connection.base_url, suffix)
    }
    fn authenticated(&self, mut request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request = request.header("x-agena-provider-api", self.provider);
        for (name, value) in &self.connection.headers {
            request = request.header(name, value);
        }
        if self.provider == "claude" && !self.connection.headers.contains_key("anthropic-version") {
            request = request.header("anthropic-version", self.anthropic_version);
        }
        request
    }
    async fn response(
        &self,
        request: reqwest::RequestBuilder,
        operation: &str,
    ) -> SdkResult<ProviderHttpResponse> {
        let request = request
            .build()
            .map_err(|error| PluginError::internal_error(&error))?;
        official_service::validate_provider_endpoint(self.host, request.url().as_str()).await?;
        let response=self.client.execute(request).await.map_err(|error|PluginError::internal(format!("cloud {operation} transport failed; remote acceptance is unknown, do not automatically repeat: {}",error.without_url())))?;
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
        self.analyze_kind(
            context,
            input,
            model,
            images_only.then_some(agena_domain::AttachmentKind::Image),
        )
        .await
    }

    pub async fn analyze_kind(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
        model: String,
        expected_kind: Option<agena_domain::AttachmentKind>,
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
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b':' | b'/'))
        {
            return Err(PluginError::invalid_params("invalid model identifier"));
        }
        if self.provider != "chatgpt" && input.detail != ImageDetail::Auto {
            return Err(PluginError::invalid_params(
                "this provider does not accept OpenAI's image detail setting; use detail=auto",
            ));
        }
        let key = &self.connection.receipt_key;
        let mut parts = Vec::new();
        let mut receipts = Vec::new();
        let mut total = 0usize;
        for source in &input.inputs {
            let (mime, kind, filename, sha, size, inline, remote) = match source {
                MediaSource::Local {
                    path,
                    expected_sha256,
                } => {
                    let local_path = self.root.join(path);
                    crate::permissions::require_path(
                        self.host.as_ref(),
                        agena_domain::PathKind::Read,
                        &local_path.to_string_lossy(),
                    )
                    .await?;
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
                            "inline analysis inputs exceed 12 MiB; use this provider's file_upload and a cloud handle",
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
                    let record = self.load_record(context, handle, key).await?;
                    if record.is_expired() {
                        return Err(PluginError::invalid_params(
                            "cloud file expired; upload an explicitly selected new input instead",
                        ));
                    }
                    if record.state != "ready" {
                        return Err(PluginError::invalid_params(format!(
                            "cloud file is {}; query file_status before using it",
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
            if expected_kind.is_some_and(|expected| kind != expected) {
                return Err(PluginError::invalid_params(
                    "input media kind does not match the requested understanding tool",
                ));
            }
            if expected_kind.is_none()
                && !matches!(
                    kind,
                    agena_domain::AttachmentKind::Pdf | agena_domain::AttachmentKind::File
                )
            {
                return Err(PluginError::invalid_params(
                    "document_understanding accepts PDF or UTF-8 text documents",
                ));
            }
            let block = match (self.provider, kind, inline.as_deref(), remote.as_deref()) {
                ("chatgpt", agena_domain::AttachmentKind::Audio, Some(bytes), _) => {
                    let format = match mime.as_str() {
                        "audio/wav" => "wav",
                        "audio/mpeg" => "mp3",
                        _ => {
                            return Err(PluginError::invalid_params(
                                "OpenAI audio understanding accepts WAV or MP3; convert explicitly first",
                            ));
                        }
                    };
                    json!({"type":"input_audio","input_audio":{"data":STANDARD.encode(bytes),"format":format}})
                }
                ("chatgpt", agena_domain::AttachmentKind::Audio, None, _) => {
                    return Err(PluginError::invalid_params(
                        "OpenAI Chat audio input requires an explicit local WAV/MP3 file, not a Files API handle",
                    ));
                }
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
            "chatgpt" if expected_kind == Some(agena_domain::AttachmentKind::Audio) => {
                parts.push(json!({"type":"text","text":input.prompt}));
                (
                    self.endpoint("chat/completions")?,
                    json!({"model":model,"messages":[{"role":"user","content":parts}],"max_completion_tokens":input.max_output_tokens,"stream":false}),
                    ProviderUsageKind::OpenAiResponses,
                )
            }
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
                    self.connection
                        .messages_url
                        .clone()
                        .unwrap_or(self.endpoint("messages")?),
                    json!({"model":model,"messages":[{"role":"user","content":parts}],"max_tokens":input.max_output_tokens,"stream":false}),
                    ProviderUsageKind::AnthropicMessages,
                )
            }
            _ => {
                parts.push(json!({"text":input.prompt}));
                (
                    official_service::gemini_model_endpoint(&self.connection.base_url, &model)?,
                    json!({"contents":[{"role":"user","parts":parts}],"generationConfig":{"maxOutputTokens":input.max_output_tokens}}),
                    ProviderUsageKind::GeminiGenerateContent,
                )
            }
        };
        let response = self
            .response(
                self.authenticated(self.client.post(url).json(&body)),
                "media analysis",
            )
            .await?;
        let operation = match expected_kind {
            Some(agena_domain::AttachmentKind::Image) => "image_understanding",
            Some(agena_domain::AttachmentKind::Audio) => "audio_understanding",
            Some(agena_domain::AttachmentKind::Video) => "video_understanding",
            _ => "document_understanding",
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
            &self.connection,
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
