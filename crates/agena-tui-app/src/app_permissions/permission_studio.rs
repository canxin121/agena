use agena_domain::{SHELL_COMMAND_RULE_GROUP, ToolPermissionConfig};

impl App {
    pub(crate) fn activate_permission_studio_selection(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
    ) -> bool {
        let Some(item) = dialog.state.selected_item().cloned() else {
            return false;
        };
        if dialog.network_rule.is_some()
            && dialog
                .state
                .selected_section()
                .is_some_and(|section| section.id != crate::PermissionStudioSectionId::NetworkRules)
        {
            self.flash_warning(self.i18n.text("permission-studio-network-save-or-cancel"));
            return false;
        }
        if !crate::permission_studio_action_editable(dialog, &item.action) {
            self.flash_warning(crate::permission_studio_inherited_message(
                &self.i18n, dialog,
            ));
            return false;
        }
        match item.action {
            PermissionStudioAction::RememberedRules => {
                self.open_remembered_permission_rules(dialog);
                false
            }
            PermissionStudioAction::CreateRule => {
                self.open_permission_studio_add_current(dialog);
                false
            }
            PermissionStudioAction::EditMode(target) => {
                if !dialog.editable {
                    self.flash_warning(permission_studio_read_only_message(
                        &self.i18n,
                        &dialog.source,
                    ));
                    false
                } else {
                    self.open_permission_studio_mode_overlay(dialog, target);
                    false
                }
            }
            PermissionStudioAction::EditNetworkPolicy => {
                self.open_permission_studio_network_editor(dialog);
                false
            }
            PermissionStudioAction::Network(action) => {
                self.activate_permission_studio_network(dialog, action);
                false
            }
            PermissionStudioAction::AddToolCommandPattern => {
                self.open_permission_studio_creator(
                    dialog,
                    PermissionStudioEditorAction::AddToolCommandPattern,
                );
                false
            }
        }
    }

    pub(crate) fn open_permission_studio_mode_overlay(
        &mut self,
        dialog: &PermissionStudioOverlay,
        target: PermissionStudioModeTarget,
    ) {
        if !crate::permission_studio_target_editable(dialog, &target) {
            self.flash_warning(crate::permission_studio_inherited_message(
                &self.i18n, dialog,
            ));
            return;
        }
        let global = matches!(dialog.source, crate::PermissionStudioSource::GlobalConfig);
        let mut value_dialog = dialog.clone();
        if global {
            value_dialog.permission = crate::PermissionStudioProjection::new(dialog).effective;
        }
        let current_value = permission_studio_mode_target_input_text(&value_dialog, &target);
        let current_value = (!current_value.trim().is_empty()).then_some(current_value);
        let mut items = if target.is_path() {
            path_permission_mode_choice_items(&self.i18n)
        } else if target.is_network() {
            permission_mode_choice_items(&self.i18n)
                .into_iter()
                .filter(|item| matches!(item.value.as_str(), "allow" | "deny"))
                .collect()
        } else {
            permission_mode_choice_items(&self.i18n)
        };
        let is_read_only_class = matches!(&target,
            PermissionStudioModeTarget::ToolCommandPattern { tool_name, pattern }
                if tool_name == "*" && pattern == "read-only");
        if is_read_only_class && let Some(auto) = items.iter_mut().find(|item| item.value == "auto")
        {
            let projection = crate::PermissionStudioProjection::new(dialog);
            auto.label = self.i18n.text_args(
                "permission-studio-read-only-follows-default",
                &agena_tui::fl_args!("value" => crate::permission_mode_label(&self.i18n,
                    crate::tool_default(&projection.effective))),
            );
        }
        let mut picker = self.build_choice_overlay(
            settings_edit_title(
                &self.i18n,
                permission_studio_mode_target_label(&self.i18n, &target).as_str(),
            ),
            String::new(),
            current_value,
            items,
            ChoiceOverlayAction::PermissionStudioMode(target.clone()),
            !global,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        );
        if (matches!(
            target,
            PermissionStudioModeTarget::PathWorkspace
                | PermissionStudioModeTarget::PathExternal
                | PermissionStudioModeTarget::ToolDefault
        ) || is_read_only_class
            || target.is_network())
            && let Some(clear) = picker.presentation.clear_action.as_mut()
        {
            if is_read_only_class
                && crate::PermissionStudioProjection::new(dialog).origin(&target)
                    == crate::PermissionStudioValueSource::Current
            {
                clear.current = false;
            }
            // Preview the same reset and domain merge as the save path. Keep
            // invalid wildcard entries visible so their state is not called inherited.
            let mut reset = dialog.clone();
            reset.permission = crate::permission_studio_mode_edit_config(dialog, &target, true);
            if crate::apply_permission_studio_mode_input(
                &self.i18n,
                &mut reset.permission,
                &target,
                "",
            )
            .is_ok()
            {
                let value = crate::PermissionStudioProjection::new(&reset).mode_item(
                    &self.i18n,
                    String::new(),
                    target.clone(),
                );
                let label = if matches!(
                    value.source,
                    Some(
                        crate::PermissionStudioValueSource::Inherited
                            | crate::PermissionStudioValueSource::Default
                    )
                ) {
                    crate::settings_layer_reset_label(
                        &self.i18n,
                        !matches!(dialog.source, crate::PermissionStudioSource::GlobalConfig),
                    )
                } else {
                    clear.label.clone()
                };
                clear.label = format!("{label} · {}", value.value);
            }
        }
        self.open_choice_overlay(picker);
    }

    pub(crate) fn open_permission_studio_network_editor(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
    ) {
        if dialog.network_rule.is_some() {
            self.flash_warning(self.i18n.text("permission-studio-network-save-or-cancel"));
            return;
        }
        if !dialog.editable {
            self.flash_warning(permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        if !crate::permission_studio_network_editable(dialog) {
            self.flash_warning(crate::permission_studio_inherited_message(
                &self.i18n, dialog,
            ));
            return;
        }
        let input =
            serde_json::to_string_pretty(&dialog.permission.network.clone().unwrap_or_default())
                .unwrap_or_default();
        dialog.editor = Some(PermissionStudioEditor::new(
            self.i18n.text("permission-studio-network-json"),
            self.i18n.text("permission-studio-network-json-help"),
            editor_save_footer(&self.i18n, true),
            true,
            Editor::from_text(input),
            PermissionStudioEditorAction::EditNetworkPolicy,
        ));
    }

    pub(crate) fn open_permission_studio_text_editor(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        target: PermissionStudioTextTarget,
    ) {
        let title = settings_edit_title(
            &self.i18n,
            permission_studio_text_target_label(&self.i18n, &target).as_str(),
        );
        let prompt = String::new();
        let footer = editor_save_footer(&self.i18n, false);
        let input = Editor::from_text(permission_studio_text_target_input_text(&target));
        dialog.editor = Some(PermissionStudioEditor::new(
            title,
            prompt,
            footer,
            false,
            input,
            PermissionStudioEditorAction::Text(target),
        ));
    }

    pub(crate) fn open_permission_studio_rename_current(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
    ) {
        if !dialog.editable {
            self.flash_warning(permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        let Some(action) = dialog.state.selected_item().map(|item| item.action.clone()) else {
            return;
        };
        if let PermissionStudioAction::EditMode(target) = &action
            && matches!(
                target,
                PermissionStudioModeTarget::PathRule { .. }
                    | PermissionStudioModeTarget::ToolName { .. }
                    | PermissionStudioModeTarget::ToolRule { .. }
            )
            && !crate::permission_studio_has_override(dialog, target)
        {
            self.flash_warning(self.i18n.text(
                if matches!(dialog.source, crate::PermissionStudioSource::GlobalConfig) {
                    "permission-studio-default-rule-no-local-edit"
                } else {
                    "permission-studio-inherited-rule-no-local-edit"
                },
            ));
            return;
        }
        let target = match action {
            PermissionStudioAction::EditMode(PermissionStudioModeTarget::PathRule { pattern }) => {
                PermissionStudioTextTarget::PathRulePattern { pattern }
            }
            PermissionStudioAction::EditMode(PermissionStudioModeTarget::ToolName { key }) => {
                PermissionStudioTextTarget::ToolNameKey { key }
            }
            PermissionStudioAction::EditMode(PermissionStudioModeTarget::ToolRule {
                tool_name,
            }) => PermissionStudioTextTarget::ToolRuleName { tool_name },
            _ => {
                self.flash_warning(self.i18n.text("permission-studio-rename-unsupported"));
                return;
            }
        };
        self.open_permission_studio_text_editor(dialog, target);
    }

    pub(crate) fn open_permission_studio_creator(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        action: PermissionStudioEditorAction,
    ) {
        if action == PermissionStudioEditorAction::AddToolCommandPattern
            && !crate::permission_studio_target_editable(
                dialog,
                &PermissionStudioModeTarget::ToolRule {
                    tool_name: SHELL_COMMAND_RULE_GROUP.to_owned(),
                },
            )
        {
            self.flash_warning(crate::permission_studio_inherited_message(
                &self.i18n, dialog,
            ));
            return;
        }
        let (title, prompt) = permission_studio_creator_spec(&self.i18n, &action);
        let input = Editor::from_text(permission_studio_creator_input_text(&action));
        dialog.editor = Some(PermissionStudioEditor::new(
            title,
            prompt,
            editor_save_footer(&self.i18n, false),
            false,
            input,
            action,
        ));
    }

    pub(crate) fn commit_permission_studio_editor(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        action: PermissionStudioEditorAction,
        input: String,
    ) -> UiResult<()> {
        match action {
            PermissionStudioEditorAction::Text(target) => {
                let mut permission = crate::permission_studio_local_config(dialog);
                let next_section = apply_permission_studio_text_input(
                    &self.i18n,
                    &mut permission,
                    &target,
                    input.as_str(),
                )?;
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::Access,
                    Some(next_section),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddPathRule => {
                let pattern = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-path-rules").as_str(),
                    input.as_str(),
                )
                .map_err(crate::UiFailure::message)?;
                let pattern = agena_domain::canonical_permission_path(&pattern)
                    .map_err(crate::UiFailure::message)?;
                if dialog
                    .permission
                    .path
                    .as_ref()
                    .is_some_and(|path| path.rules.contains_key(&pattern))
                {
                    return Err(crate::UiFailure::message(
                        self.i18n.text("permission-studio-path-duplicate"),
                    ));
                }
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::Access,
                    Some(PermissionStudioSectionId::PathRules),
                    PermissionStudioFocus::Items,
                );
                // Choose the resource mode before the first write. A temporary
                // deny rule is unsupported on some hosts and must not block
                // creating a valid readonly/readwrite rule there.
                self.open_permission_studio_mode_overlay(
                    dialog,
                    PermissionStudioModeTarget::PathRule { pattern },
                );
            }
            PermissionStudioEditorAction::EditNetworkPolicy => {
                let mut permission = dialog.permission.clone();
                apply_permission_studio_network_input(&mut permission, &input)?;
                self.persist_permission_studio(dialog, permission)?;
            }
            PermissionStudioEditorAction::NetworkRuleField(field) => {
                let draft = dialog
                    .network_rule
                    .as_mut()
                    .ok_or_else(|| crate::UiFailure::internal("network draft is no longer open"))?;
                match field {
                    crate::PermissionStudioNetworkField::Target => {
                        draft.to = input.trim().to_string()
                    }
                    crate::PermissionStudioNetworkField::Protocol => {
                        draft.protocol = input.trim().to_string()
                    }
                    crate::PermissionStudioNetworkField::Ports => {
                        draft.ports = input.trim().to_string()
                    }
                    crate::PermissionStudioNetworkField::Except => {
                        draft.except = input.trim().to_string()
                    }
                }
                crate::refresh_permission_studio_dialog(
                    &self.i18n,
                    dialog,
                    None,
                    None,
                    Some(PermissionStudioFocus::Items),
                );
            }
            PermissionStudioEditorAction::AddToolName => {
                let key = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-tool-names").as_str(),
                    input.as_str(),
                )
                .map_err(crate::UiFailure::message)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::Access,
                    Some(PermissionStudioSectionId::ToolNames),
                    PermissionStudioFocus::Items,
                );
                self.open_permission_studio_add_entries_mode(
                    PermissionStudioCatalogKind::ToolNames,
                    vec![key],
                    false,
                );
            }
            PermissionStudioEditorAction::AddToolCommandPattern => {
                let pattern = parse_permission_studio_key_input(
                    &self.i18n,
                    &self.i18n.text("permission-studio-nav-command-rules"),
                    input.as_str(),
                )
                .map_err(crate::UiFailure::message)?;
                let effective = crate::PermissionStudioProjection::new(dialog).effective;
                if effective
                    .tools
                    .as_ref()
                    .and_then(ToolPermissionConfig::command_rules)
                    .is_some_and(|rules| rules.contains_key(&pattern))
                {
                    return Err(crate::UiFailure::message(
                        self.i18n.text("permission-studio-rule-duplicate"),
                    ));
                }
                let target = PermissionStudioModeTarget::ToolCommandPattern {
                    tool_name: SHELL_COMMAND_RULE_GROUP.to_owned(),
                    pattern,
                };
                let mut permission =
                    crate::permission_studio_mode_edit_config(dialog, &target, false);
                crate::apply_permission_studio_mode_input(
                    &self.i18n,
                    &mut permission,
                    &target,
                    "auto",
                )?;
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::Access,
                    Some(PermissionStudioSectionId::ToolCommandRules),
                    PermissionStudioFocus::Items,
                );
            }
        }
        Ok(())
    }
}
use crate::{
    App, ChoiceOverlayAction, Editor, PermissionStudioAction, PermissionStudioCatalogKind,
    PermissionStudioEditor, PermissionStudioEditorAction, PermissionStudioFocus,
    PermissionStudioModeTarget, PermissionStudioOverlay, PermissionStudioPage,
    PermissionStudioSectionId, PermissionStudioTextTarget, UiResult,
    apply_permission_studio_network_input, apply_permission_studio_text_input, editor_save_footer,
    parse_permission_studio_key_input, path_permission_mode_choice_items,
    permission_mode_choice_items, permission_studio_creator_input_text,
    permission_studio_creator_spec, permission_studio_mode_target_input_text,
    permission_studio_mode_target_label, permission_studio_read_only_message,
    permission_studio_text_target_input_text, permission_studio_text_target_label,
    settings_edit_title, ui_text,
};
