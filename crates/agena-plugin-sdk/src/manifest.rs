//! Plugin manifest: the contract between a plugin and the host. Either
//! delivered as a JSON file next to a cdylib/stdio binary or returned by the
//! `meta/manifest` JSON-RPC method.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use super::manifest_support::normalize_tool_tag_name;
use super::manifest_support::{hook_subscription_for_name, normalize_schema_json, normalize_tags};
pub use agena_domain::{
    ActivityKind, InputNetworkSpec, InputPathSpec, NetworkAccessSpec, PathAccessSpec, PathKind,
    ToolPermissionContract,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Declared manifest of a plugin.
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
    /// Activity kinds contributed by this plugin. Hosts merge these into the
    /// built-in catalog so new kinds appear in transcript expansion settings
    /// automatically while the plugin is loaded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity_kinds: Vec<ActivityKind>,
    /// Metadata tags describing what this plugin does for discovery/search/UI.
    /// Tags are metadata only and never carry authority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ToolTag>,
    /// Declarative Skill packages contributed by this plugin. They are data
    /// carried in the already-validated plugin manifest, not arbitrary paths
    /// for the host to scan. Hosts must still apply their normal Skill trust,
    /// activation, and allowed-tool policies before injecting instructions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<PluginSkillDefinition>,
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
/// Transport used to talk to a plugin.
pub enum TransportKind {
    Static,
    Cdylib,
    Stdio,
    Http,
}

/// A self-contained plain-text Skill declared by a plugin manifest. This
/// mirrors `SKILL.md`'s small catalog contract while avoiding a dependency
/// from the plugin SDK to the filesystem Skill parser.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PluginSkillDefinition {
    /// Canonical catalog name. It must be unique within the contributing
    /// plugin and is subject to host-wide precedence rules.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Plain-text instructions returned when a caller reads this Skill.
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// Metadata tags describing what a tool *does* for discovery, search, UI
/// badges, and workflow hints. Tags are function/category metadata only and
/// are fully decoupled from the permission contract: a tag never carries
/// authority, and the permission engine never reads a tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolTag {
    /// Query/read-style tools (fetch data, inspect state).
    Query,
    /// State-changing tools (create, update, delete).
    Mutate,
    /// Execute or run something (shell commands, code execution).
    Execute,
    /// Operates on files/paths in the workspace.
    Filesystem,
    /// Talks to remote services (web, APIs, network targets).
    Network,
    /// Reads/consumes a remote service without changing it.
    Fetch,
    /// Discovers or lists things (search, list, index, help).
    Discovery,
    /// Interacts with a live process, server, or human session.
    Interactive,
    /// Supports planning / plan-locked workflows.
    Planning,
    /// Goal-driven or long-horizon task tools.
    Goal,
    /// Works with snapshots/checkpoints.
    Snapshot,
    /// Scheduled/background automation.
    Scheduler,
    /// Language-server-backed code intelligence.
    Lsp,
    /// MCP bridge tools.
    Mcp,
    /// Spawns or manages subtasks.
    Subtask,
    /// Custom extension tags contributed by a plugin.
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
            "query" => Self::Query,
            "mutate" => Self::Mutate,
            "execute" => Self::Execute,
            "filesystem" => Self::Filesystem,
            "network" => Self::Network,
            "fetch" => Self::Fetch,
            "discovery" => Self::Discovery,
            "interactive" => Self::Interactive,
            "planning" => Self::Planning,
            "goal" => Self::Goal,
            "snapshot" => Self::Snapshot,
            "scheduler" => Self::Scheduler,
            "lsp" => Self::Lsp,
            "mcp" => Self::Mcp,
            "subtask" => Self::Subtask,
            other => Self::Custom(other.to_string()),
        })
    }

    fn label(&self) -> &str {
        match self {
            Self::Query => "query",
            Self::Mutate => "mutate",
            Self::Execute => "execute",
            Self::Filesystem => "filesystem",
            Self::Network => "network",
            Self::Fetch => "fetch",
            Self::Discovery => "discovery",
            Self::Interactive => "interactive",
            Self::Planning => "planning",
            Self::Goal => "goal",
            Self::Snapshot => "snapshot",
            Self::Scheduler => "scheduler",
            Self::Lsp => "lsp",
            Self::Mcp => "mcp",
            Self::Subtask => "subtask",
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
/// Definition of a plugin tool.
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

    ///
    /// Tags are metadata only and never carry authority. Permission decisions
    /// read [`ToolPermissionContract`]; a tag must never be treated as a
    /// permission. `effective_tags` augments these declared tags with display
    /// tags derived from the permission contract for the same discovery/UI
    /// purposes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ToolTag>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Input and output contract of a tool.
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
/// Model-facing surface of a tool (examples).
pub struct ToolModelSurface {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
/// Documentation of a tool.
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
/// Runtime behavior policy of a tool.
pub struct ToolRuntimePolicy {
    #[serde(default)]
    pub concurrency_safe: bool,
    #[serde(default)]
    pub streaming: ToolStreamingMode,
    #[serde(default, skip_serializing_if = "ToolResultPolicy::is_default")]
    pub result_policy: ToolResultPolicy,
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

/// Types that can be used as tool input.
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

    pub fn input_schema(&self) -> serde_json::Value {
        let schema = normalize_schema_json(self.contract.input_schema.clone());
        if schema.is_null() {
            serde_json::json!({
                "type": "object",
                "properties": {}
            })
        } else {
            schema
        }
    }

    pub fn output_schema(&self) -> serde_json::Value {
        normalize_schema_json(self.contract.output_schema.clone())
    }

    pub fn effective_tags(&self) -> Vec<ToolTag> {
        // Declared metadata tags only. Tags describe what the tool does for
        // discovery/UI/workflow hints and are fully decoupled from the
        // permission contract: authority lives exclusively in
        // [`ToolPermissionContract`] and is never derived from a tag.
        normalize_tags(self.tags.iter().cloned())
    }

    pub fn has_tag(&self, tag: ToolTag) -> bool {
        self.effective_tags()
            .iter()
            .any(|existing| existing == &tag)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Streaming mode of a tool.
pub enum ToolStreamingMode {
    #[default]
    Buffered,
    Streaming,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Policy for rendering tool results.
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
/// How a tool result is rendered.
pub enum ToolResultRenderKind {
    #[default]
    Text,
    Markdown,
    Json,
    Log,
    Diff,
    Hidden,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// UI contributions declared by a plugin.
pub struct PluginUiContributions {
    /// Declarative display contributions (Phase 6): pure content plus a kind,
    /// no location/color. The host decides placement and priority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub display: Vec<PluginDisplayContribution>,
    #[serde(default, skip_serializing_if = "PluginTuiUiContributions::is_empty")]
    pub tui: PluginTuiUiContributions,
    #[serde(default, skip_serializing_if = "PluginStudioUiContributions::is_empty")]
    pub studio: PluginStudioUiContributions,
}

impl PluginUiContributions {
    pub fn is_empty(&self) -> bool {
        self.display.is_empty() && self.tui.is_empty() && self.studio.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// TUI UI contributions of a plugin.
pub struct PluginTuiUiContributions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<PluginUiThemePalette>,
}

impl PluginTuiUiContributions {
    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Studio UI contributions of a plugin.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// A display contribution of a plugin.
pub struct PluginDisplayContribution {
    pub id: String,
    pub kind: ContributionKind,
    #[serde(default)]
    pub priority: i32,
    pub content: PluginDisplayContent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a plugin display contribution.
pub enum ContributionKind {
    StatusLineText,
    Progress,
    TerminalTitle,
    TerminalNotify,
    TerminalActivity,
    FooterBlock,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// Content of a plugin display contribution.
pub enum PluginDisplayContent {
    Text { text: String },
    Progress { current: u32, total: u32 },
    TerminalActivity { value: String },
    TerminalNotify { value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A plugin theme palette.
pub struct PluginUiThemePalette {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub colors: PluginTuiThemeColors,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(transparent)]
/// A TUI color value.
pub struct PluginTuiColor(String);

impl PluginTuiColor {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if is_tui_color(&value) {
            Ok(Self(value))
        } else {
            Err(format!("invalid TUI color `{value}`"))
        }
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::str::FromStr for PluginTuiColor {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for PluginTuiColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn is_tui_color(value: &str) -> bool {
    matches!(
        value,
        "reset"
            | "black"
            | "red"
            | "green"
            | "yellow"
            | "blue"
            | "magenta"
            | "cyan"
            | "gray"
            | "dark_gray"
            | "light_red"
            | "light_green"
            | "light_yellow"
            | "light_blue"
            | "light_magenta"
            | "light_cyan"
            | "white"
    ) || value
        .strip_prefix('#')
        .is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
/// Colors of a plugin TUI theme.
pub struct PluginTuiThemeColors {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<PluginTuiColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<PluginTuiColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<PluginTuiColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<PluginTuiColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<PluginTuiColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub danger: Option<PluginTuiColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special: Option<PluginTuiColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_fg: Option<PluginTuiColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_bg: Option<PluginTuiColor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Definition of a plugin command.
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
    ///
    /// A command is an explicit control/UI route and has no independent tool
    /// permission identity. Protected effects must be delegated to a
    /// registered tool or a permission-enforcing Host API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    #[serde(default)]
    pub action: PluginUiAction,
}

/// A plugin studio command (alias of [`PluginCommandDefinition`]).
pub type PluginStudioCommand = PluginCommandDefinition;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A plugin studio control.
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
/// An option of a plugin studio control.
pub struct PluginStudioControlOption {
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A plugin studio view.
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
/// Action triggered by a plugin UI element.
pub enum PluginUiAction {
    #[default]
    None,
    InvokeTool {
        /// A tool owned by the same plugin. Hosts must execute this through
        /// the normal tool authorization path.
        tool: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
        #[serde(default)]
        submit_output_as_prompt: bool,
    },
    OpenPluginWorkbench {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
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

pub const PLUGIN_WORKBENCH_TAB_IDS: [&str; 6] = [
    "config",
    "tools",
    "commands",
    "capabilities",
    "logs",
    "diagnostics",
];

pub fn plugin_workbench_tab_id_is_supported(value: &str) -> bool {
    let value = value.trim();
    PLUGIN_WORKBENCH_TAB_IDS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(value))
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

#[cfg(test)]
mod tests {
    use super::{
        ContributionKind, PluginDisplayContent, PluginDisplayContribution, PluginTuiColor,
        PluginTuiThemeColors, PluginUiContributions,
    };
    use crate::manifest_support::normalize_schema_json;
    use serde_json::json;

    #[test]
    fn display_contribution_round_trips_without_location_or_color() {
        let contribution = PluginDisplayContribution {
            id: "plan:3".to_owned(),
            kind: ContributionKind::Progress,
            priority: 120,
            content: PluginDisplayContent::Progress {
                current: 2,
                total: 5,
            },
        };
        let wire = serde_json::to_value(&contribution).expect("serialize");
        assert!(wire.get("location").is_none());
        assert!(wire.get("color").is_none());
        let restored: PluginDisplayContribution =
            serde_json::from_value(wire).expect("deserialize");
        assert_eq!(restored, contribution);
    }

    #[test]
    fn ui_contributions_accept_declarative_display_channel() {
        let manifest_ui = PluginUiContributions {
            display: vec![PluginDisplayContribution {
                id: "terminal.activity".to_owned(),
                kind: ContributionKind::TerminalActivity,
                priority: i32::MAX - 1,
                content: PluginDisplayContent::TerminalActivity {
                    value: "idle".to_owned(),
                },
            }],
            ..PluginUiContributions::default()
        };
        let wire = serde_json::to_value(&manifest_ui).expect("serialize");
        assert!(wire.get("display").is_some());
        assert!(!manifest_ui.is_empty());
    }

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

    #[test]
    fn tui_theme_schema_rejects_noncanonical_keys_and_colors() {
        assert!("light_red".parse::<PluginTuiColor>().is_ok());
        assert!("#12aBcF".parse::<PluginTuiColor>().is_ok());
        assert!("light-red".parse::<PluginTuiColor>().is_err());
        assert!("default".parse::<PluginTuiColor>().is_err());
        assert!(
            serde_json::from_value::<PluginTuiThemeColors>(json!({
                "flash_error": "red"
            }))
            .is_err()
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
            authors: Vec::new(),
            transports: Vec::new(),
            hooks: HookSubscription::INIT | HookSubscription::SHUTDOWN,
                        tools: Vec::new(),
            commands: Vec::new(),
            activity_kinds: Vec::new(),
            tags: Vec::new(),
            skills: Vec::new(),
            ui: PluginUiContributions::default(),
            config_schema: None,
            config_schema_i18n: BTreeMap::new(),
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
