//! ChatGPT/OpenAI official provider tools exposed as ordinary Agena tools.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, PathRequest, Result as SdkResult, ToolInvokeOutput,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::cloud_media::{AnalyzeInput, FileInput, Service, UploadInput};
use super::official_service::{
    ProviderHttpResponse, ProviderUsageKind, append_prompt_to_items, configured_model, endpoint,
    env_secret, merge_object_options, post_json, provider_output, read_image_input_bounded,
    read_json_response_bounded, resolve_local_path, stable_cache_key,
};
use agena_plugin_host::sdk::ToolInvokeContext;

pub(crate) const CHATGPT_PLUGIN_ID: &str = "agena.chatgpt";

pub(crate) struct ChatGptToolsPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
    workspace_root: OnceLock<PathBuf>,
    config: OnceLock<ChatGptToolsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct ChatGptToolsConfig {
    base_url: String,
    api_key_env: String,
    model: Option<String>,
    image_model: Option<String>,
    timeout_secs: u64,
    cache_namespace: String,
    cache_mode: OpenAiPromptCacheMode,
    stable_instructions: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum OpenAiPromptCacheMode {
    #[default]
    Automatic,
    Explicit,
    Disabled,
}

impl Default for ChatGptToolsConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_owned(),
            api_key_env: "OPENAI_API_KEY".to_owned(),
            model: None,
            image_model: None,
            timeout_secs: 180,
            cache_namespace: "agena-provider-tools".to_owned(),
            cache_mode: OpenAiPromptCacheMode::Automatic,
            stable_instructions: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "stable_instructions"),
    max_chars("prompt", 64000),
    max_chars("stable_instructions", 256000)
)]
#[serde(deny_unknown_fields)]
struct ChatGptToolInput {
    /// Instruction for a new hosted request. May be omitted when message history is supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    /// Stable developer prefix eligible for an explicit OpenAI cache breakpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stable_instructions: Option<String>,
    /// Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    /// Official fields merged into this tool's declaration. `type` is protected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    tool_options: BTreeMap<String, serde_json::Value>,
    /// Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
    /// Responses API continuation token from an earlier call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    /// Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    input_items: Vec<serde_json::Value>,
    /// Optional Responses include selectors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    include: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "images[]"),
    non_empty("prompt", "images[]"),
    min_items("images", 1),
    max_items("images", 16)
)]
#[serde(deny_unknown_fields)]
struct ChatGptImageEditInput {
    prompt: String,
    images: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    options: BTreeMap<String, serde_json::Value>,
}

impl ChatGptToolsPlugin {
    fn media_service(&self) -> SdkResult<Service<'_>> {
        Ok(Service {
            provider: "chatgpt",
            root: self.workspace_root()?,
            host: self.host()?,
            base_url: &self.config()?.base_url,
            key_env: &self.config()?.api_key_env,
            timeout_secs: self.config()?.timeout_secs,
            anthropic_version: "2023-06-01",
        })
    }

    pub(crate) fn new() -> Self {
        Self {
            host: OnceLock::new(),
            workspace_root: OnceLock::new(),
            config: OnceLock::new(),
        }
    }

    fn host(&self) -> SdkResult<&Arc<dyn HostClient>> {
        self.host
            .get()
            .ok_or_else(|| PluginError::internal("ChatGPT tools plugin invoked before init"))
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("ChatGPT tools plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&ChatGptToolsConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("ChatGPT tools plugin invoked before init"))
    }

    fn model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.model.as_deref(),
            &["CHATGPT_MODEL", "OPENAI_MODEL"],
            tool,
        )
    }

    fn image_model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.image_model.as_deref(),
            &["CHATGPT_IMAGE_MODEL", "OPENAI_IMAGE_MODEL"],
            tool,
        )
    }

    async fn responses_tool(
        &self,
        tool_name: &str,
        title: &str,
        declaration: serde_json::Value,
        input: ChatGptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::official_service::hosted::validate_options(&input.tool_options, "tool_options")?;
        super::official_service::hosted::validate_options(
            &input.request_options,
            "request_options",
        )?;
        super::official_service::hosted::validate_history(
            "chatgpt",
            &serde_json::json!(&input.input_items),
        )?;
        let model = self.model(input.model, format!("chatgpt.cloud_{tool_name}").as_str())?;
        let declaration =
            merge_object_options(declaration, &input.tool_options, &["type"], "tool_options")?;
        super::official_service::hosted::validate_declaration("chatgpt", tool_name, &declaration)?;
        let previous_response_id = input.previous_response_id.clone();
        let mut provider_input = append_prompt_to_items(input.input_items, input.prompt, true)?;
        let stable_instructions = input
            .stable_instructions
            .or_else(|| self.config().ok()?.stable_instructions.clone())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let cache_mode = self.config()?.cache_mode;
        let supports_explicit_cache = model.to_ascii_lowercase().starts_with("gpt-5.6");
        if matches!(cache_mode, OpenAiPromptCacheMode::Explicit) && !supports_explicit_cache {
            return Err(PluginError::invalid_params(
                "explicit OpenAI prompt caching requires a GPT-5.6 or later model",
            ));
        }
        if matches!(cache_mode, OpenAiPromptCacheMode::Explicit)
            && previous_response_id.is_none()
            && stable_instructions.is_none()
        {
            return Err(PluginError::invalid_params(
                "explicit OpenAI prompt caching requires stable_instructions on the initial request",
            ));
        }
        if previous_response_id.is_none()
            && let Some(stable) = stable_instructions.as_deref()
        {
            let content = if matches!(cache_mode, OpenAiPromptCacheMode::Explicit) {
                serde_json::json!([{
                    "type": "input_text",
                    "text": stable,
                    "prompt_cache_breakpoint": {"mode": "explicit"}
                }])
            } else {
                serde_json::json!([{
                    "type": "input_text",
                    "text": stable
                }])
            };
            let developer = serde_json::json!({
                "role": "developer",
                "content": content
            });
            provider_input = match provider_input {
                serde_json::Value::Array(mut items) => {
                    items.insert(0, developer);
                    serde_json::Value::Array(items)
                }
                value => serde_json::Value::Array(vec![
                    developer,
                    serde_json::json!({"role":"user","content":value}),
                ]),
            };
        }
        let mut base = serde_json::json!({
            "model": model.clone(),
            "input": provider_input,
            "tools": [declaration],
            "stream": false,
        });
        if !matches!(cache_mode, OpenAiPromptCacheMode::Disabled) {
            base["prompt_cache_key"] = serde_json::Value::String(stable_cache_key(
                self.config()?.cache_namespace.as_str(),
                self.workspace_root()?,
                "chatgpt",
                model.as_str(),
                tool_name,
            ));
        }
        if supports_explicit_cache && !matches!(cache_mode, OpenAiPromptCacheMode::Disabled) {
            base["prompt_cache_options"] = match cache_mode {
                OpenAiPromptCacheMode::Automatic => {
                    serde_json::json!({"mode":"implicit","ttl":"30m"})
                }
                OpenAiPromptCacheMode::Explicit => {
                    serde_json::json!({"mode":"explicit","ttl":"30m"})
                }
                OpenAiPromptCacheMode::Disabled => unreachable!(),
            };
        }
        if let Some(previous_response_id) = previous_response_id {
            base["previous_response_id"] = serde_json::Value::String(previous_response_id);
        }
        if !input.include.is_empty() {
            base["include"] = serde_json::to_value(&input.include).map_err(|error| {
                PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize ChatGPT Responses include fields",
                    &error,
                ))
            })?;
        }
        let body = merge_object_options(
            base,
            &input.request_options,
            &[
                "model",
                "input",
                "tools",
                "stream",
                "previous_response_id",
                "prompt_cache_key",
                "prompt_cache_options",
            ],
            "request_options",
        )?;
        let url = endpoint(self.config()?.base_url.as_str(), "responses")?;
        let headers = BTreeMap::from([
            (
                "authorization".to_owned(),
                format!(
                    "Bearer {}",
                    env_secret(self.config()?.api_key_env.as_str(), "ChatGPT/OpenAI")?
                ),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]);
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            &body,
            self.config()?.timeout_secs,
            "chatgpt",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "chatgpt",
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::OpenAiResponses,
            response,
        )
        .await
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "chatgpt",
    version = env!("CARGO_PKG_VERSION"),
    summary = "OpenAI cloud search, computation and image capabilities. Inputs leave this computer; no local execution fallback.",
    settings = ChatGptToolsConfig,
    settings_default = default,
)]
impl ChatGptToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: ChatGptToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
                ctx.settings,
                "invalid ChatGPT tools plugin config",
            )?;
        self.workspace_root.set(ctx.workspace_root).map_err(|_| {
            PluginError::internal("ChatGPT tools plugin initialized more than once")
        })?;
        self.config.set(config).map_err(|_| {
            PluginError::internal("ChatGPT tools plugin initialized more than once")
        })?;
        self.host.set(host).map_err(|_| {
            PluginError::internal("ChatGPT tools plugin initialized more than once")
        })?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(name="cloud_image_understanding",tags(query,network),
        summary="Send explicit images to OpenAI cloud for understanding; not local file viewing.",
        help="Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        mutating,network(connect=self.config()?.base_url.clone()),path(requests=input.paths(self.workspace_root()?)))]
    async fn image_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "chatgpt.cloud_image_understanding")?;
        self.media_service()?
            .analyze(context, input, model, true)
            .await
    }

    #[tool(name="cloud_document_understanding",tags(query,network),
        summary="Send explicit PDF/text documents to OpenAI cloud for understanding; not local file viewing.",
        help="Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        mutating,network(connect=self.config()?.base_url.clone()),path(requests=input.paths(self.workspace_root()?)))]
    async fn document_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "chatgpt.cloud_document_understanding")?;
        self.media_service()?
            .analyze(context, input, model, false)
            .await
    }

    #[tool(name="cloud_file_upload",tags(mutate,network),
        summary="Upload one permitted local file to OpenAI cloud and return a session-owned handle.",
        help="Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.",
        mutating,network(connect=self.config()?.base_url.clone()),path(requests=vec![PathRequest::read(input.path.clone()),PathRequest::write(self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string())]))]
    async fn file_upload(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &UploadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.media_service()?.upload(context, input).await
    }

    #[tool(name="cloud_file_status",tags(query,network),
        summary="Query the remote status of an owned OpenAI cloud file, not a local path.",
        help="Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only cloud_file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.",
        mutating,network(connect=self.config()?.base_url.clone()),path(requests=vec![PathRequest::read(self.workspace_root()?.join(".agena/artifacts/provider-tools/media").display().to_string()),PathRequest::write(self.workspace_root()?.join(".agena/artifacts/provider-tools/media").display().to_string())]))]
    async fn file_status(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &FileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.media_service()?
            .file_control(context, input, false)
            .await
    }

    #[tool(name="cloud_file_delete",tags(mutate,network),
        summary="Request deletion of an owned file from OpenAI cloud; preserve the local original.",
        help="Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.",
        mutating,network(connect=self.config()?.base_url.clone()),path(requests=vec![PathRequest::read(self.workspace_root()?.join(".agena/artifacts/provider-tools/media").display().to_string()),PathRequest::write(self.workspace_root()?.join(".agena/artifacts/provider-tools/media").display().to_string())]))]
    async fn file_delete(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &FileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.media_service()?
            .file_control(context, input, true)
            .await
    }
    #[tool(
        name = "cloud_web_search",
        network(connect = self.config()?.base_url.clone()),
        path(write = self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()),
        tags(network, interactive),
        summary = "Search the web in OpenAI cloud and return sources; not a local browser operation.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options accepts the official WebSearchToolParam fields: filters.allowed_domains, search_context_size, user_location, and versioned type-compatible options. Hosted results and response_id are returned for follow-up; this plugin never executes client tool callbacks.",
        read_only,
        discovery
    )]
    async fn web_search(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "web_search",
            "ChatGPT web search",
            serde_json::json!({"type":"web_search"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_file_search",
        network(connect = self.config()?.base_url.clone()),
        path(write = self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()),
        tags(network, interactive),
        summary = "Search configured OpenAI cloud file stores, not files on this computer.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Provider file-store identifiers refer to remote resources, not local filesystem paths. Set tool_options.vector_store_ids and optional filters, max_num_results, and ranking_options exactly as documented by OpenAI.",
        read_only,
        discovery
    )]
    async fn file_search(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "file_search",
            "ChatGPT file search",
            serde_json::json!({"type":"file_search"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_code_interpreter",
        network(connect = self.config()?.base_url.clone()),
        path(write = self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()),
        tags(network, interactive),
        summary = "Run Python in an OpenAI cloud container, not the Agena local workspace.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. tool_options.container may be a container id or an auto container object with file_ids, memory_limit, and network_policy.",
        read_only
    )]
    async fn code_interpreter(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "code_interpreter",
            "ChatGPT code interpreter",
            serde_json::json!({"type":"code_interpreter","container":{"type":"auto"}}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_image_generation",
        network(connect = self.config()?.base_url.clone()),
        path(write = self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()),
        tags(network, interactive),
        summary = "Generate images in OpenAI cloud; save returned images as local attachments.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options supports action, model, background, input_fidelity, input_image_mask, moderation, output_compression, output_format, partial_images, quality, and size. Returned base64 images are persisted as managed attachments.",
        mutating
    )]
    async fn image_generation(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "image_generation",
            "ChatGPT image generation",
            serde_json::json!({"type":"image_generation"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_shell",
        network(connect = self.config()?.base_url.clone()),
        path(write = self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()),
        tags(network, interactive),
        summary = "Run shell commands in an OpenAI cloud container, never in the local terminal.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Defaults to container_auto. Only container_auto or container_reference with container_id is accepted. Local/custom environments and client callbacks are rejected. Uploaded provider files are separate from Agena local files; there is no local execution fallback.",
        mutating
    )]
    async fn shell(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "shell",
            "ChatGPT shell",
            serde_json::json!({"type":"shell","environment":{"type":"container_auto"}}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_image_edit",
        network(connect = self.config()?.base_url.clone()),summary = "Upload permitted images for editing in OpenAI cloud; save the returned image separately.", help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Permission-checked local images are uploaded to OpenAI; returned images are saved as separate local artifacts. This convenience entry preserves the official image edit endpoint alongside the Responses image_generation tool. Every input and output path is permission checked.", mutating, path(requests = input.images.iter().cloned().map(PathRequest::read).chain(std::iter::once(PathRequest::write(self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()))).collect::<Vec<_>>()))]
    async fn image_edit(&self, input: ChatGptImageEditInput) -> SdkResult<ToolInvokeOutput> {
        super::official_service::hosted::validate_options(&input.options, "options")?;
        let model = self.image_model(input.model, "chatgpt.cloud_image_edit")?;
        let url = endpoint(self.config()?.base_url.as_str(), "images/edits")?;
        super::official_service::validate_provider_endpoint(self.host()?, url.as_str()).await?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", model.clone())
            .text("prompt", input.prompt)
            .text("output_format", "png");
        for (key, value) in input.options {
            if matches!(key.as_str(), "model" | "prompt" | "image") {
                return Err(PluginError::invalid_params(format!(
                    "options.{key} is protected"
                )));
            }
            form = form.text(
                key,
                match value {
                    serde_json::Value::String(value) => value,
                    value => value.to_string(),
                },
            );
        }
        let mut image_input_bytes = 0_u64;
        for source in input.images {
            let path = resolve_local_path(self.workspace_root()?, source.as_str())?;
            let bytes = read_image_input_bounded(&path, &mut image_input_bytes, "OpenAI").await?;
            let filename = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("image.png")
                .to_owned();
            let mime = match path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("webp") => "image/webp",
                Some("gif") => "image/gif",
                _ => "image/png",
            };
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(filename)
                .mime_str(mime)
                .map_err(|error| PluginError::internal(format!("invalid image MIME: {error}")))?;
            form = form.part("image[]", part);
        }
        let response = crate::PROVIDER_HTTP_CLIENT
            .post(url)
            .timeout(std::time::Duration::from_secs(
                self.config()?.timeout_secs.max(1),
            ))
            .bearer_auth(env_secret(
                self.config()?.api_key_env.as_str(),
                "ChatGPT/OpenAI",
            )?)
            .multipart(form)
            .send()
            .await
            .map_err(|error| PluginError::internal(format!("OpenAI image edit failed: {error}")))?;
        let (status, request_id, value) =
            read_json_response_bounded(response, "OpenAI", "image edit").await?;
        if !status.is_success() {
            return Err(PluginError::internal(format!(
                "OpenAI image edit failed (HTTP {status}): {value}"
            )));
        }
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "chatgpt",
            "image_edit",
            model.as_str(),
            "ChatGPT image edit",
            ProviderUsageKind::OpenAiImage,
            ProviderHttpResponse { value, request_id },
        )
        .await
    }
}
