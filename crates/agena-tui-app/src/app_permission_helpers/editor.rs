use agena_domain::{SHELL_COMMAND_RULE_GROUP, ToolPermissionConfig};

use super::{
    normalize_permission_config, parse_permission_studio_key_input,
    parse_permission_studio_optional_mode_input, parse_permission_studio_optional_path_mode_input,
    path_permission_mode_token, permission_mode_token_text, permission_studio_mode_target_value,
    permission_studio_path_target_value, rename_path_rule, rename_tool_name, rename_tool_rule,
    set_path_default_mode,
};

pub(crate) fn permission_studio_sections(
    i18n: &I18n,
    dialog: &PermissionStudioOverlay,
) -> Vec<PermissionStudioSection> {
    let projection = super::PermissionStudioProjection::new(dialog);
    let mut sections = vec![permission_studio_path_defaults(i18n, &projection)];
    sections.extend(super::permission_studio_network_defaults(
        i18n,
        dialog,
        &projection,
    ));
    sections.push(permission_studio_tool_defaults(i18n, &projection));
    sections.push(permission_studio_path_rules(i18n, &projection));
    sections.extend(super::permission_studio_network_rules(i18n, dialog));
    sections.push(permission_studio_tool_names(i18n, &projection));
    sections.push(permission_studio_command_rules(i18n, &projection));
    sections.push(PermissionStudioSection {
        id: PermissionStudioSectionId::RememberedRules,
        label: i18n.text("permission-studio-remembered-rules"),
        items: vec![PermissionStudioItem {
            label: i18n.text("permission-studio-remembered-open"),
            value: String::new(),
            source: None,
            action: PermissionStudioAction::RememberedRules,
        }],
    });
    sections.extend(super::permission_studio_network_controls(i18n, dialog));
    sections
}

fn rules_title(i18n: &I18n, label: &str, count: usize) -> String {
    i18n.text_args(
        "permission-studio-section-rule-count",
        &agena_tui::fl_args!(
            "section" => i18n.text(label), "count" => count as i64,
        ),
    )
}

fn permission_studio_path_defaults(
    i18n: &I18n,
    projection: &super::PermissionStudioProjection,
) -> PermissionStudioSection {
    PermissionStudioSection {
        id: PermissionStudioSectionId::PathDefaults,
        label: rules_title(
            i18n,
            "permission-studio-nav-filesystem",
            projection
                .effective
                .path
                .as_ref()
                .map_or(0, |path| path.rules.len()),
        ),
        items: vec![
            projection.mode_item(
                i18n,
                i18n.text("permission-studio-workspace-label"),
                PermissionStudioModeTarget::PathWorkspace,
            ),
            projection.mode_item(
                i18n,
                i18n.text("permission-studio-external-label"),
                PermissionStudioModeTarget::PathExternal,
            ),
        ],
    }
}

fn permission_studio_tool_defaults(
    i18n: &I18n,
    projection: &super::PermissionStudioProjection,
) -> PermissionStudioSection {
    let tools = projection.effective.tools.as_ref();
    let names = tools.map_or(0, |tools| tools.names.len());
    let commands = tools
        .and_then(ToolPermissionConfig::command_rules)
        .map_or(0, indexmap::IndexMap::len);
    PermissionStudioSection {
        id: PermissionStudioSectionId::ToolDefaults,
        label: i18n.text_args(
            "permission-studio-tools-rule-count",
            &agena_tui::fl_args!("names" => names as i64, "commands" => commands as i64),
        ),
        items: vec![
            projection.mode_item(
                i18n,
                i18n.text("permission-studio-default-label"),
                PermissionStudioModeTarget::ToolDefault,
            ),
            projection.mode_item(
                i18n,
                i18n.text("permission-studio-read-only-class-label"),
                PermissionStudioModeTarget::ToolCommandPattern {
                    tool_name: "*".into(),
                    pattern: "read-only".into(),
                },
            ),
        ],
    }
}

fn local_rules_first(items: &mut [PermissionStudioItem]) {
    items.sort_by_key(|item| item.source != Some(crate::PermissionStudioValueSource::Current));
}

fn permission_studio_path_rules(
    i18n: &I18n,
    projection: &super::PermissionStudioProjection,
) -> PermissionStudioSection {
    let mut items = projection
        .effective
        .path
        .as_ref()
        .into_iter()
        .flat_map(|path| path.rules.keys())
        .map(|pattern| {
            projection.mode_item(
                i18n,
                pattern.clone(),
                PermissionStudioModeTarget::PathRule {
                    pattern: pattern.clone(),
                },
            )
        })
        .collect::<Vec<_>>();
    local_rules_first(&mut items);
    items.push(permission_studio_new_rule_item(
        i18n,
        "permission-studio-add-path-rule",
    ));
    PermissionStudioSection {
        id: PermissionStudioSectionId::PathRules,
        label: i18n.text("permission-studio-nav-path-rules"),
        items,
    }
}

fn permission_studio_tool_names(
    i18n: &I18n,
    projection: &super::PermissionStudioProjection,
) -> PermissionStudioSection {
    let mut items = projection
        .effective
        .tools
        .as_ref()
        .into_iter()
        .flat_map(|tools| tools.names.keys())
        .map(|key| {
            projection.mode_item(
                i18n,
                key.clone(),
                PermissionStudioModeTarget::ToolName { key: key.clone() },
            )
        })
        .collect::<Vec<_>>();
    local_rules_first(&mut items);
    items.push(permission_studio_new_rule_item(
        i18n,
        "permission-studio-add-name",
    ));
    PermissionStudioSection {
        id: PermissionStudioSectionId::ToolNames,
        label: i18n.text("permission-studio-nav-name-rules"),
        items,
    }
}

fn permission_studio_command_rules(
    i18n: &I18n,
    projection: &super::PermissionStudioProjection,
) -> PermissionStudioSection {
    let mut items = projection
        .effective
        .tools
        .as_ref()
        .and_then(ToolPermissionConfig::command_rules)
        .into_iter()
        .flat_map(|rules| rules.keys())
        .map(|pattern| {
            projection.mode_item(
                i18n,
                pattern.clone(),
                PermissionStudioModeTarget::ToolCommandPattern {
                    tool_name: SHELL_COMMAND_RULE_GROUP.to_owned(),
                    pattern: pattern.clone(),
                },
            )
        })
        .collect::<Vec<_>>();
    items.push(PermissionStudioItem {
        label: format!("+ {}", i18n.text("permission-studio-add-command-rule")),
        value: String::new(),
        source: None,
        action: PermissionStudioAction::AddToolCommandPattern,
    });
    PermissionStudioSection {
        id: PermissionStudioSectionId::ToolCommandRules,
        label: i18n.text("permission-studio-nav-command-rules"),
        items,
    }
}

fn permission_studio_new_rule_item(i18n: &I18n, label: &str) -> PermissionStudioItem {
    PermissionStudioItem {
        label: format!("+ {}", i18n.text(label)),
        value: String::new(),
        source: None,
        action: PermissionStudioAction::CreateRule,
    }
}

pub(crate) fn permission_studio_mode_target_label(
    i18n: &I18n,
    target: &PermissionStudioModeTarget,
) -> String {
    ui_text::t(
        i18n,
        match target {
            PermissionStudioModeTarget::PathWorkspace => "permission-studio-path-workspace",
            PermissionStudioModeTarget::PathExternal => "permission-studio-path-external",
            PermissionStudioModeTarget::NetworkEgressDefault => {
                "permission-studio-network-egress-default"
            }
            PermissionStudioModeTarget::NetworkIngressDefault => {
                "permission-studio-network-ingress-default"
            }
            PermissionStudioModeTarget::NetworkHostLoopback => {
                "permission-studio-network-host-loopback"
            }
            PermissionStudioModeTarget::ToolDefault => "permission-studio-tool-default",
            PermissionStudioModeTarget::ToolCommandPattern { tool_name, pattern }
                if tool_name == "*" && pattern == "read-only" =>
            {
                "permission-studio-read-only-class-label"
            }
            PermissionStudioModeTarget::PathRule { .. } => "permission-studio-rule-path",
            PermissionStudioModeTarget::ToolName { .. }
            | PermissionStudioModeTarget::ToolRule { .. }
            | PermissionStudioModeTarget::ToolCommandPattern { .. } => "permission-studio-rule-key",
        },
    )
}

pub(crate) fn permission_studio_mode_target_input_text(
    dialog: &PermissionStudioOverlay,
    target: &PermissionStudioModeTarget,
) -> String {
    if target.is_path() {
        return permission_studio_path_target_value(&dialog.permission, target)
            .map(path_permission_mode_token)
            .unwrap_or_default()
            .to_owned();
    }
    let permission = super::permission_studio_local_config(dialog);
    // A local wildcard group replaces the parent group as a whole. Without a
    // read-only class it uses Allow, even when the parent class is restrictive.
    if matches!(target, PermissionStudioModeTarget::ToolCommandPattern { tool_name, pattern }
        if tool_name == "*" && pattern == "read-only")
        && matches!(
            permission
                .tools
                .as_ref()
                .and_then(|tools| tools.rules.get("*")),
            Some(ToolPermissionRules::Ordered(_))
        )
        && super::read_only_class_mode(&permission).is_none()
        && !super::invalid_read_only_class(&permission)
    {
        return permission_mode_token_text(Some(agena_domain::PermissionMode::Allow));
    }
    permission_mode_token_text(permission_studio_mode_target_value(&permission, target))
}

pub(crate) fn permission_studio_text_target_label(
    i18n: &I18n,
    target: &PermissionStudioTextTarget,
) -> String {
    ui_text::t(
        i18n,
        match target {
            PermissionStudioTextTarget::PathRulePattern { .. } => "permission-studio-rule-path",
            PermissionStudioTextTarget::ToolNameKey { .. }
            | PermissionStudioTextTarget::ToolRuleName { .. } => "permission-studio-rule-key",
        },
    )
}

pub(crate) fn permission_studio_text_target_input_text(
    target: &PermissionStudioTextTarget,
) -> String {
    match target {
        PermissionStudioTextTarget::PathRulePattern { pattern }
        | PermissionStudioTextTarget::ToolNameKey { key: pattern }
        | PermissionStudioTextTarget::ToolRuleName { tool_name: pattern } => pattern.clone(),
    }
}

pub(crate) fn permission_studio_creator_spec(
    i18n: &I18n,
    action: &PermissionStudioEditorAction,
) -> (String, String) {
    match action {
        PermissionStudioEditorAction::AddPathRule => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-path-rule").as_str(),
            ),
            i18n.text("permission-studio-path-help"),
        ),
        PermissionStudioEditorAction::AddToolName => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-name").as_str(),
            ),
            String::new(),
        ),
        PermissionStudioEditorAction::AddToolCommandPattern => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-command-rule").as_str(),
            ),
            i18n.text("permission-studio-command-pattern-help"),
        ),
        _ => (String::new(), String::new()),
    }
}

pub(crate) fn permission_studio_creator_input_text(
    _action: &PermissionStudioEditorAction,
) -> String {
    String::new()
}

pub(crate) fn apply_permission_studio_mode_input(
    i18n: &I18n,
    permission: &mut PermissionConfig,
    target: &PermissionStudioModeTarget,
    input: &str,
) -> UiResult<()> {
    if target.is_path() {
        let mode = parse_permission_studio_optional_path_mode_input(i18n, input)
            .map_err(crate::UiFailure::message)?;
        match target {
            PermissionStudioModeTarget::PathWorkspace => {
                set_path_default_mode(permission, false, mode)
            }
            PermissionStudioModeTarget::PathExternal => {
                set_path_default_mode(permission, true, mode)
            }
            PermissionStudioModeTarget::PathRule { pattern } => {
                let path = permission.path.get_or_insert_with(Default::default);
                if let Some(mode) = mode {
                    path.rules
                        .insert(pattern.clone(), PathAccessModes::from_mode(mode));
                } else {
                    path.rules.shift_remove(pattern);
                }
            }
            _ => unreachable!("path target was checked"),
        }
        normalize_permission_config(permission);
        return Ok(());
    }
    let mode = parse_permission_studio_optional_mode_input(i18n, input)
        .map_err(crate::UiFailure::message)?;
    if target.is_network() {
        let action = match mode {
            None => None,
            Some(agena_domain::PermissionMode::Allow) => Some(agena_domain::NetworkAction::Allow),
            Some(agena_domain::PermissionMode::Deny) => Some(agena_domain::NetworkAction::Deny),
            _ => {
                return Err(crate::UiFailure::message(
                    i18n.text("permission-studio-network-invalid-action"),
                ));
            }
        };
        let network = permission.network.get_or_insert_with(Default::default);
        match target {
            PermissionStudioModeTarget::NetworkEgressDefault => {
                network.egress.get_or_insert_with(Default::default).default = action
            }
            PermissionStudioModeTarget::NetworkIngressDefault => {
                network.ingress.get_or_insert_with(Default::default).default = action
            }
            PermissionStudioModeTarget::NetworkHostLoopback => {
                network
                    .ingress
                    .get_or_insert_with(Default::default)
                    .host_loopback = action
            }
            _ => unreachable!("network target was checked"),
        }
        normalize_permission_config(permission);
        return Ok(());
    }
    match target {
        PermissionStudioModeTarget::PathWorkspace
        | PermissionStudioModeTarget::PathExternal
        | PermissionStudioModeTarget::PathRule { .. } => unreachable!("path modes handled above"),
        PermissionStudioModeTarget::NetworkEgressDefault
        | PermissionStudioModeTarget::NetworkIngressDefault
        | PermissionStudioModeTarget::NetworkHostLoopback => {
            unreachable!("network modes handled above")
        }
        PermissionStudioModeTarget::ToolDefault => {
            permission
                .tools
                .get_or_insert_with(Default::default)
                .default = mode;
        }
        PermissionStudioModeTarget::ToolName { key } => {
            if let Some(mode) = mode {
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .names
                    .insert(key.clone(), mode);
            } else if let Some(tools) = permission.tools.as_mut() {
                tools.names.remove(key.as_str());
            }
        }
        PermissionStudioModeTarget::ToolRule { tool_name } => {
            if let Some(mode) = mode {
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .rules
                    .insert(tool_name.clone(), ToolPermissionRules::Mode(mode));
            } else if let Some(tools) = permission.tools.as_mut() {
                tools.rules.remove(tool_name.as_str());
            }
        }
        PermissionStudioModeTarget::ToolCommandPattern { tool_name, pattern } => {
            let tools = permission.tools.get_or_insert_with(Default::default);
            let entries = match tools.rules.remove(tool_name.as_str()) {
                Some(ToolPermissionRules::Ordered(entries)) => entries,
                Some(ToolPermissionRules::Mode(existing)) => {
                    let mut entries = indexmap::IndexMap::new();
                    entries.insert("*".to_string(), existing);
                    entries
                }
                None => indexmap::IndexMap::new(),
            };
            let mut entries = entries;
            if let Some(mode) = mode {
                entries.insert(pattern.clone(), mode);
            } else {
                entries.shift_remove(pattern.as_str());
            }
            if entries.is_empty() && tool_name != SHELL_COMMAND_RULE_GROUP {
                tools.rules.remove(tool_name.as_str());
            } else {
                tools
                    .rules
                    .insert(tool_name.clone(), ToolPermissionRules::Ordered(entries));
            }
        }
    }
    normalize_permission_config(permission);
    Ok(())
}

pub(crate) fn apply_permission_studio_network_input(
    permission: &mut PermissionConfig,
    input: &str,
) -> UiResult<()> {
    let network: agena_domain::NetworkPermissionConfig = serde_json::from_str(input)
        .map_err(|error| crate::UiFailure::message(error.to_string()))?;
    // The TUI can connect to a host running a different OS. Validate the shared
    // shape here; the server checks its native backend before saving.
    network.validate().map_err(crate::UiFailure::message)?;
    permission.network = Some(network);
    normalize_permission_config(permission);
    Ok(())
}

pub(crate) fn apply_permission_studio_text_input(
    i18n: &I18n,
    permission: &mut PermissionConfig,
    target: &PermissionStudioTextTarget,
    input: &str,
) -> UiResult<PermissionStudioSectionId> {
    let value = parse_permission_studio_key_input(
        i18n,
        permission_studio_text_target_label(i18n, target).as_str(),
        input,
    )
    .map_err(crate::UiFailure::message)?;
    let section = match target {
        PermissionStudioTextTarget::PathRulePattern { pattern } => {
            let value = agena_domain::canonical_permission_path(&value)
                .map_err(crate::UiFailure::message)?;
            if pattern != &value
                && permission
                    .path
                    .as_ref()
                    .is_some_and(|path| path.rules.contains_key(&value))
            {
                return Err(crate::UiFailure::message(
                    i18n.text("permission-studio-path-duplicate"),
                ));
            }
            rename_path_rule(permission, pattern.as_str(), value.as_str());
            PermissionStudioSectionId::PathRules
        }
        PermissionStudioTextTarget::ToolNameKey { key } => {
            if key != &value
                && permission
                    .tools
                    .as_ref()
                    .is_some_and(|tools| tools.names.contains_key(&value))
            {
                return Err(crate::UiFailure::message(
                    i18n.text("permission-studio-rule-duplicate"),
                ));
            }
            rename_tool_name(permission, key.as_str(), value.as_str());
            PermissionStudioSectionId::ToolNames
        }
        PermissionStudioTextTarget::ToolRuleName { tool_name } => {
            if tool_name != &value
                && permission
                    .tools
                    .as_ref()
                    .is_some_and(|tools| tools.rules.contains_key(&value))
            {
                return Err(crate::UiFailure::message(
                    i18n.text("permission-studio-rule-duplicate"),
                ));
            }
            rename_tool_rule(permission, tool_name.as_str(), value.as_str());
            PermissionStudioSectionId::ToolCommandRules
        }
    };
    normalize_permission_config(permission);
    Ok(section)
}
use crate::{
    I18n, PathAccessModes, PermissionConfig, PermissionStudioAction, PermissionStudioEditorAction,
    PermissionStudioItem, PermissionStudioModeTarget, PermissionStudioOverlay,
    PermissionStudioSection, PermissionStudioSectionId, PermissionStudioTextTarget,
    ToolPermissionRules, UiResult, settings_edit_title, ui_text,
};

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::PermissionMode;

    #[test]
    fn duplicate_rule_rename_keeps_permission_draft() {
        let i18n = I18n::english();
        let mut permission = PermissionConfig::default();
        let tools = permission.tools.get_or_insert_with(Default::default);
        for (name, mode) in [
            ("agena.shell.exec", PermissionMode::Allow),
            ("agena.shell.spawn", PermissionMode::Deny),
        ] {
            tools.names.insert(name.to_string(), mode);
            tools
                .rules
                .insert(name.to_string(), ToolPermissionRules::Mode(mode));
        }
        let original = permission.clone();

        for (target, input) in [
            (
                PermissionStudioTextTarget::ToolNameKey {
                    key: "agena.shell.exec".to_string(),
                },
                "agena.shell.spawn",
            ),
            (
                PermissionStudioTextTarget::ToolRuleName {
                    tool_name: "agena.shell.exec".to_string(),
                },
                "agena.shell.spawn",
            ),
        ] {
            let error = apply_permission_studio_text_input(&i18n, &mut permission, &target, input)
                .unwrap_err();
            assert!(error.to_string().contains("Rule already exists"));
            assert_eq!(permission, original);
        }
    }
    #[test]
    fn invalid_native_network_json_keeps_existing_permission() {
        let mut permission = PermissionConfig {
            network: Some(agena_domain::NetworkPermissionConfig::offline()),
            ..PermissionConfig::default()
        };
        let original = permission.clone();
        for input in [
            r#"{"rules":{"example.com":"allow"}}"#,
            r#"{"allow":[{"to":"*","ports":[]}]}"#,
            r#"{"allow":[{"to":"*","protocol":"icmp","ports":[443]}]}"#,
        ] {
            assert!(apply_permission_studio_network_input(&mut permission, input).is_err());
            assert_eq!(permission, original);
        }
        apply_permission_studio_network_input(
            &mut permission,
            r#"{"default":"deny","allow":[],"deny":[]}"#,
        )
        .unwrap();
        assert_eq!(
            permission.network.unwrap().egress.unwrap().allow,
            Some(vec![])
        );
    }

    #[test]
    fn empty_network_json_is_an_explicit_offline_policy() {
        let mut permission = PermissionConfig {
            network: Some(agena_domain::NetworkPermissionConfig::allow_all()),
            ..PermissionConfig::default()
        };
        apply_permission_studio_network_input(&mut permission, "{}").unwrap();
        normalize_permission_config(&mut permission);
        assert_eq!(
            permission.network,
            Some(agena_domain::NetworkPermissionConfig::offline())
        );
        assert!(!permission.is_empty());
    }

    #[test]
    fn native_network_json_defers_platform_capabilities_to_execution_host() {
        for (input, supported_platform, unsupported_platform) in [
            (
                r#"{"version":2,"default":"deny","allow":[{"to":"192.0.2.10","protocol":"tcp","ports":[443]}]}"#,
                "linux",
                "macos",
            ),
            (
                r#"{"version":2,"default":"deny","inbound":"allow","hostLoopback":"allow"}"#,
                "windows",
                "linux",
            ),
        ] {
            let mut permission = PermissionConfig::default();
            apply_permission_studio_network_input(&mut permission, input).unwrap();
            let network = permission.network.unwrap();
            assert_eq!(
                serde_json::to_value(&network).unwrap(),
                serde_json::from_str::<serde_json::Value>(input).unwrap()
            );
            network.validate_for_platform(supported_platform).unwrap();
            assert!(network.validate_for_platform(unsupported_platform).is_err());
        }
    }
}
