//! Google Gemini Interactions and image capabilities exposed as ordinary Agena tools.

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
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::official_service::{
    ProviderUsageKind, append_prompt_to_items, configured_model, endpoint, env_secret,
    merge_object_options, post_json, provider_output, read_image_input_bounded, resolve_local_path,
};

pub(crate) const GEMINI_PLUGIN_ID: &str = "agena.gemini";

pub(crate) struct GeminiToolsPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
    workspace_root: OnceLock<PathBuf>,
    config: OnceLock<GeminiToolsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct GeminiToolsConfig {
    base_url: String,
    api_key_env: String,
    model: Option<String>,
    image_model: Option<String>,
    timeout_secs: u64,
    stable_system_instruction: Option<String>,
}

impl Default for GeminiToolsConfig {
    fn default() -> Self {
        Self {
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_owned(),
            api_key_env: "GEMINI_API_KEY".to_owned(),
            model: None,
            image_model: None,
            timeout_secs: 180,
            stable_system_instruction: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "stable_system_instruction"),
    max_chars("prompt", 64000),
    max_chars("stable_system_instruction", 256000)
)]
#[serde(deny_unknown_fields)]
struct GeminiToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    /// Stable prefix used to improve Gemini implicit cache reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stable_system_instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    tool_options: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_interaction_id: Option<String>,
    /// Official Interactions steps, including function_result callbacks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    input_steps: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model"),
    non_empty("prompt"),
    max_chars("prompt", 64000)
)]
#[serde(deny_unknown_fields)]
struct GeminiImageGenerateInput {
    prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    generation_config: BTreeMap<String, serde_json::Value>,
    /// Existing Gemini cachedContents resource name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cached_content: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "images[]"),
    non_empty("prompt", "images[]"),
    min_items("images", 1),
    max_items("images", 16)
)]
#[serde(deny_unknown_fields)]
struct GeminiImageEditInput {
    prompt: String,
    images: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    generation_config: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cached_content: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
}

impl GeminiToolsPlugin {
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
            .ok_or_else(|| PluginError::internal("Gemini tools plugin invoked before init"))
    }
    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("Gemini tools plugin invoked before init"))
    }
    fn config(&self) -> SdkResult<&GeminiToolsConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("Gemini tools plugin invoked before init"))
    }
    fn model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.model.as_deref(),
            &["GEMINI_MODEL", "GOOGLE_GENAI_MODEL"],
            tool,
        )
    }
    fn image_model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.image_model.as_deref(),
            &["GEMINI_IMAGE_MODEL", "GOOGLE_GENAI_IMAGE_MODEL"],
            tool,
        )
    }

    async fn interactions_tool(
        &self,
        tool_name: &str,
        title: &str,
        declaration: serde_json::Value,
        input: GeminiToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model, format!("gemini.{tool_name}").as_str())?;
        let declaration =
            merge_object_options(declaration, &input.tool_options, &["type"], "tool_options")?;
        let provider_input = append_prompt_to_items(input.input_steps, input.prompt, false)?;
        let mut base = serde_json::json!({
            "model": model.clone(),
            "input": provider_input,
            "tools": [declaration],
            "stream": false
        });
        if let Some(system_instruction) = input
            .stable_system_instruction
            .or_else(|| self.config().ok()?.stable_system_instruction.clone())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            base["system_instruction"] = serde_json::Value::String(system_instruction);
        }
        if let Some(previous) = input.previous_interaction_id {
            base["previous_interaction_id"] = serde_json::Value::String(previous);
        }
        let body = merge_object_options(
            base,
            &input.request_options,
            &[
                "model",
                "input",
                "tools",
                "stream",
                "previous_interaction_id",
                "system_instruction",
            ],
            "request_options",
        )?;
        let url = endpoint(self.config()?.base_url.as_str(), "interactions")?;
        let headers = BTreeMap::from([
            (
                "x-goog-api-key".to_owned(),
                env_secret(self.config()?.api_key_env.as_str(), "Gemini")?,
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]);
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            &body,
            self.config()?.timeout_secs,
            "gemini",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "gemini",
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::GeminiInteractions,
            response,
        )
        .await
    }

    async fn generate_content(
        &self,
        tool_name: &str,
        title: &str,
        model: String,
        parts: Vec<serde_json::Value>,
        mut generation_config: BTreeMap<String, serde_json::Value>,
        cached_content: Option<String>,
        request_options: BTreeMap<String, serde_json::Value>,
    ) -> SdkResult<ToolInvokeOutput> {
        generation_config
            .entry("responseModalities".to_owned())
            .or_insert_with(|| serde_json::json!(["TEXT", "IMAGE"]));
        let model_path = if model.starts_with("models/") {
            model.clone()
        } else {
            format!("models/{model}")
        };
        let url = endpoint(
            self.config()?.base_url.as_str(),
            format!("{model_path}:generateContent").as_str(),
        )?;
        let headers = BTreeMap::from([
            (
                "x-goog-api-key".to_owned(),
                env_secret(self.config()?.api_key_env.as_str(), "Gemini")?,
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]);
        let mut base = serde_json::json!({
            "contents": [{"role":"user","parts":parts}],
            "generationConfig": generation_config
        });
        if let Some(cached_content) = cached_content
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            base["cachedContent"] = serde_json::Value::String(cached_content);
        }
        let body = merge_object_options(
            base,
            &request_options,
            &["contents", "generationConfig", "cachedContent"],
            "request_options",
        )?;
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            &body,
            self.config()?.timeout_secs,
            "gemini",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "gemini",
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::GeminiGenerateContent,
            response,
        )
        .await
    }
}

#[agena_plugin_host::sdk::agena_plugin(namespace="agena", name="gemini", version=env!("CARGO_PKG_VERSION"), summary="Google Gemini Interactions and image capabilities exposed as ordinary Agena tools.", settings=GeminiToolsConfig, settings_default=default)]
impl GeminiToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: GeminiToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
                ctx.settings,
                "invalid Gemini tools plugin config",
            )?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::internal("Gemini tools plugin initialized more than once"))?;
        self.config
            .set(config)
            .map_err(|_| PluginError::internal("Gemini tools plugin initialized more than once"))?;
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("Gemini tools plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(network, interactive),
        summary = "Run Gemini hosted code execution.",
        help = "Uses the official Interactions code_execution declaration. Continue any function calls with function_result steps in input_steps.",
        read_only
    )]
    async fn code_execution(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "code_execution",
            "Gemini code execution",
            serde_json::json!({"type":"code_execution"}),
            input,
        )
        .await
    }

    #[tool(
        tags(network, interactive),
        summary = "Fetch and ground URLs with Gemini URL Context.",
        help = "Uses the official url_context tool. Put URLs in the prompt or official request fields.",
        read_only,
        discovery
    )]
    async fn url_context(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "url_context",
            "Gemini URL context",
            serde_json::json!({"type":"url_context"}),
            input,
        )
        .await
    }

    #[tool(
        tags(network, interactive),
        summary = "Search Google with Gemini grounding.",
        help = "tool_options.search_types accepts web_search, image_search, and enterprise_web_search.",
        read_only,
        discovery
    )]
    async fn google_search(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "google_search",
            "Gemini Google Search",
            serde_json::json!({"type":"google_search"}),
            input,
        )
        .await
    }

    #[tool(
        tags(network, interactive),
        summary = "Search Gemini File Search stores.",
        help = "tool_options supports file_search_store_names, metadata_filter, and top_k.",
        read_only,
        discovery
    )]
    async fn file_search(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "file_search",
            "Gemini file search",
            serde_json::json!({"type":"file_search"}),
            input,
        )
        .await
    }

    #[tool(
        tags(network, interactive),
        summary = "Use Google Maps grounding through Gemini.",
        help = "tool_options supports enable_widget, latitude, and longitude.",
        read_only,
        discovery
    )]
    async fn google_maps(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "google_maps",
            "Gemini Google Maps",
            serde_json::json!({"type":"google_maps"}),
            input,
        )
        .await
    }

    #[tool(
        tags(network, interactive),
        summary = "Run Gemini Computer Use and return official pending calls.",
        help = "tool_options supports browser/mobile/desktop environments, safety policy controls, prompt-injection detection, and excluded predefined functions. Continue with function_result steps.",
        mutating
    )]
    async fn computer_use(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "computer_use",
            "Gemini computer use",
            serde_json::json!({"type":"computer_use"}),
            input,
        )
        .await
    }

    #[tool(
        tags(network, interactive),
        summary = "Connect Gemini to a remote MCP server.",
        help = "tool_options supports url, name, headers, and allowed_tools according to the current Interactions MCPServer schema.",
        mutating
    )]
    async fn mcp_server(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "mcp_server",
            "Gemini MCP server",
            serde_json::json!({"type":"mcp_server"}),
            input,
        )
        .await
    }

    #[tool(
        tags(network, interactive),
        summary = "Use Gemini Retrieval across Vertex AI Search, RAG Store, Exa, or Parallel AI Search.",
        help = "Pass retrieval_types and the official *_search_config fields in tool_options.",
        read_only,
        discovery
    )]
    async fn retrieval(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "retrieval",
            "Gemini retrieval",
            serde_json::json!({"type":"retrieval"}),
            input,
        )
        .await
    }

    #[tool(
        tags(network, interactive),
        name = "function",
        summary = "Send an official Gemini function declaration through Interactions.",
        help = "Set the official name, description, and JSON schema fields in tool_options; continue with function_result steps.",
        mutating
    )]
    async fn function_tool(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "function",
            "Gemini function",
            serde_json::json!({"type":"function"}),
            input,
        )
        .await
    }

    #[tool(
        tags(network, interactive),
        summary = "Generate images with Gemini's image response modality.",
        help = "Uses generateContent with responseModalities TEXT and IMAGE. Configure GEMINI_IMAGE_MODEL or input.model. Inline image data is persisted as managed attachments.",
        mutating
    )]
    async fn image_generation(
        &self,
        input: GeminiImageGenerateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.image_model(input.model, "gemini.image_generation")?;
        self.generate_content(
            "image_generation",
            "Gemini image generation",
            model,
            vec![serde_json::json!({"text":input.prompt})],
            input.generation_config,
            input.cached_content,
            input.request_options,
        )
        .await
    }

    #[tool(summary="Edit permitted local images with Gemini multimodal image generation.", help="Uploads permission-checked local images as inlineData and requests an IMAGE response. Returned images are persisted as managed attachments.", mutating, path(requests=input.images.iter().cloned().map(PathRequest::read).collect::<Vec<_>>()))]
    async fn image_edit(&self, input: GeminiImageEditInput) -> SdkResult<ToolInvokeOutput> {
        let model = self.image_model(input.model, "gemini.image_edit")?;
        let mut parts = Vec::new();
        let mut image_input_bytes = 0_u64;
        for source in input.images {
            let path = resolve_local_path(self.workspace_root()?, source.as_str())?;
            let bytes = read_image_input_bounded(&path, &mut image_input_bytes, "Gemini").await?;
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
            parts.push(serde_json::json!({"inlineData":{"mimeType":mime,"data":base64::engine::general_purpose::STANDARD.encode(bytes)}}));
        }
        parts.push(serde_json::json!({"text":input.prompt}));
        self.generate_content(
            "image_edit",
            "Gemini image edit",
            model,
            parts,
            input.generation_config,
            input.cached_content,
            input.request_options,
        )
        .await
    }
}
