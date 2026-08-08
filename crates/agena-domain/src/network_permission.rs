//! Declarative network permission configuration.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::PermissionMode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Permission configuration for network access by target class.
pub struct NetworkPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internet: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loopback: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub rules: IndexMap<String, PermissionMode>,
}

impl NetworkPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.internet.is_none()
            && self.private.is_none()
            && self.loopback.is_none()
            && self.rules.is_empty()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        if overlay.internet.is_some() {
            self.internet = overlay.internet;
        }
        if overlay.private.is_some() {
            self.private = overlay.private;
        }
        if overlay.loopback.is_some() {
            self.loopback = overlay.loopback;
        }
        self.rules.extend(overlay.rules);
    }
}
