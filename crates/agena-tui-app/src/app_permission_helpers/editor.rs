use super::{
    normalize_permission_config, parse_permission_studio_key_input,
    parse_permission_studio_optional_mode_input, parse_permission_studio_optional_path_mode_input,
    path_access_modes_summary, path_permission_mode_token, permission_mode_input_text,
    permission_mode_label, permission_mode_token_text, permission_studio_mode_target_value,
    permission_studio_path_target_value, rename_path_rule, rename_tool_name, rename_tool_rule,
    set_path_default_mode,
};

pub(crate) fn permission_studio_sections(
    i18n: &I18n,
    dialog: &PermissionStudioOverlay,
) -> Vec<PermissionStudioSection> {
    match &dialog.page {
        PermissionStudioPage::PathDefaults => vec![PermissionStudioSection {
            id: PermissionStudioSectionId::PathDefaults,
            label: ui_text::t(i18n, "permission-studio-page-path-defaults"),
            items: vec![
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-workspace"),
                    value: permission_studio_path_default_summary(i18n, dialog, false),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathWorkspace,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-external"),
                    value: permission_studio_path_default_summary(i18n, dialog, true),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathExternal,
                    ),
                },
            ],
        }],
        PermissionStudioPage::PathRules => {
            let mut rules = dialog
                .permission
                .path
                .as_ref()
                .map(|path| path.rules.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            rules.sort();
            let mut rule_items = rules
                .into_iter()
                .map(|pattern| {
                    let modes = dialog
                        .permission
                        .path
                        .as_ref()
                        .and_then(|path| path.rules.get(pattern.as_str()));
                    PermissionStudioItem {
                        label: pattern.clone(),
                        value: path_access_modes_summary(i18n, modes),
                        action: PermissionStudioAction::EditMode(
                            PermissionStudioModeTarget::PathRule { pattern },
                        ),
                    }
                })
                .collect::<Vec<_>>();
            rule_items.insert(0, permission_studio_new_rule_item(i18n));
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::PathRules,
                label: ui_text::t(i18n, "permission-studio-page-path-rules"),
                items: rule_items,
            }]
        }
        PermissionStudioPage::NetworkDefaults => {
            super::permission_studio_network_defaults(i18n, dialog)
        }
        PermissionStudioPage::NetworkRules => super::permission_studio_network_rules(i18n, dialog),
        PermissionStudioPage::ToolNames => {
            let mut keys = dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.names.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            keys.sort();
            let mut name_items = keys
                .into_iter()
                .map(|key| PermissionStudioItem {
                    label: key.clone(),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.names.get(key.as_str()).copied()),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::ToolName { key },
                    ),
                })
                .collect::<Vec<_>>();
            name_items.insert(0, permission_studio_new_rule_item(i18n));
            vec![
                PermissionStudioSection {
                    id: PermissionStudioSectionId::ToolNames,
                    label: ui_text::t(i18n, "permission-studio-page-names"),
                    items: name_items,
                },
                // What a tool the table above does not name falls back to. It
                // sits in its own section so the two kinds of row - "one
                // specific tool" and "everything else" - stay visually apart.
                PermissionStudioSection {
                    id: PermissionStudioSectionId::ToolDefaults,
                    label: ui_text::t(i18n, "permission-studio-page-tool-defaults"),
                    items: vec![PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-tool-default"),
                        value: permission_mode_input_text(
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .and_then(|tools| tools.default),
                            i18n,
                        ),
                        action: PermissionStudioAction::EditMode(
                            PermissionStudioModeTarget::ToolDefault,
                        ),
                    }],
                },
            ]
        }
        PermissionStudioPage::ToolCommandRules => {
            // Shell-capable tools can be restricted by command pattern
            // (including the command classes `no-op` / `routine` /
            // `dangerous`). The `*` tool name is not a shell tool: it carries
            // the read-only class, which applies to every tool whose declared
            // tags say read-only.
            let mut keys = dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.rules.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            keys.retain(|tool_name| {
                matches!(
                    tool_name.as_str(),
                    "agena.shell.exec"
                        | "agena.shell.spawn"
                        | "agena.shell.watch"
                        | "agena.shell.open"
                        | "agena.shell.write"
                        | "*"
                )
            });
            keys.sort();
            keys.sort_by_key(|tool_name| tool_name == "*");
            let mut tool_rule_items = keys
                .into_iter()
                .flat_map(|tool_name| {
                    let wildcard = tool_name == "*";
                    let rules = dialog
                        .permission
                        .tools
                        .as_ref()
                        .and_then(|tools| tools.rules.get(tool_name.as_str()));
                    match rules {
                        Some(ToolPermissionRules::Ordered(entries)) => {
                            let mut items = entries
                                .iter()
                                .map(|(pattern, mode)| PermissionStudioItem {
                                    label: format!("{tool_name} · {pattern}"),
                                    value: permission_mode_label(i18n, *mode),
                                    action: PermissionStudioAction::EditMode(
                                        PermissionStudioModeTarget::ToolCommandPattern {
                                            tool_name: tool_name.clone(),
                                            pattern: pattern.clone(),
                                        },
                                    ),
                                })
                                .collect::<Vec<_>>();
                            if !wildcard {
                                items.push(PermissionStudioItem {
                                    label: format!(
                                        "{tool_name} · {}",
                                        ui_text::t(
                                            i18n,
                                            "permission-studio-command-pattern-suffix"
                                        )
                                    ),
                                    value: ui_text::t(i18n, "value-add"),
                                    action: PermissionStudioAction::AddToolCommandPattern {
                                        tool_name,
                                    },
                                });
                            }
                            items
                        }
                        Some(ToolPermissionRules::Mode(mode)) if !wildcard => {
                            vec![
                                PermissionStudioItem {
                                    label: format!("{tool_name} · *"),
                                    value: permission_mode_label(i18n, *mode),
                                    action: PermissionStudioAction::EditMode(
                                        PermissionStudioModeTarget::ToolCommandPattern {
                                            tool_name: tool_name.clone(),
                                            pattern: "*".to_string(),
                                        },
                                    ),
                                },
                                PermissionStudioItem {
                                    label: format!(
                                        "{tool_name} · {}",
                                        ui_text::t(
                                            i18n,
                                            "permission-studio-command-pattern-suffix"
                                        )
                                    ),
                                    value: ui_text::t(i18n, "value-add"),
                                    action: PermissionStudioAction::AddToolCommandPattern {
                                        tool_name,
                                    },
                                },
                            ]
                        }
                        _ => Vec::new(),
                    }
                })
                .collect::<Vec<_>>();
            tool_rule_items.insert(0, permission_studio_new_rule_item(i18n));
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::ToolCommandRules,
                label: ui_text::t(i18n, "permission-studio-page-tool-rules"),
                items: tool_rule_items,
            }]
        }
    }
}

fn permission_studio_path_default_summary(
    i18n: &I18n,
    dialog: &PermissionStudioOverlay,
    external: bool,
) -> String {
    let selected = dialog.permission.path.as_ref();
    let parent = dialog.inherited_permission.path.as_ref();
    let (value, inherited) = if external {
        (
            selected.and_then(|path| path.external.as_ref()),
            parent.and_then(|path| path.external.as_ref()),
        )
    } else {
        (
            selected.and_then(|path| path.workspace.as_ref()),
            parent.and_then(|path| path.workspace.as_ref()),
        )
    };
    if value.is_some() {
        return path_access_modes_summary(i18n, value);
    }
    format!(
        "{} · {}",
        ui_text::t(i18n, "value-inherit"),
        path_access_modes_summary(i18n, inherited),
    )
}

fn permission_studio_new_rule_item(i18n: &I18n) -> PermissionStudioItem {
    PermissionStudioItem {
        label: ui_text::t(i18n, "permission-studio-new-rule-label"),
        value: ui_text::t(i18n, "permission-studio-new-rule-value"),
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
    let permission = if target.is_network() {
        super::permission_studio_custom_network(dialog)
    } else {
        dialog.permission.clone()
    };
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
        PermissionStudioEditorAction::AddToolRule => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-tool-rule").as_str(),
            ),
            String::new(),
        ),
        PermissionStudioEditorAction::AddToolCommandPattern { tool_name } => (
            settings_edit_title(
                i18n,
                i18n.text_args(
                    "permission-studio-command-pattern-title",
                    &agena_tui::fl_args!("tool_name" => tool_name.clone()),
                )
                .as_str(),
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
            None => Some(agena_domain::NetworkAction::Deny),
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
            // An emptied rule set is dropped so the entry falls back to the
            // built-in one of the same name (or, for a tool the built-ins say
            // nothing about, to `tools.default`).
            if entries.is_empty() {
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
    Ok(())
}

pub(crate) fn apply_permission_studio_text_input(
    i18n: &I18n,
    permission: &mut PermissionConfig,
    target: &PermissionStudioTextTarget,
    input: &str,
) -> UiResult<PermissionStudioPage> {
    let value = parse_permission_studio_key_input(
        i18n,
        permission_studio_text_target_label(i18n, target).as_str(),
        input,
    )
    .map_err(crate::UiFailure::message)?;
    let page = match target {
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
            PermissionStudioPage::PathRules
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
            PermissionStudioPage::ToolNames
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
            PermissionStudioPage::ToolCommandRules
        }
    };
    normalize_permission_config(permission);
    Ok(page)
}
use crate::{
    I18n, PathAccessModes, PermissionConfig, PermissionStudioAction, PermissionStudioEditorAction,
    PermissionStudioItem, PermissionStudioModeTarget, PermissionStudioOverlay,
    PermissionStudioPage, PermissionStudioSection, PermissionStudioSectionId,
    PermissionStudioTextTarget, ToolPermissionRules, UiResult, settings_edit_title, ui_text,
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
                r#"{"default":"deny","allow":[{"to":"192.0.2.10","protocol":"tcp","ports":[443]}]}"#,
                "linux",
                "macos",
            ),
            (
                r#"{"default":"deny","inbound":"allow","hostLoopback":"allow"}"#,
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
