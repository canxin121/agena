//! Declarative network permission configuration.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::PermissionMode;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

impl PartialEq for NetworkPermissionConfig {
    fn eq(&self, other: &Self) -> bool {
        self.internet == other.internet
            && self.private == other.private
            && self.loopback == other.loopback
            && self.rules.iter().eq(other.rules.iter())
    }
}

impl Eq for NetworkPermissionConfig {}

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
        // Rules use last-match precedence. Replacing an inherited key must also
        // move it into the overlay's order after the remaining inherited rules.
        for (pattern, mode) in overlay.rules {
            self.rules.shift_remove(&pattern);
            self.rules.insert(pattern, mode);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reordered_network_rules_are_different_policies() {
        let broad_last: NetworkPermissionConfig =
            serde_json::from_str(r#"{"rules":{"8.8.8.8:443":"deny","8.0.0.0/8":"allow"}}"#)
                .unwrap();
        let narrow_last: NetworkPermissionConfig =
            serde_json::from_str(r#"{"rules":{"8.0.0.0/8":"allow","8.8.8.8:443":"deny"}}"#)
                .unwrap();
        assert_eq!(broad_last, broad_last.clone());
        assert_ne!(broad_last, narrow_last);
    }

    #[test]
    fn network_overlay_reorders_replaced_rules_after_inherited_entries() {
        let mut base: NetworkPermissionConfig = serde_json::from_str(
            r#"{"rules":{"8.8.8.8:443":"deny","8.0.0.0/8":"allow","9.9.9.9":"deny"}}"#,
        )
        .unwrap();
        let overlay: NetworkPermissionConfig =
            serde_json::from_str(r#"{"rules":{"8.0.0.0/8":"allow","8.8.8.8:443":"deny"}}"#)
                .unwrap();
        base.merge_from(overlay);
        assert_eq!(
            base.rules.keys().map(String::as_str).collect::<Vec<_>>(),
            ["9.9.9.9", "8.0.0.0/8", "8.8.8.8:443"],
        );
    }
}
