//! Serializable plugin package and configuration values shared by hosts and
//! authenticated inspectors. Loading, signature checks, and lifecycle stay in the host.
use crate::PluginKey;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use url::Url;

/// ed25519 signature over a plugin artifact. The public key is looked up by
/// `key_id` in `plugins.host.trusted_keys`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Signature material of a plugin package.
pub struct PluginSignature {
    pub key_id: String,
    /// Hex-encoded raw signature bytes (64 bytes for ed25519).
    pub signature: String,
}

/// One configured plugin under `plugins.list.<id>`. The host knows how to
/// load the `package`; `settings` is plugin-owned JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// Configured plugin entry.
pub struct ConfiguredPlugin {
    #[serde(default = "default_plugin_enabled")]
    pub enabled: bool,
    pub package: PluginPackage,
    #[serde(default)]
    pub settings: serde_json::Value,
    #[serde(default)]
    pub timeouts: TimeoutsConfig,
    /// Host-owned activation dependencies. `requires` is a hard dependency:
    /// the plugin stays inactive when a required plugin is missing, disabled,
    /// cyclic, or failed to initialize. `after` is only a deterministic load
    /// ordering hint and never blocks activation.
    #[serde(default, skip_serializing_if = "PluginActivationConfig::is_empty")]
    pub activation: PluginActivationConfig,
}

impl Default for ConfiguredPlugin {
    fn default() -> Self {
        Self {
            enabled: true,
            package: PluginPackage::Static {},
            settings: serde_json::Value::Null,
            timeouts: TimeoutsConfig::default(),
            activation: PluginActivationConfig::default(),
        }
    }
}

fn default_plugin_enabled() -> bool {
    true
}

/// Generic plugin package/transport descriptor. Built-in plugins use the same
/// `Static` package kind as any other in-process plugin available to the host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
/// How a plugin is packaged and loaded.
pub enum PluginPackage {
    Static {},
    Cdylib {
        path: PathBuf,
        /// Optional sha256 hex digest of the cdylib bytes. If set, the
        /// host computes the digest at load time and refuses to load on
        /// mismatch. Requires the `signing` cargo feature.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
        /// Optional ed25519 signature in hex over the cdylib bytes; the
        /// public key is looked up in `plugins.host.trusted_keys`. Requires
        /// the `signing` cargo feature.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<PluginSignature>,
    },
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        cwd: Option<PathBuf>,
        #[serde(default)]
        restart: RestartPolicy,
        /// Optional sha256 of the executable resolved using the child's PATH
        /// and cwd. Its absolute path is retained and checked before every
        /// spawn, including supervised restarts. Requires `signing`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    Http {
        url: Url,
        #[serde(default)]
        auth: HttpAuth,
    },
    Wasm {
        path: PathBuf,
        /// Optional sha256 of the wasm bytes for supply-chain verification.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
}

impl ConfiguredPlugin {
    pub fn static_settings(settings: serde_json::Value) -> Self {
        Self {
            enabled: true,
            package: PluginPackage::Static {},
            settings,
            timeouts: TimeoutsConfig::default(),
            activation: PluginActivationConfig::default(),
        }
    }

    pub fn static_default() -> Self {
        Self::static_settings(serde_json::Value::Null)
    }

    pub fn settings(&self) -> &serde_json::Value {
        &self.settings
    }

    pub fn timeouts(&self) -> &TimeoutsConfig {
        &self.timeouts
    }

    pub fn kind_str(&self) -> &'static str {
        match &self.package {
            PluginPackage::Static { .. } => "static",
            PluginPackage::Cdylib { .. } => "cdylib",
            PluginPackage::Stdio { .. } => "stdio",
            PluginPackage::Http { .. } => "http",
            PluginPackage::Wasm { .. } => "wasm",
        }
    }

    pub fn disabled(&self) -> bool {
        !self.enabled
    }
}

/// Dependency-driven activation of one configured plugin instance.
///
/// Dependencies are deliberately expressed between configured plugin ids in
/// this first host-level primitive. A later service registry can resolve a
/// service requirement to a provider id before feeding the same activation
/// planner; keeping the planner id-based prevents service lookup and process
/// transport concerns from becoming entangled.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginActivationConfig {
    /// Plugins that must be active before this plugin may initialize.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<PluginKey>,
    /// Existing plugins that should initialize first when possible. Missing,
    /// disabled, failed, or cyclic hints are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<PluginKey>,
}

impl PluginActivationConfig {
    pub fn is_empty(&self) -> bool {
        self.requires.is_empty() && self.after.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Timeouts applied to plugin operations.
pub struct TimeoutsConfig {
    /// `meta/init` timeout (default 10s).
    pub init: Option<DurationSpec>,
    /// `tool.execute.before/after` timeout (default 30s).
    pub tool_hook: Option<DurationSpec>,
    /// `tool.invoke` timeout (default 5min).
    pub tool_invoke: Option<DurationSpec>,
    /// `chat.*` timeout (default 5s).
    pub chat: Option<DurationSpec>,
    /// `shell.env` / `command.execute.before` / `config` (default 2s).
    pub fast: Option<DurationSpec>,
}

impl TimeoutsConfig {
    pub fn merged(&self, parent: &TimeoutsConfig) -> TimeoutsConfig {
        TimeoutsConfig {
            init: self.init.clone().or_else(|| parent.init.clone()),
            tool_hook: self.tool_hook.clone().or_else(|| parent.tool_hook.clone()),
            tool_invoke: self
                .tool_invoke
                .clone()
                .or_else(|| parent.tool_invoke.clone()),
            chat: self.chat.clone().or_else(|| parent.chat.clone()),
            fast: self.fast.clone().or_else(|| parent.fast.clone()),
        }
    }

    pub fn init_or(&self, default: Duration) -> Duration {
        self.init.as_ref().map(|d| d.0).unwrap_or(default)
    }

    pub fn tool_hook_or(&self, default: Duration) -> Duration {
        self.tool_hook.as_ref().map(|d| d.0).unwrap_or(default)
    }

    pub fn tool_invoke_or(&self, default: Duration) -> Duration {
        self.tool_invoke.as_ref().map(|d| d.0).unwrap_or(default)
    }

    pub fn chat_or(&self, default: Duration) -> Duration {
        self.chat.as_ref().map(|d| d.0).unwrap_or(default)
    }

    pub fn fast_or(&self, default: Duration) -> Duration {
        self.fast.as_ref().map(|d| d.0).unwrap_or(default)
    }
}

/// Parses strings like `"5s"`, `"30s"`, `"2m"` for human-readable timeouts.
#[derive(Debug, Clone, PartialEq)]
/// A duration specification.
pub struct DurationSpec(pub Duration);

impl Serialize for DurationSpec {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        let secs = self.0.as_secs();
        ser.serialize_str(&format!("{secs}s"))
    }
}

impl<'de> Deserialize<'de> for DurationSpec {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(serde::de::Error::custom("empty duration"));
        }
        let (num_part, unit) = match trimmed
            .chars()
            .rev()
            .take_while(|c| c.is_alphabetic())
            .count()
        {
            0 => (trimmed, "s"),
            n => trimmed.split_at(trimmed.len() - n),
        };
        let value: u64 = num_part
            .parse()
            .map_err(|e| serde::de::Error::custom(format!("invalid duration number: {e}")))?;
        let dur = match unit {
            "ms" => Duration::from_millis(value),
            "s" | "" => Duration::from_secs(value),
            "m" => Duration::from_secs(value * 60),
            "h" => Duration::from_secs(value * 3600),
            other => return Err(serde::de::Error::custom(format!("unknown unit `{other}`"))),
        };
        Ok(DurationSpec(dur))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Restart policy of a plugin transport.
pub struct RestartPolicy {
    #[serde(default = "default_restart_policy")]
    pub policy: RestartMode,
    #[serde(default = "default_min_backoff")]
    pub min_backoff: DurationSpec,
    #[serde(default = "default_max_backoff")]
    pub max_backoff: DurationSpec,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            policy: default_restart_policy(),
            min_backoff: default_min_backoff(),
            max_backoff: default_max_backoff(),
            max_retries: default_max_retries(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
/// Mode of plugin restart.
pub enum RestartMode {
    /// Never restart automatically, including after protocol failures.
    Never,
    /// Restart after a nonzero/signal exit or a broken protocol/pipe. A clean
    /// exit with an intact protocol leaves the plugin stopped.
    OnFailure,
    /// Restart after any exit, subject to the retry budget and safe cleanup.
    Always,
}

fn default_restart_policy() -> RestartMode {
    RestartMode::OnFailure
}

fn default_min_backoff() -> DurationSpec {
    DurationSpec(Duration::from_secs(1))
}

fn default_max_backoff() -> DurationSpec {
    DurationSpec(Duration::from_secs(30))
}

fn default_max_retries() -> u32 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[derive(Default)]
/// HTTP authentication of a plugin endpoint.
pub enum HttpAuth {
    #[default]
    None,
    Bearer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_env: Option<String>,
    },
    Basic {
        username: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password_env: Option<String>,
    },
}
