//! OpenAI official-service tools exposed as ordinary Agena execution tools.
//!
//! These tools are discovered, described, authorized, and invoked through the
//! same Tool API catalog as every other bundled plugin. Their implementation
//! happens to call OpenAI HTTP endpoints; that is an implementation detail,
//! not a second model-visible tool system.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::attachment::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, PathRequest, Result as SdkResult, ToolInvokeOutput,
};
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::official_service::{read_image_input_bounded, read_json_response_bounded};

pub(crate) const OPENAI_PLUGIN_ID: &str = "agena.openai";
const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024;

pub(crate) struct OpenAiToolsPlugin {
    workspace_root: OnceLock<PathBuf>,
    config: OnceLock<OpenAiToolsConfig>,
    client: OnceLock<reqwest::Client>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct OpenAiToolsConfig {
    base_url: String,
    api_key_env: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    responses_model: Option<String>,
    image_model: String,
    timeout_secs: u64,
}

impl Default for OpenAiToolsConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_owned(),
            api_key_env: "OPENAI_API_KEY".to_owned(),
            responses_model: None,
            image_model: "gpt-image-1".to_owned(),
            timeout_secs: 180,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ImageBackground {
    Auto,
    Opaque,
    Transparent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
enum ImageSize {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "1024x1024")]
    Square,
    #[serde(rename = "1536x1024")]
    Landscape,
    #[serde(rename = "1024x1536")]
    Portrait,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ImageQuality {
    Auto,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ImageModeration {
    Auto,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct ImageOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<ImageBackground>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<ImageSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quality: Option<ImageQuality>,
    #[serde(skip_serializing_if = "Option::is_none")]
    moderation: Option<ImageModeration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("query", "model"), non_empty("query"), max_chars("query", 32000))]
#[serde(deny_unknown_fields)]
struct OpenAiWebSearchInput {
    /// Search question or research instruction sent to OpenAI Responses.
    query: String,
    /// Optional model override. Otherwise plugin config or OPENAI_MODEL is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("prompt"), non_empty("prompt"), max_chars("prompt", 32000))]
#[serde(deny_unknown_fields)]
struct ImageGenerateInput {
    /// Detailed description of the image to create.
    prompt: String,
    #[serde(default, flatten)]
    options: ImageOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "images[]"),
    non_empty("prompt", "images[]"),
    max_chars("prompt", 32000),
    min_items("images", 1),
    max_items("images", 16)
)]
#[serde(deny_unknown_fields)]
struct ImageEditInput {
    /// Description of the requested transformation.
    prompt: String,
    /// Permitted local image paths used as edit references.
    images: Vec<String>,
    #[serde(default, flatten)]
    options: ImageOptions,
}

impl OpenAiToolsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            workspace_root: OnceLock::new(),
            config: OnceLock::new(),
            client: OnceLock::new(),
        }
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("OpenAI tools plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&OpenAiToolsConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("OpenAI tools plugin invoked before init"))
    }

    fn client(&self) -> SdkResult<&reqwest::Client> {
        self.client
            .get()
            .ok_or_else(|| PluginError::internal("OpenAI tools plugin invoked before init"))
    }

    fn endpoint(&self, path: &str) -> SdkResult<String> {
        let base = self.config()?.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(PluginError::invalid_params(
                "OpenAI tools base_url must not be empty",
            ));
        }
        Ok(format!("{base}/{}", path.trim_start_matches('/')))
    }

    fn responses_model(&self, requested: Option<String>) -> SdkResult<String> {
        requested
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .or_else(|| self.config().ok()?.responses_model.clone())
            .or_else(|| std::env::var("OPENAI_MODEL").ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                PluginError::invalid_params(
                    "openai.web_search requires input.model, plugin config responses_model, or OPENAI_MODEL",
                )
            })
    }

    fn image_model(&self) -> SdkResult<String> {
        let value = self.config()?.image_model.trim();
        if value.is_empty() {
            return Err(PluginError::invalid_params(
                "OpenAI tools image_model must not be empty",
            ));
        }
        Ok(value.to_owned())
    }

    fn api_key(&self) -> SdkResult<String> {
        let env_name = self.config()?.api_key_env.trim();
        if env_name.is_empty() {
            return Err(PluginError::invalid_params(
                "OpenAI tools api_key_env must not be empty",
            ));
        }
        std::env::var(env_name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "OpenAI API credential is unavailable; set environment variable {env_name}"
                ))
            })
    }

    fn resolve_input_path(&self, value: &str) -> SdkResult<PathBuf> {
        let value = value.trim();
        if value.is_empty() {
            return Err(PluginError::invalid_params("image path must not be empty"));
        }
        let path = Path::new(value);
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root()?.join(path)
        })
    }

    async fn image_response_output(
        &self,
        title: &str,
        model: String,
        response: serde_json::Value,
    ) -> SdkResult<ToolInvokeOutput> {
        let encoded = response
            .pointer("/data/0/b64_json")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                let message = response
                    .pointer("/error/message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("OpenAI image response did not contain data[0].b64_json");
                PluginError::internal(message)
            })?;
        const MAX_IMAGE_BASE64_BYTES: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4 + 4;
        if encoded.len() > MAX_IMAGE_BASE64_BYTES {
            return Err(PluginError::internal(
                "OpenAI image exceeds the 50 MiB artifact limit",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| {
                PluginError::internal(format!("invalid OpenAI image data: {error}"))
            })?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(PluginError::internal(
                "OpenAI image exceeds the 50 MiB artifact limit",
            ));
        }
        let path = self
            .workspace_root()?
            .join(".agena/artifacts/openai/images")
            .join(format!("{}.png", uuid::Uuid::new_v4().simple()));
        let size_bytes = bytes.len() as u64;
        let sha256 = hex::encode(Sha256::digest(bytes.as_slice()));
        crate::artifact_file::persist_new(path.clone(), bytes, "OpenAI image").await?;
        let revised_prompt = response
            .pointer("/data/0/revised_prompt")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let attachment = AttachmentItem {
            kind: AttachmentKind::Image,
            mime: "image/png".to_owned(),
            source: AttachmentSource::LocalPath {
                path: path.to_string_lossy().to_string(),
            },
            filename: path
                .file_name()
                .and_then(|value| value.to_str())
                .map(ToOwned::to_owned),
            title: Some(title.to_owned()),
            size_bytes: Some(size_bytes),
            sha256: Some(sha256.clone()),
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        };
        Ok(ToolInvokeOutput::from_parts(
            title,
            format!("image/png · {size_bytes} bytes"),
            format!("Saved OpenAI image artifact to '{}'.", path.display()),
            Some(serde_json::json!({
                "provider": "openai",
                "model": model,
                "path": path,
                "mime": "image/png",
                "size_bytes": size_bytes,
                "sha256": sha256,
                "revised_prompt": revised_prompt,
            })),
            std::collections::BTreeMap::from([
                ("agena.effect".to_owned(), "file_changes".to_owned()),
                ("provider".to_owned(), "openai".to_owned()),
                ("model".to_owned(), model),
                ("path".to_owned(), path.to_string_lossy().to_string()),
            ]),
            vec![attachment],
        ))
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "openai",
    version = env!("CARGO_PKG_VERSION"),
    summary = "OpenAI official service capabilities exposed as ordinary Agena tools.",
    settings = OpenAiToolsConfig,
    settings_default = default,
)]
impl OpenAiToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, _host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: OpenAiToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
                ctx.settings,
                "invalid OpenAI tools plugin config",
            )?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs.max(1)))
            .build()
            .map_err(|error| {
                PluginError::internal(format!("cannot create OpenAI HTTP client: {error}"))
            })?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::internal("OpenAI tools plugin initialized more than once"))?;
        self.config
            .set(config)
            .map_err(|_| PluginError::internal("OpenAI tools plugin initialized more than once"))?;
        self.client
            .set(client)
            .map_err(|_| PluginError::internal("OpenAI tools plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(network, interactive),
        summary = "Search the public web through OpenAI's official Responses web-search service.",
        help = "This is an ordinary Agena execution tool. Discovery, help, permission checks, and invocation all go through tools_list/tools_search/tools_help/tools_call; only the implementation uses an OpenAI endpoint.",
        read_only,
        discovery,
        examples(r#"{"query":"Latest Rust language release notes","model":"gpt-4.1"}"#)
    )]
    async fn web_search(&self, input: OpenAiWebSearchInput) -> SdkResult<ToolInvokeOutput> {
        let url = self.endpoint("responses")?;
        let model = self.responses_model(input.model)?;
        let response = self
            .client()?
            .post(url.as_str())
            .bearer_auth(self.api_key()?)
            .json(&serde_json::json!({
                "model": model.clone(),
                "input": input.query,
                "tools": [{"type": "web_search"}],
                "include": ["web_search_call.action.sources"]
            }))
            .send()
            .await
            .map_err(|error| {
                PluginError::internal(format!("OpenAI web search request failed: {error}"))
            })?;
        let (status, _request_id, value) =
            read_json_response_bounded(response, "OpenAI", "web search").await?;
        if !status.is_success() {
            let message = value
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("OpenAI web search failed");
            return Err(PluginError::internal(format!("{message} (HTTP {status})")));
        }
        let output_text = openai_response_text(&value)
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "OpenAI web search completed without a text summary.".to_owned());
        Ok(ToolInvokeOutput::from_parts(
            "OpenAI web search",
            "Response received",
            output_text,
            Some(value),
            std::collections::BTreeMap::from([
                ("provider".to_owned(), "openai".to_owned()),
                ("model".to_owned(), model),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        tags(network, interactive),
        summary = "Generate an image through OpenAI's image generation endpoint.",
        help = "This is an ordinary openai.image_generation execution tool. It performs its own permission-checked OpenAI request and persists the returned base64 image into Agena's managed artifact directory.",
        mutating,



        examples(r#"{"prompt":"A watercolor map of a floating city","size":"1536x1024","quality":"high"}"#)
    )]
    async fn image_generation(&self, input: ImageGenerateInput) -> SdkResult<ToolInvokeOutput> {
        let url = self.endpoint("images/generations")?;
        let model = self.image_model()?;
        let mut body = serde_json::json!({
            "model": model.clone(),
            "prompt": input.prompt,
            "output_format": "png"
        });
        apply_image_options_to_json(&mut body, &input.options)?;
        let response = self
            .client()?
            .post(url.as_str())
            .bearer_auth(self.api_key()?)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                PluginError::internal(format!("OpenAI image request failed: {error}"))
            })?;
        let (status, _request_id, value) =
            read_json_response_bounded(response, "OpenAI", "image generation").await?;
        if !status.is_success() {
            return Err(openai_api_error("image generation", status, &value));
        }
        self.image_response_output("OpenAI image generation", model, value)
            .await
    }

    #[tool(
        tags(network, interactive),
        summary = "Edit permitted local images through OpenAI's image edit endpoint.",
        help = "Every source path is permission-checked before it is uploaded. The completed image is persisted as a managed local attachment, and this tool has the same catalog and permission status as every other Agena execution tool.",
        mutating,




        path(requests = input.images.iter().cloned().map(PathRequest::read).collect::<Vec<_>>()),
        examples(r#"{"prompt":"Replace the sky with an aurora","images":["assets/source.png"]}"#)
    )]
    async fn image_edit(&self, input: ImageEditInput) -> SdkResult<ToolInvokeOutput> {
        let url = self.endpoint("images/edits")?;
        let model = self.image_model()?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", model.clone())
            .text("prompt", input.prompt)
            .text("output_format", "png");
        form = apply_image_options_to_form(form, &input.options)?;
        let mut image_input_bytes = 0_u64;
        for source in input.images {
            let path = self.resolve_input_path(source.as_str())?;
            let bytes = read_image_input_bounded(&path, &mut image_input_bytes, "OpenAI").await?;
            let filename = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("image.png")
                .to_owned();
            let mime = image_mime_from_path(&path);
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(filename)
                .mime_str(mime)
                .map_err(|error| PluginError::internal(format!("invalid image MIME: {error}")))?;
            form = form.part("image[]", part);
        }
        let response = self
            .client()?
            .post(url.as_str())
            .bearer_auth(self.api_key()?)
            .multipart(form)
            .send()
            .await
            .map_err(|error| PluginError::internal(format!("OpenAI image edit failed: {error}")))?;
        let (status, _request_id, value) =
            read_json_response_bounded(response, "OpenAI", "image edit").await?;
        if !status.is_success() {
            return Err(openai_api_error("image edit", status, &value));
        }
        self.image_response_output("OpenAI image edit", model, value)
            .await
    }
}

fn openai_response_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.get("output_text").and_then(serde_json::Value::as_str) {
        return Some(text.to_owned());
    }
    let text = value
        .get("output")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|item| {
            item.get("content")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn image_option<T: Serialize>(value: Option<T>) -> SdkResult<Option<String>> {
    value
        .map(|value| {
            serde_json::to_value(value)
                .map_err(|error| PluginError::internal_error(&error))
                .and_then(|value| {
                    value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        PluginError::internal("image option did not serialize as text")
                    })
                })
        })
        .transpose()
}

fn apply_image_options_to_json(
    body: &mut serde_json::Value,
    options: &ImageOptions,
) -> SdkResult<()> {
    let object = body
        .as_object_mut()
        .ok_or_else(|| PluginError::internal("image request body is not an object"))?;
    for (key, value) in [
        ("background", image_option(options.background)?),
        ("size", image_option(options.size)?),
        ("quality", image_option(options.quality)?),
        ("moderation", image_option(options.moderation)?),
    ] {
        if let Some(value) = value {
            object.insert(key.to_owned(), serde_json::Value::String(value));
        }
    }
    Ok(())
}

fn apply_image_options_to_form(
    mut form: reqwest::multipart::Form,
    options: &ImageOptions,
) -> SdkResult<reqwest::multipart::Form> {
    for (key, value) in [
        ("background", image_option(options.background)?),
        ("size", image_option(options.size)?),
        ("quality", image_option(options.quality)?),
        ("moderation", image_option(options.moderation)?),
    ] {
        if let Some(value) = value {
            form = form.text(key, value);
        }
    }
    Ok(form)
}

fn image_mime_from_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/png",
    }
}

fn openai_api_error(
    operation: &str,
    status: reqwest::StatusCode,
    value: &serde_json::Value,
) -> PluginError {
    let message = value
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("OpenAI request failed");
    PluginError::internal(format!(
        "OpenAI {operation} failed: {message} (HTTP {status})"
    ))
}
