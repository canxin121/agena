use super::{
    normalize_permission_config, parse_permission_studio_key_input,
    parse_permission_studio_optional_mode_input, permission_mode_input_text, permission_mode_label,
    permission_mode_token_text, permission_studio_mode_target_value, rename_network_rule,
    rename_path_rule, rename_tool_name, rename_tool_rule, set_path_default_mode,
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
            let mut rule_items = rules
                .into_iter()
                .flat_map(|pattern| {
                    let modes = dialog
                        .permission
                        .path
                        .as_ref()
                        .and_then(|path| path.rules.get(pattern.as_str()))
                        .cloned();
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
            rule_items.insert(0, permission_studio_new_rule_item(i18n));
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
            let mut rule_items = rules
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
            rule_items.insert(0, permission_studio_new_rule_item(i18n));
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::NetworkRules,
                label: ui_text::t(i18n, "permission-studio-page-network-rules"),
                items: rule_items,
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
            keys.retain(|tool_name| matches!(tool_name.as_str(), "agena.shell.run" | "*"));
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
                                    label: format!("{tool_name} · + command pattern"),
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
                                    label: format!("{tool_name} · + command pattern"),
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
    permission_mode_token_text(permission_studio_mode_target_value(
        &dialog.permission,
        target,
    ))
}

pub(crate) fn permission_studio_text_target_label(
    i18n: &I18n,
    target: &PermissionStudioTextTarget,
) -> String {
    ui_text::t(
        i18n,
        match target {
            PermissionStudioTextTarget::PathRulePattern { .. } => "permission-studio-rule-pattern",
            PermissionStudioTextTarget::NetworkRuleTarget { .. } => "permission-studio-rule-target",
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
        | PermissionStudioTextTarget::NetworkRuleTarget { target: pattern }
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
            String::new(),
        ),
        PermissionStudioEditorAction::AddNetworkRule => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-network-rule").as_str(),
            ),
            String::new(),
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
    let mode = parse_permission_studio_optional_mode_input(i18n, input)
        .map_err(crate::UiFailure::message)?;
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
                .cloned()
                .unwrap_or(PathAccessModes {
                    read: Some(PermissionMode::Auto),
                    write: Some(PermissionMode::Auto),
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
                .insert(pattern.clone(), next);
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
            rename_path_rule(permission, pattern.as_str(), value.as_str());
            PermissionStudioPage::PathRules
        }
        PermissionStudioTextTarget::NetworkRuleTarget { target } => {
            rename_network_rule(permission, target.as_str(), value.as_str());
            PermissionStudioPage::NetworkRules
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
use crate::{
    I18n, PathAccessModes, PermissionConfig, PermissionMode, PermissionStudioAction,
    PermissionStudioEditorAction, PermissionStudioItem, PermissionStudioModeTarget,
    PermissionStudioOverlay, PermissionStudioPage, PermissionStudioSection,
    PermissionStudioSectionId, PermissionStudioTextTarget, ToolPermissionRules, UiResult,
    settings_edit_title, ui_text,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PermissionStudioFocus, PermissionStudioPaneFocus, PermissionStudioSource,
        SectionedListState, SelectableListState,
    };

    fn empty_dialog(page: PermissionStudioPage) -> PermissionStudioOverlay {
        PermissionStudioOverlay {
            title: String::new(),
            footer: String::new(),
            source: PermissionStudioSource::GlobalConfig,
            title_context: String::new(),
            source_label: String::new(),
            scope_label: String::new(),
            editable: true,
            permission: PermissionConfig::default(),
            nav: SelectableListState::new(Vec::new(), 0),
            pane_focus: PermissionStudioPaneFocus::Content,
            page,
            state: SectionedListState::new(Vec::new(), 0, 0, PermissionStudioFocus::Items),
            editor: None,
        }
    }

    #[test]
    fn empty_rule_pages_expose_new_rule_as_the_first_item() {
        let i18n = I18n::default();
        for (page, rule_section) in [
            (
                PermissionStudioPage::PathRules,
                PermissionStudioSectionId::PathRules,
            ),
            (
                PermissionStudioPage::NetworkRules,
                PermissionStudioSectionId::NetworkRules,
            ),
            (
                PermissionStudioPage::ToolNames,
                PermissionStudioSectionId::ToolNames,
            ),
            (
                PermissionStudioPage::ToolCommandRules,
                PermissionStudioSectionId::ToolCommandRules,
            ),
        ] {
            let sections = permission_studio_sections(&i18n, &empty_dialog(page.clone()));
            let section = sections
                .iter()
                .find(|section| section.id == rule_section)
                .unwrap_or_else(|| panic!("page {page:?} has its rule section"));
            assert!(
                matches!(
                    section.items.first().map(|item| &item.action),
                    Some(PermissionStudioAction::CreateRule)
                ),
                "page {page:?} should expose + New Rule even when empty"
            );
        }
    }

    #[test]
    fn the_shipped_defaults_show_up_as_ordinary_rules() {
        // The class defaults are not keys any more: each one is an entry of the
        // collection its page already renders, so a page built from the shipped
        // configuration shows them as rule rows - no separate section.
        let i18n = I18n::default();
        let shipped = PermissionConfig::global_default();

        let mut dialog = empty_dialog(PermissionStudioPage::ToolCommandRules);
        dialog.permission = shipped.clone();
        let sections = permission_studio_sections(&i18n, &dialog);
        assert_eq!(
            sections.len(),
            1,
            "the command pages are one section now, not rules plus class defaults"
        );
        let labels = sections[0]
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        for expected in [
            "agena.shell.run · no-op",
            "agena.shell.run · routine",
            "agena.shell.run · dangerous",
            "* · read-only",
        ] {
            assert!(
                labels.contains(&expected),
                "the shipped defaults render as rule rows: {labels:?}"
            );
        }

        let mut dialog = empty_dialog(PermissionStudioPage::ToolNames);
        dialog.permission = shipped.clone();
        let sections = permission_studio_sections(&i18n, &dialog);
        let names = sections
            .iter()
            .find(|section| section.id == PermissionStudioSectionId::ToolNames)
            .expect("the tool names section is present");
        assert!(
            names
                .items
                .iter()
                .any(|item| item.label == "agena.interaction.ask"),
            "the interaction tool is a `tools.names` entry"
        );

        let mut dialog = empty_dialog(PermissionStudioPage::PathRules);
        dialog.permission = shipped;
        let sections = permission_studio_sections(&i18n, &dialog);
        let rules = sections
            .iter()
            .find(|section| section.id == PermissionStudioSectionId::PathRules)
            .expect("the path rules section is present");
        assert!(
            rules
                .items
                .iter()
                .any(|item| item.label.contains("/agena/projects")),
            "the runtime state paths are `path.rules` entries"
        );
    }

    #[test]
    fn an_emptied_command_rule_set_falls_back_to_the_built_in() {
        // Clearing the last keyword of a rule set drops the entry rather than
        // writing an empty one, so the stored config keeps meaning "the
        // built-in entry of that name applies".
        let i18n = I18n::default();
        let mut permission = PermissionConfig::global_default();
        let target = PermissionStudioModeTarget::ToolCommandPattern {
            tool_name: "*".to_owned(),
            pattern: agena_domain::TOOL_CLASS_READ_ONLY.to_owned(),
        };
        apply_permission_studio_mode_input(&i18n, &mut permission, &target, "").unwrap();
        assert!(
            !permission
                .tools
                .as_ref()
                .expect("tools section")
                .rules
                .contains_key("*"),
            "an emptied rule set is dropped"
        );

        // Writing it back restores an ordinary override.
        apply_permission_studio_mode_input(&i18n, &mut permission, &target, "auto").unwrap();
        assert!(matches!(
            permission
                .tools
                .as_ref()
                .expect("tools section")
                .rules
                .get("*"),
            Some(ToolPermissionRules::Ordered(entries))
                if entries.get("read-only") == Some(&PermissionMode::Auto)
        ));
    }
}
