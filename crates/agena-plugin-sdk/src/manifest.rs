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
    /// Short plugin summary used when hosts only need a compact overview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Detailed plugin help shown by inspect/catalog surfaces when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Preferred default presentation mode for tools exposed by this plugin
    /// when an individual tool declaration does not specify its own mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_description_mode: Option<ToolDescriptionMode>,
    /// Preferred default text density for UI surfaces that render this plugin
    /// or its tools when an individual tool declaration does not specify its
    /// own mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_display_mode: Option<UiTextDisplayMode>,
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
    /// Alternate local tool names accepted by the host for the same tool.
    /// Hosts expose these in the same namespace as `name`, e.g. a tool alias
    /// `cat` in plugin `agena.workflow` becomes `agena.workflow/cat`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional text shown before the main help block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_help: Option<String>,
    /// Optional text shown after the main help block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_help: Option<String>,
    /// Short model-visible one-line description. Hosts may use this when a
    /// tool is exposed in help mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Detailed usage help returned by host/tool catalog help flows. When
    /// omitted, hosts fall back to `description` plus the input schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Structured example invocations shown in tool help flows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    /// Preferred model-visible description mode for this tool. Host config can
    /// override it per plugin or per tool, or explicitly follow this tool's
    /// default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_mode: Option<ToolDescriptionMode>,
    /// Preferred text density for UI surfaces that render this tool. Host UI
    /// config can override it per plugin or per tool, or explicitly follow
    /// this tool's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_display_mode: Option<UiTextDisplayMode>,
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

pub trait ToolSurface: Sized {
    fn tool_name() -> &'static str;
    fn tool_decl() -> PluginToolDecl;
    fn parse_input(input: serde_json::Value) -> crate::Result<Self>;
    fn parse_json_str(input: &str) -> crate::Result<Self> {
        let value = crate::macro_support::parse_json_value_str(input)?;
        Self::parse_input(value)
    }
    fn resolve_tool(
        tool: &str,
        input: serde_json::Value,
    ) -> crate::Result<(String, serde_json::Value)>;
    fn resolve_json_str(tool: &str, input: &str) -> crate::Result<(String, serde_json::Value)> {
        let value = crate::macro_support::parse_json_value_str(input)?;
        Self::resolve_tool(tool, value)
    }
}

pub trait ToolSuiteSurface: Sized {
    fn tool_decls() -> Vec<PluginToolDecl>;
    fn parse_tool(tool: &str, input: serde_json::Value) -> crate::Result<Self>;
    fn parse_tool_json_str(tool: &str, input: &str) -> crate::Result<Self> {
        let value = crate::macro_support::parse_json_value_str(input)?;
        Self::parse_tool(tool, value)
    }
    fn resolve_tool(
        tool: &str,
        input: serde_json::Value,
    ) -> crate::Result<(String, serde_json::Value)>;
    fn resolve_tool_json_str(
        tool: &str,
        input: &str,
    ) -> crate::Result<(String, serde_json::Value)> {
        let value = crate::macro_support::parse_json_value_str(input)?;
        Self::resolve_tool(tool, value)
    }
}

pub trait ToolInputShape: Sized {
    fn input_schema() -> serde_json::Value;
    fn parse_input(input: serde_json::Value) -> crate::Result<Self>;
    fn parse_json_str(input: &str) -> crate::Result<Self> {
        let value = crate::macro_support::parse_json_value_str(input)?;
        Self::parse_input(value)
    }
}

impl PluginToolDecl {
    pub fn description_text(&self) -> &str {
        self.description.as_deref().unwrap_or("")
    }

    pub fn alias_texts(&self) -> &[String] {
        self.aliases.as_slice()
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

    pub fn before_help_text(&self) -> Option<&str> {
        self.before_help
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn after_help_text(&self) -> Option<&str> {
        self.after_help
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn example_texts(&self) -> &[String] {
        self.examples.as_slice()
    }

    pub fn preferred_description_mode(&self) -> Option<ToolDescriptionMode> {
        self.description_mode
    }

    pub fn preferred_ui_display_mode(&self) -> Option<UiTextDisplayMode> {
        self.ui_display_mode
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
    #[serde(alias = "help")]
    Brief,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiTextDisplayMode {
    #[default]
    Detailed,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolDisplayPreset {
    #[default]
    Detailed,
    Compact,
    BriefDetailed,
}

impl ToolDisplayPreset {
    pub fn tool_description_mode(self) -> ToolDescriptionMode {
        match self {
            Self::Detailed => ToolDescriptionMode::Detailed,
            Self::Compact => ToolDescriptionMode::Brief,
            Self::BriefDetailed => ToolDescriptionMode::Brief,
        }
    }

    pub fn ui_display_mode(self) -> UiTextDisplayMode {
        match self {
            Self::Detailed | Self::BriefDetailed => UiTextDisplayMode::Detailed,
            Self::Compact => UiTextDisplayMode::Summary,
        }
    }
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
                summary: None,
                help: None,
                tool_description_mode: None,
                ui_display_mode: None,
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
}

impl PluginManifestBuilder {
    pub fn compact(self) -> Self {
        self.display(ToolDisplayPreset::Compact)
    }

    pub fn brief(self) -> Self {
        self.display(ToolDisplayPreset::Compact)
    }

    pub fn brief_detailed(self) -> Self {
        self.display(ToolDisplayPreset::BriefDetailed)
    }

    pub fn detailed(self) -> Self {
        self.display(ToolDisplayPreset::Detailed)
    }

    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.inner.description = Some(d.into());
        self
    }

    pub fn long_about(self, description: impl Into<String>) -> Self {
        self.description(description)
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.inner.summary = Some(summary.into());
        self
    }

    pub fn about(self, summary: impl Into<String>) -> Self {
        self.summary(summary)
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.inner.help = Some(help.into());
        self
    }

    pub fn long_help(self, help: impl Into<String>) -> Self {
        self.help(help)
    }

    pub fn after_help(self, help: impl Into<String>) -> Self {
        self.help(help)
    }

    pub fn after_long_help(self, help: impl Into<String>) -> Self {
        self.help(help)
    }

    pub fn before_help(self, description: impl Into<String>) -> Self {
        self.description(description)
    }

    pub fn before_long_help(self, description: impl Into<String>) -> Self {
        self.description(description)
    }

    pub fn display(mut self, preset: ToolDisplayPreset) -> Self {
        self.inner.tool_description_mode = Some(preset.tool_description_mode());
        self.inner.ui_display_mode = Some(preset.ui_display_mode());
        self
    }

    pub fn tool_description_mode(mut self, mode: ToolDescriptionMode) -> Self {
        self.inner.tool_description_mode = Some(mode);
        self
    }

    pub fn ui_display_mode(mut self, mode: UiTextDisplayMode) -> Self {
        self.inner.ui_display_mode = Some(mode);
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

    pub fn tool_surface<T: ToolSurface>(self) -> Self {
        self.tool(T::tool_decl())
    }

    pub fn tools(mut self, tools: impl IntoIterator<Item = PluginToolDecl>) -> Self {
        self.inner.tools.extend(tools);
        self
    }

    pub fn tool_suite<T: ToolSuiteSurface>(self) -> Self {
        self.tools(T::tool_decls())
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
            aliases: Vec::new(),
            description: None,
            before_help: None,
            after_help: None,
            summary: None,
            help: None,
            examples: Vec::new(),
            description_mode: None,
            ui_display_mode: None,
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

    pub fn compact(self) -> Self {
        self.display(ToolDisplayPreset::Compact)
    }

    pub fn brief(self) -> Self {
        self.display(ToolDisplayPreset::Compact)
    }

    pub fn brief_detailed(self) -> Self {
        self.display(ToolDisplayPreset::BriefDetailed)
    }

    pub fn detailed(self) -> Self {
        self.display(ToolDisplayPreset::Detailed)
    }

    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }

    pub fn alias(self, alias: impl Into<String>) -> Self {
        let mut this = self;
        this.push_alias(alias);
        this
    }

    pub fn visible_alias(self, alias: impl Into<String>) -> Self {
        self.alias(alias)
    }

    pub fn aliases<I, S>(self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut this = self;
        for alias in aliases {
            this.push_alias(alias);
        }
        this
    }

    pub fn visible_aliases<I, S>(self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.aliases(aliases)
    }

    pub fn long_about(self, description: impl Into<String>) -> Self {
        self.description(description)
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn about(self, summary: impl Into<String>) -> Self {
        self.summary(summary)
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn long_help(self, help: impl Into<String>) -> Self {
        self.help(help)
    }

    pub fn after_help(self, help: impl Into<String>) -> Self {
        let mut this = self;
        this.after_help = Some(help.into());
        this
    }

    pub fn after_long_help(self, help: impl Into<String>) -> Self {
        let mut this = self;
        this.after_help = Some(help.into());
        this
    }

    pub fn before_help(self, description: impl Into<String>) -> Self {
        let mut this = self;
        this.before_help = Some(description.into());
        this
    }

    pub fn before_long_help(self, description: impl Into<String>) -> Self {
        let mut this = self;
        this.before_help = Some(description.into());
        this
    }

    pub fn display(mut self, preset: ToolDisplayPreset) -> Self {
        self.description_mode = Some(preset.tool_description_mode());
        self.ui_display_mode = Some(preset.ui_display_mode());
        self
    }

    pub fn example(mut self, example: impl Into<String>) -> Self {
        self.examples.push(example.into());
        self
    }

    pub fn examples<I, S>(mut self, examples: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.examples.extend(examples.into_iter().map(Into::into));
        self
    }

    pub fn description_mode(mut self, mode: ToolDescriptionMode) -> Self {
        self.description_mode = Some(mode);
        self
    }

    pub fn ui_display_mode(mut self, mode: UiTextDisplayMode) -> Self {
        self.ui_display_mode = Some(mode);
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

    fn push_alias(&mut self, alias: impl Into<String>) {
        let alias = alias.into().trim().to_string();
        if alias.is_empty() || alias == self.name || self.aliases.contains(&alias) {
            return;
        }
        self.aliases.push(alias);
    }
}

/// Free-form metadata attached to manifests, tools, and UI declarations.
pub type Metadata = BTreeMap<String, String>;

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::sync::{Arc, OnceLock, RwLock};

    struct DummySurface;

    impl ToolSurface for DummySurface {
        fn tool_name() -> &'static str {
            "dummy"
        }

        fn tool_decl() -> PluginToolDecl {
            PluginToolDecl::new("dummy", serde_json::json!({"type":"object"}))
        }

        fn parse_input(_input: serde_json::Value) -> crate::Result<Self> {
            Ok(Self)
        }

        fn resolve_tool(
            tool: &str,
            input: serde_json::Value,
        ) -> crate::Result<(String, serde_json::Value)> {
            Ok((tool.to_string(), input))
        }
    }

    struct DummySuite;

    impl ToolSuiteSurface for DummySuite {
        fn tool_decls() -> Vec<PluginToolDecl> {
            vec![
                PluginToolDecl::new("suite.one", serde_json::json!({"type":"object"})),
                PluginToolDecl::new("suite.two", serde_json::json!({"type":"object"})),
            ]
        }

        fn parse_tool(_tool: &str, _input: serde_json::Value) -> crate::Result<Self> {
            Ok(Self)
        }

        fn resolve_tool(
            tool: &str,
            input: serde_json::Value,
        ) -> crate::Result<(String, serde_json::Value)> {
            Ok((tool.to_string(), input))
        }
    }

    #[test]
    fn manifest_builder_accepts_tool_surface_and_tool_suite_generics() {
        let manifest = PluginManifest::builder("dummy.plugin", "1.0.0")
            .tool_surface::<DummySurface>()
            .tool_suite::<DummySuite>()
            .build();

        let tool_names = manifest
            .tools
            .iter()
            .map(|decl| decl.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(tool_names, vec!["dummy", "suite.one", "suite.two"]);
    }

    #[test]
    fn plugin_manifest_builder_supports_brief_shortcut() {
        let manifest = PluginManifest::builder("dummy.plugin", "1.0.0")
            .description("Dummy plugin.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .brief()
            .build();

        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Summary));
    }

    #[test]
    fn plugin_manifest_builder_supports_compact_shortcut() {
        let manifest = PluginManifest::builder("dummy.plugin", "1.0.0")
            .description("Dummy plugin.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .compact()
            .build();

        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Summary));
    }

    #[test]
    fn plugin_manifest_builder_supports_detailed_shortcut() {
        let manifest = PluginManifest::builder("dummy.plugin", "1.0.0")
            .description("Dummy plugin.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .detailed()
            .build();

        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Detailed)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Detailed));
    }

    #[test]
    fn plugin_manifest_builder_supports_brief_detailed_shortcut() {
        let manifest = PluginManifest::builder("dummy.plugin", "1.0.0")
            .description("Dummy plugin.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .brief_detailed()
            .build();

        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Detailed));
    }

    #[test]
    fn plugin_manifest_builder_supports_about_aliases() {
        let manifest = PluginManifest::builder("dummy.plugin", "1.0.0")
            .about("Dummy summary.")
            .long_about("Dummy description.")
            .long_help("Dummy help.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .build();

        assert_eq!(manifest.summary_text(), Some("Dummy summary."));
        assert_eq!(manifest.description_text(), "Dummy description.");
        assert_eq!(manifest.help_text(), Some("Dummy help."));
    }

    #[test]
    fn plugin_manifest_builder_supports_before_help_aliases() {
        let manifest = PluginManifest::builder("dummy.plugin", "1.0.0")
            .before_help("Dummy before-help.")
            .before_long_help("Dummy before-long-help.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .build();

        assert_eq!(manifest.description_text(), "Dummy before-long-help.");
    }

    #[test]
    fn plugin_tool_decl_supports_brief_shortcut() {
        let decl = PluginToolDecl::new("dummy", serde_json::json!({"type":"object"})).brief();

        assert_eq!(
            decl.preferred_description_mode(),
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(
            decl.preferred_ui_display_mode(),
            Some(UiTextDisplayMode::Summary)
        );
    }

    #[test]
    fn plugin_tool_decl_supports_compact_shortcut() {
        let decl = PluginToolDecl::new("dummy", serde_json::json!({"type":"object"})).compact();

        assert_eq!(
            decl.preferred_description_mode(),
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(
            decl.preferred_ui_display_mode(),
            Some(UiTextDisplayMode::Summary)
        );
    }

    #[test]
    fn plugin_tool_decl_supports_detailed_shortcut() {
        let decl = PluginToolDecl::new("dummy", serde_json::json!({"type":"object"})).detailed();

        assert_eq!(
            decl.preferred_description_mode(),
            Some(ToolDescriptionMode::Detailed)
        );
        assert_eq!(
            decl.preferred_ui_display_mode(),
            Some(UiTextDisplayMode::Detailed)
        );
    }

    #[test]
    fn plugin_tool_decl_supports_brief_detailed_shortcut() {
        let decl =
            PluginToolDecl::new("dummy", serde_json::json!({"type":"object"})).brief_detailed();

        assert_eq!(
            decl.preferred_description_mode(),
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(
            decl.preferred_ui_display_mode(),
            Some(UiTextDisplayMode::Detailed)
        );
    }

    #[test]
    fn plugin_tool_decl_supports_about_aliases() {
        let decl = PluginToolDecl::new("dummy", serde_json::json!({"type":"object"}))
            .about("Dummy summary.")
            .long_about("Dummy description.")
            .long_help("Dummy help.");

        assert_eq!(decl.summary_text(), Some("Dummy summary."));
        assert_eq!(decl.description_text(), "Dummy description.");
        assert_eq!(decl.help_text(), Some("Dummy help."));
    }

    #[test]
    fn plugin_tool_decl_supports_tool_aliases() {
        let decl = PluginToolDecl::new("dummy", serde_json::json!({"type":"object"}))
            .alias("d")
            .visible_alias("show")
            .aliases(["d", "dummy", " inspect "])
            .visible_aliases(["inspect", "lookup"]);

        assert_eq!(
            decl.alias_texts(),
            &[
                "d".to_string(),
                "show".to_string(),
                "inspect".to_string(),
                "lookup".to_string()
            ]
        );
    }

    #[test]
    fn plugin_tool_decl_supports_before_help_aliases() {
        let decl = PluginToolDecl::new("dummy", serde_json::json!({"type":"object"}))
            .before_help("Dummy before-help.")
            .before_long_help("Dummy before-long-help.");

        assert_eq!(decl.before_help_text(), Some("Dummy before-long-help."));
        assert_eq!(decl.description_text(), "");
    }

    #[test]
    fn plugin_tool_decl_supports_after_help_aliases() {
        let decl = PluginToolDecl::new("dummy", serde_json::json!({"type":"object"}))
            .after_help("Dummy after-help.")
            .after_long_help("Dummy after-long-help.");

        assert_eq!(decl.after_help_text(), Some("Dummy after-long-help."));
        assert_eq!(decl.help_text(), None);
    }

    #[test]
    fn plugin_manifest_macro_builds_surface_suite_and_dynamic_entries() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            display = brief,
            summary = "Dummy summary.",
            help = "Dummy help.",
            tool_surface = DummySurface,
            tool_suite = DummySuite,
            tools = vec![PluginToolDecl::new(
                "dynamic.tool",
                serde_json::json!({"type":"object"})
            )],
            plugin_capabilities = [HostCapability::PluginStorage],
        );

        assert_eq!(manifest.name, "dummy.plugin");
        assert_eq!(manifest.summary_text(), Some("Dummy summary."));
        assert_eq!(manifest.help_text(), Some("Dummy help."));
        assert_eq!(
            manifest.config_schema,
            Some(crate::macro_support::empty_config_schema())
        );
        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Summary));
        assert_eq!(
            manifest
                .tools
                .iter()
                .map(|decl| decl.name.as_str())
                .collect::<Vec<_>>(),
            vec!["dummy", "suite.one", "suite.two", "dynamic.tool"]
        );
        assert_eq!(
            manifest.plugin_capabilities,
            vec![HostCapability::PluginStorage]
        );
    }

    #[test]
    fn plugin_manifest_macro_supports_about_aliases() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            about = "Dummy summary.",
            long_about = "Dummy description.",
            long_help = "Dummy help.",
        );

        assert_eq!(manifest.summary_text(), Some("Dummy summary."));
        assert_eq!(manifest.description_text(), "Dummy description.");
        assert_eq!(manifest.help_text(), Some("Dummy help."));
    }

    #[test]
    fn plugin_manifest_macro_supports_after_help_aliases() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            after_help = "Dummy after-help.",
            after_long_help = "Dummy after-long-help.",
        );

        assert_eq!(manifest.help_text(), Some("Dummy after-long-help."));
    }

    #[test]
    fn plugin_manifest_macro_supports_before_help_aliases() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            before_help = "Dummy before-help.",
            before_long_help = "Dummy before-long-help.",
        );

        assert_eq!(manifest.description_text(), "Dummy before-long-help.");
    }

    #[test]
    fn plugin_manifest_macro_supports_optional_before_help_aliases() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            before_help_if_some = Some("Optional before-help."),
            before_long_help_if_some = Some("Optional before-long-help."),
        );

        assert_eq!(manifest.description_text(), "Optional before-long-help.");
    }

    #[test]
    fn plugin_manifest_macro_supports_optional_about_aliases() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            about_if_some = Some("Optional summary."),
            long_about_if_some = Some("Optional description."),
            long_help_if_some = Some("Optional help."),
        );

        assert_eq!(manifest.summary_text(), Some("Optional summary."));
        assert_eq!(manifest.description_text(), "Optional description.");
        assert_eq!(manifest.help_text(), Some("Optional help."));
    }

    #[test]
    fn plugin_manifest_macro_supports_brief_display_shorthand() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            display = brief,
        );

        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Summary));
    }

    #[test]
    fn plugin_manifest_macro_supports_compact_display_alias() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            display = compact,
        );

        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Summary));
    }

    #[test]
    fn plugin_manifest_macro_supports_brief_detailed_display_shorthand() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            display = brief_detailed,
        );

        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Detailed));
    }

    #[test]
    fn plugin_manifest_macro_supports_detailed_display_shorthand() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            display = detailed,
        );

        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Detailed)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Detailed));
    }

    #[test]
    fn plugin_manifest_macro_supports_ui_display_shorthand() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            ui_display = brief,
        );

        assert_eq!(manifest.tool_description_mode, None);
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Summary));
    }

    #[test]
    fn plugin_manifest_macro_supports_optional_display_preset() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            display_if_some = Some(ToolDisplayPreset::BriefDetailed),
        );

        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Detailed));
    }

    #[test]
    fn plugin_manifest_macro_supports_config_schema_type_and_default_shorthand() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            config_schema_type = DummyInitConfig,
            config_schema_default = default,
        );

        assert_eq!(
            manifest.config_schema,
            Some(crate::macro_support::json_schema_for_with_default(
                DummyInitConfig::default()
            ))
        );
    }

    #[test]
    fn plugin_manifest_macro_supports_config_schema_default_keyword_shorthand() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            config_schema_type = DummyInitConfig,
            config_schema_default = default,
        );

        assert_eq!(
            manifest.config_schema,
            Some(crate::macro_support::json_schema_for_with_default(
                DummyInitConfig::default()
            ))
        );
    }

    #[test]
    fn plugin_manifest_macro_accepts_optional_mode_and_text_overrides() {
        let manifest = crate::plugin_manifest!(
            id = "dummy.plugin",
            version = "1.0.0",
            description = "Dummy plugin.",
            hooks = HookSubscription::TOOL_INVOKE,
            config_schema = serde_json::json!({"type":"object"}),
            summary_if_some = Some("Optional summary."),
            help_if_some = Some("Optional help."),
            tool_description_mode_if_some = Some(ToolDescriptionMode::Brief),
            ui_display_mode_if_some = Some(UiTextDisplayMode::Summary),
        );

        assert_eq!(manifest.summary_text(), Some("Optional summary."));
        assert_eq!(manifest.help_text(), Some("Optional help."));
        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );
        assert_eq!(manifest.ui_display_mode, Some(UiTextDisplayMode::Summary));
    }

    #[test]
    fn macro_support_suggests_closest_action_names() {
        let suggestions = crate::macro_support::suggest_name_candidates(
            "describ",
            ["search", "describe", "help"],
            1,
        );
        assert_eq!(suggestions, vec!["describe".to_string()]);
        assert_eq!(
            crate::macro_support::unknown_name_message("action", "describ", &suggestions),
            "unknown action 'describ'. Did you mean `describe`?"
        );
    }

    #[allow(dead_code)]
    #[derive(Debug, JsonSchema, Serialize)]
    struct TitleFieldSchema {
        title: String,
        other: String,
    }

    #[test]
    fn macro_support_json_schema_preserves_property_named_title() {
        let schema = crate::macro_support::json_schema_for::<TitleFieldSchema>();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema should expose properties");
        assert!(properties.contains_key("title"));
        assert!(properties.contains_key("other"));
    }

    #[test]
    fn macro_support_empty_config_schema_describes_absent_plugin_config() {
        let schema = crate::macro_support::empty_config_schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema should expose properties");
        assert!(properties.is_empty());
        assert_eq!(schema.get("default"), Some(&serde_json::json!({})));
        assert_eq!(
            schema
                .get("additionalProperties")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn macro_support_json_schema_with_default_preserves_property_named_title() {
        let schema = crate::macro_support::json_schema_for_with_default(TitleFieldSchema {
            title: "alpha".to_string(),
            other: "beta".to_string(),
        });
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema should expose properties");
        assert!(properties.contains_key("title"));
        assert!(properties.contains_key("other"));
        let default = schema
            .get("default")
            .and_then(serde_json::Value::as_object)
            .expect("schema should expose default");
        assert_eq!(
            default.get("title").and_then(serde_json::Value::as_str),
            Some("alpha")
        );
        assert_eq!(
            default.get("other").and_then(serde_json::Value::as_str),
            Some("beta")
        );
    }

    #[test]
    fn macro_support_schema_usage_text_is_publicly_reexported() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query."
                }
            },
            "required": ["query"]
        });

        let text = crate::schema_usage_text(&schema).expect("usage text");
        assert!(text.contains("Arguments:"));
        assert!(text.contains("`query` <string, required>: Search query."));
    }

    #[test]
    fn macro_support_schema_example_texts_are_publicly_reexported() {
        let schema = serde_json::json!({
            "oneOf": [
                { "type": "string" },
                { "type": "integer" }
            ]
        });

        let examples = crate::schema_example_texts(&schema);
        assert!(
            examples
                .iter()
                .any(|example| example.contains("Variant 1 <string>"))
        );
        assert!(
            examples
                .iter()
                .any(|example| example.contains("Variant 2 <integer>"))
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
    struct DummyInitConfig {
        #[serde(default)]
        enabled: bool,
    }

    #[derive(Default)]
    struct DummyPluginState {
        config: OnceLock<DummyInitConfig>,
        runtime: OnceLock<String>,
        workspace_root: OnceLock<String>,
        host: RwLock<Option<String>>,
    }

    #[test]
    fn plugin_init_macro_parses_and_stores_common_plugin_state() -> crate::Result<()> {
        let state = DummyPluginState::default();
        let after_hook_ran = std::cell::Cell::new(false);
        let outcome = crate::plugin_init!(
            manifest = PluginManifest::builder("dummy.plugin", "1.0.0").build(),
            default_config = {
                field = state.config,
                ty = DummyInitConfig,
                input = serde_json::json!({"enabled": true}),
                invalid = "invalid dummy config",
                already = "dummy config already initialized"
            },
            store = {
                field = state.runtime,
                value = "runtime-state".to_string(),
                already = "dummy runtime already initialized"
            },
            workspace_root = {
                field = state.workspace_root,
                value = "/tmp/project".to_string(),
                already = "dummy workspace already initialized"
            },
            host_cell = {
                field = state.host,
                value = "host-client".to_string(),
                poisoned = "dummy host poisoned"
            },
            after = {
                after_hook_ran.set(true);
            }
        )?;

        assert_eq!(outcome.manifest.name, "dummy.plugin");
        assert_eq!(state.config.get(), Some(&DummyInitConfig { enabled: true }));
        assert_eq!(
            state.runtime.get().map(String::as_str),
            Some("runtime-state")
        );
        assert_eq!(
            state.workspace_root.get().map(String::as_str),
            Some("/tmp/project")
        );
        assert_eq!(
            state.host.read().expect("host lock").as_deref(),
            Some("host-client")
        );
        assert!(after_hook_ran.get());

        Ok(())
    }

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, crate::ToolCommand)]
    #[tool_command(
        tool = "methods.echo",
        description = "Exercise plugin_methods! against a real ToolCommand.",
        handler_receiver = PluginMethodsFixture,
        handle = PluginMethodsFixture::invoke_echo,
        stream_handle = PluginMethodsFixture::invoke_echo_stream,
        permission_paths_handle = PluginMethodsFixture::permission_paths_echo,
        permission_networks_handle = PluginMethodsFixture::permission_networks_echo,
        handle_field = text,
        handle_by_value = true,
        concurrency_safe = true
    )]
    #[serde(deny_unknown_fields)]
    struct PluginMethodsToolInput {
        text: String,
    }

    #[derive(Default)]
    struct PluginMethodsFixture {
        workspace_root: OnceLock<String>,
    }

    impl PluginMethodsFixture {
        async fn invoke_echo(&self, text: String) -> crate::Result<crate::ToolInvokeOutput> {
            Ok(crate::ToolInvokeOutput::text(format!("echo:{text}")).with_title("Methods"))
        }

        fn resolve_invoke_alias(
            &self,
            input: crate::ToolInvokeInput,
        ) -> crate::Result<crate::ToolInvokeInput> {
            let tool_name = if input.tool_name == "methods.echo.alias" {
                "methods.echo".to_string()
            } else {
                input.tool_name
            };
            Ok(crate::ToolInvokeInput { tool_name, ..input })
        }

        fn resolve_permission_alias(
            &self,
            tool: &str,
            input: &serde_json::Value,
        ) -> crate::Result<(String, serde_json::Value)> {
            let tool_name = if tool == "methods.echo.alias" {
                "methods.echo".to_string()
            } else {
                tool.to_string()
            };
            Ok((tool_name, input.clone()))
        }

        async fn invoke_echo_stream(
            &self,
            sink: crate::ToolStreamSink,
            text: String,
        ) -> crate::Result<crate::ToolStreamEnd> {
            sink.text(format!("delta:{text}")).await;
            Ok(crate::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "Methods".to_string(),
                output_text: format!("stream:{text}"),
                payload: None,
                metadata: BTreeMap::new(),
                attachments: Vec::new(),
            })
        }

        async fn permission_paths_echo(
            &self,
            text: String,
        ) -> crate::Result<Vec<crate::PathRequest>> {
            Ok(vec![crate::PathRequest::read(format!("/tmp/{text}"))])
        }

        async fn permission_networks_echo(
            &self,
            text: String,
        ) -> crate::Result<Vec<crate::NetworkRequest>> {
            Ok(vec![crate::NetworkRequest::connect(format!(
                "https://{text}.example.com"
            ))])
        }
    }

    #[crate::plugin]
    impl crate::Plugin for PluginMethodsFixture {
        crate::plugin_methods! {
            manifest {
                id = "dummy.plugin_methods",
                version = "1.0.0",
                description = "Fixture plugin used to exercise plugin_methods!.",
                hooks = HookSubscription::INIT
                    | HookSubscription::SHUTDOWN
                    | HookSubscription::TOOL_INVOKE
                    | HookSubscription::TOOL_INVOKE_STREAM
                    | HookSubscription::TOOL_BEFORE
                    | HookSubscription::TOOL_FAILURE
                    | HookSubscription::TOOL_DEFINITION
                    | HookSubscription::CHAT_MESSAGE
                    | HookSubscription::SHELL_ENV
                    | HookSubscription::CHAT_PARAMS
                    | HookSubscription::CHAT_HEADERS
                    | HookSubscription::AUTH
                    | HookSubscription::NOTIFICATION
                    | HookSubscription::CONFIG
                    | HookSubscription::USER_PROMPT_SUBMIT
                    | HookSubscription::COMMAND_AFTER,
                config_schema = serde_json::json!({"type":"object"}),
                tool_surface = PluginMethodsToolInput,
            };
            init(this, ctx, _host) {
                workspace_root = {
                    field = this.workspace_root,
                    value = ctx.workspace_root.to_string_lossy().into_owned(),
                    already = "workspace root already initialized"
                }
            };
            tool_invoke => surface(
                PluginMethodsToolInput,
                resolve = PluginMethodsFixture::resolve_invoke_alias
            );
            tool_invoke_stream => surface(
                PluginMethodsToolInput,
                resolve = PluginMethodsFixture::resolve_invoke_alias
            );
            permission_paths => surface(
                PluginMethodsToolInput,
                resolve = PluginMethodsFixture::resolve_permission_alias
            );
            permission_networks => surface(
                PluginMethodsToolInput,
                resolve = PluginMethodsFixture::resolve_permission_alias
            );
            tool_execute_before(_this, input) => {
                Ok(Some(crate::ToolBeforePatch {
                    title_override: Some(format!("before:{}", input.tool_name)),
                    ..Default::default()
                }))
            };
            shell_env(_this, _input) => {
                Ok(Some(crate::ShellEnvPatch::set("PLUGIN_METHODS", "1")))
            };
            chat_params(_this, _input) => {
                Ok(Some(crate::ChatParamsPatch {
                    params: Some(serde_json::json!({"temperature": 0})),
                }))
            };
            shutdown(_this) => {
                Ok(())
            };
            chat_message(_this, input) => {
                Ok(Some(crate::ChatMessagePatch {
                    message: Some(crate::ChatMessage::assistant(format!(
                        "echo:{}",
                        input.message.text().unwrap_or_default()
                    ))),
                    drop: false,
                }))
            };
            chat_headers(_this, _input) => {
                Ok(Some(crate::ChatHeadersPatch {
                    set: BTreeMap::from([("X-Plugin".to_string(), "methods".to_string())]),
                    remove: Vec::new(),
                }))
            };
            auth(_this, _input) => {
                Ok(Some(crate::AuthOutput {
                    kind: crate::AuthKind::ApiKey,
                    credential: serde_json::json!({"api_key":"secret"}),
                }))
            };
            notification(_this, _input) => {
                Ok(())
            };
            user_prompt_submit(_this, input) => {
                Ok(Some(crate::UserPromptSubmitPatch {
                    additional_context: Some(format!("ctx:{}", input.prompt)),
                    ..Default::default()
                }))
            };
            tool_definition(_this, input) => {
                Ok(Some(crate::ToolDefinitionPatch {
                    summary: Some(format!("summary:{}", input.tool_name)),
                    ..Default::default()
                }))
            };
            tool_execute_failure(_this, _input) => {
                Ok(())
            };
            command_execute_after(_this, input) => {
                Ok(Some(crate::CommandAfterPatch {
                    stdout: Some(format!("after:{}", input.command)),
                    ..Default::default()
                }))
            };
            config_resolved(_this, _input) => {
                Ok(Some(crate::ConfigPatch {
                    merge: Some(serde_json::json!({"plugin":{"enabled":true}})),
                }))
            };
        }
    }

    #[derive(Default)]
    struct PluginMethodFixture {
        workspace_root: OnceLock<String>,
    }

    impl PluginMethodFixture {
        async fn invoke_echo(&self, text: String) -> crate::Result<crate::ToolInvokeOutput> {
            Ok(crate::ToolInvokeOutput::text(format!("method:{text}")).with_title("Method"))
        }

        async fn invoke_echo_stream(
            &self,
            sink: crate::ToolStreamSink,
            text: String,
        ) -> crate::Result<crate::ToolStreamEnd> {
            sink.text(format!("method-delta:{text}")).await;
            Ok(crate::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "Method".to_string(),
                output_text: format!("method-stream:{text}"),
                payload: None,
                metadata: BTreeMap::new(),
                attachments: Vec::new(),
            })
        }
    }

    #[crate::plugin]
    impl crate::Plugin for PluginMethodFixture {
        crate::plugin_method! {
            manifest {
                id = "dummy.plugin_method",
                version = "1.0.0",
                description = "Fixture plugin used to exercise plugin_method! item declarations.",
                hooks = HookSubscription::INIT
                    | HookSubscription::TOOL_INVOKE
                    | HookSubscription::TOOL_INVOKE_STREAM
                    | HookSubscription::SHELL_ENV,
                config_schema = serde_json::json!({"type":"object"}),
                tool_surface = PluginMethodsToolInput,
            };
        }
        crate::plugin_method! {
            init(this, ctx, _host) {
                workspace_root = {
                    field = this.workspace_root,
                    value = ctx.workspace_root.to_string_lossy().into_owned(),
                    already = "workspace root already initialized"
                }
            };
        }
        crate::plugin_method! {
            tool_invoke(_this, input) => {
                crate::plugin_tool_invoke_surface!(input, PluginMethodsToolInput, {
                    PluginMethodsToolInput { text } => PluginMethodFixture::invoke_echo(_this, text).await
                })
            };
        }
        crate::plugin_method! {
            tool_invoke_stream(_this, input, sink) => {
                crate::plugin_tool_invoke_stream_surface!(input, sink, PluginMethodsToolInput, {
                    PluginMethodsToolInput { text } => PluginMethodFixture::invoke_echo_stream(_this, sink, text).await
                })
            };
        }
        crate::plugin_method! {
            permission_paths(_this) => surface(PluginMethodsToolInput, {
                PluginMethodsToolInput { text } => Ok(vec![
                    crate::PathRequest::read(format!("/tmp/method/{text}"))
                ])
            });
        }
        crate::plugin_method! {
            permission_networks(_this, tool, input) => {
                crate::plugin_permission_networks_surface!(tool, input, PluginMethodsToolInput, {
                    PluginMethodsToolInput { text } => Ok(vec![
                        crate::NetworkRequest::connect(format!("https://method-{text}.example.com"))
                    ])
                })
            };
        }
        crate::plugin_method! {
            shell_env(_this, _input) => {
                Ok(Some(crate::ShellEnvPatch::set("PLUGIN_METHOD", "1")))
            };
        }
    }

    #[test]
    fn plugin_methods_macro_generates_manifest_init_and_dispatch_hooks() -> crate::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let plugin = PluginMethodsFixture::default();
            let init = crate::Plugin::init(
                &plugin,
                crate::InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: std::path::PathBuf::from("/tmp/plugin-methods"),
                    plugin_id: "dummy.plugin_methods".to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: serde_json::Value::Null,
                    protocol_version: 1,
                },
                Arc::new(crate::NoopHostClient),
            )
            .await?;
            assert_eq!(init.manifest.name, "dummy.plugin_methods");
            assert_eq!(
                plugin.workspace_root.get().map(String::as_str),
                Some("/tmp/plugin-methods")
            );

            let invoke = crate::Plugin::tool_invoke(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "methods.echo.alias".to_string(),
                    session_id: 7,
                    call_id: 9,
                    workspace_root: "/tmp/plugin-methods".to_string(),
                    input: serde_json::json!({"text":"hello"}),
                },
            )
            .await?;
            assert_eq!(invoke.title, "Methods");
            assert_eq!(invoke.output_text, "echo:hello");

            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            let end = crate::Plugin::tool_invoke_stream(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "methods.echo.alias".to_string(),
                    session_id: 7,
                    call_id: 10,
                    workspace_root: "/tmp/plugin-methods".to_string(),
                    input: serde_json::json!({"text":"hello"}),
                },
                crate::ToolStreamSink::new("stream-1".to_string(), tx),
            )
            .await?;
            let chunk = rx.recv().await.expect("stream chunk");
            assert_eq!(chunk.text_delta.as_deref(), Some("delta:hello"));
            assert_eq!(end.stream_id, "stream-1");
            assert_eq!(end.output_text, "stream:hello");

            let path_requests = crate::Plugin::permission_paths(
                &plugin,
                "methods.echo.alias",
                &serde_json::json!({"text":"hello"}),
            )
            .await?;
            assert_eq!(path_requests, vec![crate::PathRequest::read("/tmp/hello")]);

            let network_requests = crate::Plugin::permission_networks(
                &plugin,
                "methods.echo.alias",
                &serde_json::json!({"text":"hello"}),
            )
            .await?;
            assert_eq!(
                network_requests,
                vec![crate::NetworkRequest::connect("https://hello.example.com")]
            );

            let before = crate::Plugin::tool_execute_before(
                &plugin,
                crate::ToolBeforeInput {
                    tool_name: "methods.echo".to_string(),
                    plugin_name: "dummy.plugin_methods".to_string(),
                    session_id: 7,
                    call_id: 11,
                    workspace_root: "/tmp/plugin-methods".to_string(),
                    tags: Vec::new(),
                    input: serde_json::json!({"text":"hello"}),
                    title_override: None,
                    metadata: BTreeMap::new(),
                },
            )
            .await?;
            assert_eq!(
                before.and_then(|patch| patch.title_override),
                Some("before:methods.echo".to_string())
            );

            let shell_env = crate::Plugin::shell_env(
                &plugin,
                crate::ShellEnvInput {
                    cwd: std::path::PathBuf::from("/tmp/plugin-methods"),
                    session_id: None,
                    call_id: None,
                },
            )
            .await?;
            assert_eq!(
                shell_env.and_then(|patch| patch.set.get("PLUGIN_METHODS").cloned()),
                Some("1".to_string())
            );

            let chat_params = crate::Plugin::chat_params(
                &plugin,
                crate::ChatParamsInput {
                    provider: "dummy".to_string(),
                    model: "dummy-model".to_string(),
                    params: serde_json::json!({}),
                },
            )
            .await?;
            assert_eq!(
                chat_params.and_then(|patch| patch.params),
                Some(serde_json::json!({"temperature": 0}))
            );

            crate::Plugin::shutdown(&plugin).await?;

            let chat_message = crate::Plugin::chat_message(
                &plugin,
                crate::ChatMessageInput {
                    session_id: 7,
                    direction: crate::ChatDirection::FromUser,
                    message: crate::ChatMessage::user("hello"),
                },
            )
            .await?;
            assert_eq!(
                chat_message
                    .and_then(|patch| patch.message)
                    .and_then(|message| message.text().map(ToString::to_string)),
                Some("echo:hello".to_string())
            );

            let chat_headers = crate::Plugin::chat_headers(
                &plugin,
                crate::ChatHeadersInput {
                    provider: "dummy".to_string(),
                    headers: BTreeMap::new(),
                },
            )
            .await?;
            assert_eq!(
                chat_headers
                    .as_ref()
                    .and_then(|patch| patch.set.get("X-Plugin"))
                    .map(String::as_str),
                Some("methods")
            );

            let auth = crate::Plugin::auth(
                &plugin,
                crate::AuthInput {
                    provider: "dummy".to_string(),
                    purpose: crate::AuthPurpose::ApiKey,
                    context: None,
                },
            )
            .await?;
            assert_eq!(
                auth.as_ref().map(|value| value.kind),
                Some(crate::AuthKind::ApiKey)
            );
            assert_eq!(
                auth.and_then(|value| value.credential.get("api_key").cloned()),
                Some(serde_json::Value::String("secret".to_string()))
            );

            crate::Plugin::notification(
                &plugin,
                crate::NotificationInput {
                    kind: "info".to_string(),
                    session_id: Some(7),
                    title: "Methods".to_string(),
                    message: "hello".to_string(),
                    payload: serde_json::json!({"source":"test"}),
                },
            )
            .await?;

            let user_prompt = crate::Plugin::user_prompt_submit(
                &plugin,
                crate::UserPromptSubmitInput {
                    session_id: 7,
                    prompt: "hello".to_string(),
                },
            )
            .await?;
            assert_eq!(
                user_prompt.and_then(|patch| patch.additional_context),
                Some("ctx:hello".to_string())
            );

            let tool_definition = crate::Plugin::tool_definition(
                &plugin,
                crate::ToolDefinitionInput {
                    tool_name: "methods.echo".to_string(),
                    plugin_name: "dummy.plugin_methods".to_string(),
                    description: "Echo text".to_string(),
                    summary: None,
                    help: None,
                    description_mode: None,
                    input_schema: serde_json::json!({"type":"object"}),
                },
            )
            .await?;
            assert_eq!(
                tool_definition.and_then(|patch| patch.summary),
                Some("summary:methods.echo".to_string())
            );

            crate::Plugin::tool_execute_failure(
                &plugin,
                crate::ToolFailureInput {
                    tool_name: "methods.echo".to_string(),
                    plugin_name: "dummy.plugin_methods".to_string(),
                    session_id: 7,
                    call_id: 12,
                    workspace_root: "/tmp/plugin-methods".to_string(),
                    input: serde_json::json!({"text":"hello"}),
                    error: "boom".to_string(),
                    is_interrupt: false,
                },
            )
            .await?;

            let command_after = crate::Plugin::command_execute_after(
                &plugin,
                crate::CommandAfterInput {
                    command: "bash".to_string(),
                    args: vec!["-lc".to_string(), "echo hello".to_string()],
                    cwd: std::path::PathBuf::from("/tmp/plugin-methods"),
                    exit_code: Some(0),
                    stdout: "hello\n".to_string(),
                    stderr: String::new(),
                    timed_out: false,
                },
            )
            .await?;
            assert_eq!(
                command_after.and_then(|patch| patch.stdout),
                Some("after:bash".to_string())
            );

            let config = crate::Plugin::config_resolved(
                &plugin,
                crate::ConfigInput {
                    current: serde_json::json!({}),
                },
            )
            .await?;
            assert_eq!(
                config.and_then(|patch| patch.merge),
                Some(serde_json::json!({"plugin":{"enabled":true}}))
            );

            crate::Result::Ok(())
        })
    }

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, crate::ToolCommand)]
    #[tool_command(
        tool = "attr.echo",
        description = "Exercise plugin_*_method attributes against a real ToolCommand.",
        handler_receiver = PluginAttributeFixture,
        handle = PluginAttributeFixture::invoke_echo,
        stream_handle = PluginAttributeFixture::invoke_echo_stream,
        permission_paths_handle = PluginAttributeFixture::permission_paths_echo,
        permission_networks_handle = PluginAttributeFixture::permission_networks_echo,
        handle_field = text,
        handle_by_value = true,
        concurrency_safe = true
    )]
    #[serde(deny_unknown_fields)]
    struct PluginAttributeToolInput {
        text: String,
    }

    #[derive(Default)]
    struct PluginAttributeFixture {
        workspace_root: OnceLock<String>,
    }

    impl PluginAttributeFixture {
        async fn invoke_echo(&self, text: String) -> crate::Result<crate::ToolInvokeOutput> {
            Ok(crate::ToolInvokeOutput::text(format!("attr:{text}")).with_title("Attr"))
        }

        async fn invoke_echo_stream(
            &self,
            sink: crate::ToolStreamSink,
            text: String,
        ) -> crate::Result<crate::ToolStreamEnd> {
            sink.text(format!("attr-delta:{text}")).await;
            Ok(crate::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "Attr".to_string(),
                output_text: format!("attr-stream:{text}"),
                payload: None,
                metadata: BTreeMap::new(),
                attachments: Vec::new(),
            })
        }

        async fn permission_paths_echo(
            &self,
            text: String,
        ) -> crate::Result<Vec<crate::PathRequest>> {
            Ok(vec![crate::PathRequest::read(format!("/tmp/attr/{text}"))])
        }

        async fn permission_networks_echo(
            &self,
            text: String,
        ) -> crate::Result<Vec<crate::NetworkRequest>> {
            Ok(vec![crate::NetworkRequest::connect(format!(
                "https://attr-{text}.example.com"
            ))])
        }

        fn resolve_permission_alias(
            &self,
            tool: &str,
            input: &serde_json::Value,
        ) -> crate::Result<(String, serde_json::Value)> {
            let resolved_tool = if tool == "attr.echo.alias" {
                "attr.echo"
            } else {
                tool
            };
            Ok((resolved_tool.to_string(), input.clone()))
        }

        fn resolve_invoke_alias(
            &self,
            input: crate::ToolInvokeInput,
        ) -> crate::Result<crate::ToolInvokeInput> {
            let tool_name = if input.tool_name == "attr.echo.alias" {
                "attr.echo".to_string()
            } else {
                input.tool_name
            };
            Ok(crate::ToolInvokeInput { tool_name, ..input })
        }
    }

    #[crate::plugin]
    impl crate::Plugin for PluginAttributeFixture {
        #[crate::plugin_manifest_method(
            id = "dummy.plugin_attr_method",
            version = "1.0.0",
            description = "Fixture plugin used to exercise plugin_*_method attributes.",
            hooks = HookSubscription::INIT
                | HookSubscription::TOOL_INVOKE
                | HookSubscription::TOOL_INVOKE_STREAM,
            config_schema = serde_json::json!({"type":"object"}),
            tool_surface = PluginAttributeToolInput,
        )]
        fn manifest(&self) -> crate::PluginManifest {}

        #[crate::plugin_init_method(
            workspace_root = {
                field = self.workspace_root,
                value = ctx.workspace_root.to_string_lossy().into_owned(),
                already = "workspace root already initialized"
            }
        )]
        async fn init(
            &self,
            ctx: crate::InitContext,
            _host: Arc<dyn crate::HostClient>,
        ) -> crate::Result<crate::InitOutcome> {
        }

        #[crate::plugin_tool_invoke_method(
            surface(PluginAttributeToolInput),
            resolve = PluginAttributeFixture::resolve_invoke_alias
        )]
        async fn tool_invoke(
            &self,
            input: crate::ToolInvokeInput,
        ) -> crate::Result<crate::ToolInvokeOutput> {
        }

        #[crate::plugin_tool_invoke_stream_method(
            surface(PluginAttributeToolInput),
            resolve = PluginAttributeFixture::resolve_invoke_alias
        )]
        async fn tool_invoke_stream(
            &self,
            input: crate::ToolInvokeInput,
            sink: crate::ToolStreamSink,
        ) -> crate::Result<crate::ToolStreamEnd> {
            let _ = sink;
        }

        #[crate::plugin_permission_paths_method(
            surface(PluginAttributeToolInput),
            resolve = PluginAttributeFixture::resolve_permission_alias
        )]
        async fn permission_paths(
            &self,
            tool: &str,
            input: &serde_json::Value,
        ) -> crate::Result<Vec<crate::PathRequest>> {
            let _ = (tool, input);
        }

        #[crate::plugin_permission_networks_method(
            surface(PluginAttributeToolInput),
            resolve = PluginAttributeFixture::resolve_permission_alias
        )]
        async fn permission_networks(
            &self,
            tool: &str,
            input: &serde_json::Value,
        ) -> crate::Result<Vec<crate::NetworkRequest>> {
            let _ = (tool, input);
        }
    }

    #[test]
    fn plugin_method_attributes_generate_manifest_init_and_dispatch_hooks() -> crate::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let plugin = PluginAttributeFixture::default();
            let init = crate::Plugin::init(
                &plugin,
                crate::InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: std::path::PathBuf::from("/tmp/plugin-attrs"),
                    plugin_id: "dummy.plugin_attr_method".to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: serde_json::Value::Null,
                    protocol_version: 1,
                },
                Arc::new(crate::NoopHostClient),
            )
            .await?;
            assert_eq!(init.manifest.name, "dummy.plugin_attr_method");
            assert_eq!(
                plugin.workspace_root.get().map(String::as_str),
                Some("/tmp/plugin-attrs")
            );

            let invoke = crate::Plugin::tool_invoke(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "attr.echo.alias".to_string(),
                    session_id: 7,
                    call_id: 12,
                    workspace_root: "/tmp/plugin-attrs".to_string(),
                    input: serde_json::json!({"text":"hello"}),
                },
            )
            .await?;
            assert_eq!(invoke.title, "Attr");
            assert_eq!(invoke.output_text, "attr:hello");

            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            let end = crate::Plugin::tool_invoke_stream(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "attr.echo.alias".to_string(),
                    session_id: 7,
                    call_id: 13,
                    workspace_root: "/tmp/plugin-attrs".to_string(),
                    input: serde_json::json!({"text":"hello"}),
                },
                crate::ToolStreamSink::new("stream-attr".to_string(), tx),
            )
            .await?;
            let chunk = rx.recv().await.expect("stream chunk");
            assert_eq!(chunk.text_delta.as_deref(), Some("attr-delta:hello"));
            assert_eq!(end.stream_id, "stream-attr");
            assert_eq!(end.output_text, "attr-stream:hello");

            let path_requests = crate::Plugin::permission_paths(
                &plugin,
                "attr.echo.alias",
                &serde_json::json!({"text":"hello"}),
            )
            .await?;
            assert_eq!(
                path_requests,
                vec![crate::PathRequest::read("/tmp/attr/hello")]
            );

            let network_requests = crate::Plugin::permission_networks(
                &plugin,
                "attr.echo.alias",
                &serde_json::json!({"text":"hello"}),
            )
            .await?;
            assert_eq!(
                network_requests,
                vec![crate::NetworkRequest::connect(
                    "https://attr-hello.example.com"
                )]
            );

            Ok(())
        })
    }

    #[test]
    fn plugin_method_macro_generates_item_level_manifest_and_dispatch() -> crate::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let plugin = PluginMethodFixture::default();
            assert_eq!(crate::Plugin::manifest(&plugin).name, "dummy.plugin_method");

            let init = crate::Plugin::init(
                &plugin,
                crate::InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: std::path::PathBuf::from("/tmp/plugin-method"),
                    plugin_id: "dummy.plugin_method".to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: serde_json::Value::Null,
                    protocol_version: 1,
                },
                Arc::new(crate::NoopHostClient),
            )
            .await?;
            assert_eq!(init.manifest.name, "dummy.plugin_method");
            assert_eq!(
                plugin.workspace_root.get().map(String::as_str),
                Some("/tmp/plugin-method")
            );

            let output = crate::Plugin::tool_invoke(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "methods.echo".to_string(),
                    session_id: 1,
                    call_id: 1,
                    workspace_root: ".".to_string(),
                    input: serde_json::json!({ "text": "hello" }),
                },
            )
            .await?;
            assert_eq!(output.output_text, "method:hello");

            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            let end = crate::Plugin::tool_invoke_stream(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "methods.echo".to_string(),
                    session_id: 1,
                    call_id: 2,
                    workspace_root: "/tmp/plugin-method".to_string(),
                    input: serde_json::json!({ "text": "hello" }),
                },
                crate::ToolStreamSink::new("method-stream".to_string(), tx),
            )
            .await?;
            let chunk = rx.recv().await.expect("stream chunk");
            assert_eq!(chunk.text_delta.as_deref(), Some("method-delta:hello"));
            assert_eq!(end.output_text, "method-stream:hello");

            let path_requests = crate::Plugin::permission_paths(
                &plugin,
                "methods.echo",
                &serde_json::json!({"text":"hello"}),
            )
            .await?;
            assert_eq!(
                path_requests,
                vec![crate::PathRequest::read("/tmp/method/hello")]
            );

            let network_requests = crate::Plugin::permission_networks(
                &plugin,
                "methods.echo",
                &serde_json::json!({"text":"hello"}),
            )
            .await?;
            assert_eq!(
                network_requests,
                vec![crate::NetworkRequest::connect(
                    "https://method-hello.example.com"
                )]
            );

            let shell_env = crate::Plugin::shell_env(
                &plugin,
                crate::ShellEnvInput {
                    cwd: std::path::PathBuf::from("/tmp/plugin-method"),
                    session_id: None,
                    call_id: None,
                },
            )
            .await?;
            assert_eq!(
                shell_env
                    .as_ref()
                    .and_then(|patch| patch.set.get("PLUGIN_METHOD"))
                    .map(String::as_str),
                Some("1")
            );
            Ok(())
        })
    }

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct DispatchSurfaceInput {
        value: String,
    }

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct DispatchShapeInput {
        value: String,
    }

    impl ToolSurface for DispatchSurfaceInput {
        fn tool_name() -> &'static str {
            "dispatch.surface"
        }

        fn tool_decl() -> PluginToolDecl {
            PluginToolDecl::new("dispatch.surface", serde_json::json!({"type":"object"}))
        }

        fn parse_input(input: serde_json::Value) -> crate::Result<Self> {
            serde_json::from_value(input)
                .map_err(|err| crate::PluginError::invalid_params(err.to_string()))
        }

        fn resolve_tool(
            tool: &str,
            input: serde_json::Value,
        ) -> crate::Result<(String, serde_json::Value)> {
            Ok((tool.to_string(), input))
        }
    }

    impl ToolInputShape for DispatchShapeInput {
        fn input_schema() -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        fn parse_input(input: serde_json::Value) -> crate::Result<Self> {
            serde_json::from_value(input)
                .map_err(|err| crate::PluginError::invalid_params(err.to_string()))
        }
    }

    enum DispatchSuiteInput {
        One(DispatchSurfaceInput),
    }

    impl ToolSuiteSurface for DispatchSuiteInput {
        fn tool_decls() -> Vec<PluginToolDecl> {
            vec![DispatchSurfaceInput::tool_decl()]
        }

        fn parse_tool(tool: &str, input: serde_json::Value) -> crate::Result<Self> {
            if tool == DispatchSurfaceInput::tool_name() {
                Ok(Self::One(DispatchSurfaceInput::parse_input(input)?))
            } else {
                Err(crate::PluginError::invalid_params(format!(
                    "unknown tool '{tool}'"
                )))
            }
        }

        fn resolve_tool(
            tool: &str,
            input: serde_json::Value,
        ) -> crate::Result<(String, serde_json::Value)> {
            if tool == DispatchSurfaceInput::tool_name() {
                Ok((tool.to_string(), input))
            } else {
                Err(crate::PluginError::invalid_params(format!(
                    "unknown tool '{tool}'"
                )))
            }
        }
    }

    struct DispatchReceiver;

    impl DispatchReceiver {
        async fn handle(&self, input: &DispatchSurfaceInput) -> crate::Result<String> {
            Ok(format!("handled: {}", input.value))
        }

        async fn handle_shape(&self, input: &DispatchShapeInput) -> crate::Result<String> {
            Ok(format!("handled-shape: {}", input.value))
        }

        async fn handle_with_context(
            &self,
            context: &crate::ToolInvokeContext<'_>,
            input: DispatchSurfaceInput,
        ) -> crate::Result<String> {
            Ok(format!(
                "handled-ctx:{}:{}:{}:{}",
                context.tool_name, context.session_id, context.call_id, input.value
            ))
        }

        async fn handle_shape_with_context(
            &self,
            context: &crate::ToolInvokeContext<'_>,
            input: DispatchShapeInput,
        ) -> crate::Result<String> {
            Ok(format!(
                "handled-shape-ctx:{}:{}:{}:{}",
                context.tool_name, context.session_id, context.call_id, input.value
            ))
        }
    }

    impl DispatchSurfaceInput {
        async fn dispatch_tool_invoke(self, receiver: &DispatchReceiver) -> crate::Result<String> {
            receiver.handle(&self).await
        }

        async fn dispatch_tool_invoke_with_context(
            self,
            receiver: &DispatchReceiver,
            context: &crate::ToolInvokeContext<'_>,
        ) -> crate::Result<String> {
            receiver.handle_with_context(context, self).await
        }

        async fn dispatch_tool_invoke_stream(
            self,
            _receiver: &DispatchReceiver,
            sink: crate::ToolStreamSink,
        ) -> crate::Result<crate::ToolStreamEnd> {
            sink.text(format!("stream:{}", self.value)).await;
            Ok(crate::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "surface-stream".to_string(),
                output_text: self.value,
                payload: None,
                metadata: Default::default(),
                attachments: Vec::new(),
            })
        }

        async fn dispatch_tool_invoke_stream_with_context(
            self,
            _receiver: &DispatchReceiver,
            context: &crate::ToolInvokeContext<'_>,
            sink: crate::ToolStreamSink,
        ) -> crate::Result<crate::ToolStreamEnd> {
            sink.text(format!("stream-ctx:{}:{}", context.tool_name, self.value))
                .await;
            Ok(crate::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "surface-stream-context".to_string(),
                output_text: format!("{}:{}", context.tool_name, self.value),
                payload: None,
                metadata: Default::default(),
                attachments: Vec::new(),
            })
        }

        async fn dispatch_permission_paths(
            self,
            _receiver: &DispatchReceiver,
        ) -> crate::Result<Vec<String>> {
            Ok(vec![self.value])
        }

        async fn dispatch_permission_networks(
            self,
            _receiver: &DispatchReceiver,
        ) -> crate::Result<Vec<String>> {
            Ok(vec![self.value])
        }
    }

    impl DispatchShapeInput {
        async fn dispatch_tool_invoke(self, receiver: &DispatchReceiver) -> crate::Result<String> {
            receiver.handle_shape(&self).await
        }

        async fn dispatch_tool_invoke_with_context(
            self,
            receiver: &DispatchReceiver,
            context: &crate::ToolInvokeContext<'_>,
        ) -> crate::Result<String> {
            receiver.handle_shape_with_context(context, self).await
        }

        async fn dispatch_tool_invoke_stream(
            self,
            _receiver: &DispatchReceiver,
            sink: crate::ToolStreamSink,
        ) -> crate::Result<crate::ToolStreamEnd> {
            sink.text(format!("shape-stream:{}", self.value)).await;
            Ok(crate::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "shape-stream".to_string(),
                output_text: self.value,
                payload: None,
                metadata: Default::default(),
                attachments: Vec::new(),
            })
        }

        async fn dispatch_tool_invoke_stream_with_context(
            self,
            _receiver: &DispatchReceiver,
            context: &crate::ToolInvokeContext<'_>,
            sink: crate::ToolStreamSink,
        ) -> crate::Result<crate::ToolStreamEnd> {
            sink.text(format!(
                "shape-stream-ctx:{}:{}",
                context.tool_name, self.value
            ))
            .await;
            Ok(crate::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "shape-stream-context".to_string(),
                output_text: format!("{}:{}", context.tool_name, self.value),
                payload: None,
                metadata: Default::default(),
                attachments: Vec::new(),
            })
        }

        async fn dispatch_permission_paths(
            self,
            _receiver: &DispatchReceiver,
        ) -> crate::Result<Vec<String>> {
            Ok(vec![self.value])
        }

        async fn dispatch_permission_networks(
            self,
            _receiver: &DispatchReceiver,
        ) -> crate::Result<Vec<String>> {
            Ok(vec![self.value])
        }
    }

    impl DispatchSuiteInput {
        async fn dispatch_tool_invoke(self, receiver: &DispatchReceiver) -> crate::Result<String> {
            match self {
                Self::One(input) => input.dispatch_tool_invoke(receiver).await,
            }
        }

        async fn dispatch_tool_invoke_with_context(
            self,
            receiver: &DispatchReceiver,
            context: &crate::ToolInvokeContext<'_>,
        ) -> crate::Result<String> {
            match self {
                Self::One(input) => {
                    input
                        .dispatch_tool_invoke_with_context(receiver, context)
                        .await
                }
            }
        }

        async fn dispatch_tool_invoke_stream(
            self,
            receiver: &DispatchReceiver,
            sink: crate::ToolStreamSink,
        ) -> crate::Result<crate::ToolStreamEnd> {
            match self {
                Self::One(input) => input.dispatch_tool_invoke_stream(receiver, sink).await,
            }
        }

        async fn dispatch_tool_invoke_stream_with_context(
            self,
            receiver: &DispatchReceiver,
            context: &crate::ToolInvokeContext<'_>,
            sink: crate::ToolStreamSink,
        ) -> crate::Result<crate::ToolStreamEnd> {
            match self {
                Self::One(input) => {
                    input
                        .dispatch_tool_invoke_stream_with_context(receiver, context, sink)
                        .await
                }
            }
        }

        async fn dispatch_permission_paths(
            self,
            receiver: &DispatchReceiver,
        ) -> crate::Result<Vec<String>> {
            match self {
                Self::One(input) => input.dispatch_permission_paths(receiver).await,
            }
        }

        async fn dispatch_permission_networks(
            self,
            receiver: &DispatchReceiver,
        ) -> crate::Result<Vec<String>> {
            match self {
                Self::One(input) => input.dispatch_permission_networks(receiver).await,
            }
        }
    }

    #[test]
    fn dispatch_macros_parse_and_route_tool_inputs() -> crate::Result<()> {
        let surface_value = crate::tool_surface_dispatch!(
            "dispatch.surface",
            serde_json::json!({"value":"surface"}),
            DispatchSurfaceInput,
            {
                DispatchSurfaceInput { value } => Ok::<_, crate::PluginError>(value)
            }
        )?;
        assert_eq!(surface_value, "surface");

        let suite_value = crate::tool_suite_dispatch!(
            "dispatch.surface",
            serde_json::json!({"value":"suite"}),
            DispatchSuiteInput,
            {
                DispatchSuiteInput::One(DispatchSurfaceInput { value }) => Ok::<_, crate::PluginError>(value)
            }
        )?;
        assert_eq!(suite_value, "suite");

        let shape_value = crate::tool_shape_dispatch!(
            serde_json::json!({"value":"shape"}),
            DispatchShapeInput,
            {
                DispatchShapeInput { value } => Ok::<_, crate::PluginError>(value)
            }
        )?;
        assert_eq!(shape_value, "shape");
        Ok(())
    }

    #[test]
    fn plugin_dispatch_macros_use_standard_invoke_and_permission_shapes() -> crate::Result<()> {
        let surface_invoke = crate::hooks::ToolInvokeInput {
            tool_name: "dispatch.surface".to_string(),
            session_id: 1,
            call_id: 2,
            workspace_root: "/tmp/project".to_string(),
            input: serde_json::json!({"value":"invoke-surface"}),
        };
        let surface_value = crate::plugin_tool_invoke_surface!(
            surface_invoke,
            DispatchSurfaceInput,
            {
                DispatchSurfaceInput { value } => Ok::<_, crate::PluginError>(value)
            }
        )?;
        assert_eq!(surface_value, "invoke-surface");

        let suite_invoke = crate::hooks::ToolInvokeInput {
            tool_name: "dispatch.surface".to_string(),
            session_id: 3,
            call_id: 4,
            workspace_root: "/tmp/project".to_string(),
            input: serde_json::json!({"value":"invoke-suite"}),
        };
        let suite_value = crate::plugin_tool_invoke_suite!(suite_invoke, DispatchSuiteInput, {
            DispatchSuiteInput::One(DispatchSurfaceInput { value }) => Ok::<_, crate::PluginError>(value)
        })?;
        assert_eq!(suite_value, "invoke-suite");

        let shape_invoke = crate::hooks::ToolInvokeInput {
            tool_name: "dynamic.skill".to_string(),
            session_id: 9,
            call_id: 10,
            workspace_root: "/tmp/project".to_string(),
            input: serde_json::json!({"value":"invoke-shape"}),
        };
        let shape_value = crate::plugin_tool_invoke_shape!(shape_invoke, DispatchShapeInput, {
            DispatchShapeInput { value } => Ok::<_, crate::PluginError>(value)
        })?;
        assert_eq!(shape_value, "invoke-shape");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let receiver = DispatchReceiver;
            let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel(8);
            let stream_sink = crate::ToolStreamSink::new("dispatch-stream".to_string(), stream_tx);

            let dispatch_surface = crate::plugin_tool_dispatch_surface!(
                &receiver,
                crate::hooks::ToolInvokeInput {
                    tool_name: "dispatch.surface".to_string(),
                    session_id: 5,
                    call_id: 6,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"dispatch-surface"}),
                },
                DispatchSurfaceInput
            )?;
            assert_eq!(dispatch_surface, "handled: dispatch-surface");

            let dispatch_surface_with_context = crate::plugin_tool_dispatch_surface_with_context!(
                &receiver,
                crate::hooks::ToolInvokeInput {
                    tool_name: "dispatch.surface".to_string(),
                    session_id: 15,
                    call_id: 16,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"dispatch-context"}),
                },
                DispatchSurfaceInput
            )?;
            assert_eq!(
                dispatch_surface_with_context,
                "handled-ctx:dispatch.surface:15:16:dispatch-context"
            );

            let dispatch_suite = crate::plugin_tool_dispatch_suite!(
                &receiver,
                crate::hooks::ToolInvokeInput {
                    tool_name: "dispatch.surface".to_string(),
                    session_id: 7,
                    call_id: 8,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"dispatch-suite"}),
                },
                DispatchSuiteInput
            )?;
            assert_eq!(dispatch_suite, "handled: dispatch-suite");

            let dispatch_suite_with_context = crate::plugin_tool_dispatch_suite_with_context!(
                &receiver,
                crate::hooks::ToolInvokeInput {
                    tool_name: "dispatch.surface".to_string(),
                    session_id: 17,
                    call_id: 18,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"dispatch-suite-context"}),
                },
                DispatchSuiteInput
            )?;
            assert_eq!(
                dispatch_suite_with_context,
                "handled-ctx:dispatch.surface:17:18:dispatch-suite-context"
            );

            let dispatch_shape = crate::plugin_tool_dispatch_shape!(
                &receiver,
                crate::hooks::ToolInvokeInput {
                    tool_name: "dynamic.skill".to_string(),
                    session_id: 19,
                    call_id: 20,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"dispatch-shape"}),
                },
                DispatchShapeInput
            )?;
            assert_eq!(dispatch_shape, "handled-shape: dispatch-shape");

            let dispatch_shape_with_context = crate::plugin_tool_dispatch_shape_with_context!(
                &receiver,
                crate::hooks::ToolInvokeInput {
                    tool_name: "dynamic.skill".to_string(),
                    session_id: 21,
                    call_id: 22,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"dispatch-shape-context"}),
                },
                DispatchShapeInput
            )?;
            assert_eq!(
                dispatch_shape_with_context,
                "handled-shape-ctx:dynamic.skill:21:22:dispatch-shape-context"
            );

            let (dispatch_stream_surface_tx, mut dispatch_stream_surface_rx) =
                tokio::sync::mpsc::channel(8);
            let dispatch_stream_surface_sink = crate::ToolStreamSink::new(
                "dispatch-stream-surface-handler".to_string(),
                dispatch_stream_surface_tx,
            );
            let dispatch_stream_surface = crate::plugin_tool_dispatch_stream_surface!(
                &receiver,
                crate::hooks::ToolInvokeInput {
                    tool_name: "dispatch.surface".to_string(),
                    session_id: 31,
                    call_id: 32,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"dispatch-stream-surface"}),
                },
                dispatch_stream_surface_sink,
                DispatchSurfaceInput
            )?;
            assert_eq!(dispatch_stream_surface.output_text, "dispatch-stream-surface");
            assert_eq!(
                dispatch_stream_surface_rx
                    .recv()
                    .await
                    .and_then(|chunk| chunk.text_delta),
                Some("stream:dispatch-stream-surface".to_string())
            );

            let (dispatch_stream_suite_tx, mut dispatch_stream_suite_rx) =
                tokio::sync::mpsc::channel(8);
            let dispatch_stream_suite_sink = crate::ToolStreamSink::new(
                "dispatch-stream-suite-handler".to_string(),
                dispatch_stream_suite_tx,
            );
            let dispatch_stream_suite = crate::plugin_tool_dispatch_stream_suite!(
                &receiver,
                crate::hooks::ToolInvokeInput {
                    tool_name: "dispatch.surface".to_string(),
                    session_id: 33,
                    call_id: 34,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"dispatch-stream-suite"}),
                },
                dispatch_stream_suite_sink,
                DispatchSuiteInput
            )?;
            assert_eq!(dispatch_stream_suite.output_text, "dispatch-stream-suite");
            assert_eq!(
                dispatch_stream_suite_rx
                    .recv()
                    .await
                    .and_then(|chunk| chunk.text_delta),
                Some("stream:dispatch-stream-suite".to_string())
            );

            let (dispatch_stream_shape_tx, mut dispatch_stream_shape_rx) =
                tokio::sync::mpsc::channel(8);
            let dispatch_stream_shape_sink = crate::ToolStreamSink::new(
                "dispatch-stream-shape-handler".to_string(),
                dispatch_stream_shape_tx,
            );
            let dispatch_stream_shape = crate::plugin_tool_dispatch_stream_shape!(
                &receiver,
                crate::hooks::ToolInvokeInput {
                    tool_name: "dynamic.skill".to_string(),
                    session_id: 35,
                    call_id: 36,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"dispatch-stream-shape"}),
                },
                dispatch_stream_shape_sink,
                DispatchShapeInput
            )?;
            assert_eq!(dispatch_stream_shape.output_text, "dispatch-stream-shape");
            assert_eq!(
                dispatch_stream_shape_rx
                    .recv()
                    .await
                    .and_then(|chunk| chunk.text_delta),
                Some("shape-stream:dispatch-stream-shape".to_string())
            );

            let (dispatch_stream_surface_ctx_tx, mut dispatch_stream_surface_ctx_rx) =
                tokio::sync::mpsc::channel(8);
            let dispatch_stream_surface_ctx_sink = crate::ToolStreamSink::new(
                "dispatch-stream-surface-context-handler".to_string(),
                dispatch_stream_surface_ctx_tx,
            );
            let dispatch_stream_surface_ctx =
                crate::plugin_tool_dispatch_stream_surface_with_context!(
                    &receiver,
                    crate::hooks::ToolInvokeInput {
                        tool_name: "dispatch.surface".to_string(),
                        session_id: 37,
                        call_id: 38,
                        workspace_root: "/tmp/project".to_string(),
                        input: serde_json::json!({"value":"dispatch-stream-surface-context"}),
                    },
                    dispatch_stream_surface_ctx_sink,
                    DispatchSurfaceInput
                )?;
            assert_eq!(
                dispatch_stream_surface_ctx.output_text,
                "dispatch.surface:dispatch-stream-surface-context"
            );
            assert_eq!(
                dispatch_stream_surface_ctx_rx
                    .recv()
                    .await
                    .and_then(|chunk| chunk.text_delta),
                Some(
                    "stream-ctx:dispatch.surface:dispatch-stream-surface-context".to_string()
                )
            );

            let (dispatch_stream_suite_ctx_tx, mut dispatch_stream_suite_ctx_rx) =
                tokio::sync::mpsc::channel(8);
            let dispatch_stream_suite_ctx_sink = crate::ToolStreamSink::new(
                "dispatch-stream-suite-context-handler".to_string(),
                dispatch_stream_suite_ctx_tx,
            );
            let dispatch_stream_suite_ctx =
                crate::plugin_tool_dispatch_stream_suite_with_context!(
                    &receiver,
                    crate::hooks::ToolInvokeInput {
                        tool_name: "dispatch.surface".to_string(),
                        session_id: 39,
                        call_id: 40,
                        workspace_root: "/tmp/project".to_string(),
                        input: serde_json::json!({"value":"dispatch-stream-suite-context"}),
                    },
                    dispatch_stream_suite_ctx_sink,
                    DispatchSuiteInput
                )?;
            assert_eq!(
                dispatch_stream_suite_ctx.output_text,
                "dispatch.surface:dispatch-stream-suite-context"
            );
            assert_eq!(
                dispatch_stream_suite_ctx_rx
                    .recv()
                    .await
                    .and_then(|chunk| chunk.text_delta),
                Some(
                    "stream-ctx:dispatch.surface:dispatch-stream-suite-context".to_string()
                )
            );

            let (dispatch_stream_shape_ctx_tx, mut dispatch_stream_shape_ctx_rx) =
                tokio::sync::mpsc::channel(8);
            let dispatch_stream_shape_ctx_sink = crate::ToolStreamSink::new(
                "dispatch-stream-shape-context-handler".to_string(),
                dispatch_stream_shape_ctx_tx,
            );
            let dispatch_stream_shape_ctx =
                crate::plugin_tool_dispatch_stream_shape_with_context!(
                    &receiver,
                    crate::hooks::ToolInvokeInput {
                        tool_name: "dynamic.skill".to_string(),
                        session_id: 41,
                        call_id: 42,
                        workspace_root: "/tmp/project".to_string(),
                        input: serde_json::json!({"value":"dispatch-stream-shape-context"}),
                    },
                    dispatch_stream_shape_ctx_sink,
                    DispatchShapeInput
                )?;
            assert_eq!(
                dispatch_stream_shape_ctx.output_text,
                "dynamic.skill:dispatch-stream-shape-context"
            );
            assert_eq!(
                dispatch_stream_shape_ctx_rx
                    .recv()
                    .await
                    .and_then(|chunk| chunk.text_delta),
                Some(
                    "shape-stream-ctx:dynamic.skill:dispatch-stream-shape-context".to_string()
                )
            );

            let stream_surface_end = crate::plugin_tool_invoke_stream_surface!(
                crate::hooks::ToolInvokeInput {
                    tool_name: "dispatch.surface".to_string(),
                    session_id: 23,
                    call_id: 24,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"stream-surface"}),
                },
                stream_sink,
                DispatchSurfaceInput,
                {
                    DispatchSurfaceInput { value } => {
                        stream_sink.text(format!("surface:{value}")).await;
                        Ok::<_, crate::PluginError>(crate::ToolStreamEnd {
                            stream_id: stream_sink.stream_id().to_string(),
                            title: "surface".to_string(),
                            output_text: value,
                            payload: None,
                            metadata: Default::default(),
                            attachments: Vec::new(),
                        })
                    }
                }
            )?;
            assert_eq!(stream_surface_end.output_text, "stream-surface");
            assert_eq!(
                stream_rx.recv().await.and_then(|chunk| chunk.text_delta),
                Some("surface:stream-surface".to_string())
            );

            let (suite_tx, mut suite_rx) = tokio::sync::mpsc::channel(8);
            let suite_sink =
                crate::ToolStreamSink::new("dispatch-stream-suite".to_string(), suite_tx);
            let stream_suite_end = crate::plugin_tool_invoke_stream_suite!(
                crate::hooks::ToolInvokeInput {
                    tool_name: "dispatch.surface".to_string(),
                    session_id: 25,
                    call_id: 26,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"stream-suite"}),
                },
                suite_sink,
                DispatchSuiteInput,
                {
                    DispatchSuiteInput::One(DispatchSurfaceInput { value }) => {
                        suite_sink.text(format!("suite:{value}")).await;
                        Ok::<_, crate::PluginError>(crate::ToolStreamEnd {
                            stream_id: suite_sink.stream_id().to_string(),
                            title: "suite".to_string(),
                            output_text: value,
                            payload: None,
                            metadata: Default::default(),
                            attachments: Vec::new(),
                        })
                    }
                }
            )?;
            assert_eq!(stream_suite_end.output_text, "stream-suite");
            assert_eq!(
                suite_rx.recv().await.and_then(|chunk| chunk.text_delta),
                Some("suite:stream-suite".to_string())
            );

            let (shape_tx, mut shape_rx) = tokio::sync::mpsc::channel(8);
            let shape_sink =
                crate::ToolStreamSink::new("dispatch-stream-shape".to_string(), shape_tx);
            let stream_shape_end = crate::plugin_tool_invoke_stream_shape!(
                crate::hooks::ToolInvokeInput {
                    tool_name: "dynamic.skill".to_string(),
                    session_id: 27,
                    call_id: 28,
                    workspace_root: "/tmp/project".to_string(),
                    input: serde_json::json!({"value":"stream-shape"}),
                },
                shape_sink,
                DispatchShapeInput,
                {
                    DispatchShapeInput { value } => {
                        shape_sink.text(format!("shape:{value}")).await;
                        Ok::<_, crate::PluginError>(crate::ToolStreamEnd {
                            stream_id: shape_sink.stream_id().to_string(),
                            title: "shape".to_string(),
                            output_text: value,
                            payload: None,
                            metadata: Default::default(),
                            attachments: Vec::new(),
                        })
                    }
                }
            )?;
            assert_eq!(stream_shape_end.output_text, "stream-shape");
            assert_eq!(
                shape_rx.recv().await.and_then(|chunk| chunk.text_delta),
                Some("shape:stream-shape".to_string())
            );

            let dispatch_suite_paths = crate::plugin_permission_dispatch_paths_suite!(
                &receiver,
                "dispatch.surface",
                &serde_json::json!({"value":"path-suite-dispatch"}),
                DispatchSuiteInput
            )?;
            assert_eq!(
                dispatch_suite_paths,
                vec!["path-suite-dispatch".to_string()]
            );

            let dispatch_suite_networks = crate::plugin_permission_dispatch_networks_suite!(
                &receiver,
                "dispatch.surface",
                &serde_json::json!({"value":"net-suite-dispatch"}),
                DispatchSuiteInput
            )?;
            assert_eq!(
                dispatch_suite_networks,
                vec!["net-suite-dispatch".to_string()]
            );

            let dispatch_shape_paths = crate::plugin_permission_dispatch_paths_shape!(
                &receiver,
                "dynamic.skill",
                &serde_json::json!({"value":"path-shape-dispatch"}),
                DispatchShapeInput
            )?;
            assert_eq!(dispatch_shape_paths, vec!["path-shape-dispatch".to_string()]);

            let dispatch_shape_networks = crate::plugin_permission_dispatch_networks_shape!(
                &receiver,
                "dynamic.skill",
                &serde_json::json!({"value":"net-shape-dispatch"}),
                DispatchShapeInput
            )?;
            assert_eq!(dispatch_shape_networks, vec!["net-shape-dispatch".to_string()]);

            Ok::<_, crate::PluginError>(())
        })?;

        let surface_paths = crate::plugin_permission_paths_surface!(
            "dispatch.surface",
            &serde_json::json!({"value":"path-surface"}),
            DispatchSurfaceInput,
            {
                DispatchSurfaceInput { value } => Ok::<_, crate::PluginError>(vec![value])
            }
        )?;
        assert_eq!(surface_paths, vec!["path-surface".to_string()]);

        let other_paths = crate::plugin_permission_paths_surface!(
            "other.tool",
            &serde_json::json!({"value":"path-surface"}),
            DispatchSurfaceInput,
            {
                DispatchSurfaceInput { value } => Ok::<_, crate::PluginError>(vec![value])
            }
        )?;
        assert!(other_paths.is_empty());

        let suite_paths = crate::plugin_permission_paths_suite!(
            "dispatch.surface",
            &serde_json::json!({"value":"path-suite"}),
            DispatchSuiteInput,
            {
                DispatchSuiteInput::One(DispatchSurfaceInput { value }) => Ok::<_, crate::PluginError>(vec![value])
            }
        )?;
        assert_eq!(suite_paths, vec!["path-suite".to_string()]);

        let shape_paths = crate::plugin_permission_paths_shape!(
            &serde_json::json!({"value":"path-shape"}),
            DispatchShapeInput,
            {
                DispatchShapeInput { value } => Ok::<_, crate::PluginError>(vec![value])
            }
        )?;
        assert_eq!(shape_paths, vec!["path-shape".to_string()]);

        let surface_networks = crate::plugin_permission_networks_surface!(
            "dispatch.surface",
            &serde_json::json!({"value":"net-surface"}),
            DispatchSurfaceInput,
            {
                DispatchSurfaceInput { value } => Ok::<_, crate::PluginError>(vec![value])
            }
        )?;
        assert_eq!(surface_networks, vec!["net-surface".to_string()]);

        let suite_networks = crate::plugin_permission_networks_suite!(
            "dispatch.surface",
            &serde_json::json!({"value":"net-suite"}),
            DispatchSuiteInput,
            {
                DispatchSuiteInput::One(DispatchSurfaceInput { value }) => Ok::<_, crate::PluginError>(vec![value])
            }
        )?;
        assert_eq!(suite_networks, vec!["net-suite".to_string()]);

        let shape_networks = crate::plugin_permission_networks_shape!(
            &serde_json::json!({"value":"net-shape"}),
            DispatchShapeInput,
            {
                DispatchShapeInput { value } => Ok::<_, crate::PluginError>(vec![value])
            }
        )?;
        assert_eq!(shape_networks, vec!["net-shape".to_string()]);

        Ok(())
    }
}
