//! Host configuration for loading and running plugins.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::sdk::PluginKey;

pub use crate::quota::QuotaConfig;

/// Top-level `plugins` config object, parsed from agena's JSON config layer.
///
/// The host only owns transport and lifecycle fields. Plugin-specific
/// settings live in [`ConfiguredPlugin::settings`] as JSON and are validated
/// against the plugin manifest at load time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
/// Top-level plugin configuration.
pub struct PluginsConfig {
    #[serde(default, skip_serializing_if = "PluginHostConfig::is_default")]
    pub host: PluginHostConfig,
    #[serde(default, skip_serializing_if = "PluginPolicyConfig::is_default")]
    pub policy: PluginPolicyConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub list: BTreeMap<String, ConfiguredPlugin>,
    /// Named deployment layers. Runtime resolves these after bundled plugin
    /// defaults are injected, then every downstream consumer reads only `list`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, crate::profiles::PluginProfile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_profiles: Vec<String>,
    /// Runtime-only provenance of the already-resolved profile layers.
    #[serde(skip)]
    pub profile_resolution: crate::profiles::PluginProfileResolutionMeta,
}

impl PluginsConfig {
    /// Resolve active profiles over the current list exactly once and collapse
    /// the runtime value back to the single `plugins.list` leaf.
    pub fn resolve_profiles_in_place(&mut self) -> Result<(), String> {
        if self.profile_resolution.resolved {
            return Ok(());
        }
        let resolution = crate::profiles::resolve_plugin_profiles(
            &self.list,
            &self.profiles,
            &self.active_profiles,
        )?;
        self.list = resolution.list;
        self.profile_resolution = resolution.meta;
        self.profiles.clear();
        self.active_profiles.clear();
        Ok(())
    }

    pub fn resolved_profile_view(
        &self,
    ) -> Result<crate::profiles::PluginProfileResolution, String> {
        if self.profile_resolution.resolved {
            return Ok(crate::profiles::PluginProfileResolution {
                list: self.list.clone(),
                meta: self.profile_resolution.clone(),
            });
        }
        crate::profiles::resolve_plugin_profiles(&self.list, &self.profiles, &self.active_profiles)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
/// Policy configuration for plugins (currently reserved; legacy keys are
/// tolerated and ignored so stale configs keep loading).
pub struct PluginPolicyConfig {}

impl PluginPolicyConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
/// Configuration of the plugin host.
pub struct PluginHostConfig {
    pub timeouts: TimeoutsConfig,
    /// Global default quota applied to plugins without their own quota.
    /// Defaults to unlimited.
    #[serde(default, skip_serializing_if = "QuotaConfig::is_unlimited_ref")]
    pub default_quota: QuotaConfig,
    /// Optional per-plugin overrides, keyed by plugin id. A plugin without an
    /// override here uses `default_quota`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quotas: BTreeMap<PluginKey, QuotaConfig>,
    /// `key_id -> hex-encoded ed25519 public key`. Signed package/artifact
    /// entries reference one of these keys.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub trusted_keys: BTreeMap<String, String>,
}

impl PluginHostConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

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
        /// Optional sha256 of the binary at `command`. Requires the
        /// `signing` cargo feature.
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

#[cfg(test)]
mod tests {
    use super::{ConfiguredPlugin, PluginActivationConfig};

    #[test]
    fn configured_plugin_activation_round_trips_and_defaults_out_of_json() {
        let configured: ConfiguredPlugin = serde_json::from_value(serde_json::json!({
            "package": { "kind": "static" },
            "activation": {
                "requires": ["example.provider"],
                "after": ["example.observer"]
            }
        }))
        .expect("decode configured plugin activation");

        assert_eq!(
            configured
                .activation
                .requires
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["example.provider"]
        );
        assert_eq!(
            configured
                .activation
                .after
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["example.observer"]
        );
        let encoded = serde_json::to_value(&configured).expect("encode configured plugin");
        assert_eq!(encoded["activation"]["requires"][0], "example.provider");

        let default = serde_json::to_value(ConfiguredPlugin::default())
            .expect("encode default configured plugin");
        assert!(default.get("activation").is_none());
        assert!(PluginActivationConfig::default().is_empty());
    }

    #[test]
    fn configured_plugin_serializes_settings_and_rejects_legacy_config_key() {
        let configured = ConfiguredPlugin::static_settings(serde_json::json!({"mode": "safe"}));
        let encoded = serde_json::to_value(&configured).expect("encode configured plugin settings");
        assert_eq!(encoded["settings"]["mode"], "safe");
        assert!(encoded.get("config").is_none());

        let error = serde_json::from_value::<ConfiguredPlugin>(serde_json::json!({
            "package": { "kind": "static" },
            "config": { "mode": "legacy" }
        }))
        .expect_err("legacy plugin config key must be rejected");
        assert!(error.to_string().contains("unknown field `config`"));
    }

    #[test]
    fn activation_contract_rejects_unknown_fields() {
        let error = serde_json::from_value::<ConfiguredPlugin>(serde_json::json!({
            "package": { "kind": "static" },
            "activation": { "requires": [], "optional": ["example.plugin"] }
        }))
        .expect_err("unknown activation field must be rejected");
        assert!(error.to_string().contains("unknown field"));
    }
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
