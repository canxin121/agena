//! OpenAI official-service tools exposed as ordinary Agena execution tools.
//!
//! These tools are discovered, described, authorized, and invoked through the
//! same Tool API catalog as every other bundled plugin. Their implementation
//! happens to call OpenAI service endpoints; that is an implementation detail,
//! not a second model-visible tool system.

use std::sync::{Arc, OnceLock};

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    HostClient, HostImageExecuteRequest, HostImageInput, HostImageOperation,
    HostNetworkPermissionCheckRequest,
};
use agena_plugin_host::sdk::{
    HostCapability, InitContext, InitOutcome, PathRequest, Result as SdkResult, ToolInvokeContext,
    ToolInvokeOutput,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const OPENAI_PLUGIN_ID: &str = "agena.openai";

pub(crate) struct OpenAiToolsPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
    config: OnceLock<OpenAiToolsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct OpenAiToolsConfig {
    base_url: String,
    api_key_env: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_model: Option<String>,
}

impl Default for OpenAiToolsConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_owned(),
            api_key_env: "OPENAI_API_KEY".to_owned(),
            default_model: None,
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
            host: OnceLock::new(),
            config: OnceLock::new(),
        }
    }

    fn host(&self) -> SdkResult<&Arc<dyn HostClient>> {
        self.host
            .get()
            .ok_or_else(|| PluginError::new("OpenAI tools plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&OpenAiToolsConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::new("OpenAI tools plugin invoked before init"))
    }

    fn responses_url(&self) -> SdkResult<String> {
        let base = self.config()?.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(PluginError::invalid_params(
                "OpenAI tools base_url must not be empty",
            ));
        }
        Ok(format!("{base}/responses"))
    }

    fn model(&self, requested: Option<String>) -> SdkResult<String> {
        requested
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .or_else(|| self.config().ok()?.default_model.clone())
            .or_else(|| std::env::var("OPENAI_MODEL").ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                PluginError::invalid_params(
                    "openai.web_search requires input.model, plugin config default_model, or OPENAI_MODEL",
                )
            })
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
                PluginError::new(format!(
                    "OpenAI API credential is unavailable; set environment variable {env_name}"
                ))
            })
    }

    async fn execute_image(
        &self,
        context: &ToolInvokeContext<'_>,
        operation: HostImageOperation,
        prompt: String,
        inputs: Vec<HostImageInput>,
        options: ImageOptions,
    ) -> SdkResult<ToolInvokeOutput> {
        let response = self
            .host()?
            .image_execute(HostImageExecuteRequest {
                session_id: Some(context.session_id),
                operation,
                prompt,
                inputs,
                background: option_value(options.background)?,
                size: option_value(options.size)?,
                quality: option_value(options.quality)?,
                moderation: option_value(options.moderation)?,
            })
            .await?;
        let operation_label = match response.operation {
            HostImageOperation::Generate => "generated",
            HostImageOperation::Edit => "edited",
        };
        let route = match response.adapter_id.as_deref() {
            Some(adapter) => format!("{}/{}/{}", response.provider_id, adapter, response.model_id),
            None => format!("{}/{}", response.provider_id, response.model_id),
        };
        let payload = serde_json::json!({
            "operation": response.operation,
            "provider_id": response.provider_id,
            "adapter_id": response.adapter_id,
            "model_id": response.model_id,
            "revised_prompt": response.revised_prompt,
            "artifacts": response.attachments.iter().map(|attachment| serde_json::json!({
                "mime": attachment.mime,
                "filename": attachment.filename,
                "size_bytes": attachment.size_bytes,
                "sha256": attachment.sha256,
                "source": attachment.source,
            })).collect::<Vec<_>>(),
        });
        Ok(ToolInvokeOutput::from_parts(
            format!("OpenAI image {operation_label}"),
            format!(
                "Image {operation_label} through route {route}; persisted {} managed attachment(s).",
                response.attachments.len()
            ),
            Some(payload),
            std::collections::BTreeMap::from([
                ("provider_id".to_owned(), response.provider_id),
                ("model_id".to_owned(), response.model_id),
                (
                    "attachment_count".to_owned(),
                    response.attachments.len().to_string(),
                ),
            ]),
            response.attachments,
        ))
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "openai",
    version = env!("CARGO_PKG_VERSION"),
    summary = "OpenAI official service capabilities exposed as ordinary Agena tools.",
    config_schema = agena_plugin_sdk::macro_support::json_schema_for_default(OpenAiToolsConfig::default()),
    display = detailed
)]
impl OpenAiToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: OpenAiToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_config(
                ctx.config,
                "invalid OpenAI tools plugin config",
            )?;
        self.config
            .set(config)
            .map_err(|_| PluginError::new("OpenAI tools plugin initialized more than once"))?;
        self.host
            .set(host)
            .map_err(|_| PluginError::new("OpenAI tools plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        summary = "Search the public web through OpenAI's official Responses web_search service.",
        help = "This is an ordinary Agena execution tool. Discovery, help, permission checks, and invocation all go through tools_list/tools_search/tools_help/tools_call; only the implementation uses OpenAI's official web_search service.",
        read_only,
        network,
        internet,
        discovery,
        display = detailed,
        capabilities(HostCapability::PermissionCheck),
        examples(r#"{"query":"Latest Rust language release notes","model":"gpt-4.1"}"#)
    )]
    async fn web_search(&self, input: OpenAiWebSearchInput) -> SdkResult<ToolInvokeOutput> {
        let url = self.responses_url()?;
        self.host()?
            .ensure_network_permission(HostNetworkPermissionCheckRequest::connect(url.clone()))
            .await?;
        let model = self.model(input.model)?;
        let response = reqwest::Client::new()
            .post(url.as_str())
            .bearer_auth(self.api_key()?)
            .json(&serde_json::json!({
                "model": model,
                "input": input.query,
                "tools": [{"type": "web_search"}],
                "include": ["web_search_call.action.sources"]
            }))
            .send()
            .await
            .map_err(|error| PluginError::new(format!("OpenAI web search request failed: {error}")))?;
        let status = response.status();
        let value: serde_json::Value = response.json().await.map_err(|error| {
            PluginError::new(format!("OpenAI web search returned invalid JSON: {error}"))
        })?;
        if !status.is_success() {
            let message = value
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("OpenAI web search failed");
            return Err(PluginError::new(format!("{message} (HTTP {status})")));
        }
        let output_text = openai_response_text(&value)
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "OpenAI web search completed without a text summary.".to_owned());
        Ok(ToolInvokeOutput::from_parts(
            "OpenAI web search",
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
        summary = "Generate an image through the configured OpenAI-compatible image service route.",
        help = "This is an ordinary openai.image_generation execution tool. The result is returned only after the host has copied provider output into the managed artifact store.",
        mutating,
        display = detailed,
        capabilities(HostCapability::ImageGeneration),
        examples(r#"{"prompt":"A watercolor map of a floating city","size":"1536x1024","quality":"high"}"#)
    )]
    async fn image_generation(
        &self,
        context: &ToolInvokeContext<'_>,
        input: ImageGenerateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.execute_image(
            context,
            HostImageOperation::Generate,
            input.prompt,
            Vec::new(),
            input.options,
        )
        .await
    }

    #[tool(
        summary = "Edit permitted local images through the configured OpenAI-compatible image service route.",
        help = "Every source path is permission-checked and materialized before crossing the provider boundary. This tool has the same catalog and permission status as every other Agena execution tool.",
        mutating,
        filesystem_read,
        display = detailed,
        capabilities(HostCapability::ImageGeneration),
        path(requests = input.images.iter().cloned().map(PathRequest::read).collect::<Vec<_>>()),
        examples(r#"{"prompt":"Replace the sky with an aurora","images":["assets/source.png"]}"#)
    )]
    async fn image_edit(
        &self,
        context: &ToolInvokeContext<'_>,
        input: ImageEditInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let inputs = input
            .images
            .into_iter()
            .map(|path| HostImageInput::Path { path })
            .collect();
        self.execute_image(
            context,
            HostImageOperation::Edit,
            input.prompt,
            inputs,
            input.options,
        )
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

fn option_value<T: Serialize>(value: Option<T>) -> SdkResult<Option<String>> {
    value
        .map(|value| {
            serde_json::to_value(value)
                .map_err(|error| PluginError::new(error.to_string()))
                .and_then(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| PluginError::new("image option did not serialize as text"))
                })
        })
        .transpose()
}
