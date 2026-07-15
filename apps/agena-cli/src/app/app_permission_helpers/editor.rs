use super::{
    network_defaults_summary, normalize_permission_config, parse_permission_studio_key_input,
    parse_permission_studio_optional_mode_input, path_access_modes_summary, path_rule_modes,
    permission_mode_input_text, permission_mode_label, permission_mode_token_text,
    permission_rule_count_summary, permission_studio_mode_target_value, rename_network_rule,
    rename_path_rule, rename_tool_name, rename_tool_rule, rename_tool_tag, set_path_default_mode,
    tool_permission_rules_summary,
};

pub(in crate::app) fn permission_studio_sections(
    i18n: &I18n,
    dialog: &PermissionStudioOverlay,
) -> Vec<PermissionStudioSection> {
    match &dialog.page {
        PermissionStudioPage::PathDefaults => vec![PermissionStudioSection {
            id: PermissionStudioSectionId::PathDefaults,
            label: ui_text::t(i18n, "permission-studio-page-path-defaults"),
            items: vec![
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-workspace-read"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .path
                            .as_ref()
                            .and_then(|path| path.workspace.as_ref())
                            .and_then(|modes| modes.read),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathWorkspaceRead,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-workspace-write"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .path
                            .as_ref()
                            .and_then(|path| path.workspace.as_ref())
                            .and_then(|modes| modes.write),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathWorkspaceWrite,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-external-read"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .path
                            .as_ref()
                            .and_then(|path| path.external.as_ref())
                            .and_then(|modes| modes.read),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathExternalRead,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-external-write"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .path
                            .as_ref()
                            .and_then(|path| path.external.as_ref())
                            .and_then(|modes| modes.write),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathExternalWrite,
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
            let rule_items = rules
                .into_iter()
                .flat_map(|pattern| {
                    let modes = dialog
                        .permission
                        .path
                        .as_ref()
                        .and_then(|path| path.rules.get(pattern.as_str()))
                        .and_then(|rule| path_rule_modes(Some(rule)));
                    vec![
                        PermissionStudioItem {
                            label: format!("{pattern} · read"),
                            value: permission_mode_input_text(
                                modes.as_ref().and_then(|modes| modes.read),
                                i18n,
                            ),
                            action: PermissionStudioAction::EditMode(
                                PermissionStudioModeTarget::PathRuleRead {
                                    pattern: pattern.clone(),
                                },
                            ),
                        },
                        PermissionStudioItem {
                            label: format!("{pattern} · write"),
                            value: permission_mode_input_text(
                                modes.as_ref().and_then(|modes| modes.write),
                                i18n,
                            ),
                            action: PermissionStudioAction::EditMode(
                                PermissionStudioModeTarget::PathRuleWrite { pattern },
                            ),
                        },
                    ]
                })
                .collect::<Vec<_>>();
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::PathRules,
                label: ui_text::t(i18n, "permission-studio-page-path-rules"),
                items: rule_items,
            }]
        }
        PermissionStudioPage::NetworkZones => vec![PermissionStudioSection {
            id: PermissionStudioSectionId::NetworkZones,
            label: ui_text::t(i18n, "permission-studio-page-network-zones"),
            items: vec![
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-network-internet"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .network
                            .as_ref()
                            .and_then(|network| network.internet),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::NetworkInternet,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-network-private"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .network
                            .as_ref()
                            .and_then(|network| network.private),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::NetworkPrivate,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-network-loopback"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .network
                            .as_ref()
                            .and_then(|network| network.loopback),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::NetworkLoopback,
                    ),
                },
            ],
        }],
        PermissionStudioPage::NetworkRules => {
            let mut rules = dialog
                .permission
                .network
                .as_ref()
                .map(|network| network.rules.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            rules.sort();
            let rule_items = rules
                .into_iter()
                .map(|target| PermissionStudioItem {
                    label: target.clone(),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .network
                            .as_ref()
                            .and_then(|network| network.rules.get(target.as_str()).copied()),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::NetworkRule { target },
                    ),
                })
                .collect::<Vec<_>>();
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::NetworkRules,
                label: ui_text::t(i18n, "permission-studio-page-network-rules"),
                items: rule_items,
            }]
        }
        PermissionStudioPage::ToolTags => {
            let mut keys = dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.tags.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            keys.sort();
            let mut tag_items = vec![PermissionStudioItem {
                label: ui_text::t(i18n, "permission-studio-tool-default"),
                value: permission_mode_input_text(
                    dialog
                        .permission
                        .tools
                        .as_ref()
                        .and_then(|tools| tools.default),
                    i18n,
                ),
                action: PermissionStudioAction::EditMode(PermissionStudioModeTarget::ToolDefault),
            }];
            tag_items.extend(
                keys.into_iter()
                    .map(|key| PermissionStudioItem {
                        label: key.clone(),
                        value: permission_mode_input_text(
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .and_then(|tools| tools.tags.get(key.as_str()).copied()),
                            i18n,
                        ),
                        action: PermissionStudioAction::EditMode(
                            PermissionStudioModeTarget::ToolTag { key },
                        ),
                    })
                    .collect::<Vec<_>>(),
            );
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::ToolTags,
                label: ui_text::t(i18n, "permission-studio-page-tags"),
                items: tag_items,
            }]
        }
        PermissionStudioPage::ToolNames => {
            let mut keys = dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.names.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            keys.sort();
            let name_items = keys
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
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::ToolNames,
                label: ui_text::t(i18n, "permission-studio-page-names"),
                items: name_items,
            }]
        }
        PermissionStudioPage::ToolCommandRules => {
            let mut keys = dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.rules.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            keys.sort();
            let tool_rule_items = keys
                .into_iter()
                .flat_map(|tool_name| {
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
                            items.push(PermissionStudioItem {
                                label: format!("{tool_name} · + command pattern"),
                                value: ui_text::t(i18n, "value-add"),
                                action: PermissionStudioAction::AddToolCommandPattern { tool_name },
                            });
                            items
                        }
                        Some(ToolPermissionRules::Mode(mode))
                            if matches!(
                                tool_name.as_str(),
                                "shell" | "bash" | "agena.shell.run" | "agena.process.run"
                            ) =>
                        {
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
                                    label: format!("{tool_name} · + command pattern"),
                                    value: ui_text::t(i18n, "value-add"),
                                    action: PermissionStudioAction::AddToolCommandPattern {
                                        tool_name,
                                    },
                                },
                            ]
                        }
                        _ => vec![PermissionStudioItem {
                            label: tool_name.clone(),
                            value: tool_permission_rules_summary(i18n, rules),
                            action: PermissionStudioAction::EditMode(
                                PermissionStudioModeTarget::ToolRule { tool_name },
                            ),
                        }],
                    }
                })
                .collect::<Vec<_>>();
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::ToolCommandRules,
                label: ui_text::t(i18n, "permission-studio-page-tool-rules"),
                items: tool_rule_items,
            }]
        }
        PermissionStudioPage::Overview => vec![
            PermissionStudioSection {
                id: PermissionStudioSectionId::RootPath,
                label: ui_text::t(i18n, "permission-studio-page-path"),
                items: vec![
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-workspace"),
                        value: path_access_modes_summary(
                            i18n,
                            dialog
                                .permission
                                .path
                                .as_ref()
                                .and_then(|path| path.workspace.as_ref()),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-external"),
                        value: path_access_modes_summary(
                            i18n,
                            dialog
                                .permission
                                .path
                                .as_ref()
                                .and_then(|path| path.external.as_ref()),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-rules"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .path
                                .as_ref()
                                .map(|path| path.rules.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                ],
            },
            PermissionStudioSection {
                id: PermissionStudioSectionId::RootNetwork,
                label: ui_text::t(i18n, "permission-studio-page-network"),
                items: vec![
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-defaults"),
                        value: network_defaults_summary(i18n, dialog.permission.network.as_ref()),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-rules"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .network
                                .as_ref()
                                .map(|network| network.rules.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                ],
            },
            PermissionStudioSection {
                id: PermissionStudioSectionId::RootTools,
                label: ui_text::t(i18n, "permission-studio-page-tools"),
                items: vec![
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-tool-default"),
                        value: permission_mode_input_text(
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .and_then(|tools| tools.default),
                            i18n,
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-page-tags"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .map(|tools| tools.tags.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-page-names"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .map(|tools| tools.names.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-page-tool-rules"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .map(|tools| tools.rules.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                ],
            },
        ],
    }
}

pub(in crate::app) fn permission_studio_mode_target_label(
    i18n: &I18n,
    target: &PermissionStudioModeTarget,
) -> String {
    ui_text::t(
        i18n,
        match target {
            PermissionStudioModeTarget::PathWorkspaceRead => {
                "permission-studio-path-workspace-read"
            }
            PermissionStudioModeTarget::PathWorkspaceWrite => {
                "permission-studio-path-workspace-write"
            }
            PermissionStudioModeTarget::PathExternalRead => "permission-studio-path-external-read",
            PermissionStudioModeTarget::PathExternalWrite => {
                "permission-studio-path-external-write"
            }
            PermissionStudioModeTarget::NetworkInternet => "permission-studio-network-internet",
            PermissionStudioModeTarget::NetworkPrivate => "permission-studio-network-private",
            PermissionStudioModeTarget::NetworkLoopback => "permission-studio-network-loopback",
            PermissionStudioModeTarget::ToolDefault => "permission-studio-tool-default",
            PermissionStudioModeTarget::PathRuleRead { .. } => "permission-studio-rule-pattern",
            PermissionStudioModeTarget::PathRuleWrite { .. } => "permission-studio-rule-pattern",
            PermissionStudioModeTarget::NetworkRule { .. } => "permission-studio-rule-target",
            PermissionStudioModeTarget::ToolTag { .. }
            | PermissionStudioModeTarget::ToolName { .. }
            | PermissionStudioModeTarget::ToolRule { .. }
            | PermissionStudioModeTarget::ToolCommandPattern { .. } => "permission-studio-rule-key",
        },
    )
}

pub(in crate::app) fn permission_studio_mode_target_input_text(
    dialog: &PermissionStudioOverlay,
    target: &PermissionStudioModeTarget,
) -> String {
    permission_mode_token_text(permission_studio_mode_target_value(
        &dialog.permission,
        target,
    ))
}

pub(in crate::app) fn permission_studio_text_target_label(
    i18n: &I18n,
    target: &PermissionStudioTextTarget,
) -> String {
    ui_text::t(
        i18n,
        match target {
            PermissionStudioTextTarget::PathRulePattern { .. } => "permission-studio-rule-pattern",
            PermissionStudioTextTarget::NetworkRuleTarget { .. } => "permission-studio-rule-target",
            PermissionStudioTextTarget::ToolTagKey { .. }
            | PermissionStudioTextTarget::ToolNameKey { .. }
            | PermissionStudioTextTarget::ToolRuleName { .. } => "permission-studio-rule-key",
        },
    )
}

pub(in crate::app) fn permission_studio_text_target_input_text(
    target: &PermissionStudioTextTarget,
) -> String {
    match target {
        PermissionStudioTextTarget::PathRulePattern { pattern }
        | PermissionStudioTextTarget::NetworkRuleTarget { target: pattern }
        | PermissionStudioTextTarget::ToolTagKey { key: pattern }
        | PermissionStudioTextTarget::ToolNameKey { key: pattern }
        | PermissionStudioTextTarget::ToolRuleName { tool_name: pattern } => pattern.clone(),
    }
}

pub(in crate::app) fn permission_studio_creator_spec(
    i18n: &I18n,
    action: &PermissionStudioEditorAction,
) -> (String, String) {
    match action {
        PermissionStudioEditorAction::AddPathRule { .. } => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-path-rule").as_str(),
            ),
            String::new(),
        ),
        PermissionStudioEditorAction::AddNetworkRule { .. } => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-network-rule").as_str(),
            ),
            String::new(),
        ),
        PermissionStudioEditorAction::AddToolTag { .. } => (
            settings_edit_title(i18n, ui_text::t(i18n, "permission-studio-add-tag").as_str()),
            String::new(),
        ),
        PermissionStudioEditorAction::AddToolName { .. } => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-name").as_str(),
            ),
            String::new(),
        ),
        PermissionStudioEditorAction::AddToolRule { .. } => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-tool-rule").as_str(),
            ),
            String::new(),
        ),
        PermissionStudioEditorAction::AddToolCommandPattern { tool_name } => (
            settings_edit_title(i18n, format!("{tool_name} command pattern").as_str()),
            "Enter a shell command glob, for example `git status` or `git push *`.".to_string(),
        ),
        _ => (String::new(), String::new()),
    }
}

pub(in crate::app) fn permission_studio_creator_input_text(
    action: &PermissionStudioEditorAction,
) -> String {
    match action {
        PermissionStudioEditorAction::AddPathRule { duplicate_from }
        | PermissionStudioEditorAction::AddNetworkRule { duplicate_from }
        | PermissionStudioEditorAction::AddToolTag { duplicate_from }
        | PermissionStudioEditorAction::AddToolName { duplicate_from }
        | PermissionStudioEditorAction::AddToolRule { duplicate_from } => {
            duplicate_from.clone().unwrap_or_default()
        }
        PermissionStudioEditorAction::AddToolCommandPattern { .. } => String::new(),
        _ => String::new(),
    }
}

pub(in crate::app) fn apply_permission_studio_mode_input(
    i18n: &I18n,
    permission: &mut PermissionConfig,
    target: &PermissionStudioModeTarget,
    input: &str,
) -> UiResult<()> {
    let mode = parse_permission_studio_optional_mode_input(i18n, input)?;
    match target {
        PermissionStudioModeTarget::PathWorkspaceRead => {
            set_path_default_mode(permission, false, true, mode);
        }
        PermissionStudioModeTarget::PathWorkspaceWrite => {
            set_path_default_mode(permission, false, false, mode);
        }
        PermissionStudioModeTarget::PathExternalRead => {
            set_path_default_mode(permission, true, true, mode);
        }
        PermissionStudioModeTarget::PathExternalWrite => {
            set_path_default_mode(permission, true, false, mode);
        }
        PermissionStudioModeTarget::NetworkInternet => {
            permission
                .network
                .get_or_insert_with(Default::default)
                .internet = mode;
        }
        PermissionStudioModeTarget::NetworkPrivate => {
            permission
                .network
                .get_or_insert_with(Default::default)
                .private = mode;
        }
        PermissionStudioModeTarget::NetworkLoopback => {
            permission
                .network
                .get_or_insert_with(Default::default)
                .loopback = mode;
        }
        PermissionStudioModeTarget::ToolDefault => {
            permission
                .tools
                .get_or_insert_with(Default::default)
                .default = mode;
        }
        PermissionStudioModeTarget::PathRuleRead { pattern }
        | PermissionStudioModeTarget::PathRuleWrite { pattern } => {
            let read = matches!(target, PermissionStudioModeTarget::PathRuleRead { .. });
            let current = permission
                .path
                .as_ref()
                .and_then(|path| path.rules.get(pattern.as_str()))
                .and_then(|rule| path_rule_modes(Some(rule)))
                .unwrap_or(PathAccessModes {
                    read: Some(PermissionMode::Ask),
                    write: Some(PermissionMode::Ask),
                });
            let mut next = current;
            if read {
                next.read = mode;
            } else {
                next.write = mode;
            }
            permission
                .path
                .get_or_insert_with(Default::default)
                .rules
                .insert(pattern.clone(), PathAccessRuleConfig::Modes(next));
        }
        PermissionStudioModeTarget::NetworkRule { target } => {
            if let Some(mode) = mode {
                permission
                    .network
                    .get_or_insert_with(Default::default)
                    .rules
                    .insert(target.clone(), mode);
            } else if let Some(network) = permission.network.as_mut() {
                network.rules.shift_remove(target.as_str());
            }
        }
        PermissionStudioModeTarget::ToolTag { key } => {
            if let Some(mode) = mode {
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .tags
                    .insert(key.clone(), mode);
            } else if let Some(tools) = permission.tools.as_mut() {
                tools.tags.remove(key.as_str());
            }
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
            tools
                .rules
                .insert(tool_name.clone(), ToolPermissionRules::Ordered(entries));
        }
    }
    normalize_permission_config(permission);
    Ok(())
}

pub(in crate::app) fn apply_permission_studio_text_input(
    i18n: &I18n,
    permission: &mut PermissionConfig,
    target: &PermissionStudioTextTarget,
    input: &str,
) -> UiResult<PermissionStudioPage> {
    let value = parse_permission_studio_key_input(
        i18n,
        permission_studio_text_target_label(i18n, target).as_str(),
        input,
    )?;
    let page = match target {
        PermissionStudioTextTarget::PathRulePattern { pattern } => {
            rename_path_rule(permission, pattern.as_str(), value.as_str());
            PermissionStudioPage::PathRules
        }
        PermissionStudioTextTarget::NetworkRuleTarget { target } => {
            rename_network_rule(permission, target.as_str(), value.as_str());
            PermissionStudioPage::NetworkRules
        }
        PermissionStudioTextTarget::ToolTagKey { key } => {
            rename_tool_tag(permission, key.as_str(), value.as_str());
            PermissionStudioPage::ToolTags
        }
        PermissionStudioTextTarget::ToolNameKey { key } => {
            rename_tool_name(permission, key.as_str(), value.as_str());
            PermissionStudioPage::ToolNames
        }
        PermissionStudioTextTarget::ToolRuleName { tool_name } => {
            rename_tool_rule(permission, tool_name.as_str(), value.as_str());
            PermissionStudioPage::ToolCommandRules
        }
    };
    normalize_permission_config(permission);
    Ok(page)
}
use crate::app::{
    I18n, PathAccessModes, PathAccessRuleConfig, PermissionConfig, PermissionMode,
    PermissionStudioAction, PermissionStudioEditorAction, PermissionStudioItem,
    PermissionStudioModeTarget, PermissionStudioOverlay, PermissionStudioPage,
    PermissionStudioSection, PermissionStudioSectionId, PermissionStudioTextTarget,
    ToolPermissionRules, UiResult, settings_edit_title, ui_text,
};
