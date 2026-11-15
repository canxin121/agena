//! Helpers for projecting permission rules into presentation state.

use agena_domain::{
    NetworkPermissionConfig, PathAccessModes, PathPermissionConfig, PathPermissionMode,
    PermissionConfig, PermissionMode, ToolPermissionConfig, ToolPermissionRules,
};
use agena_tui::i18n::I18n;

use crate::PermissionStudioModeTarget;

pub fn permission_studio_mode_target_value(
    permission: &PermissionConfig,
    target: &PermissionStudioModeTarget,
) -> Option<PermissionMode> {
    match target {
        PermissionStudioModeTarget::PathWorkspace
        | PermissionStudioModeTarget::PathExternal
        | PermissionStudioModeTarget::PathRule { .. } => None,
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

pub fn path_rule_summary(i18n: &I18n, rule: Option<&PathAccessModes>) -> String {
    path_access_modes_summary(i18n, rule)
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
    mode: Option<PathPermissionMode>,
) {
    let path = permission.path.get_or_insert_with(Default::default);
    let target = if external {
        &mut path.external
    } else {
        &mut path.workspace
    };
    *target = mode.map(PathAccessModes::from_mode);
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
    if let Some((index, _, mode)) = network.rules.shift_remove_full(from) {
        network.rules.shift_insert(index, to.to_string(), mode);
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
    match modes.sandbox_mode() {
        Ok(mode) => path_permission_mode_label(i18n, mode),
        Err(_) => i18n.text("permission-studio-path-invalid-mode"),
    }
}

pub fn permission_studio_path_target_value(
    permission: &PermissionConfig,
    target: &PermissionStudioModeTarget,
) -> Option<PathPermissionMode> {
    let path = permission.path.as_ref()?;
    let modes = match target {
        PermissionStudioModeTarget::PathWorkspace => path.workspace.as_ref(),
        PermissionStudioModeTarget::PathExternal => path.external.as_ref(),
        PermissionStudioModeTarget::PathRule { pattern } => path.rules.get(pattern),
        _ => None,
    }?;
    modes.sandbox_mode().ok()
}

pub fn path_permission_mode_token(mode: PathPermissionMode) -> &'static str {
    match mode {
        PathPermissionMode::Readonly => "readonly",
        PathPermissionMode::Readwrite => "readwrite",
        PathPermissionMode::Deny => "deny",
    }
}

pub fn path_permission_mode_label(i18n: &I18n, mode: PathPermissionMode) -> String {
    i18n.text(match mode {
        PathPermissionMode::Readonly => "permission-studio-path-readonly",
        PathPermissionMode::Readwrite => "permission-studio-path-readwrite",
        PathPermissionMode::Deny => "value-deny",
    })
}

pub fn parse_permission_studio_optional_path_mode_input(
    i18n: &I18n,
    input: &str,
) -> Result<Option<PathPermissionMode>, String> {
    match input.trim().to_ascii_lowercase().as_str() {
        "" | "clear" | "unset" => Ok(None),
        "readonly" => Ok(Some(PathPermissionMode::Readonly)),
        "readwrite" => Ok(Some(PathPermissionMode::Readwrite)),
        "deny" => Ok(Some(PathPermissionMode::Deny)),
        _ => Err(i18n.text("permission-studio-path-invalid-mode")),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_rule_rename_preserves_later_restrictions() {
        let mut permission = PermissionConfig::default();
        let network = permission.network.get_or_insert_with(Default::default);
        network
            .rules
            .insert("10.0.0.0/8".to_string(), PermissionMode::Allow);
        network
            .rules
            .insert("10.0.0.1:443".to_string(), PermissionMode::Deny);

        rename_network_rule(&mut permission, "10.0.0.0/8", "10.0.0.0/16");

        let rules = &permission.network.as_ref().unwrap().rules;
        assert_eq!(
            rules.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["10.0.0.0/16", "10.0.0.1:443"]
        );
        assert_eq!(rules["10.0.0.1:443"], PermissionMode::Deny);
    }

    #[test]
    fn path_editor_accepts_only_mxc_modes_and_clear() {
        let i18n = I18n::english();
        for (input, mode) in [
            ("readonly", PathPermissionMode::Readonly),
            ("readwrite", PathPermissionMode::Readwrite),
            ("deny", PathPermissionMode::Deny),
        ] {
            assert_eq!(
                parse_permission_studio_optional_path_mode_input(&i18n, input).unwrap(),
                Some(mode)
            );
        }
        assert_eq!(
            parse_permission_studio_optional_path_mode_input(&i18n, "clear").unwrap(),
            None
        );
        for input in ["allow", "write", "ask", "auto", "read_only"] {
            assert!(parse_permission_studio_optional_path_mode_input(&i18n, input).is_err());
        }
    }

    #[test]
    fn path_default_changes_the_whole_access_mode() {
        let mut config = PermissionConfig::default();
        set_path_default_mode(&mut config, false, Some(PathPermissionMode::Readwrite));
        assert_eq!(
            permission_studio_path_target_value(
                &config,
                &PermissionStudioModeTarget::PathWorkspace
            ),
            Some(PathPermissionMode::Readwrite)
        );
        set_path_default_mode(&mut config, false, Some(PathPermissionMode::Readonly));
        let mode = config.path.as_ref().unwrap().workspace.as_ref().unwrap();
        assert_eq!(mode.read, Some(PermissionMode::Allow));
        assert_eq!(mode.write, Some(PermissionMode::Deny));
        set_path_default_mode(&mut config, false, None);
        assert!(config.path.as_ref().unwrap().workspace.is_none());
    }

    #[test]
    fn readwrite_is_displayed_as_resource_access_not_approval_allow() {
        let i18n = I18n::english();
        let mode = PathAccessModes::from_mode(PathPermissionMode::Readwrite);
        assert_eq!(
            path_access_modes_summary(&i18n, Some(&mode)),
            "Read and write"
        );
        assert_eq!(
            path_permission_mode_token(PathPermissionMode::Readwrite),
            "readwrite"
        );
    }
}
