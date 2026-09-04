//! Declarative tool permission configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{PermissionMode, ToolPermissionRules};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
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
