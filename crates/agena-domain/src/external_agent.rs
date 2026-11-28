use std::{collections::BTreeMap, fmt, path::PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An installed agent that owns the conversation's inference and tool loop.
/// Built-in IDs are `antigravity` and `claude-code`; custom ACP agents use any
/// validated lowercase ASCII slug configured under `acp.agents`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
pub enum ExternalAgent {
    Antigravity,
    ClaudeCode,
    Custom(String),
}

/// Configuration for a single built-in override or custom ACP agent under `acp.agents.<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ExternalAgentConfig {
    /// Display name shown in Web/TUI/CLI (defaults to the built-in label or agent ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Executable program name (resolved via `PATH`) or absolute path.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// Fixed CLI arguments passed when launching the ACP stdio server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Additional environment variables set on the launched ACP process.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Whether to advertise ACP v1 client `fs` and `terminal` delegation capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_tools: Option<bool>,
    /// Extra writable native state/config directories when `AGENA_ACP_SANDBOX_FILE` is active.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable_paths: Vec<PathBuf>,
    /// Hide and disable this agent (works for both built-in and custom entries).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

/// Instance-owned ACP configuration (`acp` section in global `agena.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AcpConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<ExternalAgent, ExternalAgentConfig>,
}

/// Fully resolved launch and display specification for an enabled ACP agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedExternalAgent {
    pub id: ExternalAgent,
    pub name: String,
    pub builtin: bool,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    pub client_tools: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable_paths: Vec<PathBuf>,
}

impl ResolvedExternalAgent {
    /// Safe public catalog entry returned over `/api/v1/acp/agents` (omits `env`).
    pub fn catalog_entry(&self) -> ExternalAgentCatalogEntry {
        ExternalAgentCatalogEntry {
            id: self.id.clone(),
            name: self.name.clone(),
            builtin: self.builtin,
            command: self.command.clone(),
            args: self.args.clone(),
            available: None,
        }
    }
}

/// Public ACP agent catalog entry returned to Web, TUI, and CLI clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExternalAgentCatalogEntry {
    pub id: ExternalAgent,
    pub name: String,
    pub builtin: bool,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Whether the configured executable currently resolves on the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available: Option<bool>,
}

impl AcpConfig {
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.agents.len() > 64 {
            return Err("acp.agents may define at most 64 entries".into());
        }
        for (agent, entry) in &self.agents {
            let prefix = format!("acp.agents.{}", agent.id());
            if let Some(name) = &entry.name {
                let trimmed = name.trim();
                if trimmed.is_empty() || trimmed.len() > 128 || trimmed.chars().any(char::is_control)
                {
                    return Err(format!(
                        "{prefix}.name must be a non-empty string up to 128 bytes without control characters"
                    ));
                }
            }
            let command = entry.command.trim();
            if !entry.disabled && !agent.is_builtin() && command.is_empty() {
                return Err(format!("{prefix}.command must be non-empty"));
            }
            if !entry.command.is_empty()
                && (command.is_empty()
                    || command.len() > 4096
                    || entry.command.chars().any(char::is_control))
            {
                return Err(format!(
                    "{prefix}.command must be up to 4096 bytes without control characters"
                ));
            }
            if entry.args.len() > 128
                || entry
                    .args
                    .iter()
                    .any(|arg| arg.len() > 4096 || arg.chars().any(char::is_control))
            {
                return Err(format!(
                    "{prefix}.args may contain at most 128 arguments without control characters"
                ));
            }
            if entry.env.len() > 64 {
                return Err(format!(
                    "{prefix}.env may contain at most 64 environment variables"
                ));
            }
            for (key, value) in &entry.env {
                if !valid_configured_env_name(key) {
                    return Err(format!(
                        "{prefix}.env.{key} must be a valid non-AGENA_ environment variable name"
                    ));
                }
                if value.len() > 16_384 || value.contains('\0') {
                    return Err(format!(
                        "{prefix}.env.{key} must be up to 16384 bytes without NUL characters"
                    ));
                }
            }
            if entry.writable_paths.len() > 16
                || entry.writable_paths.iter().any(|path| !path.is_absolute())
            {
                return Err(format!(
                    "{prefix}.writable_paths may contain at most 16 absolute paths"
                ));
            }
        }
        Ok(())
    }

    pub fn resolve(&self, agent: &ExternalAgent) -> Option<ResolvedExternalAgent> {
        let entry = self.agents.get(agent);
        if entry.is_some_and(|entry| entry.disabled) {
            return None;
        }
        if let Some((default_command, default_args)) = agent.builtin_command() {
            let override_command = entry
                .map(|entry| entry.command.trim())
                .filter(|command| !command.is_empty());
            let command = override_command.unwrap_or(default_command).to_owned();
            let args = match entry {
                Some(entry) if !entry.args.is_empty() || override_command.is_some() => {
                    entry.args.clone()
                }
                _ => default_args.iter().map(|arg| (*arg).to_owned()).collect(),
            };
            let name = entry
                .and_then(|entry| entry.name.as_deref())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| agent.label())
                .to_owned();
            let client_tools = entry
                .and_then(|entry| entry.client_tools)
                .unwrap_or(!matches!(agent, ExternalAgent::Antigravity));
            return Some(ResolvedExternalAgent {
                id: agent.clone(),
                name,
                builtin: true,
                command,
                args,
                env: entry.map(|entry| entry.env.clone()).unwrap_or_default(),
                client_tools,
                writable_paths: entry
                    .map(|entry| entry.writable_paths.clone())
                    .unwrap_or_default(),
            });
        }
        let entry = entry?;
        let command = entry.command.trim();
        if command.is_empty() {
            return None;
        }
        let name = entry
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| agent.id())
            .to_owned();
        Some(ResolvedExternalAgent {
            id: agent.clone(),
            name,
            builtin: false,
            command: command.to_owned(),
            args: entry.args.clone(),
            env: entry.env.clone(),
            client_tools: entry.client_tools.unwrap_or(true),
            writable_paths: entry.writable_paths.clone(),
        })
    }

    pub fn resolve_catalog(&self) -> Vec<ResolvedExternalAgent> {
        let mut catalog = Vec::new();
        for builtin in ExternalAgent::builtins() {
            if let Some(resolved) = self.resolve(&builtin) {
                catalog.push(resolved);
            }
        }
        for agent in self.agents.keys() {
            if agent.is_builtin() {
                continue;
            }
            if let Some(resolved) = self.resolve(agent) {
                catalog.push(resolved);
            }
        }
        catalog
    }
}

fn valid_configured_env_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some(&first) = bytes.first() else {
        return false;
    };
    name.len() <= 128
        && (first.is_ascii_alphabetic() || first == b'_')
        && bytes.iter().all(|ch| ch.is_ascii_alphanumeric() || *ch == b'_')
        && !name.to_ascii_uppercase().starts_with("AGENA_")
}

pub fn valid_external_agent_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let Some(&first) = bytes.first() else {
        return false;
    };
    bytes.len() <= 64
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes
            .iter()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(*ch, b'-' | b'_'))
}

/// The routing catalog advertised by the installed ACP agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentProviders {
    pub supported: bool,
    pub providers: Vec<ExternalAgentProvider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentProvider {
    pub provider_id: String,
    pub supported: Vec<String>,
    pub required: bool,
    pub current: Option<ExternalAgentProviderTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentProviderTarget {
    pub api_type: String,
    pub base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ExternalAgentOptionKind {
    Model,
    ThoughtLevel,
    Mode,
    Config(String),
}

impl ExternalAgentOptionKind {
    pub fn label(&self) -> &str {
        match self {
            Self::Model => "model",
            Self::ThoughtLevel => "thinking level",
            Self::Mode => "mode",
            Self::Config(id) => id,
        }
    }
}

impl From<ExternalAgentOptionKind> for String {
    fn from(kind: ExternalAgentOptionKind) -> Self {
        match kind {
            ExternalAgentOptionKind::Model => "model".into(),
            ExternalAgentOptionKind::ThoughtLevel => "thought_level".into(),
            ExternalAgentOptionKind::Mode => "mode".into(),
            ExternalAgentOptionKind::Config(id) => format!("config:{id}"),
        }
    }
}

impl TryFrom<String> for ExternalAgentOptionKind {
    type Error = &'static str;

    fn try_from(kind: String) -> Result<Self, Self::Error> {
        match kind.as_str() {
            "model" => Ok(Self::Model),
            "thought_level" => Ok(Self::ThoughtLevel),
            "mode" => Ok(Self::Mode),
            other => other
                .strip_prefix("config:")
                .filter(|id| !id.trim().is_empty() && id.len() <= 4096)
                .filter(|id| !id.chars().any(char::is_control))
                .map(|id| Self::Config(id.to_owned()))
                .ok_or("invalid external agent configuration key"),
        }
    }
}

/// Explicit choices apply when the Agena session opens a native conversation.
pub type ExternalAgentSelection = BTreeMap<ExternalAgentOptionKind, String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAgentOptionChange {
    pub kind: ExternalAgentOptionKind,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentOptionValue {
    pub value: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentOptionType {
    #[default]
    Select,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentOption {
    pub id: String,
    pub kind: ExternalAgentOptionKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, rename = "type")]
    pub value_type: ExternalAgentOptionType,
    #[serde(default)]
    pub legacy_mode: bool,
    pub current_value: String,
    pub choices: Vec<ExternalAgentOptionValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentOptions {
    pub selection: ExternalAgentSelection,
    pub options: Vec<ExternalAgentOption>,
    /// Advisory commands for a view that has yet to open a native session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preview_commands: Vec<ExternalAgentCommand>,
}

/// Sign-in is performed by the installed agent using its own credential store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalAgentAuthAction {
    Inspect {
        #[serde(default)]
        terminal: bool,
    },
    Login {
        method_id: String,
    },
    Logout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAgentAuth {
    pub agent: ExternalAgent,
    pub cwd: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_args: Vec<String>,
    pub methods: Vec<ExternalAgentAuthMethod>,
    pub logout_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<ExternalAgentAuthAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAgentAuthMethod {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub flow: ExternalAgentAuthFlow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExternalAgentAuthFlow {
    Agent,
    Terminal {
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
}

/// Validate an ACP terminal login descriptor at both receipt and process launch.
pub fn validate_external_agent_terminal_launch(
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<(), &'static str> {
    let valid_value = |value: &str| value.len() <= 4096 && !value.chars().any(char::is_control);
    if args.len() > 64 || args.iter().any(|arg| !valid_value(arg)) {
        return Err("Invalid external agent terminal login arguments.");
    }
    if env.len() > 64
        || env
            .iter()
            .any(|(name, value)| !valid_terminal_environment_name(name) || !valid_value(value))
    {
        return Err("Invalid external agent terminal login environment.");
    }
    Ok(())
}

pub fn valid_terminal_environment_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
    {
        return false;
    }
    let name = name.to_ascii_uppercase();
    // Login descriptors can select credentials but cannot override process
    // initialization, executable lookup, trust roots, or proxy routing.
    !matches!(
        name.as_str(),
        "PATH"
            | "PATHEXT"
            | "COMSPEC"
            | "BASH_ENV"
            | "ENV"
            | "SHELL"
            | "HOME"
            | "USERPROFILE"
            | "APPDATA"
            | "LOCALAPPDATA"
            | "RUBYOPT"
            | "PERL5OPT"
            | "SSL_CERT_FILE"
            | "SSL_CERT_DIR"
            | "CURL_CA_BUNDLE"
            | "REQUESTS_CA_BUNDLE"
    ) && ![
        "LD_",
        "DYLD_",
        "NODE_",
        "PYTHON",
        "XDG_",
        "AGENA_",
        "CLAUDE_CONFIG_DIR",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
        && !name.ends_with("_PROXY")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentAuthStatus {
    pub state: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<ExternalAgentAuthAccount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentAuthAccount {
    pub label: String,
    #[serde(default)]
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
}

pub const EXTERNAL_AGENT_URL_INPUT_KIND: &str = "external_agent_url";
pub const EXTERNAL_AGENT_URL_OPEN_OPTION: &str = "Open authorization page";

/// Validate before presenting a native agent's web authorization action
/// or handing it to a client-side browser.
pub fn external_agent_authorization_url(address: &str) -> Option<url::Url> {
    if address.len() > 16384 || address.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(address).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let local_http = url.scheme() == "http"
        && match url.host()? {
            url::Host::Domain(host) => host == "localhost",
            url::Host::Ipv4(host) => host.is_loopback(),
            url::Host::Ipv6(host) => host.is_loopback(),
        };
    (url.scheme() == "https" || local_http).then_some(url)
}

/// A native slash command or skill advertised by an ACP session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentCommand {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skill: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skill_inline: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_path: Option<String>,
}

pub const EXTERNAL_AGENT_COMMAND_PREFIX: &str = "agent:";
pub const ACP_INLINE_ATTACHMENT_PREFIX: &str = "data:application/octet-stream;base64,";

/// Process-local ACP view IDs occupy the upper half of the safe JS integer
/// range. This lets a session-scoped client address a view without confusing it
/// with a durable Agena session or a native agent's string ID.
pub const ACP_SESSION_ID_START: i64 = 1 << 52;

pub fn is_acp_session_id(id: i64) -> bool {
    (ACP_SESSION_ID_START..(ACP_SESSION_ID_START << 1)).contains(&id)
}

impl ExternalAgentCommand {
    pub fn accepts_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 256
            && !name.starts_with('/')
            && !name.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    }

    /// The bare command token used by Agena's input and completion menus.
    pub fn qualified_name(&self) -> String {
        format!("{EXTERNAL_AGENT_COMMAND_PREFIX}{}", self.name)
    }
}

/// Remove Agena's routing prefix and preserve the native name and arguments.
pub fn external_agent_command_input(text: &str) -> Option<&str> {
    let body = text.trim_start().strip_prefix('/')?.trim_start();
    let prefix = body.get(..EXTERNAL_AGENT_COMMAND_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(EXTERNAL_AGENT_COMMAND_PREFIX) {
        return None;
    }
    let input = &body[EXTERNAL_AGENT_COMMAND_PREFIX.len()..];
    let name = input.split(char::is_whitespace).next()?;
    ExternalAgentCommand::accepts_name(name).then_some(input)
}

impl ExternalAgent {
    pub fn builtins() -> [Self; 2] {
        [Self::ClaudeCode, Self::Antigravity]
    }

    pub fn is_builtin(&self) -> bool {
        matches!(self, Self::Antigravity | Self::ClaudeCode)
    }

    pub fn builtin_command(&self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Self::Antigravity => Some(("agy", &["acp", "--process-per-session"])),
            Self::ClaudeCode => Some(("claude-agent-acp", &[])),
            Self::Custom(_) => None,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Antigravity => "antigravity",
            Self::ClaudeCode => "claude-code",
            Self::Custom(id) => id.as_str(),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Antigravity => "Antigravity",
            Self::ClaudeCode => "Claude Code",
            Self::Custom(id) => id.as_str(),
        }
    }

    /// Identity for kernel runs and transcripts. These routes are not model
    /// providers and are never registered in the inference catalog.
    pub fn run_identity(&self) -> crate::ModelRef {
        crate::ModelRef::new("external-agent", self.id())
    }
}

impl fmt::Display for ExternalAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl From<ExternalAgent> for String {
    fn from(agent: ExternalAgent) -> Self {
        agent.id().to_owned()
    }
}

impl TryFrom<String> for ExternalAgent {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "antigravity" => Ok(Self::Antigravity),
            "claude-code" => Ok(Self::ClaudeCode),
            id if valid_external_agent_id(id) => Ok(Self::Custom(value)),
            _ => Err(
                "external agent ID must be 1..=64 lowercase ASCII letters, digits, '-' or '_'",
            ),
        }
    }
}

impl std::str::FromStr for ExternalAgent {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_session_id_range_matches_upper_safe_js_integer_half() {
        assert!(!is_acp_session_id(0));
        assert!(!is_acp_session_id(ACP_SESSION_ID_START - 1));
        assert!(is_acp_session_id(ACP_SESSION_ID_START));
        assert!(is_acp_session_id((ACP_SESSION_ID_START << 1) - 1));
        assert!(!is_acp_session_id(ACP_SESSION_ID_START << 1));
    }

    #[test]
    fn option_kind_round_trips_and_rejects_empty_config_keys() {
        assert_eq!(
            ExternalAgentOptionKind::try_from("model".to_owned()),
            Ok(ExternalAgentOptionKind::Model)
        );
        assert_eq!(
            ExternalAgentOptionKind::try_from("thought_level".to_owned()),
            Ok(ExternalAgentOptionKind::ThoughtLevel)
        );
        assert_eq!(
            ExternalAgentOptionKind::try_from("mode".to_owned()),
            Ok(ExternalAgentOptionKind::Mode)
        );
        assert_eq!(
            ExternalAgentOptionKind::try_from("config:fast".to_owned()),
            Ok(ExternalAgentOptionKind::Config("fast".into()))
        );
        assert!(ExternalAgentOptionKind::try_from("config:".to_owned()).is_err());
        assert!(ExternalAgentOptionKind::try_from("unknown".to_owned()).is_err());
    }

    #[test]
    fn command_input_strips_prefix_and_rejects_escaped_or_control_names() {
        assert_eq!(external_agent_command_input("/agent:compact"), Some("compact"));
        assert_eq!(
            external_agent_command_input("  /AGENT:review  --all "),
            Some("review  --all ")
        );
        assert_eq!(external_agent_command_input("//agent:compact"), None);
        assert_eq!(external_agent_command_input("/agent:/bad"), None);
        assert_eq!(external_agent_command_input("/compact"), None);
        assert_eq!(external_agent_command_input("/agent:bad\x07cmd"), None);
    }

    #[test]
    fn authorization_url_allows_https_and_loopback_http_only() {
        assert!(external_agent_authorization_url("https://auth.example.com/login").is_some());
        assert!(external_agent_authorization_url("http://127.0.0.1:8080/callback").is_some());
        assert!(external_agent_authorization_url("http://localhost:3000/cb").is_some());
        assert!(external_agent_authorization_url("http://[::1]:3000/cb").is_some());
        assert!(external_agent_authorization_url("http://example.com/login").is_none());
        assert!(external_agent_authorization_url("https://user:pass@example.com/login").is_none());
        assert!(external_agent_authorization_url("javascript:alert(1)").is_none());
        assert!(external_agent_authorization_url("https://example.com/\n").is_none());
    }

    #[test]
    fn terminal_launch_validation_blocks_dangerous_environment_overrides() {
        for blocked in [
            "PATH",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "NODE_OPTIONS",
            "PYTHONSTARTUP",
            "BASH_ENV",
            "AGENA_DATA_DIR",
            "CLAUDE_CONFIG_DIR",
            "HTTPS_PROXY",
            "SSL_CERT_FILE",
            "bad-name",
            "",
        ] {
            assert!(
                !valid_terminal_environment_name(blocked),
                "{blocked} should be rejected"
            );
        }
        assert!(valid_terminal_environment_name("CUSTOM_AUTH_PROFILE"));
        let mut env = BTreeMap::from([("CUSTOM_AUTH_PROFILE".into(), "work".into())]);
        assert!(validate_external_agent_terminal_launch(&["--login".into()], &env).is_ok());
        assert!(validate_external_agent_terminal_launch(&["--bad\0arg".into()], &env).is_err());
        env.insert("LD_PRELOAD".into(), "/tmp/evil.so".into());
        assert!(validate_external_agent_terminal_launch(&["--login".into()], &env).is_err());
    }

    #[test]
    fn external_agent_parses_builtins_and_custom_slugs_and_rejects_invalid_ids() {
        assert_eq!(
            serde_json::from_str::<ExternalAgent>("\"antigravity\"").unwrap(),
            ExternalAgent::Antigravity
        );
        assert_eq!(
            serde_json::from_str::<ExternalAgent>("\"claude-code\"").unwrap(),
            ExternalAgent::ClaudeCode
        );
        assert_eq!(
            serde_json::to_string(&ExternalAgent::Antigravity).unwrap(),
            "\"antigravity\""
        );
        assert_eq!(
            serde_json::to_string(&ExternalAgent::ClaudeCode).unwrap(),
            "\"claude-code\""
        );

        for valid in ["codex", "gemini-cli", "my_acp_1", "0agent"] {
            let parsed: ExternalAgent = valid.parse().unwrap();
            assert_eq!(parsed, ExternalAgent::Custom(valid.into()));
            assert_eq!(parsed.id(), valid);
            assert!(!parsed.is_builtin());
            let json = serde_json::to_string(&parsed).unwrap();
            assert_eq!(json, format!("\"{valid}\""));
            assert_eq!(serde_json::from_str::<ExternalAgent>(&json).unwrap(), parsed);
        }

        let too_long = "a".repeat(65);
        for invalid in [
            "",
            "Codex",
            "-leading",
            "_leading",
            "bad name",
            "bad:id",
            "bad/id",
            "bad.id",
            too_long.as_str(),
        ] {
            assert!(
                invalid.parse::<ExternalAgent>().is_err(),
                "{invalid:?} should be rejected"
            );
            assert!(
                serde_json::from_value::<ExternalAgent>(serde_json::json!(invalid)).is_err(),
                "{invalid:?} should fail JSON deserialization"
            );
        }
    }

    #[test]
    fn acp_config_validate_checks_custom_command_env_and_writable_paths() {
        assert!(AcpConfig::default().validate().is_ok());

        let missing_command = AcpConfig {
            agents: BTreeMap::from([(
                ExternalAgent::Custom("codex".into()),
                ExternalAgentConfig::default(),
            )]),
        };
        assert!(missing_command.validate().is_err());

        let disabled_without_command = AcpConfig {
            agents: BTreeMap::from([(
                ExternalAgent::Custom("codex".into()),
                ExternalAgentConfig {
                    disabled: true,
                    ..Default::default()
                },
            )]),
        };
        assert!(disabled_without_command.validate().is_ok());

        let agena_env = AcpConfig {
            agents: BTreeMap::from([(
                ExternalAgent::Custom("codex".into()),
                ExternalAgentConfig {
                    command: "codex-acp".into(),
                    env: BTreeMap::from([("AGENA_TOKEN".into(), "x".into())]),
                    ..Default::default()
                },
            )]),
        };
        assert!(agena_env.validate().is_err());

        let invalid_env_name = AcpConfig {
            agents: BTreeMap::from([(
                ExternalAgent::Custom("codex".into()),
                ExternalAgentConfig {
                    command: "codex-acp".into(),
                    env: BTreeMap::from([("BAD-NAME".into(), "x".into())]),
                    ..Default::default()
                },
            )]),
        };
        assert!(invalid_env_name.validate().is_err());

        let relative_writable = AcpConfig {
            agents: BTreeMap::from([(
                ExternalAgent::Custom("codex".into()),
                ExternalAgentConfig {
                    command: "codex-acp".into(),
                    writable_paths: vec![PathBuf::from("relative/state")],
                    ..Default::default()
                },
            )]),
        };
        assert!(relative_writable.validate().is_err());

        let control_arg = AcpConfig {
            agents: BTreeMap::from([(
                ExternalAgent::Custom("codex".into()),
                ExternalAgentConfig {
                    command: "codex-acp".into(),
                    args: vec!["--bad\narg".into()],
                    ..Default::default()
                },
            )]),
        };
        assert!(control_arg.validate().is_err());

        let valid = AcpConfig {
            agents: BTreeMap::from([
                (
                    ExternalAgent::Antigravity,
                    ExternalAgentConfig {
                        disabled: true,
                        ..Default::default()
                    },
                ),
                (
                    ExternalAgent::Custom("codex".into()),
                    ExternalAgentConfig {
                        name: Some("Codex CLI".into()),
                        command: "codex-acp".into(),
                        args: vec!["--stdio".into()],
                        env: BTreeMap::from([("CODEX_PROFILE".into(), "work".into())]),
                        client_tools: Some(false),
                        writable_paths: vec![PathBuf::from("/tmp/codex-state")],
                        disabled: false,
                    },
                ),
            ]),
        };
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn acp_config_resolve_and_catalog_supports_custom_overrides_and_hides_env() {
        let default_catalog = AcpConfig::default().resolve_catalog();
        assert_eq!(default_catalog.len(), 2);
        assert_eq!(default_catalog[0].id, ExternalAgent::ClaudeCode);
        assert_eq!(default_catalog[0].command, "claude-agent-acp");
        assert!(default_catalog[0].args.is_empty());
        assert!(default_catalog[0].client_tools);
        assert!(default_catalog[0].builtin);

        assert_eq!(default_catalog[1].id, ExternalAgent::Antigravity);
        assert_eq!(default_catalog[1].command, "agy");
        assert_eq!(default_catalog[1].args, vec!["acp", "--process-per-session"]);
        assert!(!default_catalog[1].client_tools);
        assert!(default_catalog[1].builtin);

        let config = AcpConfig {
            agents: BTreeMap::from([
                (
                    ExternalAgent::Antigravity,
                    ExternalAgentConfig {
                        disabled: true,
                        ..Default::default()
                    },
                ),
                (
                    ExternalAgent::ClaudeCode,
                    ExternalAgentConfig {
                        name: Some("Claude Custom".into()),
                        env: BTreeMap::from([("CLAUDE_EXTRA".into(), "secret".into())]),
                        ..Default::default()
                    },
                ),
                (
                    ExternalAgent::Custom("codex".into()),
                    ExternalAgentConfig {
                        name: Some("Codex CLI".into()),
                        command: "/usr/local/bin/codex-acp".into(),
                        args: vec!["--stdio".into()],
                        env: BTreeMap::from([("API_SECRET".into(), "top-secret".into())]),
                        client_tools: None,
                        writable_paths: vec![PathBuf::from("/tmp/codex")],
                        disabled: false,
                    },
                ),
                (
                    ExternalAgent::Custom("unnamed".into()),
                    ExternalAgentConfig {
                        command: "unnamed-acp".into(),
                        client_tools: Some(false),
                        ..Default::default()
                    },
                ),
            ]),
        };
        assert!(config.resolve(&ExternalAgent::Antigravity).is_none());
        assert!(
            config
                .resolve(&ExternalAgent::Custom("missing".into()))
                .is_none()
        );

        let catalog = config.resolve_catalog();
        assert_eq!(
            catalog
                .iter()
                .map(|entry| entry.id.id())
                .collect::<Vec<_>>(),
            vec!["claude-code", "codex", "unnamed"]
        );
        // Built-in override without command keeps built-in default command & args.
        assert_eq!(catalog[0].name, "Claude Custom");
        assert_eq!(catalog[0].command, "claude-agent-acp");
        assert_eq!(
            catalog[0].env.get("CLAUDE_EXTRA").map(String::as_str),
            Some("secret")
        );

        assert_eq!(catalog[1].name, "Codex CLI");
        assert!(!catalog[1].builtin);
        assert!(catalog[1].client_tools);
        assert_eq!(catalog[2].name, "unnamed");
        assert!(!catalog[2].client_tools);

        // Catalog DTO must never serialize env secrets or writable_paths.
        let dto_json = serde_json::to_string(&catalog[1].catalog_entry()).unwrap();
        assert!(!dto_json.contains("API_SECRET"));
        assert!(!dto_json.contains("top-secret"));
        assert!(!dto_json.contains("/tmp/codex"));

        // Overriding a built-in command without explicit args clears default args.
        let override_cmd = AcpConfig {
            agents: BTreeMap::from([(
                ExternalAgent::Antigravity,
                ExternalAgentConfig {
                    command: "/opt/custom-agy".into(),
                    ..Default::default()
                },
            )]),
        };
        let resolved = override_cmd.resolve(&ExternalAgent::Antigravity).unwrap();
        assert_eq!(resolved.command, "/opt/custom-agy");
        assert!(resolved.args.is_empty());
    }
}
