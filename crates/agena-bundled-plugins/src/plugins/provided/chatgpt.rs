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

use super::official_service::{
    ProviderHttpResponse, ProviderUsageKind, append_prompt_to_items, configured_model, endpoint,
    env_secret, merge_object_options, post_json, provider_output, resolve_local_path,
    stable_cache_key,
};

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
    /// Instruction for a new request. Optional when continuation items are supplied.
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
    /// Official callback/output items used to continue Computer, Shell, Patch, MCP, or Tool Search calls.
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
        let model = self.model(input.model, format!("chatgpt.{tool_name}").as_str())?;
        let declaration =
            merge_object_options(declaration, &input.tool_options, &["type"], "tool_options")?;
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
            base["include"] = serde_json::to_value(input.include).unwrap_or_default();
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
    summary = "OpenAI Responses and image service tools exposed as ordinary Agena tools.",
    config_schema = agena_plugin_sdk::macro_support::json_schema_for_default(ChatGptToolsConfig::default()),
    display = detailed
)]
impl ChatGptToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: ChatGptToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_config(
                ctx.config,
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

    #[tool(tags(network, interactive), summary = "Use OpenAI's current Responses web_search tool.", help = "tool_options accepts the official WebSearchToolParam fields: filters.allowed_domains, search_context_size, user_location, and versioned type-compatible options. Pending calls and response_id are returned for continuation.", read_only, discovery, display = detailed)]
    async fn web_search(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "web_search",
            "ChatGPT web search",
            serde_json::json!({"type":"web_search"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Use OpenAI's compatibility web_search_preview tool.", help = "Supports official preview fields such as search_content_types, search_context_size, and user_location.", read_only, discovery, display = detailed)]
    async fn web_search_preview(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "web_search_preview",
            "ChatGPT web search preview",
            serde_json::json!({"type":"web_search_preview"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Search OpenAI vector stores with the official file_search tool.", help = "Set tool_options.vector_store_ids and optional filters, max_num_results, and ranking_options exactly as documented by OpenAI.", read_only, discovery, display = detailed)]
    async fn file_search(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "file_search",
            "ChatGPT file search",
            serde_json::json!({"type":"file_search"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Run OpenAI's current computer tool and return pending computer calls.", help = "When the response contains computer_call items, execute the requested actions in Agena's browser/computer environment and call this tool again with previous_response_id plus official computer_call_output items in input_items.", mutating, display = detailed)]
    async fn computer(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "computer",
            "ChatGPT computer",
            serde_json::json!({"type":"computer"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Run OpenAI's computer_use_preview compatibility tool.", help = "Set display_width, display_height, and environment in tool_options. Continue with computer_call_output items.", mutating, display = detailed)]
    async fn computer_use_preview(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "computer_use_preview",
            "ChatGPT computer use preview",
            serde_json::json!({"type":"computer_use_preview"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Connect OpenAI Responses to an official remote MCP server or connector.", help = "Set server_label and one of server_url, connector_id, or tunnel_id in tool_options. Official allowed_tools, authorization, headers, require_approval, defer_loading, and allowed_callers fields are preserved.", mutating, display = detailed)]
    async fn mcp(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "mcp",
            "ChatGPT MCP",
            serde_json::json!({"type":"mcp"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Run Python with OpenAI's hosted code_interpreter tool.", help = "tool_options.container may be a container id or an auto container object with file_ids, memory_limit, and network_policy.", read_only, display = detailed)]
    async fn code_interpreter(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "code_interpreter",
            "ChatGPT code interpreter",
            serde_json::json!({"type":"code_interpreter","container":{"type":"auto"}}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Enable OpenAI programmatic tool calling.", help = "This official Responses tool lets generated programs invoke eligible tools. Use input_items to continue any resulting calls.", mutating, display = detailed)]
    async fn programmatic_tool_calling(
        &self,
        input: ChatGptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "programmatic_tool_calling",
            "ChatGPT programmatic tool calling",
            serde_json::json!({"type":"programmatic_tool_calling"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Generate or edit an image with OpenAI's Responses image_generation tool.", help = "tool_options supports action, model, background, input_fidelity, input_image_mask, moderation, output_compression, output_format, partial_images, quality, and size. Returned base64 images are persisted as managed attachments.", mutating,  display = detailed)]
    async fn image_generation(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "image_generation",
            "ChatGPT image generation",
            serde_json::json!({"type":"image_generation"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Expose OpenAI's local_shell protocol tool as an ordinary Agena request.", help = "The provider returns local_shell_call items. Execute them with Agena shell permissions, then continue using previous_response_id and local_shell_call_output items.", mutating, display = detailed)]
    async fn local_shell(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "local_shell",
            "ChatGPT local shell",
            serde_json::json!({"type":"local_shell"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Expose OpenAI's shell tool with official environment configuration.", help = "tool_options.environment accepts OpenAI local/container environment objects. Execute pending shell_call items under Agena permissions and continue with shell_call_output items.", mutating, display = detailed)]
    async fn shell(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "shell",
            "ChatGPT shell",
            serde_json::json!({"type":"shell"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Use OpenAI hosted or client tool_search.", help = "Set tool_options.execution to server or client, plus optional description and parameters. Continue client calls with tool_search_output items in input_items.", read_only, discovery, display = detailed)]
    async fn tool_search(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "tool_search",
            "ChatGPT tool search",
            serde_json::json!({"type":"tool_search","execution":"server"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary = "Expose OpenAI's apply_patch protocol tool.", help = "Execute returned apply_patch_call operations through Agena's permission-checked fs.apply_patch path, then continue with apply_patch_call_output items.", mutating,   display = detailed)]
    async fn apply_patch(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "apply_patch",
            "ChatGPT apply patch",
            serde_json::json!({"type":"apply_patch"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), name = "function", summary = "Send an official OpenAI function tool declaration.", help = "Set tool_options.name, description, parameters, and strict. This remains an ordinary Agena wrapper; returned function calls are continued through input_items.", mutating, display = detailed)]
    async fn function_tool(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "function",
            "ChatGPT function tool",
            serde_json::json!({"type":"function"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), name = "custom", summary = "Send an official OpenAI custom tool declaration.", help = "Set the official custom tool name, description, and format fields in tool_options; continue custom_tool_call outputs through input_items.", mutating, display = detailed)]
    async fn custom_tool(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "custom",
            "ChatGPT custom tool",
            serde_json::json!({"type":"custom"}),
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), name = "namespace", summary = "Send an official OpenAI namespace tool declaration.", help = "Use tool_options to define the namespace and nested tools according to the current Responses schema.", mutating, display = detailed)]
    async fn namespace_tool(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "namespace",
            "ChatGPT namespace tool",
            serde_json::json!({"type":"namespace"}),
            input,
        )
        .await
    }

    #[tool(summary = "Edit permitted local images through OpenAI's Images edit endpoint.", help = "This convenience entry preserves the official image edit endpoint alongside the Responses image_generation tool. Every input and output path is permission checked.", mutating,   display = detailed, path(requests = input.images.iter().cloned().map(PathRequest::read).collect::<Vec<_>>()))]
    async fn image_edit(&self, input: ChatGptImageEditInput) -> SdkResult<ToolInvokeOutput> {
        let model = self.image_model(input.model, "chatgpt.image_edit")?;
        let url = endpoint(self.config()?.base_url.as_str(), "images/edits")?;
        super::official_service::authorize_network(self.host()?, url.as_str()).await?;
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
        for source in input.images {
            let path = resolve_local_path(self.workspace_root()?, source.as_str())?;
            let bytes = tokio::fs::read(&path).await.map_err(|error| {
                PluginError::internal(format!("cannot read image '{}': {error}", path.display()))
            })?;
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
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                self.config()?.timeout_secs.max(1),
            ))
            .build()
            .map_err(|error| {
                PluginError::internal(format!("cannot create OpenAI client: {error}"))
            })?
            .post(url)
            .bearer_auth(env_secret(
                self.config()?.api_key_env.as_str(),
                "ChatGPT/OpenAI",
            )?)
            .multipart(form)
            .send()
            .await
            .map_err(|error| PluginError::internal(format!("OpenAI image edit failed: {error}")))?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let value: serde_json::Value = response.json().await.map_err(|error| {
            PluginError::internal(format!("OpenAI image edit returned invalid JSON: {error}"))
        })?;
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
