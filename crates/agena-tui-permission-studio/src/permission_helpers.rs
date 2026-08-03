use agena_domain::{
    NetworkPermissionConfig, PathAccessModes, PathAccessRuleConfig, PathPermissionConfig,
    PermissionConfig, PermissionMode, ToolPermissionConfig, ToolPermissionRules,
};
use agena_tui::i18n::I18n;
use agena_tui_components::join_inline_segments;

use crate::PermissionStudioModeTarget;

pub fn permission_studio_mode_target_value(
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

pub fn path_rule_modes(rule: Option<&PathAccessRuleConfig>) -> Option<PathAccessModes> {
    match rule? {
        PathAccessRuleConfig::Modes(modes) => Some(modes.clone()),
        PathAccessRuleConfig::Shorthand(value) => path_access_shorthand_modes(value.as_str()),
    }
}

pub fn path_access_shorthand_modes(value: &str) -> Option<PathAccessModes> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    let both = |mode| PathAccessModes {
        read: Some(mode),
        write: Some(mode),
    };
    match normalized.as_str() {
        "allow" | "read_write" | "rw" => Some(both(PermissionMode::Allow)),
        "auto" => Some(both(PermissionMode::Auto)),
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

pub fn path_rule_summary(i18n: &I18n, rule: Option<&PathAccessRuleConfig>) -> String {
    path_rule_modes(rule)
        .map(|modes| path_access_modes_summary(i18n, Some(&modes)))
        .unwrap_or_else(|| i18n.text("value-custom"))
}

pub fn parse_permission_studio_optional_mode_input(
    i18n: &I18n,
    input: &str,
) -> Result<Option<PermissionMode>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("clear")
        || trimmed.eq_ignore_ascii_case("unset")
    {
        return Ok(None);
    }
    parse_permission_mode_token(i18n, trimmed).map(Some)
}

pub fn parse_permission_studio_key_input(
    i18n: &I18n,
    field: &str,
    input: &str,
) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(i18n.text_args(
            "permission-studio-error-empty-value",
            &agena_tui::fl_args!("field" => field.to_string()),
        ));
    }
    Ok(trimmed.to_string())
}

pub fn permission_mode_token(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Allow => "allow",
        PermissionMode::Auto => "auto",
        PermissionMode::Ask => "ask",
        PermissionMode::Deny => "deny",
    }
}

pub fn set_path_default_mode(
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

pub fn rename_path_rule(permission: &mut PermissionConfig, from: &str, to: &str) {
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

pub fn rename_network_rule(permission: &mut PermissionConfig, from: &str, to: &str) {
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

pub fn rename_tool_name(permission: &mut PermissionConfig, from: &str, to: &str) {
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

pub fn rename_tool_rule(permission: &mut PermissionConfig, from: &str, to: &str) {
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

pub fn normalize_permission_config(permission: &mut PermissionConfig) {
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

pub fn permission_mode_label(i18n: &I18n, mode: PermissionMode) -> String {
    i18n.text(match mode {
        PermissionMode::Allow => "value-allow",
        PermissionMode::Auto => "value-auto",
        PermissionMode::Ask => "value-ask",
        PermissionMode::Deny => "value-deny",
    })
}

pub fn path_access_modes_summary(i18n: &I18n, modes: Option<&PathAccessModes>) -> String {
    let Some(modes) = modes else {
        return i18n.text("value-unset");
    };
    match (modes.read, modes.write) {
        (Some(read), Some(write)) if read == write => permission_mode_label(i18n, read),
        (read, write) => join_inline_segments(vec![
            i18n.text_args(
                "permission-studio-mode-read",
                &agena_tui::fl_args!(
                    "value" => read
                        .map(|mode| permission_mode_label(i18n, mode))
                        .unwrap_or_else(|| i18n.text("value-unset"))
                ),
            ),
            i18n.text_args(
                "permission-studio-mode-write",
                &agena_tui::fl_args!(
                    "value" => write
                        .map(|mode| permission_mode_label(i18n, mode))
                        .unwrap_or_else(|| i18n.text("value-unset"))
                ),
            ),
        ]),
    }
}

fn parse_permission_mode_token(i18n: &I18n, token: &str) -> Result<PermissionMode, String> {
    match token.to_ascii_lowercase().as_str() {
        "allow" => Ok(PermissionMode::Allow),
        "auto" => Ok(PermissionMode::Auto),
        "ask" => Ok(PermissionMode::Ask),
        "deny" => Ok(PermissionMode::Deny),
        _ => Err(i18n.text("permission-rule-error-invalid-mode")),
    }
}
