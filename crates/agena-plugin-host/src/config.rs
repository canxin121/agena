use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::sdk::ToolDescriptionMode;

pub use crate::quota::QuotaConfig;

/// Top-level `[plugins]` config block, parsed from agena's config layer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginsConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub timeouts: TimeoutsConfig,
    #[serde(default)]
    pub list: BTreeMap<String, PluginEntry>,
    /// Global default quota applied to plugins without their own
    /// `[plugins.list.<id>.quota]`. Defaults to unlimited.
    #[serde(default, skip_serializing_if = "QuotaConfig::is_unlimited_ref")]
    pub default_quota: QuotaConfig,
    /// Optional per-plugin overrides, keyed by plugin id. A plugin without
    /// an entry here uses `default_quota`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quotas: BTreeMap<String, QuotaConfig>,
    /// `key_id -> hex-encoded ed25519 public key`. Cdylib entries with a
    /// `signature` field reference one of these keys.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub trusted_keys: BTreeMap<String, String>,
    /// Controls how tool descriptions are exposed to the model. The detailed
    /// help remains available through host/tool help APIs.
    #[serde(default, skip_serializing_if = "ToolPresentationConfig::is_default")]
    pub tool_presentation: ToolPresentationConfig,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeouts: TimeoutsConfig::default(),
            list: BTreeMap::new(),
            default_quota: QuotaConfig::default(),
            quotas: BTreeMap::new(),
            trusted_keys: BTreeMap::new(),
            tool_presentation: ToolPresentationConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ToolPresentationConfig {
    pub default_mode: ToolDescriptionMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, ToolDescriptionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, ToolDescriptionMode>,
}

impl Default for ToolPresentationConfig {
    fn default() -> Self {
        Self {
            default_mode: ToolDescriptionMode::Detailed,
            plugins: BTreeMap::new(),
            tools: BTreeMap::new(),
        }
    }
}

impl ToolPresentationConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn mode_for(
        &self,
        plugin_name: &str,
        original_name: &str,
        exposed_name: &str,
        tool_default: Option<ToolDescriptionMode>,
    ) -> ToolDescriptionMode {
        for key in [
            exposed_name.to_string(),
            format!("{plugin_name}/{original_name}"),
            format!("{plugin_name}/{exposed_name}"),
            format!("{plugin_name}.{original_name}"),
            original_name.to_string(),
        ] {
            if let Some(mode) = self.tools.get(key.as_str()).copied() {
                return mode;
            }
        }
        self.plugins
            .get(plugin_name)
            .copied()
            .or(tool_default)
            .unwrap_or(self.default_mode)
    }
}

/// ed25519 signature over a plugin artifact. The public key is looked up by
/// `key_id` in `[plugins.trusted_keys]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginSignature {
    pub key_id: String,
    /// Hex-encoded raw signature bytes (64 bytes for ed25519).
    pub signature: String,
}

fn default_enabled() -> bool {
    true
}

/// One entry under `[plugins.list.<id>]`. The `kind` discriminator selects
/// the transport.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginEntry {
    Static {
        #[serde(default)]
        options: serde_json::Value,
        #[serde(default)]
        timeouts: TimeoutsConfig,
    },
    Cdylib {
        path: PathBuf,
        #[serde(default)]
        options: serde_json::Value,
        #[serde(default)]
        timeouts: TimeoutsConfig,
        /// Optional sha256 hex digest of the cdylib bytes. If set, the
        /// host computes the digest at load time and refuses to load on
        /// mismatch. Requires the `signing` cargo feature.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
        /// Optional ed25519 signature in hex over the cdylib bytes; the
        /// public key is looked up in `[plugins.trusted_keys]`. Requires
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
        #[serde(default)]
        options: serde_json::Value,
        #[serde(default)]
        timeouts: TimeoutsConfig,
        /// Optional sha256 of the binary at `command`. Requires the
        /// `signing` cargo feature.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    Http {
        url: Url,
        #[serde(default)]
        auth: HttpAuth,
        #[serde(default)]
        options: serde_json::Value,
        #[serde(default)]
        timeouts: TimeoutsConfig,
    },
    Wasm {
        path: PathBuf,
        #[serde(default)]
        options: serde_json::Value,
        #[serde(default)]
        timeouts: TimeoutsConfig,
        /// Optional sha256 of the wasm bytes for supply-chain verification.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
}

impl PluginEntry {
    pub fn options(&self) -> &serde_json::Value {
        match self {
            PluginEntry::Static { options, .. }
            | PluginEntry::Cdylib { options, .. }
            | PluginEntry::Stdio { options, .. }
            | PluginEntry::Http { options, .. }
            | PluginEntry::Wasm { options, .. } => options,
        }
    }

    pub fn timeouts(&self) -> &TimeoutsConfig {
        match self {
            PluginEntry::Static { timeouts, .. }
            | PluginEntry::Cdylib { timeouts, .. }
            | PluginEntry::Stdio { timeouts, .. }
            | PluginEntry::Http { timeouts, .. }
            | PluginEntry::Wasm { timeouts, .. } => timeouts,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            PluginEntry::Static { .. } => "static",
            PluginEntry::Cdylib { .. } => "cdylib",
            PluginEntry::Stdio { .. } => "stdio",
            PluginEntry::Http { .. } => "http",
            PluginEntry::Wasm { .. } => "wasm",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TimeoutsConfig {
    /// `meta/init` timeout (default 10s).
    pub init: Option<DurationSpec>,
    /// `tool.execute.before/after` timeout (default 30s).
    pub tool_hook: Option<DurationSpec>,
    /// `tool.invoke` timeout (default 5min).
    pub tool_invoke: Option<DurationSpec>,
    /// `permission.ask` timeout (default 60s).
    pub permission_ask: Option<DurationSpec>,
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
            permission_ask: self
                .permission_ask
                .clone()
                .or_else(|| parent.permission_ask.clone()),
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

    pub fn permission_ask_or(&self, default: Duration) -> Duration {
        self.permission_ask.as_ref().map(|d| d.0).unwrap_or(default)
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
pub enum RestartMode {
    Never,
    OnFailure,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_presentation_mode_precedence_is_tool_plugin_manifest_default_global() {
        let config = ToolPresentationConfig {
            default_mode: ToolDescriptionMode::Help,
            plugins: BTreeMap::from([("plugin.alpha".to_string(), ToolDescriptionMode::Detailed)]),
            tools: BTreeMap::from([("plugin.alpha/echo".to_string(), ToolDescriptionMode::Help)]),
        };

        assert_eq!(
            config.mode_for(
                "plugin.alpha",
                "echo",
                "plugin.alpha/echo",
                Some(ToolDescriptionMode::Detailed)
            ),
            ToolDescriptionMode::Help
        );
        assert_eq!(
            config.mode_for(
                "plugin.alpha",
                "other",
                "other",
                Some(ToolDescriptionMode::Help)
            ),
            ToolDescriptionMode::Detailed
        );
        assert_eq!(
            config.mode_for(
                "plugin.beta",
                "other",
                "other",
                Some(ToolDescriptionMode::Detailed)
            ),
            ToolDescriptionMode::Detailed
        );
        assert_eq!(
            config.mode_for("plugin.beta", "plain", "plain", None),
            ToolDescriptionMode::Help
        );
    }
}
