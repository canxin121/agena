use agena_domain::{NetworkAction, NetworkPermissionConfig, PermissionConfig};
use serde_json::{Value, json};

use crate::{
    I18n, PermissionStudioAction, PermissionStudioItem, PermissionStudioModeTarget,
    PermissionStudioNetworkAction as Action, PermissionStudioNetworkField as Field,
    PermissionStudioNetworkRuleDraft, PermissionStudioOverlay, PermissionStudioSection,
    PermissionStudioSectionId, UiFailure, UiResult,
};

pub(crate) fn permission_studio_has_draft(dialog: &PermissionStudioOverlay) -> bool {
    dialog.network_rule.is_some() || dialog.editor.is_some() || dialog.save_request.is_some()
}

pub(crate) fn permission_studio_network(
    dialog: &PermissionStudioOverlay,
) -> &NetworkPermissionConfig {
    dialog
        .permission
        .network
        .as_ref()
        .or(dialog.inherited_permission.network.as_ref())
        .unwrap_or_else(|| permission_studio_default_network())
}

pub(crate) fn permission_studio_default_network() -> &'static NetworkPermissionConfig {
    static DEFAULT: std::sync::LazyLock<NetworkPermissionConfig> = std::sync::LazyLock::new(|| {
        PermissionConfig::global_default()
            .network
            .unwrap_or_else(NetworkPermissionConfig::allow_all)
    });
    &DEFAULT
}

pub(crate) fn permission_studio_custom_network(
    dialog: &PermissionStudioOverlay,
) -> PermissionConfig {
    let mut permission = dialog.permission.clone();
    permission
        .network
        .get_or_insert_with(|| permission_studio_network(dialog).clone());
    permission
}

/// Preserve the exact submitted draft after host-side validation rejects it.
/// A completion for a previous page or request must not replace a newer edit.
pub(crate) fn permission_studio_restore_failed_save(
    current: &mut PermissionStudioOverlay,
    submitted: &PermissionStudioOverlay,
) -> bool {
    if current.source != submitted.source
        || current.page != submitted.page
        || current.permission != submitted.permission
        || current.save_request.is_none()
        || current.save_request != submitted.save_request
    {
        return false;
    }
    current.save_request = None;
    if current.editor.is_none() {
        current.editor = submitted.editor.clone();
    }
    if current.network_rule.is_none() {
        current.network_rule = submitted.network_rule.clone();
    }
    true
}

fn item(i18n: &I18n, key: &str, value: String, action: Action) -> PermissionStudioItem {
    PermissionStudioItem {
        label: i18n.text(key),
        value,
        source: None,
        action: PermissionStudioAction::Network(action),
    }
}

fn action_text(i18n: &I18n, action: NetworkAction) -> String {
    i18n.text(match action {
        NetworkAction::Allow => "value-allow",
        NetworkAction::Deny => "value-deny",
    })
}

pub(crate) fn permission_studio_network_defaults(
    i18n: &I18n,
    _dialog: &PermissionStudioOverlay,
    projection: &super::PermissionStudioProjection,
) -> Vec<PermissionStudioSection> {
    let network = projection.network();
    let count = network.egress.as_ref().map_or(0, |egress| {
        egress.allow.as_ref().map_or(0, Vec::len) + egress.deny.as_ref().map_or(0, Vec::len)
    });
    vec![PermissionStudioSection {
        id: PermissionStudioSectionId::NetworkDefaults,
        label: i18n.text_args("permission-studio-section-rule-count", &agena_tui::fl_args!("section" => i18n.text("permission-studio-nav-network"), "count" => count as i64)),
        items: [
            ("permission-studio-outbound-label", PermissionStudioModeTarget::NetworkEgressDefault),
            ("permission-studio-inbound-label", PermissionStudioModeTarget::NetworkIngressDefault),
            ("permission-studio-loopback-label", PermissionStudioModeTarget::NetworkHostLoopback),
        ].into_iter().map(|(label, target)| projection.mode_item(i18n, i18n.text(label), target)).collect(),
    }]
}

pub(crate) fn permission_studio_network_controls(
    i18n: &I18n,
    dialog: &PermissionStudioOverlay,
) -> Vec<PermissionStudioSection> {
    vec![PermissionStudioSection {
        id: PermissionStudioSectionId::NetworkPolicy,
        label: i18n.text("permission-studio-network-controls"),
        items: vec![
            item(
                i18n,
                "permission-studio-network-source",
                i18n.text(if dialog.permission.network.is_some() {
                    "permission-studio-network-custom"
                } else {
                    "permission-studio-network-inherited"
                }),
                Action::Source,
            ),
            item(
                i18n,
                "permission-studio-network-offline",
                String::new(),
                Action::Offline,
            ),
            item(
                i18n,
                "permission-studio-network-outbound",
                String::new(),
                Action::Outbound,
            ),
            PermissionStudioItem {
                label: i18n.text("permission-studio-network-json"),
                value: String::new(),
                source: None,
                action: PermissionStudioAction::EditNetworkPolicy,
            },
        ],
    }]
}

pub(crate) fn permission_studio_network_rules(
    i18n: &I18n,
    dialog: &PermissionStudioOverlay,
) -> Vec<PermissionStudioSection> {
    if let Some(draft) = &dialog.network_rule {
        return vec![PermissionStudioSection {
            id: PermissionStudioSectionId::NetworkRules,
            label: i18n.text("permission-studio-network-rule-form"),
            items: vec![
                item(
                    i18n,
                    "permission-studio-network-action",
                    action_text(
                        i18n,
                        if draft.deny {
                            NetworkAction::Deny
                        } else {
                            NetworkAction::Allow
                        },
                    ),
                    Action::ToggleRuleAction,
                ),
                item(
                    i18n,
                    "permission-studio-network-target",
                    draft.to.clone(),
                    Action::Field(Field::Target),
                ),
                item(
                    i18n,
                    "permission-studio-network-protocol",
                    draft.protocol.clone(),
                    Action::Field(Field::Protocol),
                ),
                item(
                    i18n,
                    "permission-studio-network-ports",
                    if draft.ports.is_empty() {
                        i18n.text("permission-studio-network-all-ports")
                    } else {
                        draft.ports.clone()
                    },
                    Action::Field(Field::Ports),
                ),
                item(
                    i18n,
                    "permission-studio-network-except",
                    if draft.except.is_empty() {
                        i18n.text("value-unset")
                    } else {
                        draft.except.clone()
                    },
                    Action::Field(Field::Except),
                ),
                item(
                    i18n,
                    "permission-studio-network-save",
                    String::new(),
                    Action::SaveRule,
                ),
                item(
                    i18n,
                    "permission-studio-network-cancel",
                    String::new(),
                    Action::CancelRule,
                ),
            ],
        }];
    }
    let mut items = Vec::new();
    let source = if matches!(
        dialog.source,
        crate::PermissionStudioSource::EffectiveSession { .. }
    ) {
        crate::PermissionStudioValueSource::Effective
    } else if dialog.permission.network.is_some() {
        crate::PermissionStudioValueSource::Current
    } else {
        crate::PermissionStudioValueSource::Inherited
    };
    match serde_json::to_value(permission_studio_network(dialog)) {
        Ok(policy) => {
            for (deny, list) in [(false, "allow"), (true, "deny")] {
                for (index, rule) in policy
                    .get(list)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let target = rule.get("to").and_then(Value::as_str).unwrap_or("*");
                    let protocol = rule
                        .get("protocol")
                        .and_then(Value::as_str)
                        .unwrap_or("any");
                    let ports = joined_values(rule.get("ports"));
                    let except = joined_values(rule.get("except"));
                    let mut detail = format!(
                        "{} {protocol}:{}",
                        action_text(
                            i18n,
                            if deny {
                                NetworkAction::Deny
                            } else {
                                NetworkAction::Allow
                            }
                        ),
                        if ports.is_empty() {
                            "*".to_string()
                        } else {
                            ports
                        }
                    );
                    if !except.is_empty() {
                        detail.push_str(&format!(
                            " · {} {except}",
                            i18n.text("permission-studio-network-except")
                        ));
                    }
                    items.push(PermissionStudioItem {
                        label: target.to_string(),
                        value: detail,
                        source: Some(source),
                        action: PermissionStudioAction::Network(Action::EditRule { deny, index }),
                    });
                }
            }
        }
        Err(error) => items.push(PermissionStudioItem {
            label: i18n.text("permission-studio-network-json"),
            value: error.to_string(),
            source: None,
            action: PermissionStudioAction::EditNetworkPolicy,
        }),
    }
    items.push(item(
        i18n,
        "permission-studio-add-network-rule",
        String::new(),
        Action::AddRule { deny: false },
    ));
    vec![PermissionStudioSection {
        id: PermissionStudioSectionId::NetworkRules,
        label: i18n.text("permission-studio-nav-network-rules"),
        items,
    }]
}

fn joined_values(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string())
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

pub(crate) fn permission_studio_network_rule_draft(
    dialog: &PermissionStudioOverlay,
    deny: bool,
    index: Option<usize>,
) -> UiResult<PermissionStudioNetworkRuleDraft> {
    let policy =
        serde_json::to_value(permission_studio_network(dialog)).map_err(UiFailure::internal)?;
    let rule = if let Some(index) = index {
        policy
            .get(if deny { "deny" } else { "allow" })
            .and_then(Value::as_array)
            .and_then(|rules| rules.get(index))
            .cloned()
            .ok_or_else(|| UiFailure::internal("network rule is no longer present"))?
    } else {
        json!({"to":"", "protocol":if deny { "any" } else { "tcp" }})
    };
    Ok(PermissionStudioNetworkRuleDraft {
        policy,
        original: index.map(|index| (deny, index)),
        deny,
        to: rule
            .get("to")
            .and_then(Value::as_str)
            .unwrap_or("*")
            .to_string(),
        protocol: rule
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or("any")
            .to_string(),
        ports: joined_values(rule.get("ports")),
        except: joined_values(rule.get("except")),
    })
}

pub(crate) fn permission_studio_network_rule_policy(
    i18n: &I18n,
    draft: &PermissionStudioNetworkRuleDraft,
) -> UiResult<NetworkPermissionConfig> {
    let mut rule = json!({"to":draft.to.trim(), "protocol":draft.protocol});
    if !draft.ports.trim().is_empty() {
        let ports = draft
            .ports
            .split(',')
            .map(|part| {
                let part = part.trim();
                if part.is_empty() {
                    return Err(UiFailure::message(
                        i18n.text("permission-studio-network-ports-help"),
                    ));
                }
                if part.contains('-') {
                    return Ok(Value::String(part.to_string()));
                }
                if !part.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(UiFailure::message(
                        i18n.text("permission-studio-network-ports-help"),
                    ));
                }
                part.parse::<u16>().map(|port| json!(port)).map_err(|_| {
                    UiFailure::message(i18n.text("permission-studio-network-ports-help"))
                })
            })
            .collect::<UiResult<Vec<_>>>()?;
        rule["ports"] = Value::Array(ports);
    }
    if !draft.except.trim().is_empty() {
        rule["except"] = Value::Array(
            draft
                .except
                .split(',')
                .map(|part| Value::String(part.trim().to_owned()))
                .collect(),
        );
    }
    let mut policy = draft.policy.clone();
    if let Some((deny, index)) = draft.original {
        let previous = policy
            .get_mut(if deny { "deny" } else { "allow" })
            .and_then(Value::as_array_mut)
            .filter(|rules| index < rules.len())
            .ok_or_else(|| UiFailure::internal("network rule is no longer present"))?;
        previous.remove(index);
    }
    let list = if draft.deny { "deny" } else { "allow" };
    let rules = policy
        .as_object_mut()
        .ok_or_else(|| UiFailure::internal("network policy must be an object"))?
        .entry(list)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| UiFailure::internal("network rules must be an array"))?;
    let index = draft
        .original
        .filter(|(deny, _)| *deny == draft.deny)
        .map(|(_, index)| index)
        .unwrap_or(rules.len());
    rules.insert(index.min(rules.len()), rule);
    let network: NetworkPermissionConfig =
        serde_json::from_value(policy).map_err(|error| UiFailure::message(error.to_string()))?;
    network.validate().map_err(UiFailure::message)?;
    Ok(network)
}

pub(crate) fn permission_studio_network_delete_rule(
    network: &NetworkPermissionConfig,
    deny: bool,
    index: usize,
) -> UiResult<NetworkPermissionConfig> {
    let mut value = serde_json::to_value(network).map_err(UiFailure::internal)?;
    let rules = value
        .get_mut(if deny { "deny" } else { "allow" })
        .and_then(Value::as_array_mut)
        .filter(|rules| index < rules.len())
        .ok_or_else(|| UiFailure::internal("network rule is no longer present"))?;
    rules.remove(index);
    let policy: NetworkPermissionConfig =
        serde_json::from_value(value).map_err(|error| UiFailure::message(error.to_string()))?;
    policy.validate().map_err(UiFailure::message)?;
    Ok(policy)
}

pub(crate) fn permission_studio_network_field_text(
    draft: &PermissionStudioNetworkRuleDraft,
    field: Field,
) -> &str {
    match field {
        Field::Target => &draft.to,
        Field::Protocol => &draft.protocol,
        Field::Ports => &draft.ports,
        Field::Except => &draft.except,
    }
}

pub(crate) fn permission_studio_network_field_keys(field: Field) -> (&'static str, &'static str) {
    match field {
        Field::Target => (
            "permission-studio-network-target",
            "permission-studio-network-target-help",
        ),
        Field::Protocol => (
            "permission-studio-network-protocol",
            "permission-studio-network-protocol-help",
        ),
        Field::Ports => (
            "permission-studio-network-ports",
            "permission-studio-network-ports-help",
        ),
        Field::Except => (
            "permission-studio-network-except",
            "permission-studio-network-except-help",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn app() -> (tempfile::TempDir, crate::App) {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            crate::App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        app.draft_store_path = temp.path().join("drafts.json");
        app.prompt_history_path = temp.path().join("history.jsonl");
        let mut dialog = inherited_dialog();
        crate::refresh_permission_studio_dialog(
            &app.i18n,
            &mut dialog,
            Some(crate::PermissionStudioSectionId::NetworkRules),
            None,
            None,
        );
        app.current_route = crate::Route::PermissionStudio(dialog);
        (temp, app)
    }

    fn press(app: &mut crate::App, key: KeyCode, modifiers: KeyModifiers) {
        app.handle_key_event(KeyEvent::new(key, modifiers));
    }

    fn type_text(app: &mut crate::App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character), KeyModifiers::NONE);
        }
    }

    fn dialog(app: &crate::App) -> &PermissionStudioOverlay {
        let crate::Route::PermissionStudio(dialog) = &app.current_route else {
            panic!("expected permission studio route");
        };
        dialog
    }

    #[test]
    fn network_form_keyboard_edits_protocol_and_keeps_rejected_fields() {
        let (_temp, mut app) = app();
        press(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(
            dialog(&app).state.selected_section().unwrap().id,
            crate::PermissionStudioSectionId::NetworkRules
        );
        assert!(dialog(&app).network_rule.is_some());

        press(&mut app, KeyCode::Down, KeyModifiers::NONE);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        type_text(&mut app, "192.0.2.20");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(dialog(&app).network_rule.as_ref().unwrap().to, "192.0.2.20");
        press(&mut app, KeyCode::Down, KeyModifiers::NONE);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let Some(crate::Overlay::Choice(choice)) = &mut app.overlay else {
            panic!("expected protocol picker");
        };
        assert!(
            choice
                .presentation
                .select_item_where(|item| item.value == "udp")
        );
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(dialog(&app).network_rule.as_ref().unwrap().protocol, "udp");

        press(&mut app, KeyCode::Down, KeyModifiers::NONE);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        type_text(&mut app, "70000");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let before = dialog(&app).network_rule.clone();
        press(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(dialog(&app).network_rule, before);
        assert!(dialog(&app).save_request.is_none());
        assert!(dialog(&app).permission.network.is_none());
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(dialog(&app).network_rule.is_none());

        // Real rules lead the section; the add action follows them.
        press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(matches!(
            &app.overlay,
            Some(crate::Overlay::Confirm(confirm))
                if matches!(confirm.action, crate::ConfirmAction::PermissionStudioDeleteNetworkRule { deny: false, index: 0 })
        ));
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            dialog(&app).network_rule.as_ref().unwrap().original,
            Some((false, 0))
        );
    }

    #[test]
    fn permission_access_keyboard_crosses_sections_and_adds_to_empty_rules() {
        use crate::{PermissionStudioEditorAction as Edit, PermissionStudioSectionId as Section};
        let (_temp, mut app) = app();
        press(&mut app, KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(
            dialog(&app).state.selected_section().unwrap().id,
            Section::PathDefaults
        );
        press(&mut app, KeyCode::Down, KeyModifiers::NONE);
        press(&mut app, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(
            dialog(&app).state.selected_section().unwrap().id,
            Section::NetworkDefaults
        );
        for _ in 0..2 {
            press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        }
        assert_eq!(
            dialog(&app).state.selected_section().unwrap().id,
            Section::PathRules
        );
        assert_eq!(
            dialog(&app).state.selected_section().unwrap().items.len(),
            1
        );
        press(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert!(matches!(
            dialog(&app).editor.as_ref().unwrap().action,
            Edit::AddPathRule
        ));
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        for _ in 0..3 {
            press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        }
        assert_eq!(
            dialog(&app).state.selected_section().unwrap().id,
            Section::ToolCommandRules
        );
        assert_eq!(
            dialog(&app).state.selected_section().unwrap().items.len(),
            1
        );
        press(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert!(matches!(
            dialog(&app).editor.as_ref().unwrap().action,
            Edit::AddToolRule
        ));
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(
            dialog(&app).state.selected_section().unwrap().id,
            Section::ToolNames
        );
        press(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert!(
            matches!(&app.overlay, Some(crate::Overlay::Choice(choice)) if matches!(choice.action, crate::ChoiceOverlayAction::PermissionStudioAddEntries(crate::PermissionStudioCatalogKind::ToolNames)))
        );
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut app, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(
            dialog(&app).state.selected_section().unwrap().id,
            Section::NetworkPolicy
        );
    }

    #[test]
    fn permission_access_network_draft_keeps_focus_and_blocks_hub_navigation() {
        let (_temp, mut app) = app();
        press(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
        let before = dialog(&app).network_rule.clone();
        for key in [
            KeyCode::Tab,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
        ] {
            press(&mut app, key, KeyModifiers::NONE);
            assert_eq!(
                dialog(&app).state.selected_section().unwrap().id,
                crate::PermissionStudioSectionId::NetworkRules
            );
            assert_eq!(dialog(&app).network_rule, before);
        }
        press(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert_eq!(dialog(&app).network_rule, before);
        app.open_hub();
        assert_eq!(dialog(&app).network_rule, before);
    }

    #[test]
    fn permission_access_pointer_rows_keep_section_ownership() {
        let (_temp, app) = app();
        let mut page = dialog(&app).clone();
        let section = page
            .state
            .sections()
            .iter()
            .position(|section| section.id == crate::PermissionStudioSectionId::ToolNames)
            .unwrap();
        let rows = crate::permission_studio_content_rows(&page);
        let header = rows
            .iter()
            .position(|row| *row == crate::PermissionStudioRow::Section(section))
            .unwrap();
        assert_eq!(
            crate::select_permission_studio_row(&mut page, header),
            Some(false)
        );
        assert_eq!(
            page.state.selected_section().unwrap().id,
            crate::PermissionStudioSectionId::ToolNames
        );
        assert_eq!(
            crate::select_permission_studio_row(&mut page, header + 1),
            Some(true)
        );
        page.network_rule = Some(draft());
        assert_eq!(crate::select_permission_studio_row(&mut page, header), None);
    }

    #[test]
    fn permission_access_nested_return_preserves_all_drafts_and_selection() {
        let (_temp, mut app) = app();
        let mut parent = dialog(&app).clone();
        parent.permission.approval_model =
            Some(agena_domain::ApprovalModelSelection::from_model_ref(
                &agena_domain::ModelRef::new("test", "approval"),
            ));
        app.open_permission_studio_network_editor(&mut parent);
        parent.editor.as_mut().unwrap().input =
            crate::Editor::from_text("{pending json".to_owned());
        let section = parent
            .state
            .sections()
            .iter()
            .position(|section| section.id == crate::PermissionStudioSectionId::NetworkPolicy)
            .unwrap();
        parent.state.set_indices(section, 2);
        let restored = app.refresh_restored_route(crate::Route::PermissionStudio(parent.clone()));
        let crate::Route::PermissionStudio(restored) = restored else {
            unreachable!()
        };
        assert_eq!(restored.permission, parent.permission);
        assert_eq!(restored.network_rule, parent.network_rule);
        assert_eq!(
            restored.editor.as_ref().unwrap().input.text(),
            "{pending json"
        );
        assert_eq!(restored.state.selected_section_index(), section);
        assert_eq!(restored.state.selected_item_index(), 2);
        app.route_stack
            .push(crate::Route::PermissionStudio(restored));
        app.current_route = crate::Route::Main;
        app.open_hub();
        assert!(matches!(app.current_route, crate::Route::Main));
        assert_eq!(app.route_stack.len(), 1);
    }

    #[test]
    fn permission_access_edits_keep_other_domains_and_approval_model() {
        let i18n = I18n::english();
        let mut permission = PermissionConfig::global_default();
        permission.approval_model = Some(agena_domain::ApprovalModelSelection::from_model_ref(
            &agena_domain::ModelRef::new("test", "approval"),
        ));
        let original = permission.clone();
        crate::apply_permission_studio_mode_input(
            &i18n,
            &mut permission,
            &crate::PermissionStudioModeTarget::ToolDefault,
            "ask",
        )
        .unwrap();
        assert_eq!(permission.path, original.path);
        assert_eq!(permission.network, original.network);
        assert_eq!(permission.approval_model, original.approval_model);
        let with_tools = permission.clone();
        crate::apply_permission_studio_mode_input(
            &i18n,
            &mut permission,
            &crate::PermissionStudioModeTarget::PathWorkspace,
            "readonly",
        )
        .unwrap();
        assert_eq!(permission.tools, with_tools.tools);
        assert_eq!(permission.network, with_tools.network);
        crate::apply_permission_studio_mode_input(
            &i18n,
            &mut permission,
            &crate::PermissionStudioModeTarget::NetworkEgressDefault,
            "deny",
        )
        .unwrap();
        assert_eq!(permission.tools, with_tools.tools);
        assert_eq!(permission.approval_model, original.approval_model);
    }

    #[test]
    fn network_form_late_load_completion_cannot_replace_new_scope_or_draft() {
        let (_temp, mut app) = app();
        let old_request = app.begin_permission_studio_load().unwrap();
        let latest_request = app.begin_permission_studio_load().unwrap();
        assert!(!app.accept_permission_studio_load(old_request));
        assert_eq!(app.permission_studio_load_request, Some(latest_request.id));
        assert!(app.accept_permission_studio_load(latest_request));
        let crate::Route::PermissionStudio(current) = &mut app.current_route else {
            unreachable!()
        };
        current.source = crate::PermissionStudioSource::GlobalConfig;
        current.network_rule = Some(draft());
        let before = current.network_rule.clone();
        assert!(!app.accept_permission_studio_load(old_request));
        assert_eq!(
            dialog(&app).source,
            crate::PermissionStudioSource::GlobalConfig
        );
        assert_eq!(dialog(&app).network_rule, before);
        assert!(app.begin_permission_studio_load().is_none());

        let crate::Route::PermissionStudio(current) = &mut app.current_route else {
            unreachable!()
        };
        current.network_rule = None;
        let request = app.begin_permission_studio_load().unwrap();
        app.current_route = crate::Route::Main;
        assert!(!app.accept_permission_studio_load(request));
        assert!(matches!(app.current_route, crate::Route::Main));
    }

    #[test]
    fn network_form_load_accepts_the_restored_route_after_a_key_handler() {
        let (_temp, mut app) = app();
        let route = std::mem::replace(&mut app.current_route, crate::Route::Main);
        app.route_stack.push(route.clone());
        let request = app.begin_permission_studio_load().unwrap();
        app.current_route = route;
        assert!(app.accept_permission_studio_load(request));

        let request = app.begin_permission_studio_load().unwrap();
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!app.accept_permission_studio_load(request));
    }

    #[test]
    fn network_form_json_draft_survives_pending_keys_and_background_refresh() {
        let (_temp, mut app) = app();
        let mut dialog = inherited_dialog();
        app.open_permission_studio_network_editor(&mut dialog);
        let input = "{\"default\":\"deny\",\"allow\":[{\"to\":\"192.0.2.20\"}]}";
        dialog.editor.as_mut().unwrap().input = crate::Editor::from_text(input.to_owned());
        dialog.save_request = Some(uuid::Uuid::new_v4());
        for key in [
            KeyCode::Esc,
            KeyCode::Char('x'),
            KeyCode::Enter,
            KeyCode::Tab,
        ] {
            assert!(!app.handle_permission_studio_overlay_key(
                KeyEvent::new(key, KeyModifiers::NONE),
                &mut dialog
            ));
        }
        app.refresh_permission_studio_overlay(&mut dialog);
        assert_eq!(dialog.editor.as_ref().unwrap().input.text(), input);
        assert!(dialog.save_request.is_some());
        dialog.save_request = None;
        app.refresh_permission_studio_overlay(&mut dialog);
        assert_eq!(dialog.editor.as_ref().unwrap().input.text(), input);
        assert!(dialog.permission.network.is_none());
    }

    #[test]
    fn network_form_renders_selected_rule_after_scrolling_a_long_list() {
        let (_temp, mut app) = app();
        let mut dialog = inherited_dialog();
        let rules = (1..=80)
            .map(|ip| json!({"to": format!("192.0.2.{ip}")}))
            .collect::<Vec<_>>();
        dialog.inherited_permission.network =
            Some(serde_json::from_value(json!({"allow": rules})).unwrap());
        let mut renders = Vec::new();
        for locale in ["en-US", "zh-CN"] {
            app.i18n = I18n::resolve(Some(locale), None);
            crate::refresh_permission_studio_dialog(&app.i18n, &mut dialog, None, None, None);
            for (section_id, item, expected) in [
                (
                    crate::PermissionStudioSectionId::PathDefaults,
                    0,
                    app.i18n.text("permission-studio-workspace-label"),
                ),
                (
                    crate::PermissionStudioSectionId::NetworkRules,
                    79,
                    "192.0.2.80".to_string(),
                ),
                (
                    crate::PermissionStudioSectionId::RememberedRules,
                    0,
                    app.i18n.text("permission-studio-remembered-open"),
                ),
            ] {
                let section = dialog
                    .state
                    .sections()
                    .iter()
                    .position(|section| section.id == section_id)
                    .unwrap();
                dialog.state.set_indices(section, item);
                for (width, height) in [(42, 18), (80, 24), (120, 35)] {
                    let mut terminal =
                        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                            .unwrap();
                    terminal
                        .draw(|frame| {
                            app.render_permission_studio_overlay(
                                frame,
                                frame.area(),
                                &dialog,
                                agena_tui_components::SurfaceMode::Route,
                            );
                        })
                        .unwrap();
                    let rendered = terminal
                        .backend()
                        .buffer()
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n");
                    renders.push((
                        locale,
                        section_id,
                        width,
                        height,
                        expected.clone(),
                        rendered,
                    ));
                }
            }
        }
        for (locale, section_id, width, height, _, rendered) in &renders {
            if *locale == "en-US" && *section_id == crate::PermissionStudioSectionId::NetworkRules {
                eprintln!(
                    "Access permissions at {width}x{height}, selected network rule 80:\n{rendered}"
                );
            }
        }
        for (locale, section_id, width, height, expected, rendered) in renders {
            assert!(
                rendered
                    .replace(' ', "")
                    .contains(&expected.replace(' ', "")),
                "selected {section_id:?} is clipped at {locale}/{width}x{height}:\n{rendered}"
            );
            for shortcut in ["Enter", "Ctrl+N", "Ctrl+D", "Ctrl+E", "Esc"] {
                assert!(
                    rendered.contains(shortcut),
                    "{shortcut} hint is missing at {locale}/{width}x{height}:\n{rendered}"
                );
            }
        }
    }

    #[test]
    fn network_form_labels_and_keyboard_help_exist_in_every_locale() {
        for (locale, _) in agena_tui::i18n::SUPPORTED_LOCALES {
            let i18n = I18n::resolve(Some(locale), None);
            for key in [
                "permission-studio-page-access",
                "permission-studio-access-footer",
                "permission-studio-access-read-only-footer",
                "permission-studio-access-content",
                "permission-studio-remembered-rules",
                "permission-studio-remembered-create",
                "permission-studio-remembered-read-only",
                "permission-studio-remembered-empty",
                "permission-studio-network-defaults-footer",
                "permission-studio-network-rules-footer",
                "permission-studio-network-form-footer",
                "permission-studio-network-column-rule",
                "permission-studio-network-source",
                "permission-studio-network-target-help",
                "permission-studio-network-ports-help",
                "permission-studio-network-except-help",
                "permission-studio-origin-current",
                "permission-studio-origin-inherited",
                "permission-studio-origin-effective",
                "permission-studio-read-only-invalid",
                "permission-studio-workspace-label",
                "permission-studio-external-label",
                "permission-studio-path-readwrite-compact",
                "permission-studio-default-label",
                "permission-studio-read-only-class-label",
                "permission-studio-outbound-label",
                "permission-studio-inbound-label",
                "permission-studio-loopback-label",
                "permission-studio-network-controls",
                "permission-studio-add-network-rule",
                "permission-studio-inherited-rule-no-local-edit",
            ] {
                let text = i18n.text(key);
                assert!(
                    !text.trim().is_empty() && !text.contains("Unknown localization key"),
                    "{locale}/{key}: {text}"
                );
            }
        }
    }

    fn render_access(
        app: &crate::App,
        dialog: &PermissionStudioOverlay,
        width: u16,
        height: u16,
    ) -> (String, agena_tui_components::pointer::PointerMap) {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        let pointer_map = agena_tui_components::pointer::capture(|| {
            terminal
                .draw(|frame| {
                    app.render_permission_studio_overlay(
                        frame,
                        frame.area(),
                        dialog,
                        agena_tui_components::SurfaceMode::Route,
                    );
                })
                .unwrap();
        });
        let text = terminal
            .backend()
            .buffer()
            .content
            .chunks(width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        (text, pointer_map)
    }

    #[test]
    fn permission_dense_first_screen_shows_all_effective_defaults_and_counts() {
        let (_temp, mut app) = app();
        let mut page = inherited_dialog();
        page.inherited_permission = PermissionConfig::global_default();
        page.inherited_permission.network = Some(serde_json::from_value(json!({"allow": (1..=80).map(|ip| json!({"to": format!("192.0.2.{ip}")})).collect::<Vec<_>>()})).unwrap());
        page.permission = serde_json::from_value(
            json!({"tools": {"default": "ask", "rules": {"*": {"read-only": "auto"}}}}),
        )
        .unwrap();
        for locale in ["en-US", "zh-CN"] {
            app.i18n = I18n::resolve(Some(locale), None);
            crate::refresh_permission_studio_dialog(
                &app.i18n,
                &mut page,
                Some(crate::PermissionStudioSectionId::PathDefaults),
                None,
                None,
            );
            for (width, height) in [(42, 18), (80, 24), (120, 35)] {
                let (rendered, _) = render_access(&app, &page, width, height);
                if locale == "en-US" {
                    eprintln!("Dense access defaults at {width}x{height}:\n{rendered}");
                }
                let normalized = rendered.replace(' ', "");
                for key in [
                    "permission-studio-workspace-label",
                    "permission-studio-external-label",
                    "permission-studio-outbound-label",
                    "permission-studio-inbound-label",
                    "permission-studio-loopback-label",
                    "permission-studio-default-label",
                    "permission-studio-read-only-class-label",
                    "permission-studio-path-readwrite-compact",
                ] {
                    let expected = app.i18n.text(key).replace(' ', "");
                    assert!(
                        normalized.contains(&expected),
                        "{key} missing at {locale}/{width}x{height}:\n{rendered}"
                    );
                }
                // Fluent isolates interpolated values with bidi control
                // characters; terminal rendering intentionally removes them.
                // Verify exact effective counts, then compare the visible text.
                let expected_counts = [
                    app.i18n.text_args("permission-studio-section-rule-count", &agena_tui::fl_args!("section" => app.i18n.text("permission-studio-nav-filesystem"), "count" => 1_i64)),
                    app.i18n.text_args("permission-studio-section-rule-count", &agena_tui::fl_args!("section" => app.i18n.text("permission-studio-nav-network"), "count" => 80_i64)),
                    app.i18n.text_args("permission-studio-tools-rule-count", &agena_tui::fl_args!("names" => 4_i64, "commands" => 13_i64)),
                ];
                for (section, expected) in page.state.sections().iter().take(3).zip(expected_counts)
                {
                    assert_eq!(
                        section.label, expected,
                        "wrong effective rule count at {locale}"
                    );
                    let visible = crate::view::sanitize_display_text(expected);
                    assert!(
                        normalized.contains(&visible.replace(' ', "")),
                        "rule count missing at {locale}/{width}x{height}:\n{rendered}"
                    );
                }
                let tools = page
                    .state
                    .sections()
                    .iter()
                    .find(|section| section.id == crate::PermissionStudioSectionId::ToolDefaults)
                    .unwrap();
                assert!(
                    normalized.contains(
                        &crate::view::sanitize_display_text(&tools.items[1].value).replace(' ', "")
                    ),
                    "read-only resolution missing: {rendered}"
                );
                for shortcut in ["Enter", "Ctrl+N", "Ctrl+D", "Ctrl+E", "Esc"] {
                    assert!(
                        rendered.contains(shortcut),
                        "{shortcut} missing at {locale}/{width}x{height}:\n{rendered}"
                    );
                }
            }
        }
    }

    #[test]
    fn permission_dense_pointer_tracks_the_visible_scrolled_rule() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let (_temp, app) = app();
        let mut page = inherited_dialog();
        page.inherited_permission.network = Some(serde_json::from_value(json!({"allow": (1..=80).map(|ip| json!({"to": format!("192.0.2.{ip}")})).collect::<Vec<_>>()})).unwrap());
        crate::refresh_permission_studio_dialog(
            &app.i18n,
            &mut page,
            Some(crate::PermissionStudioSectionId::NetworkRules),
            Some("192.0.2.80"),
            None,
        );
        let (rendered, pointer_map) = render_access(&app, &page, 42, 18);
        let row = rendered
            .lines()
            .position(|line| line.contains("192.0.2.80"))
            .unwrap();
        let action = pointer_map.action(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: row as u16,
            modifiers: KeyModifiers::NONE,
        });
        let Some(agena_tui_components::pointer::PointerAction::List { index, .. }) = action else {
            panic!("expected scrolled row pointer target: {action:?}")
        };
        assert_eq!(
            crate::select_permission_studio_row(&mut page, index),
            Some(true)
        );
        assert_eq!(
            page.state.selected_section().unwrap().id,
            crate::PermissionStudioSectionId::NetworkRules
        );
        assert!(matches!(
            page.state.selected_item().unwrap().action,
            PermissionStudioAction::Network(Action::EditRule {
                deny: false,
                index: 79
            })
        ));
    }

    #[test]
    fn permission_inherited_rows_block_delete_and_rename_but_allow_override() {
        use crate::PermissionStudioSectionId as Section;
        let (_temp, mut app) = app();
        let mut page = inherited_dialog();
        page.inherited_permission.merge_from(serde_json::from_value(json!({
            "path": {"rules": {"/tmp/inherited": "readonly"}},
            "tools": {"names": {"custom.tool": "ask"}, "rules": {"agena.shell.exec": {"git status": "allow"}}}
        })).unwrap());
        for section in [
            Section::PathRules,
            Section::ToolNames,
            Section::ToolCommandRules,
        ] {
            crate::refresh_permission_studio_dialog(
                &app.i18n,
                &mut page,
                Some(section),
                None,
                None,
            );
            page.state
                .set_indices(page.state.selected_section_index(), 0);
            app.current_route = crate::Route::PermissionStudio(page.clone());
            press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
            assert!(app.overlay.is_none());
            assert!(dialog(&app).save_request.is_none());
            press(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL);
            assert!(dialog(&app).editor.is_none());
            assert_eq!(dialog(&app).permission, page.permission);
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
            assert!(matches!(app.overlay, Some(crate::Overlay::Choice(_))));
            press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        }
    }

    #[test]
    fn permission_dense_network_keeps_empty_add_and_all_policy_controls() {
        let (_temp, mut app) = app();
        let mut page = inherited_dialog();
        page.inherited_permission.network = Some(NetworkPermissionConfig::offline());
        crate::refresh_permission_studio_dialog(
            &app.i18n,
            &mut page,
            Some(crate::PermissionStudioSectionId::NetworkRules),
            None,
            None,
        );
        assert_eq!(page.state.selected_section().unwrap().items.len(), 1);
        app.current_route = crate::Route::PermissionStudio(page);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!dialog(&app).network_rule.as_ref().unwrap().deny);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(dialog(&app).network_rule.as_ref().unwrap().deny);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut app, KeyCode::End, KeyModifiers::NONE);
        let controls = dialog(&app).state.selected_section().unwrap();
        assert_eq!(controls.id, crate::PermissionStudioSectionId::NetworkPolicy);
        assert_eq!(controls.items.len(), 4);
        assert!(matches!(
            controls.items[0].action,
            PermissionStudioAction::Network(Action::Source)
        ));
        assert!(matches!(
            controls.items[1].action,
            PermissionStudioAction::Network(Action::Offline)
        ));
        assert!(matches!(
            controls.items[2].action,
            PermissionStudioAction::Network(Action::Outbound)
        ));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(
            dialog(&app).editor.as_ref().unwrap().action,
            crate::PermissionStudioEditorAction::EditNetworkPolicy
        ));
    }

    fn inherited_dialog() -> PermissionStudioOverlay {
        PermissionStudioOverlay {
            title: String::new(),
            footer: String::new(),
            source: crate::PermissionStudioSource::WorkspaceConfig,
            title_context: String::new(),
            editable: true,
            permission: PermissionConfig::default(),
            inherited_permission: serde_json::from_value(json!({
                "path": {"workspace": "readwrite", "external": "deny"},
                "network": {
                    "default": "deny",
                    "allow": [{"to": "192.0.2.10", "protocol": "tcp", "ports": [443]}],
                    "deny": [{"to": "10.0.0.0/8"}]
                }
            }))
            .unwrap(),
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
    fn built_in_permission_defaults_are_visible_without_creating_overrides() {
        let i18n = I18n::english();
        let mut dialog = inherited_dialog();
        dialog.source = crate::PermissionStudioSource::GlobalConfig;
        dialog.page = crate::PermissionStudioPage::Access;
        dialog.inherited_permission = PermissionConfig::global_default();
        let sections = crate::permission_studio_sections(&i18n, &dialog);
        assert_eq!(
            sections[0].items[0].value,
            i18n.text("permission-studio-path-readwrite-compact")
        );
        assert_eq!(sections[0].items[1].value, i18n.text("value-deny"));
        assert_eq!(
            sections[0].items[0].source,
            Some(crate::PermissionStudioValueSource::Inherited)
        );
        let tools = sections
            .iter()
            .find(|section| section.id == crate::PermissionStudioSectionId::ToolDefaults)
            .unwrap();
        assert_eq!(tools.items.len(), 2);
        assert!(
            tools
                .items
                .iter()
                .all(|item| item.value == i18n.text("value-allow")
                    && item.source == Some(crate::PermissionStudioValueSource::Inherited))
        );
        let network = permission_studio_network(&dialog).resolved();
        assert_eq!(
            network.egress.as_ref().unwrap().default,
            Some(NetworkAction::Allow)
        );
        assert_eq!(
            network.ingress.as_ref().unwrap().default,
            Some(NetworkAction::Deny)
        );
        assert_eq!(
            network.ingress.as_ref().unwrap().host_loopback,
            Some(NetworkAction::Deny)
        );
        assert_eq!(serde_json::to_value(&dialog.permission).unwrap(), json!({}));
        let custom = permission_studio_custom_network(&dialog);
        assert!(custom.path.is_none());
        assert_eq!(custom.network, dialog.inherited_permission.network);
        assert_eq!(
            serde_json::to_value(custom).unwrap()["network"]["default"],
            "allow"
        );
    }

    #[test]
    fn explicit_restrictions_remain_until_reset_to_inheritance() {
        let i18n = I18n::english();
        let mut dialog = inherited_dialog();
        dialog.page = crate::PermissionStudioPage::Access;
        dialog.inherited_permission = PermissionConfig::global_default();
        dialog.permission = serde_json::from_value(json!({
            "path": {"workspace": "readonly", "external": "deny"},
            "network": {}
        }))
        .unwrap();
        let sections = crate::permission_studio_sections(&i18n, &dialog);
        assert_eq!(
            sections[0].items[0].value,
            i18n.text("permission-studio-path-readonly")
        );
        assert_eq!(sections[0].items[1].value, i18n.text("value-deny"));
        assert_eq!(
            permission_studio_network(&dialog).resolved(),
            NetworkPermissionConfig::offline().resolved()
        );
        crate::apply_permission_studio_mode_input(
            &i18n,
            &mut dialog.permission,
            &crate::PermissionStudioModeTarget::PathWorkspace,
            "",
        )
        .unwrap();
        let sections = crate::permission_studio_sections(&i18n, &dialog);
        assert!(
            sections[0].items[0]
                .value
                .contains(&i18n.text("permission-studio-path-readwrite-compact"))
        );
        assert_eq!(sections[0].items[1].value, i18n.text("value-deny"));
        assert!(dialog.permission.network.is_some());
        dialog.permission.network = None;
        assert_eq!(
            permission_studio_network(&dialog)
                .resolved()
                .egress
                .unwrap()
                .default,
            Some(NetworkAction::Allow)
        );
    }

    #[test]
    fn network_form_customization_copies_the_complete_parent_policy() {
        let dialog = inherited_dialog();
        let permission = permission_studio_custom_network(&dialog);
        assert_eq!(permission.network, dialog.inherited_permission.network);
        assert!(dialog.permission.network.is_none());
        let mut rule = permission_studio_network_rule_draft(&dialog, false, None).unwrap();
        rule.to = "192.0.2.11".into();
        rule.ports = "80".into();
        let network = permission_studio_network_rule_policy(&I18n::english(), &rule).unwrap();
        let value = serde_json::to_value(network).unwrap();
        assert_eq!(value["deny"][0]["to"], "10.0.0.0/8");
        assert_eq!(value["allow"][0]["to"], "192.0.2.10");
        assert_eq!(value["allow"][1]["to"], "192.0.2.11");
    }

    #[test]
    fn network_form_server_rejection_retains_draft_and_allows_retry() {
        let mut submitted = inherited_dialog();
        submitted.network_rule = Some(draft());
        submitted.save_request = Some(uuid::Uuid::new_v4());
        let mut current = submitted.clone();
        current.network_rule = None;
        assert!(permission_studio_restore_failed_save(
            &mut current,
            &submitted
        ));
        assert_eq!(current.network_rule, submitted.network_rule);
        assert!(current.permission.network.is_none());
        assert!(current.save_request.is_none());

        current.save_request = Some(uuid::Uuid::new_v4());
        let newer_request = current.save_request;
        assert!(!permission_studio_restore_failed_save(
            &mut current,
            &submitted
        ));
        assert_eq!(current.save_request, newer_request);
    }

    fn draft() -> PermissionStudioNetworkRuleDraft {
        PermissionStudioNetworkRuleDraft {
            policy: json!({"default":"deny","deny":[{"to":"10.0.0.0/8"}]}),
            original: None,
            deny: false,
            to: "192.0.2.10".into(),
            protocol: "tcp".into(),
            ports: "443, 8000-8099".into(),
            except: String::new(),
        }
    }

    #[test]
    fn network_form_keeps_other_rules_and_parses_human_port_lists() {
        let network = permission_studio_network_rule_policy(&I18n::english(), &draft()).unwrap();
        let value = serde_json::to_value(&network).unwrap();
        assert_eq!(value["deny"][0]["to"], "10.0.0.0/8");
        assert_eq!(value["allow"][0]["ports"], json!([443, "8000-8099"]));
        assert_eq!(value["allow"][0]["protocol"], "tcp");
    }

    #[test]
    fn invalid_network_form_keeps_all_draft_fields() {
        for ports in ["443,,80", "70000", "443.5", "9000-8000"] {
            let mut draft = draft();
            draft.ports = ports.into();
            let before = draft.clone();
            assert!(permission_studio_network_rule_policy(&I18n::english(), &draft).is_err());
            assert_eq!(draft, before);
        }
        let mut draft = draft();
        draft.protocol = "icmp".into();
        assert!(permission_studio_network_rule_policy(&I18n::english(), &draft).is_err());
    }

    #[test]
    fn network_form_moves_and_deletes_only_the_selected_rule() {
        let mut draft = draft();
        draft.policy["allow"] =
            json!([{"to":"192.0.2.10","protocol":"tcp","ports":[443]},{"to":"192.0.2.11"}]);
        draft.original = Some((false, 0));
        draft.deny = true;
        let policy = permission_studio_network_rule_policy(&I18n::english(), &draft).unwrap();
        let value = serde_json::to_value(&policy).unwrap();
        assert_eq!(value["allow"].as_array().unwrap().len(), 1);
        assert_eq!(value["deny"].as_array().unwrap().len(), 2);
        let deleted = permission_studio_network_delete_rule(&policy, true, 1).unwrap();
        let value = serde_json::to_value(deleted).unwrap();
        assert_eq!(value["deny"].as_array().unwrap().len(), 1);
        assert_eq!(value["allow"][0]["to"], "192.0.2.11");
    }
}
