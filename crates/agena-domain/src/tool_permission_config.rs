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
        let overlay = overlay.with_canonical_tool_names();
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

    /// Normalize the registered tool identity spellings without policy
    /// inheritance between distinct operations.
    pub fn with_canonical_tool_names(mut self) -> Self {
        normalize_shell_keys(&mut self.names);
        normalize_shell_keys(&mut self.rules);
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
