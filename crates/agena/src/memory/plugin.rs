use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use agena_macros::StaticToolSurface;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::memory::{
    MemoryEntry, MemoryError, MemoryIndex, MemorySearchDocument, MemoryStore, MemoryType, NewMemory,
};
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, NetworkRequest, PathRequest, Plugin,
    PluginManifest, PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
    ToolTag,
};
use crate::plugin::{
    ChatMessage, ChatMessagesTransformInput, ChatMessagesTransformPatch, ChatSystemTransformInput,
    ChatSystemTransformPatch, PluginError,
};

pub const MEMORY_PLUGIN_ID: &str = "agena.memory";

const DEFAULT_MEMORY_SEARCH_LIMIT: usize = 5;
const MAX_MEMORY_SEARCH_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    pub project_instructions: ProjectInstructionsConfig,
    pub retrieval: MemoryRetrievalConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectInstructionsConfig {
    pub enabled: bool,
    pub include_global: bool,
}

impl Default for ProjectInstructionsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_global: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryRetrievalConfig {
    pub enabled: bool,
    pub limit: u32,
    pub min_query_chars: u32,
}

impl Default for MemoryRetrievalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            limit: 3,
            min_query_chars: 8,
        }
    }
}

pub struct MemoryPlugin {
    config: OnceLock<MemoryConfig>,
    workspace_root: OnceLock<PathBuf>,
    sync_lock: Mutex<()>,
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "memory",
    description = "Persistent memory command. Use action `search`, `get`, `list`, `write`, or `delete` to manage durable user/project memory.",
    summary = "Search, inspect, and update durable memory records.",
    help = "Use action `search` to find relevant memories, `get` to read one record, `list` to inspect the catalog, `write` to create or replace a memory file, and `delete` to remove an obsolete memory.",
    tags(ToolTag::ReadOnly, ToolTag::Mutating),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum MemoryToolInput {
    #[tool(exec = "search")]
    Search {
        #[serde(flatten)]
        args: MemorySearchInput,
    },
    #[tool(exec = "get")]
    Get {
        #[serde(flatten)]
        args: MemoryGetInput,
    },
    #[tool(exec = "list")]
    List {
        #[serde(flatten)]
        args: MemoryListInput,
    },
    #[tool(exec = "write")]
    Write {
        #[serde(flatten)]
        args: MemoryWriteInput,
    },
    #[tool(exec = "delete")]
    Delete {
        #[serde(flatten)]
        args: MemoryDeleteInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemorySearchInput {
    query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemoryGetInput {
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemoryListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemoryWriteInput {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory_type: Option<MemoryType>,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemoryDeleteInput {
    name: String,
}

#[derive(Debug, Serialize)]
struct MemoryRecordOutput {
    name: String,
    description: String,
    memory_type: Option<String>,
    file_name: String,
    body: String,
}

impl MemoryPlugin {
    pub fn new() -> Self {
        Self {
            config: OnceLock::new(),
            workspace_root: OnceLock::new(),
            sync_lock: Mutex::new(()),
        }
    }

    fn config(&self) -> SdkResult<&MemoryConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::new("memory plugin invoked before init"))
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::new("memory plugin invoked before init"))
    }

    fn store(&self) -> SdkResult<MemoryStore> {
        Ok(MemoryStore::for_workspace(self.workspace_root()?))
    }

    fn search_index(&self) -> SdkResult<MemoryIndex> {
        Ok(MemoryIndex::for_workspace(self.workspace_root()?))
    }

    fn sync_documents(&self, store: &MemoryStore) -> SdkResult<Vec<MemorySearchDocument>> {
        Ok(store
            .list()
            .map_err(memory_error_to_plugin)?
            .into_iter()
            .map(memory_document_from_entry)
            .collect())
    }

    fn sync_index(&self, documents: &[MemorySearchDocument]) -> SdkResult<()> {
        self.search_index()?
            .replace_documents(documents)
            .map_err(|err| PluginError::new(format!("failed to rebuild memory index: {err}")))
    }

    fn run_search(
        &self,
        query: &str,
        limit: usize,
        documents: &[MemorySearchDocument],
    ) -> SdkResult<Vec<MemorySearchDocument>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        self.search_index()?
            .search(query, limit)
            .map_err(|err| PluginError::new(format!("memory search failed: {err}")))
    }

    async fn sync_and_search_documents(
        &self,
        query: &str,
        limit: usize,
    ) -> SdkResult<Vec<MemorySearchDocument>> {
        let store = self.store()?;
        let documents = self.sync_documents(&store)?;
        let _guard = self.sync_lock.lock().await;
        self.sync_index(&documents)?;
        self.run_search(query, limit, &documents)
    }

    async fn search_documents(
        &self,
        query: &str,
        limit: usize,
    ) -> SdkResult<Vec<MemorySearchDocument>> {
        self.sync_and_search_documents(query, limit).await
    }

    async fn invoke_search(&self, input: &MemorySearchInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.trim();
        if query.is_empty() {
            return Err(PluginError::invalid_params(
                "memory search requires a non-empty query",
            ));
        }
        let limit = input
            .limit
            .unwrap_or(DEFAULT_MEMORY_SEARCH_LIMIT as u32)
            .clamp(1, MAX_MEMORY_SEARCH_LIMIT as u32) as usize;
        let results = self.search_documents(query, limit).await?;
        let mut lines = vec![format!(
            "Found {} memory item(s) matching '{}'.",
            results.len(),
            query
        )];
        for memory in &results {
            lines.push(format!(
                "- {} [{}]: {}",
                memory.name,
                memory.memory_type.as_deref().unwrap_or("untyped"),
                memory.description
            ));
        }
        let payload =
            serde_json::to_value(&results).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(lines.join("\n"))
            .with_title("memory search")
            .with_payload(payload))
    }

    async fn invoke_get(&self, input: &MemoryGetInput) -> SdkResult<ToolInvokeOutput> {
        let store = self.store()?;
        let entry = store
            .get(input.name.trim())
            .map_err(memory_error_to_plugin)?;
        let payload = serde_json::to_value(memory_record_output(&entry))
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(format_memory_entry(&entry))
            .with_title("memory get")
            .with_payload(payload))
    }

    async fn invoke_list(&self, input: &MemoryListInput) -> SdkResult<ToolInvokeOutput> {
        let store = self.store()?;
        let limit = input.limit.unwrap_or(50).clamp(1, 200) as usize;
        let entries = store
            .list()
            .map_err(memory_error_to_plugin)?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let payload =
            serde_json::to_value(entries.iter().map(memory_record_output).collect::<Vec<_>>())
                .map_err(|err| PluginError::new(err.to_string()))?;
        let text = if entries.is_empty() {
            "No memory records found.".to_string()
        } else {
            entries
                .iter()
                .map(|entry| {
                    format!(
                        "- {} [{}]: {}",
                        memory_name(entry),
                        entry
                            .frontmatter
                            .r#type
                            .map(MemoryType::label)
                            .unwrap_or("untyped"),
                        memory_description(entry)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(ToolInvokeOutput::text(text)
            .with_title("memory list")
            .with_payload(payload))
    }

    async fn invoke_write(&self, input: &MemoryWriteInput) -> SdkResult<ToolInvokeOutput> {
        let name = validate_memory_name(input.name.as_str())?;
        let content = input.content.trim();
        if content.is_empty() {
            return Err(PluginError::invalid_params(
                "memory write requires non-empty content",
            ));
        }
        let store = self.store()?;
        match store.forget(name.as_str()) {
            Ok(()) | Err(MemoryError::NotFound(_)) => {}
            Err(err) => return Err(memory_error_to_plugin(err)),
        }
        let description = input.description.trim();
        let entry = store
            .save(NewMemory {
                name: name.clone(),
                description: description.to_string(),
                memory_type: input.memory_type,
                body: content.to_string(),
                index_line: Some(memory_index_line(name.as_str(), description, content)),
            })
            .map_err(memory_error_to_plugin)?;
        let payload = serde_json::to_value(memory_record_output(&entry))
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(
            ToolInvokeOutput::text(format!("Saved memory '{}'.", memory_name(&entry)))
                .with_title("memory write")
                .with_payload(payload),
        )
    }

    async fn invoke_delete(&self, input: &MemoryDeleteInput) -> SdkResult<ToolInvokeOutput> {
        let name = validate_memory_name(input.name.as_str())?;
        let store = self.store()?;
        store
            .forget(name.as_str())
            .map_err(memory_error_to_plugin)?;
        Ok(
            ToolInvokeOutput::text(format!("Deleted memory '{}'.", name))
                .with_title("memory delete"),
        )
    }

    fn memory_retrieval_query(
        &self,
        input: &ChatMessagesTransformInput,
    ) -> SdkResult<Option<String>> {
        let latest_user = input
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .and_then(ChatMessage::text)
            .map(str::to_string);
        let Some(latest_user) = latest_user else {
            return Ok(None);
        };
        let latest_user = latest_user.trim().to_string();
        if should_skip_memory_retrieval(latest_user.as_str()) {
            return Ok(None);
        }
        let min_chars = self.config()?.retrieval.min_query_chars as usize;
        Ok((latest_user.len() >= min_chars).then_some(latest_user))
    }
}

#[async_trait]
impl Plugin for MemoryPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(MEMORY_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("Persistent memory with searchable retrieval and write tools.")
            .hooks(
                HookSubscription::INIT
                    | HookSubscription::TOOL_INVOKE
                    | HookSubscription::CHAT_SYSTEM_TRANSFORM
                    | HookSubscription::CHAT_MESSAGES_TRANSFORM,
            )
            .config_schema(crate::entry::definition::json_schema_for::<MemoryConfig>())
            .tool(memory_decl())
            .build()
    }

    async fn init(&self, ctx: InitContext, _host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config = parse_memory_config(ctx.config)?;
        self.config
            .set(config)
            .map_err(|_| PluginError::new("memory plugin initialized more than once"))?;
        let _ = self.workspace_root.set(ctx.workspace_root);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        if input.tool_name != "memory" {
            return Err(PluginError::invalid_params(format!(
                "unknown memory plugin tool '{}'",
                input.tool_name
            )));
        }
        match parse_memory_input(input.input)? {
            MemoryToolInput::Search { args } => self.invoke_search(&args).await,
            MemoryToolInput::Get { args } => self.invoke_get(&args).await,
            MemoryToolInput::List { args } => self.invoke_list(&args).await,
            MemoryToolInput::Write { args } => self.invoke_write(&args).await,
            MemoryToolInput::Delete { args } => self.invoke_delete(&args).await,
        }
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        if tool != "memory" {
            return Ok(Vec::new());
        }
        let store = self.store()?;
        let parsed = parse_memory_input(input.clone())?;
        let request = match parsed {
            MemoryToolInput::Get { .. } | MemoryToolInput::List { .. } => {
                PathRequest::read(store.dir().display().to_string())
            }
            MemoryToolInput::Search { .. }
            | MemoryToolInput::Write { .. }
            | MemoryToolInput::Delete { .. } => {
                PathRequest::write(store.dir().display().to_string())
            }
        };
        Ok(vec![request])
    }

    async fn permission_networks(
        &self,
        _tool: &str,
        _input: &serde_json::Value,
    ) -> SdkResult<Vec<NetworkRequest>> {
        Ok(Vec::new())
    }

    async fn chat_system_transform(
        &self,
        _input: ChatSystemTransformInput,
    ) -> SdkResult<Option<ChatSystemTransformPatch>> {
        let workspace_root = self.workspace_root()?;
        let config = self.config()?;
        if !config.project_instructions.enabled {
            return Ok(None);
        }
        let mut layers = Vec::new();
        if config.project_instructions.include_global
            && let Some(global) = super::discover_global()
        {
            layers.push(global);
        }
        layers.extend(super::discover(workspace_root));
        let Some(section) = super::render_section(&layers) else {
            return Ok(None);
        };
        Ok(Some(ChatSystemTransformPatch {
            append: Some(format!("\n\n{section}")),
            ..Default::default()
        }))
    }

    async fn chat_messages_transform(
        &self,
        input: ChatMessagesTransformInput,
    ) -> SdkResult<Option<ChatMessagesTransformPatch>> {
        let config = self.config()?;
        if !config.retrieval.enabled {
            return Ok(None);
        }
        let Some(query) = self.memory_retrieval_query(&input)? else {
            return Ok(None);
        };
        let limit = config
            .retrieval
            .limit
            .clamp(1, MAX_MEMORY_SEARCH_LIMIT as u32) as usize;
        let memories = match self.search_documents(query.as_str(), limit).await {
            Ok(memories) => memories,
            Err(err) => {
                tracing::warn!(target: "agena::memory", "memory retrieval skipped: {err}");
                return Ok(None);
            }
        };
        if memories.is_empty() {
            return Ok(None);
        }
        let mut messages = input.messages;
        let insert_at = messages
            .iter()
            .take_while(|message| message.role == "system")
            .count();
        messages.insert(
            insert_at,
            ChatMessage::system(render_memory_context(&memories)),
        );
        Ok(Some(ChatMessagesTransformPatch {
            messages: Some(messages),
        }))
    }
}

fn memory_decl() -> PluginToolDecl {
    MemoryToolInput::tool_decl()
}

fn parse_memory_input(input: serde_json::Value) -> SdkResult<MemoryToolInput> {
    MemoryToolInput::parse_input(input)
}

fn parse_memory_config(value: serde_json::Value) -> SdkResult<MemoryConfig> {
    if value.is_null() {
        return Ok(MemoryConfig::default());
    }
    let config = serde_json::from_value::<MemoryConfig>(value)
        .map_err(|err| PluginError::new(format!("invalid memory plugin config: {err}")))?;
    if config.retrieval.limit == 0 {
        return Err(PluginError::new(
            "memory plugin config `retrieval.limit` must be greater than 0",
        ));
    }
    if config.retrieval.min_query_chars == 0 {
        return Err(PluginError::new(
            "memory plugin config `retrieval.min_query_chars` must be greater than 0",
        ));
    }
    Ok(config)
}

fn memory_error_to_plugin(err: MemoryError) -> PluginError {
    PluginError::new(err.to_string())
}

fn memory_document_from_entry(entry: MemoryEntry) -> MemorySearchDocument {
    let id = entry.file_name.trim_end_matches(".md").to_string();
    let name = memory_name(&entry);
    let description = memory_description(&entry);
    let memory_type = entry
        .frontmatter
        .r#type
        .map(|kind| kind.label().to_string());
    let path = entry.path.display().to_string();
    let body = entry.body;
    MemorySearchDocument::new(id, name, description, memory_type, body, path)
}

fn memory_record_output(entry: &MemoryEntry) -> MemoryRecordOutput {
    MemoryRecordOutput {
        name: memory_name(entry),
        description: memory_description(entry),
        memory_type: entry
            .frontmatter
            .r#type
            .map(|kind| kind.label().to_string()),
        file_name: entry.file_name.clone(),
        body: entry.body.clone(),
    }
}

fn memory_name(entry: &MemoryEntry) -> String {
    if entry.frontmatter.name.trim().is_empty() {
        entry.file_name.trim_end_matches(".md").to_string()
    } else {
        entry.frontmatter.name.trim().to_string()
    }
}

fn memory_description(entry: &MemoryEntry) -> String {
    if entry.frontmatter.description.trim().is_empty() {
        first_line(entry.body.as_str())
    } else {
        entry.frontmatter.description.trim().to_string()
    }
}

fn first_line(content: &str) -> String {
    content
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn format_memory_entry(entry: &MemoryEntry) -> String {
    let mut lines = vec![format!("Name: {}", memory_name(entry))];
    if let Some(memory_type) = entry.frontmatter.r#type {
        lines.push(format!("Type: {}", memory_type.label()));
    }
    let description = memory_description(entry);
    if !description.is_empty() {
        lines.push(format!("Description: {description}"));
    }
    lines.push(String::new());
    lines.push(entry.body.clone());
    lines.join("\n")
}

fn render_memory_context(memories: &[MemorySearchDocument]) -> String {
    let mut lines = vec![
        "Relevant durable memory: use this as context, but verify concrete repo facts before acting.".to_string(),
    ];
    for memory in memories {
        lines.push(format!(
            "- {} [{}]: {}",
            memory.name,
            memory.memory_type.as_deref().unwrap_or("untyped"),
            memory.description
        ));
        lines.push(truncate_body(memory.body.as_str(), 600));
    }
    lines.join("\n")
}

fn truncate_body(body: &str, limit: usize) -> String {
    let trimmed = body.trim();
    if trimmed.len() <= limit {
        return trimmed.to_string();
    }
    format!("{}...", &trimmed[..limit])
}

fn should_skip_memory_retrieval(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    [
        "ignore memory",
        "don't use memory",
        "do not use memory",
        "not use memory",
    ]
    .into_iter()
    .any(|needle| lowered.contains(needle))
}

fn validate_memory_name(raw: &str) -> SdkResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(PluginError::invalid_params("memory name must not be empty"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(PluginError::invalid_params(
            "memory name must not contain path separators",
        ));
    }
    Ok(trimmed.trim_end_matches(".md").to_string())
}

fn memory_index_line(name: &str, description: &str, content: &str) -> String {
    let hook = if description.trim().is_empty() {
        truncate_body(first_line(content).as_str(), 120)
    } else {
        truncate_body(description.trim(), 120)
    };
    format!("- [{name}]({name}.md) — {hook}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_memory_name_rejects_path_separators() {
        assert!(validate_memory_name("team/preference").is_err());
        assert!(validate_memory_name("team\\preference").is_err());
    }

    #[test]
    fn memory_index_line_prefers_description() {
        let line = memory_index_line("user_style", "Keep responses terse", "body");
        assert!(line.contains("[user_style](user_style.md)"));
        assert!(line.contains("Keep responses terse"));
    }

    #[test]
    fn memory_tool_input_rejects_unknown_fields() {
        let err = MemoryToolInput::parse_input(serde_json::json!({
            "action": "search",
            "query": "launch plan",
            "backend": "legacy"
        }))
        .expect_err("memory tool should reject unknown fields");
        assert!(err.to_string().contains("unknown field `backend`"));
    }

    #[test]
    fn memory_config_rejects_unknown_fields() {
        let err = serde_json::from_value::<MemoryConfig>(serde_json::json!({
            "search": {}
        }))
        .expect_err("memory config should reject unknown fields");
        assert!(err.to_string().contains("unknown field `search`"));
    }
}
