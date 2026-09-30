//! Declarative path permission configuration.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::PathAccessModes;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Permission configuration for path access by class and per-rule.
pub struct PathPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathAccessModes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<PathAccessModes>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub rules: IndexMap<String, PathAccessModes>,
}

impl PathPermissionConfig {
    pub fn is_empty(&self) -> bool {
        self.workspace.is_none() && self.external.is_none() && self.rules.is_empty()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        if let Some(workspace) = overlay.workspace {
            match self.workspace.as_mut() {
                Some(current) => current.merge_from(workspace),
                None => self.workspace = Some(workspace),
            }
        }
        if let Some(external) = overlay.external {
            match self.external.as_mut() {
                Some(current) => current.merge_from(external),
                None => self.external = Some(external),
            }
        }
        // `extend` is what makes a rule the user writes win over the built-in
        // entry of the same pattern: both live in this one map.
        self.rules.extend(overlay.rules);
    }
}
