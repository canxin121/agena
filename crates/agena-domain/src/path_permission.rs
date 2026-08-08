//! Declarative path permission configuration.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{PathAccessModes, PathAccessRuleConfig};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Permission configuration for path access by class and per-rule.
pub struct PathPermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathAccessModes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<PathAccessModes>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub rules: IndexMap<String, PathAccessRuleConfig>,
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
        self.rules.extend(overlay.rules);
    }
}
