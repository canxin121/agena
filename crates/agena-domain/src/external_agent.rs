use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An installed agent that owns the conversation's inference and tool loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalAgent {
    Antigravity,
    ClaudeCode,
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

/// Explicit choices survive new native sessions and server restarts.
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

fn valid_terminal_environment_name(name: &str) -> bool {
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
    pub const fn command(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Antigravity => ("agy", &["acp", "--process-per-session"]),
            Self::ClaudeCode => ("claude-agent-acp", &[]),
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Antigravity => "antigravity",
            Self::ClaudeCode => "claude-code",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Antigravity => "Antigravity",
            Self::ClaudeCode => "Claude Code",
        }
    }

    /// Identity for kernel runs and transcripts. These routes are not model
    /// providers and are never registered in the inference catalog.
    pub fn run_identity(self) -> crate::ModelRef {
        crate::ModelRef::new("external-agent", self.id())
    }
}

impl std::str::FromStr for ExternalAgent {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "antigravity" => Ok(Self::Antigravity),
            "claude-code" => Ok(Self::ClaudeCode),
            _ => Err("expected antigravity or claude-code"),
        }
    }
}
