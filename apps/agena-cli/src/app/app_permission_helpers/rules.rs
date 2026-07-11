use super::path_access_modes_summary;

pub(in crate::app) fn permission_studio_mode_target_value(
    permission: &PermissionConfig,
    target: &PermissionStudioModeTarget,
) -> Option<PermissionMode> {
    match target {
        PermissionStudioModeTarget::PathWorkspaceRead => permission
            .path
            .as_ref()
            .and_then(|path| path.workspace.as_ref())
            .and_then(|modes| modes.read),
        PermissionStudioModeTarget::PathWorkspaceWrite => permission
            .path
            .as_ref()
            .and_then(|path| path.workspace.as_ref())
            .and_then(|modes| modes.write),
        PermissionStudioModeTarget::PathExternalRead => permission
            .path
            .as_ref()
            .and_then(|path| path.external.as_ref())
            .and_then(|modes| modes.read),
        PermissionStudioModeTarget::PathExternalWrite => permission
            .path
            .as_ref()
            .and_then(|path| path.external.as_ref())
            .and_then(|modes| modes.write),
        PermissionStudioModeTarget::NetworkInternet => permission
            .network
            .as_ref()
            .and_then(|network| network.internet),
        PermissionStudioModeTarget::NetworkPrivate => permission
            .network
            .as_ref()
            .and_then(|network| network.private),
        PermissionStudioModeTarget::NetworkLoopback => permission
            .network
            .as_ref()
            .and_then(|network| network.loopback),
        PermissionStudioModeTarget::ToolDefault => {
            permission.tools.as_ref().and_then(|tools| tools.default)
        }
        PermissionStudioModeTarget::PathRuleRead { pattern } => permission
            .path
            .as_ref()
            .and_then(|path| path.rules.get(pattern.as_str()))
            .and_then(|rule| path_rule_modes(Some(rule)))
            .and_then(|modes| modes.read),
        PermissionStudioModeTarget::PathRuleWrite { pattern } => permission
            .path
            .as_ref()
            .and_then(|path| path.rules.get(pattern.as_str()))
            .and_then(|rule| path_rule_modes(Some(rule)))
            .and_then(|modes| modes.write),
        PermissionStudioModeTarget::NetworkRule { target } => permission
            .network
            .as_ref()
            .and_then(|network| network.rules.get(target.as_str()).copied()),
        PermissionStudioModeTarget::ToolTag { key } => permission
            .tools
            .as_ref()
            .and_then(|tools| tools.tags.get(key.as_str()).copied()),
        PermissionStudioModeTarget::ToolName { key } => permission
            .tools
            .as_ref()
            .and_then(|tools| tools.names.get(key.as_str()).copied()),
        PermissionStudioModeTarget::ToolRule { tool_name } => permission
            .tools
            .as_ref()
            .and_then(|tools| tools.rules.get(tool_name.as_str()))
            .and_then(|rules| match rules {
                ToolPermissionRules::Mode(mode) => Some(*mode),
                ToolPermissionRules::Ordered(entries) => entries.get("*").copied(),
            }),
        PermissionStudioModeTarget::ToolCommandPattern { tool_name, pattern } => permission
            .tools
            .as_ref()
            .and_then(|tools| tools.rules.get(tool_name.as_str()))
            .and_then(|rules| match rules {
                ToolPermissionRules::Ordered(entries) => entries.get(pattern.as_str()).copied(),
                ToolPermissionRules::Mode(mode) if pattern == "*" => Some(*mode),
                ToolPermissionRules::Mode(_) => None,
            }),
    }
}

pub(in crate::app) fn path_rule_modes(
    rule: Option<&PathAccessRuleConfig>,
) -> Option<PathAccessModes> {
    match rule? {
        PathAccessRuleConfig::Modes(modes) => Some(modes.clone()),
        PathAccessRuleConfig::Shorthand(value) => path_access_shorthand_modes(value.as_str()),
    }
}

pub(in crate::app) fn path_access_shorthand_modes(value: &str) -> Option<PathAccessModes> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    let both = |mode| PathAccessModes {
        read: Some(mode),
        write: Some(mode),
    };
    match normalized.as_str() {
        "allow" | "read_write" | "rw" => Some(both(PermissionMode::Allow)),
        "ask" => Some(both(PermissionMode::Ask)),
        "deny" | "none" => Some(both(PermissionMode::Deny)),
        "read" | "read_only" | "ro" => Some(PathAccessModes {
            read: Some(PermissionMode::Allow),
            write: Some(PermissionMode::Deny),
        }),
        "write" | "write_only" | "wo" => Some(PathAccessModes {
            read: Some(PermissionMode::Deny),
            write: Some(PermissionMode::Allow),
        }),
        _ => None,
    }
}

pub(in crate::app) fn path_rule_summary(
    i18n: &I18n,
    rule: Option<&PathAccessRuleConfig>,
) -> String {
    path_rule_modes(rule)
        .map(|modes| path_access_modes_summary(i18n, Some(&modes)))
        .unwrap_or_else(|| ui_text::t(i18n, "value-custom"))
}

pub(in crate::app) fn tool_permission_rules_summary(
    i18n: &I18n,
    rules: Option<&ToolPermissionRules>,
) -> String {
    let Some(rules) = rules else {
        return ui_text::t(i18n, "value-unset");
    };
    match rules {
        ToolPermissionRules::Mode(mode) => permission_mode_label(i18n, *mode),
        ToolPermissionRules::Ordered(entries) => {
            let fallback = entries.get("*").copied();
            let qualifier_count = entries
                .keys()
                .filter(|pattern| pattern.as_str() != "*")
                .count();
            let mut parts = Vec::new();
            if let Some(mode) = fallback {
                parts.push(permission_mode_label(i18n, mode));
            }
            if qualifier_count > 0 {
                parts.push(i18n.text_args(
                    "value-rule-count",
                    &crate::fl_args!("count" => qualifier_count as i64),
                ));
            }
            if parts.is_empty() {
                ui_text::t(i18n, "value-custom")
            } else {
                join_inline_segments(parts)
            }
        }
    }
}

pub(in crate::app) fn parse_permission_studio_optional_mode_input(
    i18n: &I18n,
    input: &str,
) -> UiResult<Option<PermissionMode>> {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("clear")
        || trimmed.eq_ignore_ascii_case("unset")
    {
        return Ok(None);
    }
    parse_permission_mode_token(i18n, trimmed).map(Some)
}

pub(in crate::app) fn parse_permission_studio_key_input(
    i18n: &I18n,
    field: &str,
    input: &str,
) -> UiResult<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(i18n.text_args(
            "permission-studio-error-empty-value",
            &crate::fl_args!("field" => field.to_string()),
        ));
    }
    Ok(trimmed.to_string())
}

pub(in crate::app) fn permission_mode_token(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Allow => "allow",
        PermissionMode::Ask => "ask",
        PermissionMode::Deny => "deny",
    }
}

pub(in crate::app) fn set_path_default_mode(
    permission: &mut PermissionConfig,
    external: bool,
    read: bool,
    mode: Option<PermissionMode>,
) {
    let path = permission.path.get_or_insert_with(Default::default);
    let target = if external {
        &mut path.external
    } else {
        &mut path.workspace
    };
    let modes = target.get_or_insert_with(Default::default);
    if read {
        modes.read = mode;
    } else {
        modes.write = mode;
    }
    if modes.read.is_none() && modes.write.is_none() {
        *target = None;
    }
}

pub(in crate::app) fn rename_path_rule(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(path) = permission.path.as_mut() else {
        return;
    };
    if let Some(rule) = path.rules.shift_remove(from) {
        path.rules.insert(to.to_string(), rule);
    }
}

pub(in crate::app) fn rename_network_rule(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(network) = permission.network.as_mut() else {
        return;
    };
    if let Some(mode) = network.rules.shift_remove(from) {
        network.rules.insert(to.to_string(), mode);
    }
}

pub(in crate::app) fn rename_tool_tag(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(tools) = permission.tools.as_mut() else {
        return;
    };
    if let Some(mode) = tools.tags.remove(from) {
        tools.tags.insert(to.to_string(), mode);
    }
}

pub(in crate::app) fn rename_tool_name(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(tools) = permission.tools.as_mut() else {
        return;
    };
    if let Some(mode) = tools.names.remove(from) {
        tools.names.insert(to.to_string(), mode);
    }
}

pub(in crate::app) fn rename_tool_rule(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(tools) = permission.tools.as_mut() else {
        return;
    };
    if let Some(rule) = tools.rules.remove(from) {
        tools.rules.insert(to.to_string(), rule);
    }
}

pub(in crate::app) fn normalize_permission_config(permission: &mut PermissionConfig) {
    if permission
        .path
        .as_ref()
        .is_some_and(PathPermissionConfig::is_empty)
    {
        permission.path = None;
    }
    if permission
        .network
        .as_ref()
        .is_some_and(NetworkPermissionConfig::is_empty)
    {
        permission.network = None;
    }
    if permission
        .tools
        .as_ref()
        .is_some_and(ToolPermissionConfig::is_empty)
    {
        permission.tools = None;
    }
}

pub(in crate::app) fn permission_mode_label(i18n: &I18n, mode: PermissionMode) -> String {
    ui_text::t(
        i18n,
        match mode {
            PermissionMode::Allow => "value-allow",
            PermissionMode::Ask => "value-ask",
            PermissionMode::Deny => "value-deny",
        },
    )
}
use crate::app::{
    I18n, NetworkPermissionConfig, PathAccessModes, PathAccessRuleConfig, PathPermissionConfig,
    PermissionConfig, PermissionMode, PermissionStudioModeTarget, ToolPermissionConfig,
    ToolPermissionRules, UiResult, join_inline_segments, parse_permission_mode_token, ui_text,
};
