//! Host configuration for loading and running plugins.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
#[serde(default, deny_unknown_fields)]
/// Policy configuration for plugins.
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

pub use agena_plugin_sdk::configuration::{
    ConfiguredPlugin, DurationSpec, HttpAuth, PluginActivationConfig, PluginPackage,
    PluginSignature, RestartMode, RestartPolicy, TimeoutsConfig,
};
