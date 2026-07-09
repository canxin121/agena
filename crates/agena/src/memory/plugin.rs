use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use agena_macros::ToolInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::memory::{
    MemoryError, MemoryIndex, MemoryRecord, MemorySearchDocument, MemoryStore, MemoryType,
    NewMemory,
};
use crate::plugin::sdk::{Result as SdkResult, ToolInvokeOutput};
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

fn memory_config_schema() -> serde_json::Value {
    crate::tool::definition::json_schema_for_default_with_metadata(
        default_memory_config(),
        &[
            (
                "",
                "Memory Plugin Config",
                "Controls project instruction injection and memory retrieval defaults for agena.memory.",
            ),
            (
                "/properties/project_instructions",
                "Project Instructions",
                "Controls whether project instruction memories are injected into the chat system prompt.",
            ),
            (
                "/properties/project_instructions/properties/enabled",
                "Enabled",
                "Allows project instruction memories to be injected into conversations.",
            ),
            (
                "/properties/project_instructions/properties/include_global",
                "Include Global",
                "Also includes global instruction memories when building the project instruction prompt.",
            ),
            (
                "/properties/retrieval",
                "Retrieval",
                "Defaults for memory search behavior.",
            ),
            (
                "/properties/retrieval/properties/enabled",
                "Enabled",
                "Allows memory search results to be retrieved automatically.",
            ),
            (
                "/properties/retrieval/properties/limit",
                "Default Limit",
                "Default number of memory results returned when a search call omits limit.",
            ),
            (
                "/properties/retrieval/properties/min_query_chars",
                "Minimum Query Characters",
                "Shortest query length required before automatic retrieval runs.",
            ),
        ],
    )
}

fn default_memory_config() -> MemoryConfig {
    MemoryConfig {
        project_instructions: ProjectInstructionsConfig::default(),
        retrieval: MemoryRetrievalConfig::default(),
    }
}

pub struct MemoryPlugin {
    config: OnceLock<MemoryConfig>,
    workspace_root: OnceLock<PathBuf>,
    sync_lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("query"), non_empty("query"))]
#[serde(deny_unknown_fields)]
struct MemorySearchInput {
    query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("name"),
    trim_suffix("name", ".md"),
    non_empty("name"),
    forbid_substrings("name", "/", "\\")
)]
#[serde(deny_unknown_fields)]
struct MemoryGetInput {
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct MemoryListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("name", "description", "content"),
    trim_suffix("name", ".md"),
    non_empty("name", "content"),
    forbid_substrings("name", "/", "\\")
)]
#[serde(deny_unknown_fields)]
struct MemoryWriteInput {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory_type: Option<MemoryType>,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("name"),
    trim_suffix("name", ".md"),
    non_empty("name"),
    forbid_substrings("name", "/", "\\")
)]
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

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "memory",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Persistent memory with searchable retrieval and write tools.",
    config_schema = memory_config_schema(),
    display = brief
)]
impl MemoryPlugin {
    pub fn new() -> Self {
        Self {
            config: OnceLock::new(),
            workspace_root: OnceLock::new(),
            sync_lock: Mutex::new(()),
        }
    }

    #[hook(init)]
    async fn init(
        &self,
        ctx: crate::plugin::sdk::InitContext,
        _host: std::sync::Arc<dyn crate::plugin::sdk::HostClient>,
    ) -> SdkResult<crate::plugin::sdk::InitOutcome> {
        self.config
            .set(parse_memory_config(ctx.config)?)
            .map_err(|_| PluginError::new("memory plugin initialized more than once"))?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::new("memory plugin workspace root already initialized"))?;
        Ok(crate::plugin::sdk::InitOutcome::ack(
            crate::plugin::sdk::Plugin::manifest(self),
        ))
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

    #[tool(
        summary = "Search durable memory records.",
        read_only,
        display = brief,
        path(write = self.store_dir_permission_path()?)
    )]
    async fn invoke_search(&self, input: &MemorySearchInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.as_str();
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
        let payload = serde_json::json!({
            "query": query,
            "limit": limit,
            "results": results,
        });
        Ok(ToolInvokeOutput::from_parts(
            "memory search",
            lines.join("\n"),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Read one durable memory record.",
        read_only,
        display = brief,
        path(read = self.store_dir_permission_path()?)
    )]
    async fn invoke_get(&self, input: &MemoryGetInput) -> SdkResult<ToolInvokeOutput> {
        let store = self.store()?;
        let entry = store
            .get(input.name.as_str())
            .map_err(memory_error_to_plugin)?;
        let payload = serde_json::to_value(memory_record_output(&entry))
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::from_parts(
            "memory get",
            format_memory_entry(&entry),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "List durable memory records.",
        read_only,
        display = brief,
        path(read = self.store_dir_permission_path()?)
    )]
    async fn invoke_list(&self, input: &MemoryListInput) -> SdkResult<ToolInvokeOutput> {
        let store = self.store()?;
        let limit = input.limit.unwrap_or(50).clamp(1, 200) as usize;
        let entries = store
            .list()
            .map_err(memory_error_to_plugin)?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let memories = entries.iter().map(memory_record_output).collect::<Vec<_>>();
        let payload = serde_json::json!({
            "limit": limit,
            "memories": memories,
        });
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
        Ok(ToolInvokeOutput::from_parts(
            "memory list",
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Write one durable memory record.",
        mutating,
        display = brief,
        path(write = self.store_dir_permission_path()?)
    )]
    async fn invoke_write(&self, input: &MemoryWriteInput) -> SdkResult<ToolInvokeOutput> {
        let name = input.name.clone();
        let content = input.content.as_str();
        let store = self.store()?;
        match store.forget(name.as_str()) {
            Ok(()) | Err(MemoryError::NotFound(_)) => {}
            Err(err) => return Err(memory_error_to_plugin(err)),
        }
        let description = input.description.as_str();
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
        Ok(ToolInvokeOutput::from_parts(
            "memory write",
            format!("Saved memory '{}'.", memory_name(&entry)),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Delete one durable memory record.",
        mutating,
        display = brief,
        path(write = self.store_dir_permission_path()?)
    )]
    async fn invoke_delete(&self, input: &MemoryDeleteInput) -> SdkResult<ToolInvokeOutput> {
        let name = input.name.clone();
        let store = self.store()?;
        store
            .forget(name.as_str())
            .map_err(memory_error_to_plugin)?;
        Ok(ToolInvokeOutput::from_parts(
            "memory delete",
            format!("Deleted memory '{}'.", name),
            None,
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    fn store_dir_permission_path(&self) -> SdkResult<String> {
        let store = self.store()?;
        Ok(store.dir().display().to_string())
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

    #[hook(chat.system)]
    async fn chat_system_transform(
        &self,
        _input: ChatSystemTransformInput,
    ) -> Result<Option<ChatSystemTransformPatch>, PluginError> {
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

    #[hook(chat.messages)]
    async fn chat_messages_transform(
        &self,
        input: ChatMessagesTransformInput,
    ) -> Result<Option<ChatMessagesTransformPatch>, PluginError> {
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

fn memory_document_from_entry(entry: MemoryRecord) -> MemorySearchDocument {
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

fn memory_record_output(entry: &MemoryRecord) -> MemoryRecordOutput {
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

fn memory_name(entry: &MemoryRecord) -> String {
    if entry.frontmatter.name.trim().is_empty() {
        entry.file_name.trim_end_matches(".md").to_string()
    } else {
        entry.frontmatter.name.trim().to_string()
    }
}

fn memory_description(entry: &MemoryRecord) -> String {
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

fn format_memory_entry(entry: &MemoryRecord) -> String {
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

fn memory_index_line(name: &str, description: &str, content: &str) -> String {
    let hook = if description.trim().is_empty() {
        truncate_body(first_line(content).as_str(), 120)
    } else {
        truncate_body(description.trim(), 120)
    };
    format!("- [{name}]({name}.md) — {hook}")
}
