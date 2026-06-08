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
    /// `cat` in plugin `agena.catalog` becomes `agena_catalog__cat`.
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
    #[serde(default, skip_serializing_if = "ToolResultPolicy::is_default")]
    pub result_policy: ToolResultPolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_capabilities: Vec<HostCapability>,
}

pub trait ToolSurface: Sized {
    fn tool_name() -> &'static str;
    fn tool_decl() -> PluginToolDecl;
    fn parse_input(input: serde_json::Value) -> crate::Result<Self>;
    fn parse_tool(tool: &str, input: serde_json::Value) -> crate::Result<Self> {
        let decl = Self::tool_decl();
        if tool == decl.name || decl.aliases.iter().any(|alias| alias == tool) {
            return Self::parse_input(input);
        }
        Err(crate::PluginError::invalid_params(format!(
            "unknown {} tool '{tool}'",
            Self::tool_name()
        )))
    }
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
    Brief,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiTextDisplayMode {
    #[default]
    Detailed,
    Summary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolResultPolicy {
    /// Maximum text characters sent back to the model. The host truncates
    /// `ToolInvokeOutput.output_text` after `tool.execute.after` hooks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_model_chars: Option<usize>,
    /// Maximum preview lines rendered in compact UI surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_lines: Option<usize>,
    /// Persist full text output to the workspace result store when the model
    /// output is truncated by this policy.
    #[serde(default)]
    pub persist_large_output: bool,
    #[serde(default)]
    pub ui_render_kind: ToolResultRenderKind,
}

impl ToolResultPolicy {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultRenderKind {
    #[default]
    Text,
    Markdown,
    Json,
    Log,
    Diff,
    Hidden,
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

impl HookSubscription {
    pub fn names(self) -> Vec<&'static str> {
        HOOK_NAMES
            .iter()
            .filter_map(|(name, flag)| self.contains(*flag).then_some(*name))
            .collect()
    }

    pub fn all_named() -> &'static [(&'static str, HookSubscription)] {
        HOOK_NAMES
    }

    pub fn for_name(name: &str) -> Option<HookSubscription> {
        hook_subscription_for_name(name)
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
            } else {
                return Err(serde::de::Error::custom(format!(
                    "unknown hook subscription `{n}`"
                )));
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

pub fn hook_subscription_for_name(name: &str) -> Option<HookSubscription> {
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
            result_policy: ToolResultPolicy::default(),
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

    pub fn result_policy(mut self, policy: ToolResultPolicy) -> Self {
        self.result_policy = policy;
        self
    }

    pub fn max_model_chars(mut self, max_model_chars: usize) -> Self {
        self.result_policy.max_model_chars = Some(max_model_chars);
        self
    }

    pub fn preview_lines(mut self, preview_lines: usize) -> Self {
        self.result_policy.preview_lines = Some(preview_lines);
        self
    }

    pub fn persist_large_output(mut self, persist: bool) -> Self {
        self.result_policy.persist_large_output = persist;
        self
    }

    pub fn ui_render_kind(mut self, kind: ToolResultRenderKind) -> Self {
        self.result_policy.ui_render_kind = kind;
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
    use std::sync::{Arc, OnceLock};

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
    fn plugin_tool_decl_supports_result_policy_builder() {
        let decl = PluginToolDecl::new("dummy", serde_json::json!({"type":"object"}))
            .max_model_chars(1200)
            .preview_lines(8)
            .persist_large_output(true)
            .ui_render_kind(ToolResultRenderKind::Markdown);

        assert_eq!(decl.result_policy.max_model_chars, Some(1200));
        assert_eq!(decl.result_policy.preview_lines, Some(8));
        assert!(decl.result_policy.persist_large_output);
        assert_eq!(
            decl.result_policy.ui_render_kind,
            ToolResultRenderKind::Markdown
        );

        let value = serde_json::to_value(&decl).expect("tool decl serializes");
        assert_eq!(value["result_policy"]["max_model_chars"], 1200);
        assert_eq!(value["result_policy"]["preview_lines"], 8);
        assert_eq!(value["result_policy"]["persist_large_output"], true);
        assert_eq!(value["result_policy"]["ui_render_kind"], "markdown");
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
    struct PluginDocFixture;

    /// Fixture plugin described only by doc comments.
    ///
    /// Longer help is also carried into the generated manifest.
    #[crate::plugin(id = "dummy.plugin_doc", version = "1.0.0")]
    impl PluginDocFixture {}

    #[test]
    fn plugin_macro_uses_impl_docs_for_manifest_text() {
        let manifest = crate::Plugin::manifest(&PluginDocFixture);
        assert_eq!(
            manifest.description.as_deref(),
            Some(
                "Fixture plugin described only by doc comments.\n\nLonger help is also carried into the generated manifest."
            )
        );
        assert_eq!(
            manifest.summary.as_deref(),
            Some("Fixture plugin described only by doc comments.")
        );
        assert_eq!(
            manifest.help.as_deref(),
            Some(
                "Fixture plugin described only by doc comments.\n\nLonger help is also carried into the generated manifest."
            )
        );
    }

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, crate::ToolCommand)]
    #[tool_command(
        tool = "layer.echo",
        aliases("layer.echo.alias"),
        description = "Exercise plugin-layer tool aggregation.",
        trim("text"),
        non_empty("text"),
        streaming = "streaming",
        concurrency_safe = true
    )]
    #[serde(deny_unknown_fields)]
    struct PluginLayerToolInput {
        text: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, crate::ToolCommand)]
    #[tool_command(
        tool = "layer.lookup",
        aliases("layer.lookup.alias"),
        description = "Exercise plugin-layer suite aggregation.",
        trim("query"),
        non_empty("query"),
        concurrency_safe = true
    )]
    #[serde(deny_unknown_fields)]
    struct PluginLayerLookupInput {
        query: String,
    }

    #[derive(Debug, crate::ToolSubcommands)]
    enum PluginLayerSuiteInput {
        Lookup(PluginLayerLookupInput),
    }

    #[derive(Default, crate::PluginConfigStore)]
    struct PluginLayerFixture {
        #[config(default)]
        config: crate::PluginConfig<DummyInitConfig>,
        workspace_root: OnceLock<String>,
    }

    #[crate::plugin(
        id = "dummy.plugin_layer",
        version = "1.0.0",
        description = "Fixture plugin used to exercise plugin-layer aggregation.",
        config,
        display = compact
    )]
    impl PluginLayerFixture {
        #[tool]
        async fn echo(&self, input: PluginLayerToolInput) -> crate::ToolInvokeOutput {
            crate::ToolInvokeOutput::text(format!("layer:{}", input.text)).with_title("Layer")
        }

        #[tool_suite]
        fn lookup(&self, input: PluginLayerSuiteInput) -> crate::Result<crate::ToolInvokeOutput> {
            match input {
                PluginLayerSuiteInput::Lookup(input) => Ok(crate::ToolInvokeOutput::text(format!(
                    "lookup:{}",
                    input.query
                ))
                .with_title("Layer Suite")),
            }
        }

        #[stream(for = echo)]
        async fn echo_stream(
            &self,
            input: PluginLayerToolInput,
            sink: crate::ToolStreamSink,
        ) -> crate::ToolInvokeOutput {
            sink.text(format!("layer-delta:{}", input.text)).await;
            crate::ToolInvokeOutput::text(format!("layer-stream:{}", input.text))
                .with_title("Layer")
        }

        #[stream(for = lookup)]
        async fn lookup_stream(
            &self,
            sink: crate::ToolStreamSink,
            input: PluginLayerSuiteInput,
        ) -> String {
            let PluginLayerSuiteInput::Lookup(input) = input;
            sink.text(format!("lookup-delta:{}", input.query)).await;
            format!("lookup-stream:{}", input.query)
        }

        #[permission(paths)]
        fn echo_paths(&self, input: PluginLayerToolInput) -> Vec<crate::PathRequest> {
            vec![crate::PathRequest::read(format!(
                "/tmp/layer/{}",
                input.text
            ))]
        }

        #[permission(networks, suite)]
        fn lookup_networks(&self, input: PluginLayerSuiteInput) -> crate::NetworkRequest {
            match input {
                PluginLayerSuiteInput::Lookup(input) => {
                    crate::NetworkRequest::connect(format!("https://{}.example.com", input.query))
                }
            }
        }

        #[hook]
        async fn init(
            &self,
            ctx: crate::InitContext,
            _host: Arc<dyn crate::HostClient>,
        ) -> crate::Result<crate::InitOutcome> {
            self.workspace_root
                .set(ctx.workspace_root.to_string_lossy().into_owned())
                .map_err(|_| crate::PluginError::invalid_params("workspace root already set"))?;
            Ok(crate::InitOutcome::ack(crate::Plugin::manifest(self)))
        }

        #[hook]
        fn shell_env(&self, _input: crate::ShellEnvInput) -> crate::ShellEnvPatch {
            crate::ShellEnvPatch::set("PLUGIN_LAYER", "1")
        }
    }

    #[test]
    fn plugin_layer_macro_generates_manifest_dispatch_permissions_and_hooks() -> crate::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let plugin = PluginLayerFixture::default();
            let manifest = crate::Plugin::manifest(&plugin);
            assert_eq!(manifest.name, "dummy.plugin_layer");
            assert!(manifest.hooks.contains(HookSubscription::INIT));
            assert!(manifest.hooks.contains(HookSubscription::TOOL_INVOKE));
            assert!(
                manifest
                    .hooks
                    .contains(HookSubscription::TOOL_INVOKE_STREAM)
            );
            assert!(manifest.hooks.contains(HookSubscription::SHELL_ENV));
            assert_eq!(manifest.tools.len(), 2);
            let echo_decl = manifest
                .tools
                .iter()
                .find(|tool| tool.name == "layer.echo")
                .expect("echo tool");
            assert_eq!(echo_decl.aliases, vec!["layer.echo.alias"]);
            let lookup_decl = manifest
                .tools
                .iter()
                .find(|tool| tool.name == "layer.lookup")
                .expect("lookup tool");
            assert_eq!(lookup_decl.aliases, vec!["layer.lookup.alias"]);
            assert_eq!(
                manifest
                    .config_schema
                    .as_ref()
                    .and_then(|schema| schema.get("default")),
                Some(&serde_json::json!({"enabled": false}))
            );

            let init = crate::Plugin::init(
                &plugin,
                crate::InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: std::path::PathBuf::from("/tmp/plugin-layer"),
                    plugin_id: "dummy.plugin_layer".to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: serde_json::json!({"enabled": true}),
                    protocol_version: 1,
                },
                Arc::new(crate::NoopHostClient),
            )
            .await?;
            assert_eq!(init.manifest.name, "dummy.plugin_layer");
            assert_eq!(
                plugin.config.get(),
                Some(&DummyInitConfig { enabled: true })
            );
            assert_eq!(
                plugin.workspace_root.get().map(String::as_str),
                Some("/tmp/plugin-layer")
            );

            let output = crate::Plugin::tool_invoke(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "layer.echo.alias".to_string(),
                    session_id: 1,
                    call_id: 2,
                    workspace_root: "/tmp/plugin-layer".to_string(),
                    input: serde_json::json!({"text":"  hello  "}),
                },
            )
            .await?;
            assert_eq!(output.title, "Layer");
            assert_eq!(output.output_text, "layer:hello");

            let suite_output = crate::Plugin::tool_invoke(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "layer.lookup.alias".to_string(),
                    session_id: 1,
                    call_id: 3,
                    workspace_root: "/tmp/plugin-layer".to_string(),
                    input: serde_json::json!({"query":"  docs  "}),
                },
            )
            .await?;
            assert_eq!(suite_output.title, "Layer Suite");
            assert_eq!(suite_output.output_text, "lookup:docs");

            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            let end = crate::Plugin::tool_invoke_stream(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "layer.echo.alias".to_string(),
                    session_id: 1,
                    call_id: 4,
                    workspace_root: "/tmp/plugin-layer".to_string(),
                    input: serde_json::json!({"text":"  hello  "}),
                },
                crate::ToolStreamSink::new("layer-stream".to_string(), tx),
            )
            .await?;
            let chunk = rx.recv().await.expect("stream chunk");
            assert_eq!(chunk.text_delta.as_deref(), Some("layer-delta:hello"));
            assert_eq!(end.stream_id, "layer-stream");
            assert_eq!(end.title, "Layer");
            assert_eq!(end.output_text, "layer-stream:hello");

            let (suite_tx, mut suite_rx) = tokio::sync::mpsc::channel(4);
            let suite_end = crate::Plugin::tool_invoke_stream(
                &plugin,
                crate::ToolInvokeInput {
                    tool_name: "layer.lookup.alias".to_string(),
                    session_id: 1,
                    call_id: 5,
                    workspace_root: "/tmp/plugin-layer".to_string(),
                    input: serde_json::json!({"query":"  docs  "}),
                },
                crate::ToolStreamSink::new("layer-suite-stream".to_string(), suite_tx),
            )
            .await?;
            let suite_chunk = suite_rx.recv().await.expect("suite stream chunk");
            assert_eq!(suite_chunk.text_delta.as_deref(), Some("lookup-delta:docs"));
            assert_eq!(suite_end.stream_id, "layer-suite-stream");
            assert_eq!(suite_end.output_text, "lookup-stream:docs");

            let path_requests = crate::Plugin::permission_paths(
                &plugin,
                "layer.echo.alias",
                &serde_json::json!({"text":"  hello  "}),
            )
            .await?;
            assert_eq!(
                path_requests,
                vec![crate::PathRequest::read("/tmp/layer/hello")]
            );

            let network_requests = crate::Plugin::permission_networks(
                &plugin,
                "layer.lookup.alias",
                &serde_json::json!({"query":"  docs  "}),
            )
            .await?;
            assert_eq!(
                network_requests,
                vec![crate::NetworkRequest::connect("https://docs.example.com")]
            );

            let shell_env = crate::Plugin::shell_env(
                &plugin,
                crate::ShellEnvInput {
                    cwd: std::path::PathBuf::from("/tmp/plugin-layer"),
                    session_id: None,
                    call_id: None,
                },
            )
            .await?;
            assert_eq!(
                shell_env
                    .as_ref()
                    .and_then(|patch| patch.set.get("PLUGIN_LAYER"))
                    .map(String::as_str),
                Some("1")
            );

            crate::Result::Ok(())
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
}
