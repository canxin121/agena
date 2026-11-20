//! Declarative tool permission rule shapes.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::PermissionMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
/// A tool-level mode or an ordered rule group.
pub enum ToolPermissionRules {
    Mode(PermissionMode),
    Ordered(IndexMap<String, PermissionMode>),
}

impl PartialEq for ToolPermissionRules {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Mode(left), Self::Mode(right)) => left == right,
            (Self::Ordered(left), Self::Ordered(right)) => left.iter().eq(right.iter()),
            _ => false,
        }
    }
}

impl Eq for ToolPermissionRules {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reordered_command_rules_are_different_policies() {
        let broad_last: ToolPermissionRules =
            serde_json::from_str(r#"{"git push *":"deny","git *":"ask"}"#).unwrap();
        let narrow_last: ToolPermissionRules =
            serde_json::from_str(r#"{"git *":"ask","git push *":"deny"}"#).unwrap();
        assert_eq!(broad_last, broad_last.clone());
        assert_ne!(broad_last, narrow_last);
    }

    #[test]
    fn tool_aliases_with_opposite_rule_order_are_rejected_before_normalization() {
        let error = serde_json::from_str::<crate::PermissionConfig>(
            r#"{"tools":{"rules":{
                "shell.exec":{"git push *":"deny","git *":"ask"},
                "agena.shell.exec":{"git *":"ask","git push *":"deny"}
            }}}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("same tool"), "{error}");
    }

    #[test]
    fn legacy_terminal_defaults_collapse_into_one_ordered_group() {
        let classes = serde_json::json!({"no-op":"allow","routine":"allow","dangerous":"deny"});
        let config: crate::PermissionConfig = serde_json::from_value(serde_json::json!({
            "tools": {
                "rules": {
                    "shell.exec": classes,
                    "agena_shell_spawn": classes,
                    "agena.shell.open": classes,
                    "agena.shell.watch": classes,
                    "agena.shell.write": {"dangerous":"deny"},
                    "agena.monitor.start": "ask",
                    "*": {"read-only":"allow"},
                }
            }
        }))
        .unwrap();
        let rules = &config.tools.as_ref().unwrap().rules;
        assert_eq!(rules.len(), 3);
        assert_eq!(
            serde_json::to_value(&rules[crate::SHELL_COMMAND_RULE_GROUP]).unwrap(),
            classes
        );
        assert_eq!(
            rules["agena.monitor.start"],
            ToolPermissionRules::Mode(PermissionMode::Ask)
        );
        assert_eq!(
            config,
            serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap()
        );
    }
}
