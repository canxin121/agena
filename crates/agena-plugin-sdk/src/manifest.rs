//! Plugin manifest: the contract between a plugin and the host. Either
//! delivered as a JSON file next to a cdylib/stdio binary or returned by the
//! `meta/manifest` JSON-RPC method.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use super::manifest_support::normalize_tool_tag_name;
use super::manifest_support::{hook_subscription_for_name, normalize_schema_json, normalize_tags};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub namespace: String,
    pub name: String,
    pub version: String,
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
    pub commands: Vec<PluginCommandDefinition>,
    /// Plugin-level host capabilities. Useful for plugins that need to
    /// call host APIs without exposing any model-visible tool. These are merged
    /// into the effective capability set alongside the per-tool
    /// definitions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugin_capabilities: Vec<HostCapability>,
    /// UI contributions owned by this plugin. TUI-facing content and Studio
    /// Web-facing views/controls are intentionally split so each host can
    /// consume only the view it can render.
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

    fn label(&self) -> &str {
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
        f.write_str(self.label())
    }
}

impl AsRef<str> for ToolTag {
    fn as_ref(&self) -> &str {
        self.label()
    }
}

impl Serialize for ToolTag {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
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
    #[serde(
        default,
        skip_serializing_if = "crate::manifest_support::serde_json_value_is_empty_schema"
    )]
    pub output_schema: serde_json::Value,
    #[serde(default)]
    pub strict: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ToolModelSurface {
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

pub trait ToolInput: Sized {
    fn input_schema() -> serde_json::Value;
    fn parse_input(input: serde_json::Value) -> crate::Result<Self>;
    fn input_paths() -> Vec<InputPathSpec> {
        Vec::new()
    }
    fn input_networks() -> Vec<InputNetworkSpec> {
        Vec::new()
    }
    fn input_tags() -> Vec<ToolTag> {
        Vec::new()
    }
    fn input_example() -> Option<serde_json::Value> {
        None
    }
    fn input_usage() -> Option<String> {
        let schema = Self::input_schema();
        Self::input_example()
            .and_then(|example| {
                let merged = crate::macro_support::merge_example_with_schema(&schema, &example);
                crate::macro_support::command_usage_text_for_schema(&schema, &merged)
            })
            .or_else(|| crate::macro_support::command_usage_text_from_schema(&schema))
    }
    fn parse_json_str(input: &str) -> crate::Result<Self> {
        let value = crate::macro_support::parse_json_value_str(input)?;
        Self::parse_input(value)
    }
}

fn simple_tool_input_schema<T>() -> serde_json::Value
where
    T: schemars::JsonSchema,
{
    crate::macro_support::json_schema_for::<T>()
}

fn simple_tool_input_parse<T>(input: serde_json::Value) -> crate::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    crate::macro_support::parse_typed_json_value(input)
}

macro_rules! impl_simple_tool_input {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ToolInput for $ty {
                fn input_schema() -> serde_json::Value {
                    simple_tool_input_schema::<Self>()
                }

                fn parse_input(input: serde_json::Value) -> crate::Result<Self> {
                    simple_tool_input_parse(input)
                }
            }
        )+
    };
}

impl_simple_tool_input!(
    String, bool, i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f32, f64,
);

impl<T> ToolInput for Option<T>
where
    T: schemars::JsonSchema + serde::de::DeserializeOwned,
{
    fn input_schema() -> serde_json::Value {
        simple_tool_input_schema::<Self>()
    }

    fn parse_input(input: serde_json::Value) -> crate::Result<Self> {
        simple_tool_input_parse(input)
    }
}

impl<T> ToolInput for Vec<T>
where
    T: schemars::JsonSchema + serde::de::DeserializeOwned,
{
    fn input_schema() -> serde_json::Value {
        simple_tool_input_schema::<Self>()
    }

    fn parse_input(input: serde_json::Value) -> crate::Result<Self> {
        simple_tool_input_parse(input)
    }
}

impl ToolInput for serde_json::Value {
    fn input_schema() -> serde_json::Value {
        simple_tool_input_schema::<Self>()
    }

    fn parse_input(input: serde_json::Value) -> crate::Result<Self> {
        Ok(input)
    }
}

impl ToolDefinition {
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

    pub fn input_schema(&self) -> serde_json::Value {
        normalize_schema_json(self.contract.input_schema.clone())
    }

    pub fn output_schema(&self) -> serde_json::Value {
        normalize_schema_json(self.contract.output_schema.clone())
    }

    pub fn effective_tags(&self) -> Vec<ToolTag> {
        let mut tags = normalize_tags(self.permissions.tags.iter().cloned());
        let mut push_normalized_tag = |tag: ToolTag| {
            if !tags.iter().any(|existing| existing == &tag) {
                tags.push(tag);
            }
        };
        for spec in &self.permissions.input_paths {
            match spec.kind {
                PathKind::Read => push_normalized_tag(ToolTag::FilesystemRead),
                PathKind::Write => push_normalized_tag(ToolTag::FilesystemWrite),
            }
        }
        for spec in &self.permissions.path_access {
            match spec.kind {
                PathKind::Read => push_normalized_tag(ToolTag::FilesystemRead),
                PathKind::Write => push_normalized_tag(ToolTag::FilesystemWrite),
            }
        }
        if !self.permissions.input_networks.is_empty()
            || !self.permissions.network_access.is_empty()
        {
            push_normalized_tag(ToolTag::Network);
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
pub struct PluginCommandDefinition {
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
    /// Optional JSON schema for command arguments accepted by the plugin
    /// command handler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    /// Plugin method-backed commands set this to the command id. Hosts can use
    /// it to route UI/slash invocations back through `command/invoke`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    #[serde(default)]
    pub action: PluginUiAction,
}

pub type PluginStudioCommand = PluginCommandDefinition;

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
    InvokeCommand {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
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
    /// Value used when `jsonpath` has no matches. This is useful for inputs
    /// whose omitted field has a meaningful permission target, such as the
    /// workspace root represented by an empty path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
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
    /// Value used when `jsonpath` has no matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
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

#[cfg(test)]
mod tests {
    use crate::manifest_support::normalize_schema_json;
    use serde_json::json;

    #[test]
    fn normalize_schema_preserves_property_names() {
        let schema = json!({
            "type": "object",
            "properties": {
                "valid_name": { "type": "string" },
                "invalid-name": { "type": "string" },
                "WithCaps": { "type": "string" }
            },
            "required": ["valid_name", "invalid-name", "WithCaps"]
        });

        let normalized = normalize_schema_json(schema);
        let properties = normalized
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("object properties");
        assert!(properties.contains_key("valid_name"));
        assert!(properties.contains_key("invalid-name"));
        assert!(properties.contains_key("WithCaps"));
        assert_eq!(
            normalized.get("required"),
            Some(&json!(["valid_name", "invalid-name", "WithCaps"]))
        );
    }
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
        names.iter().try_fold(HookSubscription::empty(), |out, n| {
            if let Some(flag) = hook_subscription_for_name(n.as_str()) {
                Ok(out | flag)
            } else {
                Err(serde::de::Error::custom(format!(
                    "unknown hook subscription `{n}`"
                )))
            }
        })
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

/// Free-form metadata attached to manifests, tools, and UI definitions.
pub type Metadata = BTreeMap<String, String>;
