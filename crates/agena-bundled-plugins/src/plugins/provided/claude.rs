//! Anthropic Claude beta/server tools exposed as ordinary Agena tools.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::official_service::{
    ProviderUsageKind, configured_model, dedup_strings, endpoint, env_secret, merge_object_options,
    post_json, provider_output,
};

pub(crate) const CLAUDE_PLUGIN_ID: &str = "agena.claude";

pub(crate) struct ClaudeToolsPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
    workspace_root: OnceLock<PathBuf>,
    config: OnceLock<ClaudeToolsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct ClaudeToolsConfig {
    base_url: String,
    api_key_env: String,
    anthropic_version: String,
    model: Option<String>,
    max_tokens: u32,
    beta_headers: Vec<String>,
    timeout_secs: u64,
    cache_ttl: ClaudeCacheTtl,
    stable_system: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum ClaudeCacheTtl {
    Disabled,
    #[default]
    FiveMinutes,
    OneHour,
}

impl Default for ClaudeToolsConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.anthropic.com".to_owned(),
            api_key_env: "ANTHROPIC_API_KEY".to_owned(),
            anthropic_version: "2023-06-01".to_owned(),
            model: None,
            max_tokens: 4096,
            beta_headers: Vec::new(),
            timeout_secs: 180,
            cache_ttl: ClaudeCacheTtl::FiveMinutes,
            stable_system: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "stable_system"),
    max_chars("prompt", 64000),
    max_chars("stable_system", 256000)
)]
#[serde(deny_unknown_fields)]
struct ClaudeToolInput {
    /// New user instruction. Optional when messages already contain tool_result continuation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    /// Stable system prefix placed before dynamic messages for cache reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stable_system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_ttl: Option<ClaudeCacheTtl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    tool_options: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
    /// Full Anthropic messages used to continue tool_use/tool_result loops.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    messages: Vec<serde_json::Value>,
    /// Additional official Anthropic beta feature headers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    beta_headers: Vec<String>,
}

impl ClaudeToolsPlugin {
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
            .ok_or_else(|| PluginError::internal("Claude tools plugin invoked before init"))
    }
    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("Claude tools plugin invoked before init"))
    }
    fn config(&self) -> SdkResult<&ClaudeToolsConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("Claude tools plugin invoked before init"))
    }
    fn model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.model.as_deref(),
            &["CLAUDE_MODEL", "ANTHROPIC_MODEL"],
            tool,
        )
    }

    async fn messages_tool(
        &self,
        tool_name: &str,
        title: &str,
        declaration: serde_json::Value,
        default_betas: &[&str],
        input: ClaudeToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model, format!("claude.{tool_name}").as_str())?;
        let declaration = merge_object_options(
            declaration,
            &input.tool_options,
            &["type", "name"],
            "tool_options",
        )?;
        let mut messages = input.messages;
        if let Some(prompt) = input
            .prompt
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            messages.push(serde_json::json!({"role":"user","content":prompt}));
        }
        if messages.is_empty() {
            return Err(PluginError::invalid_params(
                "an initial Claude provider-tool request requires prompt or messages",
            ));
        }
        let mut base = serde_json::json!({
            "model": model.clone(),
            "max_tokens": input.max_tokens.unwrap_or(self.config()?.max_tokens),
            "messages": messages,
            "tools": [declaration],
            "stream": false
        });
        if let Some(system) = input
            .stable_system
            .or_else(|| self.config().ok()?.stable_system.clone())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            base["system"] = serde_json::Value::String(system);
        }
        match input.cache_ttl.unwrap_or(self.config()?.cache_ttl) {
            ClaudeCacheTtl::Disabled => {}
            ClaudeCacheTtl::FiveMinutes => {
                base["cache_control"] = serde_json::json!({
                    "type": "ephemeral",
                    "ttl": "5m"
                });
            }
            ClaudeCacheTtl::OneHour => {
                base["cache_control"] = serde_json::json!({
                    "type": "ephemeral",
                    "ttl": "1h"
                });
            }
        }
        let body = merge_object_options(
            base,
            &input.request_options,
            &[
                "model",
                "max_tokens",
                "messages",
                "tools",
                "stream",
                "system",
                "cache_control",
            ],
            "request_options",
        )?;
        let url = endpoint(self.config()?.base_url.as_str(), "v1/messages")?;
        let mut headers = BTreeMap::from([
            (
                "x-api-key".to_owned(),
                env_secret(self.config()?.api_key_env.as_str(), "Claude/Anthropic")?,
            ),
            (
                "anthropic-version".to_owned(),
                self.config()?.anthropic_version.clone(),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]);
        let betas = dedup_strings(
            self.config()?
                .beta_headers
                .iter()
                .cloned()
                .chain(default_betas.iter().map(|value| value.to_string()))
                .chain(input.beta_headers),
        );
        if !betas.is_empty() {
            headers.insert("anthropic-beta".to_owned(), betas.join(","));
        }
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            &body,
            self.config()?.timeout_secs,
            "claude",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "claude",
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::AnthropicMessages,
            response,
        )
        .await
    }
}

#[agena_plugin_host::sdk::agena_plugin(namespace="agena", name="claude", version=env!("CARGO_PKG_VERSION"), summary="Anthropic Claude server and client tools exposed as ordinary Agena tools.", config_schema=agena_plugin_sdk::macro_support::json_schema_for_default(ClaudeToolsConfig::default()), display=detailed)]
impl ClaudeToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: ClaudeToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_config(
                ctx.config,
                "invalid Claude tools plugin config",
            )?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::internal("Claude tools plugin initialized more than once"))?;
        self.config
            .set(config)
            .map_err(|_| PluginError::internal("Claude tools plugin initialized more than once"))?;
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("Claude tools plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(tags(network, interactive), summary="Use Claude's current Bash client tool.", help="The declaration uses bash_20250124. Execute returned bash tool_use blocks through Agena shell permissions, append assistant content and user tool_result content to messages, then call this tool again.", mutating, display=detailed)]
    async fn bash(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "bash",
            "Claude Bash",
            serde_json::json!({"type":"bash_20250124","name":"bash"}),
            &["computer-use-2025-01-24"],
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary="Run Claude's latest hosted code execution tool.", help="Uses code_execution_20260521 with persistent REPL state. Official allowed_callers, cache_control, defer_loading, and strict fields may be supplied in tool_options.", read_only, display=detailed)]
    async fn code_execution(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "code_execution",
            "Claude code execution",
            serde_json::json!({"type":"code_execution_20260521","name":"code_execution"}),
            &["code-execution-2026-05-21"],
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary="Run Claude Computer Use and return pending computer actions.", help="Uses computer_20251124. Set display_width_px and display_height_px in tool_options. Agena executors should normalize left_mouse_down/left_mouse_up, drag paths, key combinations, screenshots, zoom, and cursor actions before returning tool_result blocks.", mutating, display=detailed)]
    async fn computer(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "computer",
            "Claude computer",
            serde_json::json!({"type":"computer_20251124","name":"computer"}),
            &["computer-use-2025-11-24"],
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary="Use Claude's memory client tool.", help="Uses memory_20250818. Execute returned memory commands against Agena's permission-checked memory store and continue with tool_result blocks.", mutating, display=detailed)]
    async fn memory(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "memory",
            "Claude memory",
            serde_json::json!({"type":"memory_20250818","name":"memory"}),
            &["context-management-2025-06-27"],
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary="Use Claude's current text editor client tool.", help="Uses text_editor_20250728 with name str_replace_based_edit_tool. Execute view/create/str_replace/insert operations through Agena filesystem permissions and continue with tool_result blocks.", mutating,   display=detailed)]
    async fn text_editor(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "text_editor",
            "Claude text editor",
            serde_json::json!({"type":"text_editor_20250728","name":"str_replace_based_edit_tool"}),
            &["computer-use-2025-01-24"],
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary="Search the web with Claude's latest server web search.", help="Uses web_search_20260318. tool_options supports allowed_callers, allowed_domains, blocked_domains, cache_control, defer_loading, max_uses, response_inclusion, strict, and user_location.", read_only, discovery, display=detailed)]
    async fn web_search(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "web_search",
            "Claude web search",
            serde_json::json!({"type":"web_search_20260318","name":"web_search"}),
            &["web-search-2026-03-18"],
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary="Fetch web documents with Claude's latest server web fetch.", help="Uses web_fetch_20260318. tool_options supports allowed/blocked domains, citations, max_content_tokens, max_uses, response_inclusion, strict, and use_cache.", read_only, discovery, display=detailed)]
    async fn web_fetch(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "web_fetch",
            "Claude web fetch",
            serde_json::json!({"type":"web_fetch_20260318","name":"web_fetch"}),
            &["web-fetch-2026-03-18"],
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary="Ask an Anthropic advisor model with Claude's advisor server tool.", help="Uses advisor_20260301. Set tool_options.model and optional caching, max_tokens, max_uses, allowed_callers, cache_control, defer_loading, and strict.", read_only, display=detailed)]
    async fn advisor(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "advisor",
            "Claude advisor",
            serde_json::json!({"type":"advisor_20260301","name":"advisor"}),
            &["advisor-2026-03-01"],
            input,
        )
        .await
    }

    #[tool(tags(network, interactive), summary="Use Claude's BM25 deferred tool search.", help="Uses tool_search_tool_bm25_20251119. The returned tool_reference/server_tool_use content remains in the provider response and can be continued through messages.", read_only, discovery, display=detailed)]
    async fn tool_search_bm25(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool("tool_search_bm25", "Claude BM25 tool search", serde_json::json!({"type":"tool_search_tool_bm25_20251119","name":"tool_search_tool_bm25"}), &["advanced-tool-use-2025-11-20"], input).await
    }

    #[tool(tags(network, interactive), summary="Use Claude's regex deferred tool search.", help="Uses tool_search_tool_regex_20251119 and supports official allowed_callers, cache_control, defer_loading, and strict options.", read_only, discovery, display=detailed)]
    async fn tool_search_regex(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool("tool_search_regex", "Claude regex tool search", serde_json::json!({"type":"tool_search_tool_regex_20251119","name":"tool_search_tool_regex"}), &["advanced-tool-use-2025-11-20"], input).await
    }

    #[tool(tags(network, interactive), summary="Configure a Claude MCP toolset.", help="Set tool_options.mcp_server_name plus optional configs and default_config. The wrapper sends the official mcp_toolset declaration and returns approval/tool-use content for continuation.", mutating, display=detailed)]
    async fn mcp_toolset(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "mcp_toolset",
            "Claude MCP toolset",
            serde_json::json!({"type":"mcp_toolset"}),
            &["mcp-client-2025-04-04"],
            input,
        )
        .await
    }
}
