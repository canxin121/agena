//! Plugin manifest: the contract between a plugin and the host. Either
//! delivered as a JSON file next to a cdylib/stdio binary or returned by the
//! `meta/manifest` JSON-RPC method.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(default)]
    pub transports: Vec<TransportKind>,
    #[serde(default)]
    pub hooks: HookSubscription,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<PluginToolDecl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<PluginStudioCommand>,
    /// Plugin-level host capabilities. Useful for plugins that need to
    /// call host APIs without exposing any model-visible tool. These are merged
    /// into the effective capability set alongside the per-tool
    /// declarations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugin_capabilities: Vec<HostCapability>,
    /// UI contributions owned by this plugin. TUI-facing content and Studio
    /// Web-facing views/controls are intentionally split so each host can
    /// consume only the surface it can render.
    #[serde(default, skip_serializing_if = "PluginUiContributions::is_empty")]
    pub ui: PluginUiContributions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_schema: Option<serde_json::Value>,
    /// Optional localized schema overlays keyed by locale, for hosts that
    /// render generic JSON Schema config editors.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config_schema_i18n: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Static,
    Cdylib,
    Stdio,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolTag {
    ReadOnly,
    Mutating,
    Task,
    FilesystemRead,
    FilesystemWrite,
    Network,
    Internet,
    Shell,
    Interactive,
    Discovery,
    Planning,
    Goal,
    Worktree,
    Scheduler,
    Lsp,
    Mcp,
    Subtask,
    PrivateNetwork,
    Custom(String),
}

impl ToolTag {
    pub fn custom(tag: impl AsRef<str>) -> Option<Self> {
        let normalized = normalize_tool_tag_name(tag)?;
        Some(Self::Custom(normalized))
    }

    pub fn from_tag(tag: impl AsRef<str>) -> Option<Self> {
        let normalized = normalize_tool_tag_name(tag)?;
        Some(match normalized.as_str() {
            "read_only" => Self::ReadOnly,
            "mutating" => Self::Mutating,
            "task" => Self::Task,
            "filesystem_read" => Self::FilesystemRead,
            "filesystem_write" => Self::FilesystemWrite,
            "network" => Self::Network,
            "internet" => Self::Internet,
            "shell" => Self::Shell,
            "interactive" => Self::Interactive,
            "discovery" => Self::Discovery,
            "planning" => Self::Planning,
            "goal" => Self::Goal,
            "worktree" => Self::Worktree,
            "scheduler" => Self::Scheduler,
            "lsp" => Self::Lsp,
            "mcp" => Self::Mcp,
            "subtask" => Self::Subtask,
            "private_network" => Self::PrivateNetwork,
            other => Self::Custom(other.to_string()),
        })
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Mutating => "mutating",
            Self::Task => "task",
            Self::FilesystemRead => "filesystem_read",
            Self::FilesystemWrite => "filesystem_write",
            Self::Network => "network",
            Self::Internet => "internet",
            Self::Shell => "shell",
            Self::Interactive => "interactive",
            Self::Discovery => "discovery",
            Self::Planning => "planning",
            Self::Goal => "goal",
            Self::Worktree => "worktree",
            Self::Scheduler => "scheduler",
            Self::Lsp => "lsp",
            Self::Mcp => "mcp",
            Self::Subtask => "subtask",
            Self::PrivateNetwork => "private_network",
            Self::Custom(value) => value.as_str(),
        }
    }
}

impl fmt::Display for ToolTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ToolTag {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ToolTag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_tag(value).ok_or_else(|| serde::de::Error::custom("tool tag cannot be empty"))
    }
}

impl From<&ToolTag> for ToolTag {
    fn from(value: &ToolTag) -> Self {
        value.clone()
    }
}

pub fn normalize_tool_tag_name(tag: impl AsRef<str>) -> Option<String> {
    let normalized = tag
        .as_ref()
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    (!normalized.is_empty()).then_some(normalized)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginToolDecl {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Short model-visible one-line description. Hosts may use this when a
    /// tool is exposed in help mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Detailed usage help returned by host/tool catalog help flows. When
    /// omitted, hosts fall back to `description` plus the input schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Preferred model-visible description mode for this tool. Host config can
    /// override it per plugin or per tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_mode: Option<ToolDescriptionMode>,
    #[serde(default)]
    pub input_schema: serde_json::Value,
    /// Declarative path-permission specs. The host extracts paths from the
    /// tool input via JSONPath before invocation and audits them as
    /// [`PathKind`]. Use [`Plugin::permission_paths`] for paths that can only
    /// be derived dynamically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_paths: Vec<InputPathSpec>,
    /// Declarative network-permission specs. The host extracts hosts/URLs from
    /// the tool input via JSONPath before invocation and audits them as
    /// outbound connect targets. Use [`Plugin::permission_networks`] for
    /// targets that can only be derived dynamically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_networks: Vec<InputNetworkSpec>,
    /// Static local filesystem targets used by this tool regardless of input.
    /// Use this for fixed workspace paths owned by a plugin; use
    /// [`Plugin::permission_paths`] for targets that can only be derived
    /// dynamically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_access: Vec<PathAccessSpec>,
    /// Static outbound network targets used by this tool regardless of input.
    /// Typical values are URLs (`https://api.example.com/search`) or
    /// `host:port` patterns (`api.example.com:443`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network_access: Vec<NetworkAccessSpec>,
    /// Host-policy tags used to derive the tool default permission when the
    /// tool does not have an exact tool rule. Hosts also use canonical tags
    /// such as `read_only`, `task`, `network`, and `filesystem_write` to drive
    /// catalog filtering, plan-mode defaults, and other runtime policy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ToolTag>,
    pub concurrency_safe: bool,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub streaming: ToolStreamingMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_capabilities: Vec<HostCapability>,
}

impl PluginToolDecl {
    pub fn description_text(&self) -> &str {
        self.description.as_deref().unwrap_or("")
    }

    pub fn summary_text(&self) -> Option<&str> {
        self.summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn help_text(&self) -> Option<&str> {
        self.help
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn sanitized_input_schema(&self) -> serde_json::Value {
        sanitize_schema_json(self.input_schema.clone())
    }

    pub fn effective_tags(&self) -> Vec<ToolTag> {
        let mut tags = normalize_tags(self.tags.iter().cloned());
        for spec in &self.input_paths {
            match spec.kind {
                PathKind::Read => push_normalized_tag(&mut tags, ToolTag::FilesystemRead),
                PathKind::Write => push_normalized_tag(&mut tags, ToolTag::FilesystemWrite),
            }
        }
        for spec in &self.path_access {
            match spec.kind {
                PathKind::Read => push_normalized_tag(&mut tags, ToolTag::FilesystemRead),
                PathKind::Write => push_normalized_tag(&mut tags, ToolTag::FilesystemWrite),
            }
        }
        if !self.input_networks.is_empty() || !self.network_access.is_empty() {
            push_normalized_tag(&mut tags, ToolTag::Network);
        }
        tags
    }

    pub fn has_tag(&self, tag: ToolTag) -> bool {
        self.effective_tags()
            .iter()
            .any(|existing| existing == &tag)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolStreamingMode {
    #[default]
    Buffered,
    Streaming,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolDescriptionMode {
    #[default]
    Detailed,
    Help,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostCapability {
    AskUser,
    SpawnSubtask,
    ListTools,
    SessionRegistry,
    MonitorRegistry,
    ReadConfig,
    ReloadConfig,
    InvokeTool,
    PublishEvent,
    SubscribeEvents,
    Scheduler,
    WorktreeRegistry,
    LspRegistry,
    ToolRegistry,
    PluginStorage,
    PluginSecrets,
    PluginStatus,
    CronScheduler,
    AgentRegistry,
    HookRegistry,
    McpRegistry,
    Statusline,
    Theme,
    PermissionUi,
    PermissionDecision,
    PermissionCheck,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PluginUiContributions {
    #[serde(default, skip_serializing_if = "PluginTuiUiContributions::is_empty")]
    pub tui: PluginTuiUiContributions,
    #[serde(default, skip_serializing_if = "PluginStudioUiContributions::is_empty")]
    pub studio: PluginStudioUiContributions,
}

impl PluginUiContributions {
    pub fn is_empty(&self) -> bool {
        self.tui.is_empty() && self.studio.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PluginTuiUiContributions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statusline_segments: Vec<PluginTuiStatuslineSegment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<PluginUiThemePalette>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<PluginTuiContentBlock>,
}

impl PluginTuiUiContributions {
    pub fn is_empty(&self) -> bool {
        self.statusline_segments.is_empty()
            && self.themes.is_empty()
            && self.content_blocks.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PluginStudioUiContributions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<PluginStudioControl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<PluginStudioView>,
}

impl PluginStudioUiContributions {
    pub fn is_empty(&self) -> bool {
        self.controls.is_empty() && self.views.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginTuiStatuslineSegment {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginUiThemePalette {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginTuiContentBlock {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default = "default_tui_content_location")]
    pub location: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginStudioCommand {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default = "default_studio_category")]
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(default = "default_studio_command_location")]
    pub location: String,
    #[serde(default)]
    pub action: PluginUiAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginStudioControl {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default = "default_studio_control_location")]
    pub location: String,
    #[serde(default = "default_studio_control_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<PluginStudioControlOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub action: PluginUiAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginStudioControlOption {
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginStudioView {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default = "default_studio_view_location")]
    pub location: String,
    #[serde(default = "default_studio_view_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<PluginStudioControl>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginUiAction {
    #[default]
    None,
    InvokeTool {
        tool: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
        #[serde(default)]
        submit_output_as_prompt: bool,
    },
    OpenRoute {
        route: String,
    },
    OpenUrl {
        url: String,
    },
    SubmitPrompt {
        prompt: String,
    },
}

fn default_tui_content_location() -> String {
    "composer_footer".to_string()
}

fn default_studio_category() -> String {
    "Plugin".to_string()
}

fn default_studio_command_location() -> String {
    "command_palette".to_string()
}

fn default_studio_control_location() -> String {
    "plugin_panel".to_string()
}

fn default_studio_control_kind() -> String {
    "button".to_string()
}

fn default_studio_view_location() -> String {
    "plugins".to_string()
}

fn default_studio_view_kind() -> String {
    "markdown".to_string()
}

/// Single declarative path extraction rule. `jsonpath` is a subset:
/// dot-paths (`$.path`, `$.files[*].path`). The host extracts each match
/// from the tool input JSON, classifies it under [`PathKind`], and runs it
/// through the permission auditor before the tool body executes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputPathSpec {
    pub jsonpath: String,
    pub kind: PathKind,
    /// If true, missing matches are silently ignored instead of erroring.
    #[serde(default)]
    pub optional: bool,
}

/// Single declarative network extraction rule. `jsonpath` uses the same subset
/// as [`InputPathSpec`]. Each match must resolve to a string URL, host, or
/// host:port target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputNetworkSpec {
    pub jsonpath: String,
    /// If true, missing matches are silently ignored instead of erroring.
    #[serde(default)]
    pub optional: bool,
}

/// One static filesystem target used by a plugin tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathAccessSpec {
    pub path: String,
    pub kind: PathKind,
}

/// One static outbound network target used by a plugin tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkAccessSpec {
    pub target: String,
}

fn sanitize_schema_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut object) => {
            object.remove("$schema");
            object.remove("title");
            let mut cleaned = object
                .into_iter()
                .map(|(key, value)| (key, sanitize_schema_json(value)))
                .collect::<serde_json::Map<String, serde_json::Value>>();
            if !cleaned.contains_key("type") && schema_map_is_object_like(&cleaned) {
                cleaned.insert(
                    "type".to_string(),
                    serde_json::Value::String("object".to_string()),
                );
            }
            if cleaned
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "object")
                && !cleaned.contains_key("properties")
            {
                cleaned.insert(
                    "properties".to_string(),
                    serde_json::Value::Object(serde_json::Map::new()),
                );
            }
            serde_json::Value::Object(cleaned)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sanitize_schema_json).collect())
        }
        other => other,
    }
}

fn schema_map_is_object_like(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    if map
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == "object")
    {
        return true;
    }
    if map.contains_key("properties") || map.contains_key("required") {
        return true;
    }
    ["oneOf", "anyOf", "allOf"].into_iter().any(|key| {
        map.get(key)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|items| !items.is_empty() && items.iter().all(schema_value_is_object_like))
    })
}

fn schema_value_is_object_like(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(schema_map_is_object_like)
}

fn push_normalized_tag(tags: &mut Vec<ToolTag>, tag: ToolTag) {
    if !tags.iter().any(|existing| existing == &tag) {
        tags.push(tag);
    }
}

fn normalize_tags<I>(tags: I) -> Vec<ToolTag>
where
    I: IntoIterator<Item = ToolTag>,
{
    let mut normalized = Vec::new();
    for tag in tags {
        push_normalized_tag(&mut normalized, tag);
    }
    normalized
}

bitflags::bitflags! {
    /// A bitset describing which hooks the plugin actually wants to receive.
    /// The host uses this to skip dispatch for plugins that didn't subscribe.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct HookSubscription: u64 {
        const INIT                      = 1 << 0;
        const SHUTDOWN                  = 1 << 1;
        const TOOL_BEFORE               = 1 << 2;
        const TOOL_AFTER                = 1 << 3;
        const TOOL_INVOKE               = 1 << 4;
        const TOOL_INVOKE_STREAM        = 1 << 17;
        const EVENT                     = 1 << 5;
        const CHAT_MESSAGE              = 1 << 6;
        const CHAT_PARAMS               = 1 << 7;
        const CHAT_HEADERS              = 1 << 8;
        const CHAT_SYSTEM_TRANSFORM     = 1 << 9;
        const AUTH                      = 1 << 10;
        const PROVIDER_LIST             = 1 << 11;
        const PERMISSION_ASK            = 1 << 12;
        const COMMAND_BEFORE            = 1 << 13;
        const SHELL_ENV                 = 1 << 14;
        const CONFIG                    = 1 << 15;
        // new hooks
        const SESSION_START             = 1 << 18;
        const SESSION_END               = 1 << 19;
        const USER_PROMPT_SUBMIT        = 1 << 21;
        const TOOL_FAILURE              = 1 << 22;
        const AGENT_STOP                = 1 << 23;
        const TOOL_DEFINITION           = 1 << 24;
        const COMMAND_AFTER             = 1 << 25;
        const CHAT_MESSAGES_TRANSFORM   = 1 << 26;
        const PRE_RUN                  = 1 << 27;
        const POST_RUN                 = 1 << 28;
        const NOTIFICATION              = 1 << 29;
    }
}

impl Default for HookSubscription {
    fn default() -> Self {
        HookSubscription::empty()
    }
}

impl Serialize for HookSubscription {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        let mut names = Vec::new();
        for (name, flag) in HOOK_NAMES {
            if self.contains(*flag) {
                names.push(*name);
            }
        }
        names.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for HookSubscription {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(de)?;
        let mut out = HookSubscription::empty();
        for n in &names {
            if let Some(flag) = hook_subscription_for_name(n.as_str()) {
                out |= flag;
            }
        }
        Ok(out)
    }
}

const HOOK_NAMES: &[(&str, HookSubscription)] = &[
    ("init", HookSubscription::INIT),
    ("shutdown", HookSubscription::SHUTDOWN),
    ("tool.execute.before", HookSubscription::TOOL_BEFORE),
    ("tool.execute.after", HookSubscription::TOOL_AFTER),
    ("tool.execute.failure", HookSubscription::TOOL_FAILURE),
    ("tool.invoke", HookSubscription::TOOL_INVOKE),
    ("tool.invoke.stream", HookSubscription::TOOL_INVOKE_STREAM),
    ("tool.definition", HookSubscription::TOOL_DEFINITION),
    ("event", HookSubscription::EVENT),
    ("chat.message", HookSubscription::CHAT_MESSAGE),
    (
        "chat.messages.transform",
        HookSubscription::CHAT_MESSAGES_TRANSFORM,
    ),
    ("chat.params", HookSubscription::CHAT_PARAMS),
    ("chat.headers", HookSubscription::CHAT_HEADERS),
    (
        "chat.system.transform",
        HookSubscription::CHAT_SYSTEM_TRANSFORM,
    ),
    ("auth", HookSubscription::AUTH),
    ("provider.list", HookSubscription::PROVIDER_LIST),
    (
        "permission.ask_permission",
        HookSubscription::PERMISSION_ASK,
    ),
    ("notification", HookSubscription::NOTIFICATION),
    ("command.execute.before", HookSubscription::COMMAND_BEFORE),
    ("command.execute.after", HookSubscription::COMMAND_AFTER),
    ("shell.env", HookSubscription::SHELL_ENV),
    ("config", HookSubscription::CONFIG),
    ("session.start", HookSubscription::SESSION_START),
    ("session.end", HookSubscription::SESSION_END),
    ("user.prompt.submit", HookSubscription::USER_PROMPT_SUBMIT),
    ("agent.stop", HookSubscription::AGENT_STOP),
    ("pre_run", HookSubscription::PRE_RUN),
    ("post_run", HookSubscription::POST_RUN),
];

fn hook_subscription_for_name(name: &str) -> Option<HookSubscription> {
    HOOK_NAMES
        .iter()
        .find_map(|(hook_name, flag)| (*hook_name == name).then_some(*flag))
}

/// Builder for ergonomic manifest construction inside `Plugin::manifest`.
pub struct PluginManifestBuilder {
    inner: PluginManifest,
}

impl PluginManifest {
    pub fn builder(name: impl Into<String>, version: impl Into<String>) -> PluginManifestBuilder {
        PluginManifestBuilder {
            inner: PluginManifest {
                schema_version: 1,
                name: name.into(),
                version: version.into(),
                description: None,
                authors: Vec::new(),
                transports: Vec::new(),
                hooks: HookSubscription::INIT | HookSubscription::SHUTDOWN,
                tools: Vec::new(),
                commands: Vec::new(),
                plugin_capabilities: Vec::new(),
                ui: PluginUiContributions::default(),
                config_schema: None,
                config_schema_i18n: BTreeMap::new(),
            },
        }
    }
}

impl PluginManifestBuilder {
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.inner.description = Some(d.into());
        self
    }

    pub fn author(mut self, a: impl Into<String>) -> Self {
        self.inner.authors.push(a.into());
        self
    }

    pub fn transports(mut self, t: impl IntoIterator<Item = TransportKind>) -> Self {
        self.inner.transports.extend(t);
        self
    }

    pub fn hooks(mut self, h: HookSubscription) -> Self {
        self.inner.hooks |= h;
        self
    }

    pub fn tool(mut self, tool: PluginToolDecl) -> Self {
        self.inner.tools.push(tool);
        self
    }

    pub fn tools(mut self, tools: impl IntoIterator<Item = PluginToolDecl>) -> Self {
        self.inner.tools.extend(tools);
        self
    }

    pub fn command(mut self, command: PluginStudioCommand) -> Self {
        self.inner.commands.push(command);
        self
    }

    pub fn commands(mut self, commands: impl IntoIterator<Item = PluginStudioCommand>) -> Self {
        self.inner.commands.extend(commands);
        self
    }

    pub fn plugin_capability(mut self, capability: HostCapability) -> Self {
        if !self.inner.plugin_capabilities.contains(&capability) {
            self.inner.plugin_capabilities.push(capability);
        }
        self
    }

    pub fn plugin_capabilities(
        mut self,
        capabilities: impl IntoIterator<Item = HostCapability>,
    ) -> Self {
        for capability in capabilities {
            if !self.inner.plugin_capabilities.contains(&capability) {
                self.inner.plugin_capabilities.push(capability);
            }
        }
        self
    }

    pub fn config_schema(mut self, schema: serde_json::Value) -> Self {
        self.inner.config_schema = Some(schema);
        self
    }

    pub fn config_schema_locale(
        mut self,
        locale: impl Into<String>,
        schema: serde_json::Value,
    ) -> Self {
        self.inner.config_schema_i18n.insert(locale.into(), schema);
        self
    }

    pub fn config_schema_i18n(
        mut self,
        schemas: impl IntoIterator<Item = (String, serde_json::Value)>,
    ) -> Self {
        self.inner.config_schema_i18n.extend(schemas);
        self
    }

    pub fn ui(mut self, ui: PluginUiContributions) -> Self {
        self.inner.ui = ui;
        self
    }

    pub fn tui_statusline_segment(mut self, segment: PluginTuiStatuslineSegment) -> Self {
        self.inner.ui.tui.statusline_segments.push(segment);
        self
    }

    pub fn tui_theme(mut self, theme: PluginUiThemePalette) -> Self {
        self.inner.ui.tui.themes.push(theme);
        self
    }

    pub fn tui_content_block(mut self, block: PluginTuiContentBlock) -> Self {
        self.inner.ui.tui.content_blocks.push(block);
        self
    }

    pub fn studio_control(mut self, control: PluginStudioControl) -> Self {
        self.inner.ui.studio.controls.push(control);
        self
    }

    pub fn studio_view(mut self, view: PluginStudioView) -> Self {
        self.inner.ui.studio.views.push(view);
        self
    }

    pub fn build(self) -> PluginManifest {
        self.inner
    }
}

impl PluginToolDecl {
    pub fn new(name: impl Into<String>, schema: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            description: None,
            summary: None,
            help: None,
            description_mode: None,
            input_schema: schema,
            input_paths: Vec::new(),
            input_networks: Vec::new(),
            path_access: Vec::new(),
            network_access: Vec::new(),
            tags: Vec::new(),
            concurrency_safe: false,
            strict: false,
            streaming: ToolStreamingMode::Buffered,
            host_capabilities: Vec::new(),
        }
    }

    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn description_mode(mut self, mode: ToolDescriptionMode) -> Self {
        self.description_mode = Some(mode);
        self
    }

    pub fn input_path(mut self, spec: InputPathSpec) -> Self {
        self.input_paths.push(spec);
        self
    }

    pub fn input_network(mut self, spec: InputNetworkSpec) -> Self {
        self.input_networks.push(spec);
        self
    }

    pub fn path_access(mut self, spec: PathAccessSpec) -> Self {
        self.path_access.push(spec);
        self
    }

    pub fn network_access(mut self, spec: NetworkAccessSpec) -> Self {
        self.network_access.push(spec);
        self
    }

    pub fn tag(mut self, tag: ToolTag) -> Self {
        self.tags.push(tag);
        self
    }

    pub fn tags<I>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = ToolTag>,
    {
        self.tags = tags.into_iter().collect();
        self
    }

    pub fn concurrency_safe(mut self, concurrency_safe: bool) -> Self {
        self.concurrency_safe = concurrency_safe;
        self
    }

    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    pub fn streaming(mut self, streaming: ToolStreamingMode) -> Self {
        self.streaming = streaming;
        self
    }

    pub fn host_capability(mut self, capability: HostCapability) -> Self {
        self.host_capabilities.push(capability);
        self
    }

    pub fn host_capabilities(
        mut self,
        capabilities: impl IntoIterator<Item = HostCapability>,
    ) -> Self {
        self.host_capabilities.extend(capabilities);
        self
    }
}

/// Free-form metadata attached to manifests, tools, and UI declarations.
pub type Metadata = BTreeMap<String, String>;
