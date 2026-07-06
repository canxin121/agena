//! Plugin manifest: the contract between a plugin and the host. Either
//! delivered as a JSON file next to a cdylib/stdio binary or returned by the
//! `meta/manifest` JSON-RPC method.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub namespace: String,
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
    /// Preferred default presentation mode for tools published by this plugin
    /// when an individual tool definition does not specify its own mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_description_mode: Option<ToolDescriptionMode>,
    /// Preferred default text density for UI surfaces that render this plugin
    /// or its tools when an individual tool definition does not specify its
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
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<PluginStudioCommand>,
    /// Plugin-level host capabilities. Useful for plugins that need to
    /// call host APIs without exposing any model-visible tool. These are merged
    /// into the effective capability set alongside the per-tool
    /// definitions.
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
    Snapshot,
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
            "snapshot" => Self::Snapshot,
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
            Self::Snapshot => "snapshot",
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
pub struct ToolDefinition {
    pub name: String,
    #[serde(default)]
    pub contract: ToolContract,
    #[serde(default)]
    pub model: ToolModelSurface,
    #[serde(default)]
    pub docs: ToolDocs,
    #[serde(default)]
    pub runtime: ToolRuntimePolicy,
    #[serde(default)]
    pub permissions: ToolPermissionContract,
    #[serde(default)]
    pub display: ToolDisplay,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<HostCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolContract {
    #[serde(default)]
    pub input_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "serde_json_value_is_empty_schema")]
    pub output_schema: serde_json::Value,
    #[serde(default)]
    pub strict: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ToolModelSurface {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ToolDocs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolRuntimePolicy {
    #[serde(default)]
    pub concurrency_safe: bool,
    #[serde(default)]
    pub streaming: ToolStreamingMode,
    #[serde(default, skip_serializing_if = "ToolResultPolicy::is_default")]
    pub result_policy: ToolResultPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ToolPermissionContract {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_paths: Vec<InputPathSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_networks: Vec<InputNetworkSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_access: Vec<PathAccessSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network_access: Vec<NetworkAccessSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ToolTag>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ToolDisplay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_mode: Option<ToolDescriptionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_display_mode: Option<UiTextDisplayMode>,
}

impl Default for ToolContract {
    fn default() -> Self {
        Self {
            input_schema: serde_json::Value::Null,
            output_schema: serde_json::Value::Null,
            strict: false,
        }
    }
}

impl Default for ToolRuntimePolicy {
    fn default() -> Self {
        Self {
            concurrency_safe: false,
            streaming: ToolStreamingMode::Buffered,
            result_policy: ToolResultPolicy::default(),
        }
    }
}

pub trait ToolSurface: Sized {
    fn tool_name() -> &'static str;
    fn tool_definition() -> ToolDefinition;
    fn tool_definitions() -> Vec<ToolDefinition> {
        vec![Self::tool_definition()]
    }
    fn parse_input(input: serde_json::Value) -> crate::Result<Self>;
    fn parse_tool(tool: &str, input: serde_json::Value) -> crate::Result<Self> {
        let definition = Self::tool_definition();
        if tool == definition.name {
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
    fn tool_definitions() -> Vec<ToolDefinition>;
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

impl ToolDefinition {
    pub fn description_text(&self) -> &str {
        self.model.description.as_deref().unwrap_or("")
    }

    pub fn summary_text(&self) -> Option<&str> {
        self.docs
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn help_text(&self) -> Option<&str> {
        self.docs
            .help
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn before_help_text(&self) -> Option<&str> {
        self.docs
            .before_help
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn after_help_text(&self) -> Option<&str> {
        self.docs
            .after_help
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn example_texts(&self) -> &[String] {
        self.model.examples.as_slice()
    }

    pub fn preferred_description_mode(&self) -> Option<ToolDescriptionMode> {
        self.display.description_mode
    }

    pub fn preferred_ui_display_mode(&self) -> Option<UiTextDisplayMode> {
        self.display.ui_display_mode
    }

    pub fn sanitized_input_schema(&self) -> serde_json::Value {
        sanitize_schema_json(self.contract.input_schema.clone())
    }

    pub fn sanitized_output_schema(&self) -> serde_json::Value {
        sanitize_schema_json(self.contract.output_schema.clone())
    }

    pub fn effective_tags(&self) -> Vec<ToolTag> {
        let mut tags = normalize_tags(self.permissions.tags.iter().cloned());
        for spec in &self.permissions.input_paths {
            match spec.kind {
                PathKind::Read => push_normalized_tag(&mut tags, ToolTag::FilesystemRead),
                PathKind::Write => push_normalized_tag(&mut tags, ToolTag::FilesystemWrite),
            }
        }
        for spec in &self.permissions.path_access {
            match spec.kind {
                PathKind::Read => push_normalized_tag(&mut tags, ToolTag::FilesystemRead),
                PathKind::Write => push_normalized_tag(&mut tags, ToolTag::FilesystemWrite),
            }
        }
        if !self.permissions.input_networks.is_empty()
            || !self.permissions.network_access.is_empty()
        {
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
    SnapshotRegistry,
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
    sanitize_schema_json_value(value, true)
}

fn serde_json_value_is_empty_schema(value: &serde_json::Value) -> bool {
    matches!(value, serde_json::Value::Null)
        || value.as_object().is_some_and(|object| object.is_empty())
}

fn sanitize_schema_json_value(
    value: serde_json::Value,
    remove_schema_metadata: bool,
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut object) => {
            if remove_schema_metadata {
                object.remove("$schema");
                object.remove("title");
            }
            let mut cleaned = serde_json::Map::new();
            for (key, value) in object {
                let sanitized = match key.as_str() {
                    "properties" | "$defs" | "definitions" | "patternProperties"
                    | "dependentSchemas" => match value {
                        serde_json::Value::Object(map) => serde_json::Value::Object(
                            map.into_iter()
                                .map(|(nested_key, nested_value)| {
                                    (nested_key, sanitize_schema_json_value(nested_value, true))
                                })
                                .collect(),
                        ),
                        other => sanitize_schema_json_value(other, true),
                    },
                    _ => sanitize_schema_json_value(value, true),
                };
                cleaned.insert(key, sanitized);
            }
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
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|item| sanitize_schema_json_value(item, true))
                .collect(),
        ),
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

impl PluginManifest {
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            namespace: namespace.into(),
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
        }
    }

    pub fn from_full_name(full_name: impl AsRef<str>, version: impl Into<String>) -> Self {
        let full_name = full_name.as_ref().trim();
        let (namespace, name) = full_name
            .split_once('.')
            .filter(|(namespace, name)| !namespace.is_empty() && !name.is_empty())
            .map(|(namespace, name)| (namespace.to_string(), name.to_string()))
            .unwrap_or_else(|| ("local".to_string(), full_name.to_string()));
        Self::new(namespace, name, version)
    }

    pub fn set_display(&mut self, preset: ToolDisplayPreset) {
        self.tool_description_mode = Some(preset.tool_description_mode());
        self.ui_display_mode = Some(preset.ui_display_mode());
    }

    pub fn add_plugin_capability(&mut self, capability: HostCapability) {
        if !self.plugin_capabilities.contains(&capability) {
            self.plugin_capabilities.push(capability);
        }
    }

    pub fn add_plugin_capabilities(
        &mut self,
        capabilities: impl IntoIterator<Item = HostCapability>,
    ) {
        for capability in capabilities {
            self.add_plugin_capability(capability);
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

impl ToolDefinition {
    pub fn new(name: impl Into<String>, schema: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            contract: ToolContract {
                input_schema: schema,
                ..ToolContract::default()
            },
            model: ToolModelSurface::default(),
            docs: ToolDocs::default(),
            runtime: ToolRuntimePolicy::default(),
            permissions: ToolPermissionContract::default(),
            display: ToolDisplay::default(),
            capabilities: Vec::new(),
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
        self.model.description = Some(d.into());
        self
    }

    pub fn long_about(self, description: impl Into<String>) -> Self {
        self.description(description)
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.docs.summary = Some(summary.into());
        self
    }

    pub fn about(self, summary: impl Into<String>) -> Self {
        self.summary(summary)
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.docs.help = Some(help.into());
        self
    }

    pub fn long_help(self, help: impl Into<String>) -> Self {
        self.help(help)
    }

    pub fn after_help(self, help: impl Into<String>) -> Self {
        let mut this = self;
        this.docs.after_help = Some(help.into());
        this
    }

    pub fn after_long_help(self, help: impl Into<String>) -> Self {
        let mut this = self;
        this.docs.after_help = Some(help.into());
        this
    }

    pub fn before_help(self, description: impl Into<String>) -> Self {
        let mut this = self;
        this.docs.before_help = Some(description.into());
        this
    }

    pub fn before_long_help(self, description: impl Into<String>) -> Self {
        let mut this = self;
        this.docs.before_help = Some(description.into());
        this
    }

    pub fn display(mut self, preset: ToolDisplayPreset) -> Self {
        self.display.description_mode = Some(preset.tool_description_mode());
        self.display.ui_display_mode = Some(preset.ui_display_mode());
        self
    }

    pub fn example(mut self, example: impl Into<String>) -> Self {
        self.model.examples.push(example.into());
        self
    }

    pub fn examples<I, S>(mut self, examples: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.model
            .examples
            .extend(examples.into_iter().map(Into::into));
        self
    }

    pub fn description_mode(mut self, mode: ToolDescriptionMode) -> Self {
        self.display.description_mode = Some(mode);
        self
    }

    pub fn ui_display_mode(mut self, mode: UiTextDisplayMode) -> Self {
        self.display.ui_display_mode = Some(mode);
        self
    }

    pub fn input_path(mut self, spec: InputPathSpec) -> Self {
        self.permissions.input_paths.push(spec);
        self
    }

    pub fn output_schema(mut self, schema: serde_json::Value) -> Self {
        self.contract.output_schema = schema;
        self
    }

    pub fn input_network(mut self, spec: InputNetworkSpec) -> Self {
        self.permissions.input_networks.push(spec);
        self
    }

    pub fn path_access(mut self, spec: PathAccessSpec) -> Self {
        self.permissions.path_access.push(spec);
        self
    }

    pub fn network_access(mut self, spec: NetworkAccessSpec) -> Self {
        self.permissions.network_access.push(spec);
        self
    }

    pub fn tag(mut self, tag: ToolTag) -> Self {
        self.permissions.tags.push(tag);
        self
    }

    pub fn tags<I>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = ToolTag>,
    {
        self.permissions.tags = tags.into_iter().collect();
        self
    }

    pub fn concurrency_safe(mut self, concurrency_safe: bool) -> Self {
        self.runtime.concurrency_safe = concurrency_safe;
        self
    }

    pub fn strict(mut self, strict: bool) -> Self {
        self.contract.strict = strict;
        self
    }

    pub fn streaming(mut self, streaming: ToolStreamingMode) -> Self {
        self.runtime.streaming = streaming;
        self
    }

    pub fn result_policy(mut self, policy: ToolResultPolicy) -> Self {
        self.runtime.result_policy = policy;
        self
    }

    pub fn max_model_chars(mut self, max_model_chars: usize) -> Self {
        self.runtime.result_policy.max_model_chars = Some(max_model_chars);
        self
    }

    pub fn preview_lines(mut self, preview_lines: usize) -> Self {
        self.runtime.result_policy.preview_lines = Some(preview_lines);
        self
    }

    pub fn persist_large_output(mut self, persist: bool) -> Self {
        self.runtime.result_policy.persist_large_output = persist;
        self
    }

    pub fn ui_render_kind(mut self, kind: ToolResultRenderKind) -> Self {
        self.runtime.result_policy.ui_render_kind = kind;
        self
    }

    pub fn capability(mut self, capability: HostCapability) -> Self {
        self.capabilities.push(capability);
        self
    }

    pub fn capabilities(mut self, capabilities: impl IntoIterator<Item = HostCapability>) -> Self {
        self.capabilities.extend(capabilities);
        self
    }
}

/// Free-form metadata attached to manifests, tools, and UI definitions.
pub type Metadata = BTreeMap<String, String>;
