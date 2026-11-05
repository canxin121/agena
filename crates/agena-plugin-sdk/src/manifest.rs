//! Plugin manifest: the contract between a plugin and the host. Either
//! delivered as a JSON file next to a cdylib/stdio binary or returned by the
//! `meta/manifest` JSON-RPC method.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use super::manifest_support::normalize_tool_tag_name;
use super::manifest_support::{normalize_schema_json, normalize_tags};
pub use agena_domain::{AccessKind, ActivityKind, PathKind};
pub use agena_plugin_contracts::{
    CommandDefinition, CommandDiagnostic, CommandDocs, CommandHostEffect, CommandInvokeInput,
    CommandResult, CommandStatus, CommandTarget, MAX_JSON_ESCAPE_BYTES, MAX_JSON_ESCAPE_DEPTH,
    PathInputKind, PluginServiceDeclarations, PluginServiceExport, PluginServiceImport,
    PluginServiceInvokeInput, PluginServiceInvokeOutput, PluginServiceMethod, SettingsConstraints,
    SettingsContract, SettingsNode, SettingsNodeKind, SettingsOption, SettingsVariant,
};

/// Explicit marker for plugins that intentionally expose an empty editable
/// settings object. This is different from omitting `settings = ...`: hosts
/// can render a real (empty) contract and validation still rejects unknown
/// keys instead of treating configuration as open-ended JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EmptyPluginSettings {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
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
    /// UI-facing translations keyed by BCP-47 locale. The base fields above
    /// remain the stable default documentation sent to models.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub translations: BTreeMap<String, PluginManifestTranslation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(default)]
    pub transports: Vec<TransportKind>,
    #[serde(default)]
    pub hooks: HookSubscription,
    /// Each declared handler is preserved independently of its subscription.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hook_handlers: Vec<HookHandlerDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandDefinition>,
    /// Declared service seams. The host resolves imports before initialization
    /// and is the only component allowed to bind a consumer to a provider.
    #[serde(default, skip_serializing_if = "PluginServiceDeclarations::is_empty")]
    pub services: PluginServiceDeclarations,
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
    /// Neutral plugin surface contributions. Operations are kept in the
    /// manifest's `commands` collection; this field is terminal-only
    /// presentation and never defines configuration or executable behavior.
    #[serde(default, skip_serializing_if = "PluginSurfaceContributions::is_empty")]
    pub surface: PluginSurfaceContributions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<SettingsContract>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
/// UI translations of a plugin's documentation and settings presentation.
pub struct PluginManifestTranslation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Settings titles and descriptions keyed by their canonical English copy.
    /// Identifiers, defaults, values and validation constraints remain unchanged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
/// UI translations for one locale of a tool's documentation.
pub struct ToolDocsTranslation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

/// Author-owned translations for one hook handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookHandlerTranslation {
    #[serde(deserialize_with = "deserialize_required_hook_text")]
    pub summary: String,
    #[serde(deserialize_with = "deserialize_required_hook_text")]
    pub help: String,
}

/// Matcher metadata, not an additional policy or execution authority.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HookHandlerFilters {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// One authored function associated with an event. UI descriptions come from
/// the plugin, while runtime state belongs to the Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookHandlerDefinition {
    /// Stable within the plugin, generated from the Rust method name.
    pub id: String,
    pub event: String,
    #[serde(deserialize_with = "deserialize_required_hook_text")]
    pub summary: String,
    #[serde(deserialize_with = "deserialize_required_hook_text")]
    pub help: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub translations: BTreeMap<String, HookHandlerTranslation>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub filters: HookHandlerFilters,
    #[serde(default)]
    pub is_async: bool,
}

impl HookHandlerDefinition {
    pub fn summary_for_locale(&self, locale: &str) -> &str {
        translated_field(&self.translations, locale, |translation| {
            Some(translation.summary.as_str())
        })
        .unwrap_or(&self.summary)
    }

    pub fn help_for_locale(&self, locale: &str) -> &str {
        translated_field(&self.translations, locale, |translation| {
            Some(translation.help.as_str())
        })
        .unwrap_or(&self.help)
    }
}

fn deserialize_required_hook_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let text = String::deserialize(deserializer)?;
    if text.trim().is_empty() {
        return Err(D::Error::custom("hook summary and help must be non-empty"));
    }
    Ok(text)
}

/// The tag vocabulary a tool declares for discovery, search, UI badges,
/// profile filtering, and workflow hints.
///
/// Tags are the tool's single self-description surface. There is no parallel
/// set of behavior flags: everything the host reads about *what a tool is* —
/// that it runs shell commands, opens a live session, only reads, or mutates
/// state — is one of these tags. Tags carry no authority; the permission
/// engine decides, and a tool that performs its own I/O is responsible for
/// asking the host first.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolTag {
    /// Query/read-style tools (fetch data, inspect state).
    Query,
    /// State-changing tools (create, update, delete).
    Mutate,
    /// Execute or run something (shell commands, code execution).
    Execute,
    /// Runs arbitrary shell commands. Selects the shell executor, routes the
    /// tool through the shell command-class rules, and makes the tool
    /// ineligible for the read-only builtin profile.
    Shell,
    /// Agent coordination tool; excluded from no-agent model
    /// profiles and granted a long invoke budget.
    Agent,
    /// Declared read-only: the tool only reads. Used for the read-only builtin
    /// profile, the `tools.rules."*".read-only` class default, and discovery.
    ReadOnly,
    /// Operates on files/paths in the workspace.
    Filesystem,
    /// Talks to remote services (web, APIs, network targets).
    Network,
    /// Reads/consumes a remote service without changing it.
    Fetch,
    /// Discovers or lists things (search, list, index, help).
    Discovery,
    /// Declared interactive: the tool opens a live session with a process or a
    /// human. Used for builtin profiles, discovery, the invoke timeout budget,
    /// and planning eligibility.
    Interactive,
    /// Supports planning / plan-locked workflows.
    Planning,
    /// Goal-driven or long-horizon task tools.
    Goal,
    /// Scheduled/background automation.
    Scheduler,
    /// Language-server-backed code intelligence.
    Lsp,
    /// MCP bridge tools.
    Mcp,
    /// Custom extension tags contributed by a plugin.
    Custom(String),
}

impl ToolTag {
    /// Read one tag by its spelling, for callers that match on names.
    pub fn is_shell(tags: &[Self]) -> bool {
        tags.contains(&Self::Shell)
    }

    /// Whether `tags` describes an interactive tool.
    pub fn is_interactive(tags: &[Self]) -> bool {
        tags.contains(&Self::Interactive)
    }

    /// Whether `tags` describes an agent coordination tool.
    pub fn is_agent(tags: &[Self]) -> bool {
        tags.contains(&Self::Agent)
    }

    /// Whether `tags` describes a state-changing tool.
    pub fn is_mutating(tags: &[Self]) -> bool {
        tags.contains(&Self::Mutate)
    }

    /// Whether `tags` declares a tool that only reads: read-only, and neither
    /// shell, interactive, mutating, nor an agent coordination tool.
    ///
    /// This is the single definition of the read-only class. The permission
    /// engine's `tools.rules."*".read-only` default, the read-only builtin
    /// profile, and the runtime's tool descriptor all read it, so a tool is
    /// read-only in exactly one sense.
    pub fn is_read_only(tags: &[Self]) -> bool {
        tags.contains(&Self::ReadOnly)
            && !Self::is_shell(tags)
            && !Self::is_interactive(tags)
            && !Self::is_mutating(tags)
            && !Self::is_agent(tags)
    }
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
            "shell" => Self::Shell,
            "agent" => Self::Agent,
            "read_only" | "read-only" | "readonly" => Self::ReadOnly,
            "filesystem" => Self::Filesystem,
            "network" => Self::Network,
            "fetch" => Self::Fetch,
            "discovery" => Self::Discovery,
            "interactive" => Self::Interactive,
            "planning" => Self::Planning,
            "goal" => Self::Goal,
            "scheduler" => Self::Scheduler,
            "lsp" => Self::Lsp,
            "mcp" => Self::Mcp,
            other => Self::Custom(other.to_string()),
        })
    }

    fn label(&self) -> &str {
        match self {
            Self::Query => "query",
            Self::Mutate => "mutate",
            Self::Execute => "execute",
            Self::Shell => "shell",
            Self::Agent => "agent",
            Self::ReadOnly => "read_only",
            Self::Filesystem => "filesystem",
            Self::Network => "network",
            Self::Fetch => "fetch",
            Self::Discovery => "discovery",
            Self::Interactive => "interactive",
            Self::Planning => "planning",
            Self::Goal => "goal",
            Self::Scheduler => "scheduler",
            Self::Lsp => "lsp",
            Self::Mcp => "mcp",
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
#[serde(deny_unknown_fields)]
/// Definition of a plugin tool.
pub struct ToolDefinition {
    pub name: String,
    #[serde(default)]
    pub contract: ToolContract,
    #[serde(default)]
    pub docs: ToolDocs,
    #[serde(default)]
    pub runtime: ToolRuntimePolicy,
    /// What this tool is, as declared by the plugin.
    ///
    /// This is the tool's single self-description surface: the host reads
    /// `shell`, `agent`, `read_only`, `interactive`, and `mutate` here to select
    /// an executor, decide whether the tool may run concurrently, filter the
    /// builtin profiles, and pick the shell command-class rules. A tool that
    /// performs its own I/O is responsible for asking the host
    /// (`HostClient::require_path_permission` and friends) before it touches
    /// anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ToolTag>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// Input and output contract of a tool.
pub struct ToolContract {
    #[serde(default)]
    pub input_schema: serde_json::Value,
    #[serde(
        default,
        skip_serializing_if = "crate::manifest_support::serde_json_value_is_empty_schema"
    )]
    pub output_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
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
    /// UI-facing translations keyed by BCP-47 locale. Model-facing tool
    /// definitions continue to use the stable default fields above.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub translations: BTreeMap<String, ToolDocsTranslation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// Runtime behavior policy of a tool.
///
/// Tools own synchronization of their state and external effects. The host
/// bounds total concurrency independently of provider protocol capabilities.
/// Output length is bounded by the session layer.
pub struct ToolRuntimePolicy {
    #[serde(default)]
    pub streaming: ToolStreamingMode,
}

impl Default for ToolContract {
    fn default() -> Self {
        Self {
            input_schema: serde_json::Value::Null,
            output_schema: serde_json::Value::Null,
        }
    }
}

impl Default for ToolRuntimePolicy {
    fn default() -> Self {
        Self {
            streaming: ToolStreamingMode::Buffered,
        }
    }
}

/// Types that can be used as tool input.
pub trait ToolInput: Sized {
    fn input_schema() -> serde_json::Value;
    fn parse_input(input: serde_json::Value) -> crate::Result<Self>;
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

    pub fn summary_for_locale(&self, locale: &str) -> Option<&str> {
        self.docs.summary_for_locale(locale)
    }

    pub fn help_for_locale(&self, locale: &str) -> Option<&str> {
        self.docs.help_for_locale(locale)
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

    /// The tool's tags, deduplicated, for discovery, search, UI, profile
    /// filtering, and executor selection.
    ///
    /// This is the declared tag list and nothing else — there is no second
    /// declaration surface to fold in. It carries no authority: the host never
    /// derives a permission decision from a tag.
    pub fn effective_tags(&self) -> Vec<ToolTag> {
        normalize_tags(self.tags.iter().cloned())
    }

    pub fn has_tag(&self, tag: ToolTag) -> bool {
        self.effective_tags()
            .iter()
            .any(|existing| existing == &tag)
    }
}

impl ToolDocs {
    pub fn summary_for_locale(&self, locale: &str) -> Option<&str> {
        translated_field(&self.translations, locale, |translation| {
            translation.summary.as_deref()
        })
        .or_else(|| non_empty_text(self.summary.as_deref()))
    }

    pub fn help_for_locale(&self, locale: &str) -> Option<&str> {
        translated_field(&self.translations, locale, |translation| {
            translation.help.as_deref()
        })
        .or_else(|| non_empty_text(self.help.as_deref()))
    }

    pub fn before_help_for_locale(&self, locale: &str) -> Option<&str> {
        translated_field(&self.translations, locale, |translation| {
            translation.before_help.as_deref()
        })
        .or_else(|| non_empty_text(self.before_help.as_deref()))
    }

    pub fn after_help_for_locale(&self, locale: &str) -> Option<&str> {
        translated_field(&self.translations, locale, |translation| {
            translation.after_help.as_deref()
        })
        .or_else(|| non_empty_text(self.after_help.as_deref()))
    }
}

fn non_empty_text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn translated_field<'a, T>(
    translations: &'a BTreeMap<String, T>,
    locale: &str,
    field: impl Fn(&'a T) -> Option<&'a str>,
) -> Option<&'a str> {
    let locale = locale.trim().replace('_', "-");
    let language = locale.split('-').next().filter(|tag| !tag.is_empty());
    let candidates = [Some(locale.as_str()), language, Some("en-US"), Some("en")];
    for candidate in candidates.into_iter().flatten() {
        if let Some((_, translation)) = translations
            .iter()
            .find(|(tag, _)| tag.eq_ignore_ascii_case(candidate))
            && let Some(value) = field(translation)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        {
            return Some(value);
        }
    }
    None
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Streaming mode of a tool.
pub enum ToolStreamingMode {
    #[default]
    Buffered,
    Streaming,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// Terminal-only presentation contributions declared by a plugin.
pub struct PluginSurfaceContributions {
    /// Declarative display contributions (Phase 6): pure content plus a kind,
    /// no location/color. The host decides placement and priority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub display: Vec<PluginDisplayContribution>,
    #[serde(default, skip_serializing_if = "PluginTerminalContributions::is_empty")]
    pub terminal: PluginTerminalContributions,
}

impl PluginSurfaceContributions {
    pub fn is_empty(&self) -> bool {
        self.display.is_empty() && self.terminal.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// Terminal-only presentation contributions of a plugin.
pub struct PluginTerminalContributions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<PluginTerminalThemePalette>,
}

impl PluginTerminalContributions {
    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
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
pub struct PluginTerminalThemePalette {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub colors: PluginTerminalThemeColors,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(transparent)]
/// A TUI color value.
pub struct PluginTerminalColor(String);

impl PluginTerminalColor {
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

impl std::str::FromStr for PluginTerminalColor {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for PluginTerminalColor {
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
pub struct PluginTerminalThemeColors {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<PluginTerminalColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<PluginTerminalColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<PluginTerminalColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<PluginTerminalColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<PluginTerminalColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub danger: Option<PluginTerminalColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special: Option<PluginTerminalColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_fg: Option<PluginTerminalColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_bg: Option<PluginTerminalColor>,
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
        const TOOL_FAILURE              = 1 << 4;
        const TOOL_INVOKE               = 1 << 5;
        const TOOL_INVOKE_STREAM        = 1 << 6;
        const TOOL_DEFINITION           = 1 << 7;
        const CHAT_MESSAGE              = 1 << 8;
        const CHAT_MESSAGES_TRANSFORM   = 1 << 9;
        const CHAT_PARAMS               = 1 << 10;
        const CHAT_HEADERS              = 1 << 11;
        const CHAT_SYSTEM_TRANSFORM     = 1 << 12;
        const PROVIDER_LIST             = 1 << 13;
        const NOTIFICATION              = 1 << 14;
        const COMMAND_BEFORE            = 1 << 15;
        const COMMAND_AFTER             = 1 << 16;
        const SHELL_ENV                 = 1 << 17;
        const CONFIG                    = 1 << 18;
        const SESSION_START             = 1 << 19;
        const SESSION_END               = 1 << 20;
        const USER_PROMPT_SUBMIT        = 1 << 21;
        const AGENT_STOP                = 1 << 22;
        const AGENT_CANCEL              = 1 << 23;
        const PRE_RUN                   = 1 << 24;
        const POST_RUN                  = 1 << 25;
        const MODEL_REQUEST_BEFORE      = 1 << 26;
        const MODEL_REQUEST_AFTER       = 1 << 27;
        const MODEL_REQUEST_ERROR       = 1 << 28;
        const COMPACTION_BEFORE         = 1 << 29;
        const COMPACTION_AFTER          = 1 << 30;
        const SUBAGENT_START            = 1 << 31;
        const SUBAGENT_STOP             = 1 << 32;
        const PERMISSION_REQUEST        = 1 << 33;
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
    ("tool.before", HookSubscription::TOOL_BEFORE),
    ("tool.after", HookSubscription::TOOL_AFTER),
    ("tool.failure", HookSubscription::TOOL_FAILURE),
    ("tool.invoke", HookSubscription::TOOL_INVOKE),
    ("tool.invoke.stream", HookSubscription::TOOL_INVOKE_STREAM),
    ("tool.definition", HookSubscription::TOOL_DEFINITION),
    ("chat.message", HookSubscription::CHAT_MESSAGE),
    ("chat.messages", HookSubscription::CHAT_MESSAGES_TRANSFORM),
    ("chat.params", HookSubscription::CHAT_PARAMS),
    ("chat.headers", HookSubscription::CHAT_HEADERS),
    ("chat.system", HookSubscription::CHAT_SYSTEM_TRANSFORM),
    ("provider.list", HookSubscription::PROVIDER_LIST),
    ("notification", HookSubscription::NOTIFICATION),
    ("shell.before", HookSubscription::COMMAND_BEFORE),
    ("shell.after", HookSubscription::COMMAND_AFTER),
    ("shell.env", HookSubscription::SHELL_ENV),
    ("config.resolved", HookSubscription::CONFIG),
    ("session.start", HookSubscription::SESSION_START),
    ("session.end", HookSubscription::SESSION_END),
    ("prompt.submit", HookSubscription::USER_PROMPT_SUBMIT),
    ("agent.stop", HookSubscription::AGENT_STOP),
    ("agent.cancel", HookSubscription::AGENT_CANCEL),
    ("run.pre", HookSubscription::PRE_RUN),
    ("run.post", HookSubscription::POST_RUN),
    (
        "model.request.before",
        HookSubscription::MODEL_REQUEST_BEFORE,
    ),
    ("model.request.after", HookSubscription::MODEL_REQUEST_AFTER),
    ("model.request.error", HookSubscription::MODEL_REQUEST_ERROR),
    ("compaction.before", HookSubscription::COMPACTION_BEFORE),
    ("compaction.after", HookSubscription::COMPACTION_AFTER),
    ("subagent.start", HookSubscription::SUBAGENT_START),
    ("subagent.stop", HookSubscription::SUBAGENT_STOP),
    ("permission.request", HookSubscription::PERMISSION_REQUEST),
];

fn hook_subscription_for_name(name: &str) -> Option<HookSubscription> {
    HOOK_NAMES
        .iter()
        .find_map(|(known, flag)| (*known == name).then_some(*flag))
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
            summary: None,
            help: None,
            translations: BTreeMap::new(),
            authors: Vec::new(),
            transports: Vec::new(),
            hooks: HookSubscription::INIT | HookSubscription::SHUTDOWN,
            hook_handlers: Vec::new(),
            tools: Vec::new(),
            commands: Vec::new(),
            services: PluginServiceDeclarations::default(),
            activity_kinds: Vec::new(),
            tags: Vec::new(),
            skills: Vec::new(),
            surface: PluginSurfaceContributions::default(),
            settings: None,
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

    pub fn summary_for_locale(&self, locale: &str) -> Option<&str> {
        translated_field(&self.translations, locale, |translation| {
            translation.summary.as_deref()
        })
        .or_else(|| self.summary_text())
    }

    pub fn help_for_locale(&self, locale: &str) -> Option<&str> {
        translated_field(&self.translations, locale, |translation| {
            translation.help.as_deref()
        })
        .or_else(|| self.help_text())
    }

    /// Localize the settings tree in a UI-owned manifest snapshot. The base
    /// manifest used by validation and model contracts is left to the caller.
    pub fn localize_settings(&mut self, locale: &str) {
        if let Some(settings) = &mut self.settings {
            settings.map_presentation_text(|source| {
                translated_field(&self.translations, locale, |translation| {
                    translation.settings.get(source).map(String::as_str)
                })
                .unwrap_or(source)
                .to_owned()
            });
        }
    }
}

/// Free-form metadata attached to manifests, tools, and UI definitions.
pub type Metadata = BTreeMap<String, String>;

#[cfg(test)]
mod hook_catalog_tests {
    use super::*;

    #[test]
    fn every_named_hook_has_a_unique_serializable_subscription() {
        let mut seen = std::collections::HashSet::new();
        let mut all = HookSubscription::empty();
        for (name, flag) in HookSubscription::all_named() {
            assert!(seen.insert(name), "duplicate hook name: {name}");
            assert_eq!(HookSubscription::for_name(name), Some(*flag));
            assert_eq!(flag.names(), vec![*name]);
            let encoded = serde_json::to_string(flag).unwrap();
            let decoded: HookSubscription = serde_json::from_str(&encoded).unwrap();
            assert_eq!(&decoded, flag, "roundtrip failed for {name}");
            assert!(!all.contains(*flag), "duplicate hook subscription: {name}");
            all |= *flag;
        }
        assert_eq!(all, HookSubscription::all());
    }

    #[test]
    fn hook_handler_documents_cannot_omit_or_empty_required_fields() {
        let good = serde_json::json!({
            "id": "guard_shell",
            "event": "tool.before",
            "summary": "Guard shell commands",
            "help": "Checks commands before shell execution.",
            "translations": {
                "zh-CN": {
                    "summary": "检查 Shell 命令",
                    "help": "在执行前检查 Shell 命令。"
                }
            }
        });
        let parsed: HookHandlerDefinition = serde_json::from_value(good.clone()).unwrap();
        assert_eq!(parsed.summary_for_locale("zh-CN"), "检查 Shell 命令");
        assert_eq!(
            parsed.help_for_locale("fr-FR"),
            "Checks commands before shell execution."
        );

        for missing in ["summary", "help"] {
            let mut value = good.clone();
            value.as_object_mut().unwrap().remove(missing);
            assert!(
                serde_json::from_value::<HookHandlerDefinition>(value).is_err(),
                "{missing} must be present"
            );
        }
        for field in ["summary", "help"] {
            let mut value = good.clone();
            value[field] = serde_json::json!(" ");
            assert!(
                serde_json::from_value::<HookHandlerDefinition>(value).is_err(),
                "{field} must be nonempty"
            );
            let mut translated = good.clone();
            translated["translations"]["zh-CN"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(
                serde_json::from_value::<HookHandlerDefinition>(translated).is_err(),
                "translation must include {field}"
            );
        }
    }
}
