//! Anthropic Claude beta/server tools exposed as ordinary Agena tools.

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
    ProviderUsageKind, configured_model, dedup_strings, endpoint, env_secret, merge_object_options,
    post_json, provider_output,
};
use agena_plugin_host::sdk::ToolInvokeContext;

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
    /// New user instruction. Optional when messages continue hosted server execution.
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
    /// Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    messages: Vec<serde_json::Value>,
    /// Additional official Anthropic beta feature headers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    beta_headers: Vec<String>,
}

impl ClaudeToolsPlugin {
    fn media_service(&self) -> SdkResult<Service<'_>> {
        Ok(Service {
            provider: "claude",
            root: self.workspace_root()?,
            host: self.host()?,
            base_url: &self.config()?.base_url,
            key_env: &self.config()?.api_key_env,
            timeout_secs: self.config()?.timeout_secs,
            anthropic_version: self.config()?.anthropic_version.as_str(),
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
        super::official_service::hosted::validate_options(&input.tool_options, "tool_options")?;
        super::official_service::hosted::validate_options(
            &input.request_options,
            "request_options",
        )?;
        super::official_service::hosted::validate_history(
            "claude",
            &serde_json::json!(&input.messages),
        )?;
        let model = self.model(input.model, format!("claude.cloud_{tool_name}").as_str())?;
        let declaration = merge_object_options(
            declaration,
            &input.tool_options,
            &["type", "name"],
            "tool_options",
        )?;
        super::official_service::hosted::validate_declaration("claude", tool_name, &declaration)?;
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

#[agena_plugin_host::sdk::agena_plugin(namespace="agena", name="claude", version=env!("CARGO_PKG_VERSION"), summary="Anthropic cloud search, fetch, computation and advisor capabilities. Inputs leave this computer; no local execution fallback.", settings=ClaudeToolsConfig, settings_default=default)]
impl ClaudeToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: ClaudeToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
                ctx.settings,
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

    #[tool(name="cloud_image_understanding",tags(query,network),
        summary="Send explicit images to Anthropic cloud for understanding; not local file viewing.",
        help="Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        mutating,network(connect=self.config()?.base_url.clone()),path(requests=input.paths(self.workspace_root()?)))]
    async fn image_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "claude.cloud_image_understanding")?;
        self.media_service()?
            .analyze(context, input, model, true)
            .await
    }

    #[tool(name="cloud_document_understanding",tags(query,network),
        summary="Send explicit PDF/text documents to Anthropic cloud for understanding; not local file viewing.",
        help="Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        mutating,network(connect=self.config()?.base_url.clone()),path(requests=input.paths(self.workspace_root()?)))]
    async fn document_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "claude.cloud_document_understanding")?;
        self.media_service()?
            .analyze(context, input, model, false)
            .await
    }

    #[tool(name="cloud_file_upload",tags(mutate,network),
        summary="Upload one permitted local file to Anthropic cloud and return a session-owned handle.",
        help="Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.",
        mutating,network(connect=self.config()?.base_url.clone()),path(requests=vec![PathRequest::read(input.path.clone()),PathRequest::write(self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string())]))]
    async fn file_upload(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &UploadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.media_service()?.upload(context, input).await
    }

    #[tool(name="cloud_file_status",tags(query,network),
        summary="Query the remote status of an owned Anthropic cloud file, not a local path.",
        help="Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only cloud_file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.",
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
        summary="Request deletion of an owned file from Anthropic cloud; preserve the local original.",
        help="Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.",
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
        name = "cloud_code_execution",
        network(connect = self.config()?.base_url.clone()),
        path(write = self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()),
        tags(network, interactive),
        summary = "Execute code in Anthropic cloud infrastructure, not on this computer.",
        help = "Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Uses code_execution_20260521 with persistent REPL state. Official allowed_callers, cache_control, defer_loading, and strict fields may be supplied in tool_options.",
        read_only
    )]
    async fn code_execution(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "code_execution",
            "Claude code execution",
            serde_json::json!({"type":"code_execution_20260521","name":"code_execution"}),
            &[],
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_web_search",
        network(connect = self.config()?.base_url.clone()),
        path(write = self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()),
        tags(network, interactive),
        summary = "Search the web in Anthropic cloud and return sources; not a local browser operation.",
        help = "Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_search_20260318. tool_options supports allowed_callers, allowed_domains, blocked_domains, cache_control, defer_loading, max_uses, response_inclusion, strict, and user_location.",
        read_only,
        discovery
    )]
    async fn web_search(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "web_search",
            "Claude web search",
            serde_json::json!({"type":"web_search_20260318","name":"web_search"}),
            &[],
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_web_fetch",
        network(connect = self.config()?.base_url.clone()),
        path(write = self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()),
        tags(network, interactive),
        summary = "Fetch and process web content in Anthropic cloud, not through the local browser.",
        help = "Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_fetch_20260318. tool_options supports allowed/blocked domains, citations, max_content_tokens, max_uses, response_inclusion, strict, and use_cache.",
        read_only,
        discovery
    )]
    async fn web_fetch(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "web_fetch",
            "Claude web fetch",
            serde_json::json!({"type":"web_fetch_20260318","name":"web_fetch"}),
            &[],
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_advisor",
        network(connect = self.config()?.base_url.clone()),
        path(write = self.workspace_root()?.join(".agena/artifacts/provider-tools").display().to_string()),
        tags(network, interactive),
        summary = "Consult an advisor model in Anthropic cloud using the supplied context.",
        help = "Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Only supplied context is available; the local repository and session transcript are not automatically uploaded. Uses advisor_20260301. Set tool_options.model and optional caching, max_tokens, max_uses, allowed_callers, cache_control, defer_loading, and strict.",
        read_only
    )]
    async fn advisor(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "advisor",
            "Claude advisor",
            serde_json::json!({"type":"advisor_20260301","name":"advisor"}),
            &["advisor-tool-2026-03-01"],
            input,
        )
        .await
    }
}
