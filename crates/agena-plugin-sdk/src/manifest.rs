//! Plugin manifest: the contract between a plugin and the host. Either
//! delivered as a TOML file next to a cdylib/stdio binary or returned by the
//! `meta/manifest` JSON-RPC method.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    pub entries: Vec<PluginEntryDecl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Static,
    Cdylib,
    Stdio,
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginEntryDecl {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_entry_behavior")]
    pub behavior: EntryBehavior,
    #[serde(default)]
    pub input_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expose_as: Option<String>,
    /// Declarative path-permission specs. The host extracts paths from the
    /// entry input via JSONPath before invocation and audits them as
    /// [`PathKind`]. Use [`Plugin::permission_paths`] for paths that can only
    /// be derived dynamically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_paths: Vec<InputPathSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_terms: Vec<String>,
    #[serde(default)]
    pub load_priority: EntryLoadPriority,
    /// When omitted, hosts should derive the default from behavior
    /// (`read_only => true`, everything else => false) for backwards
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency_safe: Option<bool>,
    #[serde(default)]
    pub requires_user_interaction: bool,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub plan_mode_policy: PlanModePolicy,
    #[serde(default)]
    pub streaming: EntryStreamingMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_capabilities: Vec<HostCapability>,
}

fn default_entry_behavior() -> EntryBehavior {
    EntryBehavior::ReadOnly
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryBehavior {
    ReadOnly,
    WriteSandboxed,
    WriteUnsandboxed,
    Task,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryLoadPriority {
    Always,
    #[default]
    Standard,
    Deferred,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanModePolicy {
    #[default]
    Derived,
    Allowed,
    Blocked,
    ConditionalShellReadOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryStreamingMode {
    #[default]
    Buffered,
    Streaming,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostCapability {
    AskUser,
    SpawnSubtask,
    ListTools,
    MonitorRegistry,
    ReadConfig,
    InvokeTool,
    PublishEvent,
    SubscribeEvents,
    Scheduler,
    PlanRegistry,
    WorktreeRegistry,
    SkillsManager,
    LspRegistry,
    EntryRegistry,
    PluginStorage,
    PluginSecrets,
    PluginStatus,
    CronScheduler,
}

/// Single declarative path extraction rule. `jsonpath` is a subset:
/// dot-paths (`$.path`, `$.files[*].path`). The host extracts each match
/// from the entry input JSON, classifies it under [`PathKind`], and runs it
/// through the permission auditor before the tool body executes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputPathSpec {
    pub jsonpath: String,
    pub kind: PathKind,
    /// If true, missing matches are silently ignored instead of erroring.
    #[serde(default)]
    pub optional: bool,
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
        const SESSION_COMPACTING        = 1 << 16;
        // new hooks
        const SESSION_START             = 1 << 18;
        const SESSION_END               = 1 << 19;
        const SESSION_COMPACTED         = 1 << 20;
        const USER_PROMPT_SUBMIT        = 1 << 21;
        const TOOL_FAILURE              = 1 << 22;
        const AGENT_STOP                = 1 << 23;
        const TOOL_DEFINITION           = 1 << 24;
        const COMMAND_AFTER             = 1 << 25;
        const CHAT_MESSAGES_TRANSFORM   = 1 << 26;
        const PRE_TURN                  = 1 << 27;
        const POST_TURN                 = 1 << 28;
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
    ("permission.ask", HookSubscription::PERMISSION_ASK),
    ("command.execute.before", HookSubscription::COMMAND_BEFORE),
    ("command.execute.after", HookSubscription::COMMAND_AFTER),
    ("shell.env", HookSubscription::SHELL_ENV),
    ("config", HookSubscription::CONFIG),
    ("session.start", HookSubscription::SESSION_START),
    ("session.end", HookSubscription::SESSION_END),
    ("session.compacting", HookSubscription::SESSION_COMPACTING),
    ("session.compacted", HookSubscription::SESSION_COMPACTED),
    ("user.prompt.submit", HookSubscription::USER_PROMPT_SUBMIT),
    ("agent.stop", HookSubscription::AGENT_STOP),
    ("pre_turn", HookSubscription::PRE_TURN),
    ("post_turn", HookSubscription::POST_TURN),
];

fn hook_subscription_for_name(name: &str) -> Option<HookSubscription> {
    HOOK_NAMES
        .iter()
        .find_map(|(hook_name, flag)| (*hook_name == name).then_some(*flag))
        .or(match name {
            "permission_request" => Some(HookSubscription::PERMISSION_ASK),
            "pre_compaction" => Some(HookSubscription::SESSION_COMPACTING),
            "post_compaction" => Some(HookSubscription::SESSION_COMPACTED),
            _ => None,
        })
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
                entries: Vec::new(),
                options_schema: None,
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

    pub fn entry(mut self, entry: PluginEntryDecl) -> Self {
        self.inner.entries.push(entry);
        self
    }

    pub fn entries(mut self, entries: impl IntoIterator<Item = PluginEntryDecl>) -> Self {
        self.inner.entries.extend(entries);
        self
    }

    pub fn options_schema(mut self, schema: serde_json::Value) -> Self {
        self.inner.options_schema = Some(schema);
        self
    }

    pub fn build(self) -> PluginManifest {
        self.inner
    }
}

impl PluginEntryDecl {
    pub fn new(name: impl Into<String>, schema: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            description: None,
            behavior: EntryBehavior::ReadOnly,
            input_schema: schema,
            expose_as: None,
            input_paths: Vec::new(),
            search_terms: Vec::new(),
            load_priority: EntryLoadPriority::Standard,
            concurrency_safe: None,
            requires_user_interaction: false,
            strict: false,
            plan_mode_policy: PlanModePolicy::Derived,
            streaming: EntryStreamingMode::Buffered,
            host_capabilities: Vec::new(),
        }
    }

    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }

    pub fn behavior(mut self, b: EntryBehavior) -> Self {
        self.behavior = b;
        self
    }

    pub fn expose_as(mut self, name: impl Into<String>) -> Self {
        self.expose_as = Some(name.into());
        self
    }

    pub fn input_path(mut self, spec: InputPathSpec) -> Self {
        self.input_paths.push(spec);
        self
    }

    pub fn search_terms<I, S>(mut self, search_terms: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.search_terms = search_terms
            .into_iter()
            .map(Into::into)
            .filter(|term| !term.trim().is_empty())
            .collect();
        self
    }

    pub fn load_priority(mut self, load_priority: EntryLoadPriority) -> Self {
        self.load_priority = load_priority;
        self
    }

    pub fn always_load(self) -> Self {
        self.load_priority(EntryLoadPriority::Always)
    }

    pub fn deferred_load(self) -> Self {
        self.load_priority(EntryLoadPriority::Deferred)
    }

    pub fn concurrency_safe(mut self, concurrency_safe: bool) -> Self {
        self.concurrency_safe = Some(concurrency_safe);
        self
    }

    pub fn requires_user_interaction(mut self, requires_user_interaction: bool) -> Self {
        self.requires_user_interaction = requires_user_interaction;
        self
    }

    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    pub fn plan_mode_policy(mut self, policy: PlanModePolicy) -> Self {
        self.plan_mode_policy = policy;
        self
    }

    pub fn streaming(mut self, streaming: EntryStreamingMode) -> Self {
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

/// Map an entry name and any plugin-scoped options to the registry-side key.
pub type Metadata = BTreeMap<String, String>;
