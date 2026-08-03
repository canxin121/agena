//! Declarative tool permission configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{PermissionMode, ToolPermissionRules};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<PermissionMode>,
    /// Modes keyed by tool capability (`read_only`, `shell`, `interactive`,
    /// `task`). The permission engine consumes these; tool tags are metadata
    /// and never carry authority.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<String, PermissionMode>,
    #[serde(default, rename = "names", skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, PermissionMode>,
    #[serde(default, skip)]
    pub plugin: BTreeMap<String, PermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, ToolPermissionRules>,
}


impl<'de> Deserialize<'de> for ToolPermissionConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Default, Serialize, Deserialize)]
        #[serde(default)]
        struct Flat {
            default: Option<PermissionMode>,
            #[serde(rename = "names")]
            names: BTreeMap<String, PermissionMode>,
            rules: BTreeMap<String, ToolPermissionRules>,
            tags: Option<BTreeMap<String, PermissionMode>>,
        }
        let flat = Flat::deserialize(deserializer)?;
        let mut value = ToolPermissionConfig {
            default: flat.default,
            capabilities: BTreeMap::new(),
            names: flat.names,
            plugin: BTreeMap::new(),
            rules: flat.rules,
        };
        // One-time migration: legacy `tags` entries that describe contract
        // capabilities are folded into `capabilities`; tags that are pure
        // metadata (never consumed by the policy engine) are dropped.
        if let Some(tags) = flat.tags {
            for (key, mode) in tags {
                if matches!(key.as_str(), "read_only" | "shell" | "interactive" | "task") {
                    value.capabilities.insert(key, mode);
                }
            }
        }
        Ok(value)
    }
}

impl ToolPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.default.is_none()
            && self.capabilities.is_empty()
            && self.names.is_empty()
            && self.plugin.is_empty()
            && self.rules.is_empty()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        if overlay.default.is_some() {
            self.default = overlay.default;
        }
        self.capabilities.extend(overlay.capabilities);
        self.names.extend(overlay.names);
        self.plugin.extend(overlay.plugin);
        self.rules.extend(overlay.rules);
    }
}
