use crate::{
    I18n, PermissionConfig, PermissionMode, PermissionStudioModeTarget as Target,
    PermissionStudioOverlay, PermissionStudioProjection, PermissionStudioValueSource,
    ToolPermissionRules, normalize_permission_config,
};

/// Source changes use exactly the same replacement boundary as the domain:
/// each network field is independent; each tools.rules entry is a complete group.
pub(crate) fn permission_studio_source_is_local(
    dialog: &PermissionStudioOverlay,
    target: &Target,
) -> bool {
    let projection = PermissionStudioProjection::new(dialog);
    match target {
        Target::ToolRule { tool_name } | Target::ToolCommandPattern { tool_name, .. } => projection
            .selected
            .tools
            .as_ref()
            .is_some_and(|tools| tools.rules.contains_key(tool_name)),
        _ => projection.origin(target) == PermissionStudioValueSource::Current,
    }
}

pub(crate) fn permission_studio_source_config(
    i18n: &I18n,
    dialog: &PermissionStudioOverlay,
    target: &Target,
    local: bool,
) -> Result<PermissionConfig, String> {
    let projection = PermissionStudioProjection::new(dialog);
    if local
        && target.is_network()
        && dialog.inherited_permission.network.is_none()
        && !permission_studio_source_is_local(dialog, target)
    {
        return Err(i18n.text("settings-inherited-unavailable"));
    }
    let network = projection.network();
    let mut selected = projection.selected;
    let effective = projection.effective;
    if target.is_network() {
        let input = if local {
            crate::permission_studio_mode_target_value(
                &PermissionConfig {
                    network: Some(network),
                    ..Default::default()
                },
                target,
            )
            .map(crate::permission_mode_token)
            .ok_or_else(|| i18n.text("settings-inherited-unavailable"))?
        } else {
            ""
        };
        crate::apply_permission_studio_mode_input(i18n, &mut selected, target, input)
            .map_err(|error| error.to_string())?;
    } else if let Target::ToolRule { tool_name } | Target::ToolCommandPattern { tool_name, .. } =
        target
    {
        if local {
            let rules = effective
                .tools
                .as_ref()
                .and_then(|tools| tools.rules.get(tool_name))
                .cloned()
                .or_else(|| {
                    matches!(
                        tool_name.as_str(),
                        "*" | agena_domain::SHELL_COMMAND_RULE_GROUP
                    )
                    .then(|| ToolPermissionRules::Ordered(Default::default()))
                })
                .ok_or_else(|| i18n.text("value-unset"))?;
            selected
                .tools
                .get_or_insert_with(Default::default)
                .rules
                .insert(tool_name.clone(), rules);
        } else if let Some(tools) = selected.tools.as_mut() {
            tools.rules.remove(tool_name);
        }
    } else {
        match target {
            Target::PathWorkspace | Target::PathExternal => {
                let from = effective.path.as_ref();
                let value = if local {
                    if matches!(target, Target::PathWorkspace) {
                        from.and_then(|path| path.workspace.clone())
                    } else {
                        from.and_then(|path| path.external.clone())
                    }
                } else {
                    None
                };
                if local && value.is_none() {
                    return Err(i18n.text("settings-inherited-unavailable"));
                }
                let path = selected.path.get_or_insert_with(Default::default);
                if matches!(target, Target::PathWorkspace) {
                    path.workspace = value;
                } else {
                    path.external = value;
                }
            }
            Target::PathRule { pattern } => {
                if local {
                    let value = effective
                        .path
                        .as_ref()
                        .and_then(|path| path.rules.get(pattern))
                        .cloned()
                        .ok_or_else(|| i18n.text("value-unset"))?;
                    selected
                        .path
                        .get_or_insert_with(Default::default)
                        .rules
                        .insert(pattern.clone(), value);
                } else if let Some(path) = selected.path.as_mut() {
                    path.rules.shift_remove(pattern);
                }
            }
            Target::ToolDefault => {
                selected.tools.get_or_insert_with(Default::default).default = if local {
                    Some(
                        effective
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.default)
                            .ok_or_else(|| i18n.text("settings-inherited-unavailable"))?,
                    )
                } else {
                    None
                };
            }
            Target::ToolName { key } => {
                if local {
                    let value = effective
                        .tools
                        .as_ref()
                        .and_then(|tools| tools.names.get(key))
                        .copied()
                        .ok_or_else(|| i18n.text("value-unset"))?;
                    selected
                        .tools
                        .get_or_insert_with(Default::default)
                        .names
                        .insert(key.clone(), value);
                } else if let Some(tools) = selected.tools.as_mut() {
                    tools.names.remove(key);
                }
            }
            _ => unreachable!("network and command groups handled above"),
        }
    }
    normalize_permission_config(&mut selected);
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dialog(local: serde_json::Value, inherited: serde_json::Value) -> PermissionStudioOverlay {
        PermissionStudioOverlay {
            title: String::new(),
            footer: String::new(),
            source: crate::PermissionStudioSource::Session { session_id: 10 },
            title_context: "session".into(),
            editable: true,
            permission: serde_json::from_value(local).unwrap(),
            inherited_permission: serde_json::from_value(inherited).unwrap(),
            network_rule: None,
            save_request: None,
            page: crate::PermissionStudioPage::Access,
            state: agena_tui_components::SectionedListState::new(
                Vec::new(),
                0,
                0,
                crate::PermissionStudioFocus::Items,
            ),
            editor: None,
        }
    }

    #[test]
    fn source_pins_equal_default_then_restores_inheritance_without_losing_siblings() {
        let i18n = I18n::english();
        let mut state = dialog(
            json!({"tools":{"default":"deny"}}),
            json!({"path":{"workspace":"readwrite","external":"readonly"}}),
        );
        state.permission =
            permission_studio_source_config(&i18n, &state, &Target::PathWorkspace, true).unwrap();
        assert!(permission_studio_source_is_local(
            &state,
            &Target::PathWorkspace
        ));
        assert_eq!(
            serde_json::to_value(&state.permission).unwrap()["path"]["workspace"],
            "readwrite"
        );
        let after =
            permission_studio_source_config(&i18n, &state, &Target::PathWorkspace, false).unwrap();
        assert!(after.path.is_none());
        assert_eq!(after.tools.unwrap().default, Some(PermissionMode::Deny));
    }

    #[test]
    fn source_network_pins_and_resets_only_the_selected_field() {
        let i18n = I18n::english();
        for (target, key) in [
            (Target::NetworkEgressDefault, "default"),
            (Target::NetworkIngressDefault, "inbound"),
            (Target::NetworkHostLoopback, "hostLoopback"),
        ] {
            let mut state = dialog(
                json!({"tools":{"default":"ask"},"network":{"version":2,"allow":[]}}),
                json!({"network":{"version":2,"default":"allow","inbound":"allow","hostLoopback":"allow","deny":[{"to":"10.0.0.0/8"}]}}),
            );
            state.permission =
                permission_studio_source_config(&i18n, &state, &target, true).unwrap();
            let raw = serde_json::to_value(&state.permission).unwrap();
            assert_eq!(raw["network"], json!({"version":2,key:"allow","allow":[]}));
            assert!(permission_studio_source_is_local(&state, &target));
            state.permission =
                permission_studio_source_config(&i18n, &state, &target, false).unwrap();
            assert_eq!(
                serde_json::to_value(&state.permission).unwrap()["network"],
                json!({"version":2,"allow":[]})
            );
            assert!(!permission_studio_source_is_local(&state, &target));
            assert_eq!(
                state.permission.tools.as_ref().unwrap().default,
                Some(PermissionMode::Ask)
            );
        }
    }

    #[test]
    fn source_network_lists_preserve_empty_local_and_unrelated_inheritance() {
        let mut state = dialog(
            json!({"network":{"version":2,"inbound":"allow"}}),
            json!({"network":{"version":2,"default":"allow","allow":[{"to":"192.0.2.10"}],"deny":[{"to":"10.0.0.0/8"}]}}),
        );
        state.permission = crate::permission_studio_network_list_source_config(
            &I18n::english(),
            &state,
            false,
            "local",
        )
        .unwrap();
        let raw = serde_json::to_value(&state.permission).unwrap();
        assert_eq!(raw["network"].as_object().unwrap().len(), 3);
        assert_eq!(raw["network"]["version"], 2);
        assert_eq!(raw["network"]["inbound"], "allow");
        assert_eq!(raw["network"]["allow"][0]["to"], "192.0.2.10");
        state.permission = crate::permission_studio_network_list_source_config(
            &I18n::english(),
            &state,
            true,
            "empty",
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&state.permission).unwrap()["network"]["deny"],
            json!([])
        );
        assert_eq!(
            PermissionStudioProjection::new(&state).network_list_origin(true),
            PermissionStudioValueSource::Current
        );
        state.permission = crate::permission_studio_network_list_source_config(
            &I18n::english(),
            &state,
            false,
            "inherit",
        )
        .unwrap();
        let raw = serde_json::to_value(&state.permission).unwrap();
        assert!(raw["network"].get("allow").is_none());
        assert_eq!(raw["network"]["inbound"], "allow");
        assert_eq!(raw["network"]["deny"], json!([]));
        assert_eq!(
            PermissionStudioProjection::new(&state).network_list_origin(false),
            PermissionStudioValueSource::Inherited
        );
    }

    #[test]
    fn source_command_pattern_changes_ownership_of_shared_group() {
        let i18n = I18n::english();
        let target = Target::ToolCommandPattern {
            tool_name: "shell.*".into(),
            pattern: "git status".into(),
        };
        let mut state = dialog(
            json!({"tools":{"rules":{"custom.tool":"deny"}}}),
            json!({"tools":{"rules":{"shell.*":{"git status":"allow","git push":"deny"}}}}),
        );
        let group = state.inherited_permission.tools.as_ref().unwrap().rules["shell.*"].clone();
        state.permission = permission_studio_source_config(&i18n, &state, &target, true).unwrap();
        assert_eq!(
            state.permission.tools.as_ref().unwrap().rules["shell.*"],
            group
        );
        let missing_pattern = Target::ToolCommandPattern {
            tool_name: "shell.*".into(),
            pattern: "not present".into(),
        };
        assert!(permission_studio_source_is_local(&state, &missing_pattern));
        let after = permission_studio_source_config(&i18n, &state, &target, false).unwrap();
        let rules = after.tools.unwrap().rules;
        assert!(!rules.contains_key("shell.*"));
        assert_eq!(
            rules["custom.tool"],
            ToolPermissionRules::Mode(PermissionMode::Deny)
        );
    }

    #[test]
    fn source_restore_preserves_remaining_path_rule_order() {
        let i18n = I18n::english();
        let state = dialog(
            json!({"path":{"rules":{
                "/a":"readonly", "/b":"deny", "/c":"readwrite", "/d":"readonly"
            }}}),
            json!({}),
        );
        let after = permission_studio_source_config(
            &i18n,
            &state,
            &Target::PathRule {
                pattern: "/b".into(),
            },
            false,
        )
        .unwrap();
        assert_eq!(
            after
                .path
                .unwrap()
                .rules
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["/a", "/c", "/d"]
        );
    }

    #[test]
    fn source_preserves_read_only_auto_and_explicit_empty_group() {
        let i18n = I18n::english();
        let target = Target::ToolCommandPattern {
            tool_name: "*".into(),
            pattern: "read-only".into(),
        };
        let mut state = dialog(
            json!({}),
            json!({"tools":{"rules":{"*":{"read-only":"auto"}}}}),
        );
        state.permission = permission_studio_source_config(&i18n, &state, &target, true).unwrap();
        assert_eq!(
            super::super::read_only_class_mode(&state.permission),
            Some(PermissionMode::Auto)
        );
        state.permission = serde_json::from_value(json!({"tools":{"rules":{"*":{}}}})).unwrap();
        let pinned = permission_studio_source_config(&i18n, &state, &target, true).unwrap();
        assert_eq!(
            serde_json::to_value(pinned).unwrap()["tools"]["rules"]["*"],
            json!({})
        );
    }
}
