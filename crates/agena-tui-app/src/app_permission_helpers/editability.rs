use crate::{
    I18n, PermissionConfig, PermissionStudioAction as Action, PermissionStudioModeTarget as Target,
    PermissionStudioNetworkAction as NetworkAction, PermissionStudioOverlay,
    PermissionStudioSource, PermissionStudioValueSource,
};

pub(crate) fn permission_studio_inherited_message(
    i18n: &I18n,
    dialog: &PermissionStudioOverlay,
) -> String {
    crate::settings_inherited_locked_message(
        i18n,
        matches!(
            dialog.source,
            PermissionStudioSource::Session { .. }
                | PermissionStudioSource::EffectiveSession { .. }
        ),
    )
}

pub(crate) fn permission_studio_target_editable(
    dialog: &PermissionStudioOverlay,
    target: &Target,
) -> bool {
    let new_rule = match target {
        Target::PathRule { pattern } => !dialog
            .inherited_permission
            .path
            .as_ref()
            .is_some_and(|path| path.rules.contains_key(pattern)),
        Target::ToolName { key } => !dialog
            .inherited_permission
            .tools
            .as_ref()
            .is_some_and(|tools| tools.names.contains_key(key)),
        Target::ToolCommandPattern { tool_name, pattern }
            if tool_name == "*" && pattern == "read-only" =>
        {
            false
        }
        Target::ToolRule { tool_name } | Target::ToolCommandPattern { tool_name, .. } => !dialog
            .inherited_permission
            .tools
            .as_ref()
            .is_some_and(|tools| tools.rules.contains_key(tool_name)),
        _ => false,
    };
    dialog.editable
        && (matches!(dialog.source, PermissionStudioSource::GlobalConfig)
            || super::permission_studio_source_is_local(dialog, target)
            || new_rule)
}

pub(crate) fn permission_studio_network_list_editable(
    dialog: &PermissionStudioOverlay,
    deny: bool,
) -> bool {
    dialog.editable
        && (matches!(dialog.source, PermissionStudioSource::GlobalConfig)
            || super::PermissionStudioProjection::new(dialog).network_list_origin(deny)
                == PermissionStudioValueSource::Current)
}

pub(crate) fn permission_studio_network_editable(dialog: &PermissionStudioOverlay) -> bool {
    dialog.editable
        && (matches!(dialog.source, PermissionStudioSource::GlobalConfig)
            || [
                Target::NetworkEgressDefault,
                Target::NetworkIngressDefault,
                Target::NetworkHostLoopback,
            ]
            .iter()
            .all(|target| super::permission_studio_source_is_local(dialog, target))
                && [false, true]
                    .into_iter()
                    .all(|deny| permission_studio_network_list_editable(dialog, deny)))
}

pub(crate) fn permission_studio_action_editable(
    dialog: &PermissionStudioOverlay,
    action: &Action,
) -> bool {
    if !dialog.editable {
        return matches!(action, Action::RememberedRules);
    }
    match action {
        Action::EditMode(target) => permission_studio_target_editable(dialog, target),
        Action::EditNetworkPolicy => permission_studio_network_editable(dialog),
        Action::AddToolCommandPattern => permission_studio_target_editable(
            dialog,
            &Target::ToolRule {
                tool_name: agena_domain::SHELL_COMMAND_RULE_GROUP.to_owned(),
            },
        ),
        Action::Network(action) => match action {
            NetworkAction::Source
            | NetworkAction::ListSource { .. }
            | NetworkAction::CancelRule => true,
            NetworkAction::Offline | NetworkAction::Outbound => {
                permission_studio_network_editable(dialog)
            }
            NetworkAction::AddRule { deny } | NetworkAction::EditRule { deny, .. } => {
                permission_studio_network_list_editable(dialog, *deny)
            }
            NetworkAction::ToggleRuleAction => dialog
                .network_rule
                .as_ref()
                .is_some_and(|draft| permission_studio_network_list_editable(dialog, !draft.deny)),
            NetworkAction::Field(_) | NetworkAction::SaveRule => {
                dialog.network_rule.as_ref().is_some_and(|draft| {
                    permission_studio_network_list_editable(dialog, draft.deny)
                        && draft.original.is_none_or(|(deny, _)| {
                            permission_studio_network_list_editable(dialog, deny)
                        })
                })
            }
            NetworkAction::DeleteRule => {
                dialog
                    .state
                    .selected_item()
                    .is_some_and(|item| match &item.action {
                        Action::Network(NetworkAction::EditRule { deny, .. }) => {
                            permission_studio_network_list_editable(dialog, *deny)
                        }
                        _ => false,
                    })
            }
        },
        // Creating a new map key explicitly creates a local rule. Existing
        // inherited keys are protected again by the mutation guard below.
        Action::CreateRule | Action::RememberedRules => true,
    }
}

/// Value edits cannot create an override for an existing inherited setting.
/// Source transitions call the separately named persistence entry point.
pub(crate) fn permission_studio_change_is_local(
    dialog: &PermissionStudioOverlay,
    next: &PermissionConfig,
) -> bool {
    if matches!(dialog.source, PermissionStudioSource::GlobalConfig) {
        return true;
    }
    let old = serde_json::to_value(&dialog.permission).unwrap_or_default();
    let next = serde_json::to_value(next).unwrap_or_default();
    let parent = serde_json::to_value(&dialog.inherited_permission).unwrap_or_default();
    for path in [
        "/path/workspace",
        "/path/external",
        "/tools/default",
        "/network/default",
        "/network/inbound",
        "/network/hostLoopback",
        "/network/allow",
        "/network/deny",
        "/approval_model",
    ] {
        let before = old.pointer(path).filter(|value| !value.is_null());
        let after = next.pointer(path).filter(|value| !value.is_null());
        if before != after && before.is_none() && after.is_some() {
            return false;
        }
    }
    for path in ["/path/rules", "/tools/names", "/tools/rules"] {
        if let Some(entries) = next.pointer(path).and_then(serde_json::Value::as_object) {
            for (key, value) in entries {
                let before = old.pointer(path).and_then(|entries| entries.get(key));
                if before != Some(value)
                    && before.is_none()
                    && parent
                        .pointer(path)
                        .and_then(|entries| entries.get(key))
                        .is_some()
                {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn dialog() -> PermissionStudioOverlay {
        PermissionStudioOverlay {
            title: String::new(), footer: String::new(), title_context: String::new(),
            source: PermissionStudioSource::WorkspaceConfig, editable: true,
            permission: PermissionConfig::default(),
            inherited_permission: serde_json::from_value(json!({
                "path":{"workspace":"readwrite","external":"readonly","rules":{"/parent":"readonly"}},
                "tools":{"default":"allow","names":{"parent.tool":"ask"},"rules":{"shell":{"routine":"allow"}}},
                "network":{"version":2,"default":"allow","inbound":"deny","hostLoopback":"deny","allow":[{"to":"192.0.2.10"}],"deny":[]}
            })).unwrap(),
            network_rule: None, save_request: None, page: crate::PermissionStudioPage::Access, editor: None,
            state: agena_tui_components::SectionedListState::new(Vec::new(),0,0,crate::PermissionStudioFocus::Items),
        }
    }
    #[test]
    fn inherited_permission_values_and_lists_require_explicit_source() {
        let mut state = dialog();
        for target in [
            Target::PathWorkspace,
            Target::PathExternal,
            Target::ToolDefault,
            Target::NetworkEgressDefault,
            Target::NetworkIngressDefault,
            Target::NetworkHostLoopback,
            Target::PathRule {
                pattern: "/parent".into(),
            },
            Target::ToolName {
                key: "parent.tool".into(),
            },
            Target::ToolRule {
                tool_name: "shell".into(),
            },
        ] {
            assert!(!permission_studio_target_editable(&state, &target));
            let next =
                crate::permission_studio_source_config(&I18n::english(), &state, &target, true)
                    .unwrap();
            assert!(
                !permission_studio_change_is_local(&state, &next),
                "source must use explicit persistence: {target:?}"
            );
        }
        for deny in [false, true] {
            assert!(!permission_studio_action_editable(
                &state,
                &Action::Network(NetworkAction::AddRule { deny })
            ));
            assert!(!permission_studio_action_editable(
                &state,
                &Action::Network(NetworkAction::EditRule { deny, index: 0 })
            ));
            assert!(permission_studio_action_editable(
                &state,
                &Action::Network(NetworkAction::ListSource { deny })
            ));
        }
        assert!(!permission_studio_action_editable(
            &state,
            &Action::EditNetworkPolicy
        ));
        assert!(!permission_studio_action_editable(
            &state,
            &Action::Network(NetworkAction::Offline)
        ));
        state.permission = crate::permission_studio_source_config(
            &I18n::english(),
            &state,
            &Target::PathWorkspace,
            true,
        )
        .unwrap();
        assert!(permission_studio_target_editable(
            &state,
            &Target::PathWorkspace
        ));
        assert!(!permission_studio_target_editable(
            &state,
            &Target::PathExternal
        ));
        let mut changed = state.permission.clone();
        changed.path.as_mut().unwrap().workspace = serde_json::from_value(json!("deny")).unwrap();
        assert!(permission_studio_change_is_local(&state, &changed));
    }
    #[test]
    fn network_source_is_per_list_and_full_json_requires_every_field_local() {
        let mut state = dialog();
        state.permission = crate::permission_studio_network_list_source_config(
            &I18n::english(),
            &state,
            false,
            "local",
        )
        .unwrap();
        assert!(permission_studio_network_list_editable(&state, false));
        assert!(!permission_studio_network_list_editable(&state, true));
        assert!(!permission_studio_network_editable(&state));
        state.permission.network =
            Some(super::super::PermissionStudioProjection::new(&state).network());
        assert!(permission_studio_network_editable(&state));
        let mut next = state.permission.clone();
        next.network = Some(agena_domain::NetworkPermissionConfig::default());
        assert!(permission_studio_change_is_local(&state, &next));
    }
    #[test]
    fn new_rules_and_global_defaults_are_directly_editable_without_unlocking_inherited_keys() {
        let mut state = dialog();
        assert!(permission_studio_target_editable(
            &state,
            &Target::PathRule {
                pattern: "/new".into()
            }
        ));
        let new: PermissionConfig =
            serde_json::from_value(json!({"path":{"rules":{"/new":"deny"}}})).unwrap();
        assert!(permission_studio_change_is_local(&state, &new));
        let inherited: PermissionConfig =
            serde_json::from_value(json!({"path":{"rules":{"/parent":"deny"}}})).unwrap();
        assert!(!permission_studio_change_is_local(&state, &inherited));
        state.source = PermissionStudioSource::GlobalConfig;
        assert!(permission_studio_target_editable(
            &state,
            &Target::PathWorkspace
        ));
        assert!(permission_studio_network_editable(&state));
        assert!(permission_studio_change_is_local(&state, &inherited));
    }
    #[test]
    fn unavailable_parent_is_never_copied_but_explicit_empty_list_remains_possible() {
        let mut state = dialog();
        state.inherited_permission = PermissionConfig::default();
        for target in [
            Target::PathWorkspace,
            Target::PathExternal,
            Target::ToolDefault,
            Target::NetworkEgressDefault,
        ] {
            assert!(
                crate::permission_studio_source_config(&I18n::english(), &state, &target, true)
                    .is_err()
            );
        }
        assert!(
            crate::permission_studio_network_list_source_config(
                &I18n::english(),
                &state,
                false,
                "local"
            )
            .is_err()
        );
        let empty = crate::permission_studio_network_list_source_config(
            &I18n::english(),
            &state,
            false,
            "empty",
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(empty).unwrap()["network"]["allow"],
            json!([])
        );
    }
}
