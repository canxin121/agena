//! Declarative tool permission configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{PermissionMode, ToolPermissionRules};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(deny_unknown_fields)]
/// Permission configuration for tools.
pub struct ToolPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<PermissionMode>,
    #[serde(default, rename = "names", skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, PermissionMode>,
    #[serde(default, skip)]
    pub plugin: BTreeMap<String, PermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, ToolPermissionRules>,
    /// Set when deserialization saw a legacy `tags` group. The map itself is
    /// dropped (it carries no authority); this flag only lets validation warn.
    #[serde(skip)]
    pub declared_tags_present: bool,
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
            #[serde(default)]
            tags: Option<BTreeMap<String, PermissionMode>>,
        }
        let flat = Flat::deserialize(deserializer)?;
        let mut value = ToolPermissionConfig {
            default: flat.default,
            names: flat.names,
            plugin: BTreeMap::new(),
            rules: flat.rules,
            declared_tags_present: false,
        };
        // Legacy `tags` groups are metadata and carry no authority. The
        // permission engine never reads capability modes; tool modes come only
        // from `default`, `names`, and `rules`. `tags` is dropped here and a
        // warning is surfaced during config validation when it is present.
        if flat.tags.is_some() {
            value.declared_tags_present = true;
        }
        Ok(value)
    }
}

impl ToolPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.default.is_none()
            && self.names.is_empty()
            && self.plugin.is_empty()
            && self.rules.is_empty()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        if overlay.default.is_some() {
            self.default = overlay.default;
        }
        self.names.extend(overlay.names);
        self.plugin.extend(overlay.plugin);
        self.rules.extend(overlay.rules);
    }
}
