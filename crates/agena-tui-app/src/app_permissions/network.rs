use crate::{
    App, ChoiceItem, ChoiceOverlayAction, ConfirmAction, Editor, I18n, Overlay,
    PermissionStudioEditor, PermissionStudioEditorAction, PermissionStudioFocus,
    PermissionStudioNetworkAction as Action, PermissionStudioNetworkField as Field,
    PermissionStudioOverlay, PermissionStudioPage, PermissionStudioSectionId, UiFailure, UiResult,
    editor_save_footer, permission_studio_custom_network, permission_studio_network,
    permission_studio_network_delete_rule, permission_studio_network_field_keys,
    permission_studio_network_field_text, permission_studio_network_rule_draft,
    permission_studio_network_rule_policy, permission_studio_read_only_message,
    refresh_permission_studio_dialog,
};

impl App {
    pub(crate) fn activate_permission_studio_network(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        action: Action,
    ) {
        if !dialog.editable {
            self.flash_warning(permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        if dialog.save_request.is_some() {
            self.flash_warning(self.i18n.text("permission-studio-network-saving"));
            return;
        }
        let result = self.apply_permission_studio_network_action(dialog, action);
        if let Err(error) = result {
            self.flash_error(error);
        }
    }

    fn apply_permission_studio_network_action(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        action: Action,
    ) -> UiResult<()> {
        match action {
            Action::Source => {
                if dialog.network_rule.is_some() {
                    return Err(UiFailure::message(
                        self.i18n.text("permission-studio-network-save-or-cancel"),
                    ));
                }
                if dialog.permission.network.is_some() {
                    self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
                        self.i18n.text("permission-studio-network-inherit-title"),
                        vec![self.i18n.text("permission-studio-network-inherit-help")],
                        ConfirmAction::PermissionStudioDeleteNetworkPolicy,
                    )));
                } else {
                    let permission = permission_studio_custom_network(dialog);
                    self.persist_permission_studio(dialog, permission)?;
                }
            }
            Action::Offline | Action::Outbound => {
                if dialog.network_rule.is_some() {
                    return Err(UiFailure::message(
                        self.i18n.text("permission-studio-network-save-or-cancel"),
                    ));
                }
                self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
                    self.i18n.text(if action == Action::Offline {
                        "permission-studio-network-offline"
                    } else {
                        "permission-studio-network-outbound"
                    }),
                    vec![self.i18n.text("permission-studio-network-preset-help")],
                    ConfirmAction::PermissionStudioNetworkPreset {
                        allow: action == Action::Outbound,
                    },
                )));
            }
            Action::AddRule { deny } | Action::EditRule { deny, .. } => {
                if dialog.network_rule.is_some() {
                    return Ok(());
                }
                let index = match action {
                    Action::EditRule { index, .. } => Some(index),
                    _ => None,
                };
                dialog.network_rule =
                    Some(permission_studio_network_rule_draft(dialog, deny, index)?);
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::Access,
                    Some(PermissionStudioSectionId::NetworkRules),
                    PermissionStudioFocus::Items,
                );
            }
            Action::Field(field) => self.open_permission_studio_network_field(dialog, field)?,
            Action::ToggleRuleAction => {
                let draft = dialog
                    .network_rule
                    .as_mut()
                    .ok_or_else(|| UiFailure::internal("network draft is no longer open"))?;
                draft.deny = !draft.deny;
                refresh_network_form(&self.i18n, dialog);
            }
            Action::SaveRule => {
                let draft = dialog
                    .network_rule
                    .as_ref()
                    .ok_or_else(|| UiFailure::internal("network draft is no longer open"))?;
                let network = permission_studio_network_rule_policy(&self.i18n, draft)?;
                let mut permission = dialog.permission.clone();
                permission.network = Some(network);
                // Retain the entire form until the server accepts the save.
                self.persist_permission_studio(dialog, permission)?;
            }
            Action::CancelRule => {
                dialog.network_rule = None;
                refresh_network_form(&self.i18n, dialog);
            }
            Action::DeleteRule => {
                let Some(crate::PermissionStudioAction::Network(Action::EditRule { deny, index })) =
                    dialog.state.selected_item().map(|item| item.action.clone())
                else {
                    return Err(UiFailure::message(
                        self.i18n.text("flash-permission-studio-no-selection"),
                    ));
                };
                let selected = dialog
                    .state
                    .selected_item()
                    .map(|item| item.label.clone())
                    .unwrap_or_default();
                self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
                    self.i18n.text("overlay-permission-studio-delete-title"),
                    vec![selected],
                    ConfirmAction::PermissionStudioDeleteNetworkRule { deny, index },
                )));
            }
        }
        Ok(())
    }

    fn open_permission_studio_network_field(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        field: Field,
    ) -> UiResult<()> {
        let draft = dialog
            .network_rule
            .as_ref()
            .ok_or_else(|| UiFailure::internal("network draft is no longer open"))?;
        let (label, help) = permission_studio_network_field_keys(field);
        if field == Field::Protocol {
            let items = ["tcp", "udp", "icmp", "any"]
                .into_iter()
                .map(|protocol| ChoiceItem {
                    label: protocol.to_string(),
                    detail: String::new(),
                    value: protocol.to_string(),
                    search_text: protocol.to_string(),
                    current: draft.protocol == protocol,
                })
                .collect();
            self.open_choice_overlay(self.build_choice_overlay(
                self.i18n.text(label),
                self.i18n.text(help),
                Some(draft.protocol.clone()),
                items,
                ChoiceOverlayAction::PermissionStudioNetworkProtocol,
                false,
                agena_tui::choice::ChoicePresentationStyle::SelectOnly,
            ));
        } else {
            dialog.editor = Some(PermissionStudioEditor::new(
                self.i18n.text(label),
                self.i18n.text(help),
                editor_save_footer(&self.i18n, false),
                false,
                Editor::from_text(permission_studio_network_field_text(draft, field).to_owned()),
                PermissionStudioEditorAction::NetworkRuleField(field),
            ));
        }
        Ok(())
    }

    pub(crate) fn set_permission_studio_network_preset(&mut self, allow: bool) {
        self.delete_permission_studio_config(move |permission| {
            permission.network = Some(if allow {
                agena_domain::NetworkPermissionConfig::allow_all()
            } else {
                agena_domain::NetworkPermissionConfig::offline()
            });
        });
    }

    pub(crate) fn delete_permission_studio_network_rule(&mut self, deny: bool, index: usize) {
        let Some((host, mut dialog)) = self.take_permission_studio_dialog() else {
            return;
        };
        let result =
            permission_studio_network_delete_rule(permission_studio_network(&dialog), deny, index)
                .and_then(|network| {
                    let mut permission = dialog.permission.clone();
                    permission.network = Some(network);
                    self.persist_permission_studio(&mut dialog, permission)
                });
        self.restore_permission_studio_dialog(host, dialog);
        if let Err(error) = result {
            self.flash_error(error);
        }
    }
}

fn refresh_network_form(i18n: &I18n, dialog: &mut PermissionStudioOverlay) {
    refresh_permission_studio_dialog(i18n, dialog, None, None, Some(PermissionStudioFocus::Items));
}
