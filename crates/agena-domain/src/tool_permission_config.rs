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
            "agena.shell.logs" | "shell.logs" | "agena_shell_logs" => Some("agena.shell.logs"),
            "agena.shell.list" | "shell.list" | "agena_shell_list" => Some("agena.shell.list"),
            "agena.shell.stop" | "shell.stop" | "agena_shell_stop" => Some("agena.shell.stop"),
            "agena.shell.resize" | "shell.resize" | "agena_shell_resize" => {
                Some("agena.shell.resize")
            }
            "agena.shell.signal" | "shell.signal" | "agena_shell_signal" => {
                Some("agena.shell.signal")
            }
            "agena.shell.run" | "shell.run" | "agena_shell_run" => Some("agena.shell.run"),
            _ => None,
        }
    }

    /// Retired launch aliases are compatibility inputs only, never executable
    /// tool names. Preserve their policy on each explicit replacement mode.
    pub fn retired_shell_launch_replacements(name: &str) -> Option<[&'static str; 3]> {
        match Self::canonical_shell_tool_name(name) {
            Some("agena.shell.run") => {
                Some(["agena.shell.exec", "agena.shell.spawn", "agena.shell.open"])
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
        let overlay = overlay.with_current_shell_names();
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

    /// Preserve existing command policy when the retired multi-mode launch
    /// tool is replaced. Explicit rules for a new tool win within one layer.
    pub fn with_current_shell_names(mut self) -> Self {
        normalize_shell_keys(&mut self.names);
        normalize_shell_keys(&mut self.rules);
        // Command listeners moved into Shell. Preserve their previous policy
        // while keeping the WebSocket tool's own name policy intact.
        if let Some(mode) = self.names.get("agena.monitor.start").copied() {
            self.names
                .entry("agena.shell.watch".to_owned())
                .or_insert(mode);
        }
        if let Some(rules) = self.rules.get("agena.monitor.start").cloned() {
            self.rules
                .entry("agena.shell.watch".to_owned())
                .or_insert(rules);
        }
        for old in ["agena.shell.run"] {
            let targets =
                Self::retired_shell_launch_replacements(old).expect("retired shell launch");
            if let Some(mode) = self.names.remove(old) {
                for target in targets {
                    self.names.entry(target.to_owned()).or_insert(mode);
                }
            }
            if let Some(rules) = self.rules.remove(old) {
                // These command denials were global before launch modes were
                // split. Preserve them on terminal input and command monitors,
                // without extending launch approvals to either operation.
                if let ToolPermissionRules::Ordered(entries) = &rules {
                    let denied = entries
                        .iter()
                        .filter(|(pattern, mode)| {
                            **mode == PermissionMode::Deny && pattern.trim() != "*"
                        })
                        .map(|(pattern, mode)| (pattern.clone(), *mode))
                        .collect::<indexmap::IndexMap<_, _>>();
                    if !denied.is_empty() {
                        for target in [
                            "agena.shell.write",
                            "agena.shell.watch",
                            "agena.monitor.start",
                        ] {
                            if !self.names.contains_key(target) && !self.rules.contains_key(target)
                            {
                                self.rules.insert(
                                    target.to_owned(),
                                    ToolPermissionRules::Ordered(denied.clone()),
                                );
                            }
                        }
                    }
                }
                for target in targets {
                    self.rules
                        .entry(target.to_owned())
                        .or_insert_with(|| rules.clone());
                }
            }
        }
        self
    }
}

fn normalize_shell_keys<T>(map: &mut BTreeMap<String, T>) {
    let aliases = map
        .keys()
        .filter_map(|name| {
            let canonical = ToolPermissionConfig::canonical_command_policy_tool_name(name)?;
            (name != canonical).then(|| (name.clone(), canonical))
        })
        .collect::<Vec<_>>();
    for (alias, canonical) in aliases {
        if let Some(value) = map.remove(&alias) {
            map.entry(canonical.to_owned()).or_insert(value);
        }
    }
}
