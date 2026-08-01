use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::sdk::ToolDescriptionMode;
pub use crate::sdk::manifest::UiTextDisplayMode;
use crate::sdk::{PluginKey, ToolKey};

pub use crate::quota::QuotaConfig;

/// Top-level `plugins` config object, parsed from agena's JSON config layer.
///
/// The host only owns transport, presentation policy and lifecycle fields. Plugin-specific
/// configuration lives in [`ConfiguredPlugin::config`] as JSON and is validated
/// against the plugin manifest at load time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginsConfig {
    #[serde(default, skip_serializing_if = "PluginHostConfig::is_default")]
    pub host: PluginHostConfig,
    #[serde(default, skip_serializing_if = "PluginPolicyConfig::is_default")]
    pub policy: PluginPolicyConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub list: BTreeMap<String, ConfiguredPlugin>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginPolicyConfig {
    /// Controls how tool descriptions are shown to the model. Brief mode
    /// keeps detailed help available through host/tool help APIs.
    #[serde(default, skip_serializing_if = "ToolPresentationConfig::is_default")]
    pub tool_presentation: ToolPresentationConfig,
    /// Controls how plugin and tool metadata is rendered in UI surfaces.
    #[serde(default, skip_serializing_if = "UiPresentationConfig::is_default")]
    pub ui_presentation: UiPresentationConfig,
}

impl PluginPolicyConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ToolPresentationConfig {
    pub default_mode: ToolDescriptionMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<PluginKey, ToolDescriptionOverride>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<ToolKey, ToolDescriptionOverride>,
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
        plugin_key: &PluginKey,
        tool_key: &ToolKey,
        tool_default: Option<ToolDescriptionMode>,
    ) -> ToolDescriptionMode {
        if let Some(mode) = self.tools.get(tool_key).copied() {
            return resolve_tool_description_override(mode, tool_default, self.default_mode);
        }
        if let Some(mode) = self.plugins.get(plugin_key).copied() {
            return resolve_tool_description_override(mode, tool_default, self.default_mode);
        }
        tool_default.unwrap_or(self.default_mode)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolDescriptionOverride {
    #[default]
    ToolDefault,
    Detailed,
    Brief,
}

fn resolve_tool_description_override(
    mode: ToolDescriptionOverride,
    tool_default: Option<ToolDescriptionMode>,
    fallback: ToolDescriptionMode,
) -> ToolDescriptionMode {
    match mode {
        ToolDescriptionOverride::ToolDefault => tool_default.unwrap_or(fallback),
        ToolDescriptionOverride::Detailed => ToolDescriptionMode::Detailed,
        ToolDescriptionOverride::Brief => ToolDescriptionMode::Brief,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiPresentationConfig {
    pub default_mode: UiTextDisplayMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<PluginKey, UiPresentationOverride>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<ToolKey, UiPresentationOverride>,
}

impl Default for UiPresentationConfig {
    fn default() -> Self {
        Self {
            default_mode: UiTextDisplayMode::Detailed,
            plugins: BTreeMap::new(),
            tools: BTreeMap::new(),
        }
    }
}

impl UiPresentationConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn mode_for(
        &self,
        plugin_key: &PluginKey,
        tool_key: &ToolKey,
        tool_default: Option<UiTextDisplayMode>,
    ) -> UiTextDisplayMode {
        if let Some(mode) = self.tools.get(tool_key).copied() {
            return resolve_ui_presentation_override(mode, tool_default, self.default_mode);
        }
        if let Some(mode) = self.plugins.get(plugin_key).copied() {
            return resolve_ui_presentation_override(mode, tool_default, self.default_mode);
        }
        tool_default.unwrap_or(self.default_mode)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiPresentationOverride {
    #[default]
    Default,
    Detailed,
    Summary,
}

fn resolve_ui_presentation_override(
    mode: UiPresentationOverride,
    tool_default: Option<UiTextDisplayMode>,
    fallback: UiTextDisplayMode,
) -> UiTextDisplayMode {
    match mode {
        UiPresentationOverride::Default => tool_default.unwrap_or(fallback),
        UiPresentationOverride::Detailed => UiTextDisplayMode::Detailed,
        UiPresentationOverride::Summary => UiTextDisplayMode::Summary,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
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
pub struct PluginSignature {
    pub key_id: String,
    /// Hex-encoded raw signature bytes (64 bytes for ed25519).
    pub signature: String,
}

/// One configured plugin under `plugins.list.<id>`. The host knows how to
/// load the `package`; `config` is plugin-owned JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredPlugin {
    #[serde(default = "default_plugin_enabled")]
    pub enabled: bool,
    pub package: PluginPackage,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub timeouts: TimeoutsConfig,
}

impl Default for ConfiguredPlugin {
    fn default() -> Self {
        Self {
            enabled: true,
            package: PluginPackage::Static {},
            config: serde_json::Value::Null,
            timeouts: TimeoutsConfig::default(),
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

impl ConfiguredPlugin {
    pub fn static_config(config: serde_json::Value) -> Self {
        Self {
            enabled: true,
            package: PluginPackage::Static {},
            config,
            timeouts: TimeoutsConfig::default(),
        }
    }

    pub fn static_default() -> Self {
        Self::static_config(serde_json::Value::Null)
    }

    pub fn config(&self) -> &serde_json::Value {
        &self.config
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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
mod presentation_tests {
    use super::*;

    #[test]
    fn tool_and_ui_policy_resolve_tool_then_plugin_then_default() {
        let plugin: PluginKey = "agena.settings".parse().expect("plugin key");
        let tool: ToolKey = "agena.settings.get".parse().expect("tool key");

        let mut tool_policy = ToolPresentationConfig {
            default_mode: ToolDescriptionMode::Brief,
            ..ToolPresentationConfig::default()
        };
        assert_eq!(
            tool_policy.mode_for(&plugin, &tool, None),
            ToolDescriptionMode::Brief
        );
        tool_policy
            .plugins
            .insert(plugin.clone(), ToolDescriptionOverride::Detailed);
        assert_eq!(
            tool_policy.mode_for(&plugin, &tool, None),
            ToolDescriptionMode::Detailed
        );
        tool_policy
            .tools
            .insert(tool.clone(), ToolDescriptionOverride::Brief);
        assert_eq!(
            tool_policy.mode_for(&plugin, &tool, None),
            ToolDescriptionMode::Brief
        );

        let mut ui_policy = UiPresentationConfig {
            default_mode: UiTextDisplayMode::Summary,
            ..UiPresentationConfig::default()
        };
        assert_eq!(
            ui_policy.mode_for(&plugin, &tool, None),
            UiTextDisplayMode::Summary
        );
        ui_policy
            .plugins
            .insert(plugin.clone(), UiPresentationOverride::Detailed);
        assert_eq!(
            ui_policy.mode_for(&plugin, &tool, None),
            UiTextDisplayMode::Detailed
        );
        ui_policy
            .tools
            .insert(tool.clone(), UiPresentationOverride::Summary);
        assert_eq!(
            ui_policy.mode_for(&plugin, &tool, None),
            UiTextDisplayMode::Summary
        );
    }
}
