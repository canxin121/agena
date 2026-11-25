//! Declarative tool permission configuration.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    COMMAND_CLASS_DANGEROUS, COMMAND_CLASS_NO_OP, COMMAND_CLASS_ROUTINE, PermissionMode,
    ToolPermissionRules,
};

/// The ordered rule group shared by every terminal command entry point.
pub const SHELL_COMMAND_RULE_GROUP: &str = "shell.*";

pub fn command_class_keyword(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().replace('_', "-").as_str() {
        "no-op" | "noop" => Some(COMMAND_CLASS_NO_OP),
        "routine" => Some(COMMAND_CLASS_ROUTINE),
        "dangerous" => Some(COMMAND_CLASS_DANGEROUS),
        _ => None,
    }
}

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
    pub fn command_rules(&self) -> Option<&IndexMap<String, PermissionMode>> {
        match self.rules.get(SHELL_COMMAND_RULE_GROUP)? {
            ToolPermissionRules::Ordered(entries) => Some(entries),
            ToolPermissionRules::Mode(_) => None,
        }
    }

    pub fn canonical_command_policy_tool_name(name: &str) -> Option<&'static str> {
        match name {
            "monitor.start" | "agena.monitor.start" | "agena_monitor_start" => {
                Some("agena.monitor.start")
            }
            _ => Self::canonical_shell_tool_name(name),
        }
    }

    pub fn canonical_shell_tool_name(name: &str) -> Option<&'static str> {
        match name {
            "agena.shell.exec" | "shell.exec" | "agena_shell_exec" => Some("agena.shell.exec"),
            "agena.shell.spawn" | "shell.spawn" | "agena_shell_spawn" => Some("agena.shell.spawn"),
            "agena.shell.watch" | "shell.watch" | "agena_shell_watch" => Some("agena.shell.watch"),
            "agena.shell.open" | "shell.open" | "agena_shell_open" => Some("agena.shell.open"),
            "agena.shell.write" | "shell.write" | "agena_shell_write" => Some("agena.shell.write"),
            "agena.shell.read" | "shell.read" | "agena_shell_read" => Some("agena.shell.read"),
            "agena.shell.list" | "shell.list" | "agena_shell_list" => Some("agena.shell.list"),
            "agena.shell.stop" | "shell.stop" | "agena_shell_stop" => Some("agena.shell.stop"),
            "agena.shell.resize" | "shell.resize" | "agena_shell_resize" => {
                Some("agena.shell.resize")
            }
            "agena.shell.signal" | "shell.signal" | "agena_shell_signal" => {
                Some("agena.shell.signal")
            }
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.default.is_none()
            && self.names.is_empty()
            && self.plugin.is_empty()
            && self.rules.is_empty()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        let overlay = overlay.normalized();
        if overlay.default.is_some() {
            self.default = overlay.default;
        }
        // `extend` on all three maps is what makes the user's own entry win
        // over the built-in entry of the same key, and what makes deleting a
        // key fall back to the built-in one instead of switching it off.
        self.names.extend(overlay.names);
        self.plugin.extend(overlay.plugin);
        self.rules.extend(overlay.rules);
    }

    /// Reject contradictory spellings before canonicalization can discard one.
    pub fn validate_aliases(&self) -> Result<(), String> {
        fn check<T: PartialEq>(values: &BTreeMap<String, T>, section: &str) -> Result<(), String> {
            let mut seen = BTreeMap::<&str, (&str, &T)>::new();
            for (name, value) in values {
                let canonical = canonical_policy_tool_name(name).unwrap_or(name);
                if let Some((previous_name, previous)) = seen.insert(canonical, (name, value))
                    && previous != value
                {
                    return Err(format!(
                        "permission.tools.{section}: {previous_name:?} and {name:?} identify the same tool but have conflicting rules; keep one spelling"
                    ));
                }
            }
            Ok(())
        }
        check(&self.names, "names")?;
        check(&self.rules, "rules")
    }

    pub fn with_canonical_tool_names(mut self) -> Self {
        normalize_tool_keys(&mut self.names);
        normalize_tool_keys(&mut self.rules);
        self
    }

    /// Normalize tool aliases and fold old per-tool command tables into the
    /// shared table. Scalar rules still configure the tool itself.
    pub fn normalized(self) -> Self {
        let mut config = self.with_canonical_tool_names();
        let mut legacy = Vec::new();
        config.rules.retain(|name, rules| {
            if is_legacy_command_rule_tool(name)
                && let ToolPermissionRules::Ordered(entries) = rules
            {
                legacy.push(entries.clone());
                return false;
            }
            true
        });
        if !legacy.is_empty() {
            config
                .rules
                .entry(SHELL_COMMAND_RULE_GROUP.to_owned())
                .or_insert_with(|| {
                    ToolPermissionRules::Ordered(merge_legacy_command_rules(legacy))
                });
        }
        config
    }
}

fn is_legacy_command_rule_tool(name: &str) -> bool {
    matches!(
        name,
        "agena.shell.exec"
            | "agena.shell.spawn"
            | "agena.shell.watch"
            | "agena.shell.open"
            | "agena.shell.write"
    )
}

fn merge_legacy_command_rules(
    groups: Vec<IndexMap<String, PermissionMode>>,
) -> IndexMap<String, PermissionMode> {
    let mut merged = IndexMap::new();
    let mut prior_patterns = None;
    let mut multiple_pattern_lists = false;
    for group in groups {
        let patterns = group
            .iter()
            .filter(|(pattern, _)| command_class_keyword(pattern).is_none())
            .map(|(pattern, mode)| (pattern.clone(), *mode))
            .collect::<Vec<_>>();
        if !patterns.is_empty() {
            if let Some(prior) = &prior_patterns {
                multiple_pattern_lists |= prior != &patterns;
            } else {
                prior_patterns = Some(patterns);
            }
        }
        // Aliases within a legacy group used the last occurrence.
        let normalized = group
            .into_iter()
            .map(|(pattern, mode)| {
                let key = command_class_keyword(&pattern)
                    .map(str::to_owned)
                    .unwrap_or(pattern);
                (key, mode)
            })
            .collect::<IndexMap<_, _>>();
        for (key, mode) in normalized {
            let entry = merged.entry(key).or_insert(mode);
            if restrictiveness(mode) > restrictiveness(*entry) {
                *entry = mode;
            }
        }
    }
    // Separate legacy lists could disagree on overlapping patterns. When
    // combining those lists, the stricter match takes precedence.
    if multiple_pattern_lists {
        merged.sort_by(|left, left_mode, right, right_mode| {
            let priority = |pattern: &str, mode| match command_class_keyword(pattern) {
                Some(_) => 0,
                None => 1 + restrictiveness(mode),
            };
            priority(left, *left_mode).cmp(&priority(right, *right_mode))
        });
    }
    merged
}

fn restrictiveness(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Allow => 0,
        PermissionMode::Auto => 1,
        PermissionMode::Ask => 2,
        PermissionMode::Deny => 3,
    }
}

fn canonical_policy_tool_name(name: &str) -> Option<&'static str> {
    match name {
        "agena.code_mode.execute" | "code_mode.execute" | "tool_script.run" => {
            Some("agena.tool_script.run")
        }
        _ => ToolPermissionConfig::canonical_command_policy_tool_name(name),
    }
}

fn normalize_tool_keys<T>(map: &mut BTreeMap<String, T>) {
    let aliases = map
        .keys()
        .filter_map(|name| {
            let canonical = canonical_policy_tool_name(name)?;
            (name != canonical).then(|| (name.clone(), canonical))
        })
        .collect::<Vec<_>>();
    for (alias, canonical) in aliases {
        if let Some(value) = map.remove(&alias) {
            map.entry(canonical.to_owned()).or_insert(value);
        }
    }
}
