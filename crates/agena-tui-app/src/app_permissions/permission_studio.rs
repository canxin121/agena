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
            PermissionStudioAction::AddToolCommandPattern { tool_name } => {
                if !dialog.editable {
                    self.flash_warning(permission_studio_read_only_message(
                        &self.i18n,
                        &dialog.source,
                    ));
                    false
                } else {
                    self.open_permission_studio_creator(
                        dialog,
                        PermissionStudioEditorAction::AddToolCommandPattern { tool_name },
                    );
                    false
                }
            }
        }
    }

    pub(crate) fn open_permission_studio_mode_overlay(
        &mut self,
        dialog: &PermissionStudioOverlay,
        target: PermissionStudioModeTarget,
    ) {
        let current_value = permission_studio_mode_target_input_text(dialog, &target);
        let current_value = (!current_value.trim().is_empty()).then_some(current_value);
        self.open_choice_overlay(self.build_choice_overlay(
            settings_edit_title(
                &self.i18n,
                permission_studio_mode_target_label(&self.i18n, &target).as_str(),
            ),
            String::new(),
            current_value,
            if target.is_path() {
                path_permission_mode_choice_items(&self.i18n)
            } else if target.is_network() {
                permission_mode_choice_items(&self.i18n)
                    .into_iter()
                    .filter(|item| matches!(item.value.as_str(), "allow" | "deny"))
                    .collect()
            } else {
                permission_mode_choice_items(&self.i18n)
            },
            ChoiceOverlayAction::PermissionStudioMode(target.clone()),
            !target.is_network(),
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        ));
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
        let input = serde_json::to_string_pretty(crate::permission_studio_network(dialog))
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
            self.flash_warning(
                self.i18n
                    .text("permission-studio-inherited-rule-no-local-edit"),
            );
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
            PermissionStudioEditorAction::AddToolRule => {
                let tool_name = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-tool-rules").as_str(),
                    input.as_str(),
                )
                .map_err(crate::UiFailure::message)?;
                if !matches!(
                    tool_name.as_str(),
                    "agena.shell.exec"
                        | "agena.shell.spawn"
                        | "agena.shell.watch"
                        | "agena.shell.open"
                        | "agena.shell.write"
                ) {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "permission-studio-command-rules-shell-only",
                    ));
                    return Ok(());
                }
                let mut permission = crate::permission_studio_local_config(dialog);
                let rule = ToolPermissionRules::Mode(PermissionMode::Auto);
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .rules
                    .insert(tool_name.clone(), rule);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::Access,
                    Some(PermissionStudioSectionId::ToolCommandRules),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddToolCommandPattern { tool_name } => {
                let pattern = parse_permission_studio_key_input(
                    &self.i18n,
                    "command pattern",
                    input.as_str(),
                )
                .map_err(crate::UiFailure::message)?;
                let target = PermissionStudioModeTarget::ToolCommandPattern {
                    tool_name: tool_name.clone(),
                    pattern: pattern.clone(),
                };
                let mut permission =
                    crate::permission_studio_mode_edit_config(dialog, &target, false);
                let tools = permission.tools.get_or_insert_with(Default::default);
                let existing = tools.rules.remove(tool_name.as_str());
                let mut entries = match existing {
                    Some(ToolPermissionRules::Ordered(entries)) => entries,
                    Some(ToolPermissionRules::Mode(mode)) => {
                        let mut entries = indexmap::IndexMap::new();
                        entries.insert("*".to_string(), mode);
                        entries
                    }
                    None => indexmap::IndexMap::new(),
                };
                entries.insert(pattern, PermissionMode::Auto);
                tools
                    .rules
                    .insert(tool_name, ToolPermissionRules::Ordered(entries));
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
    App, ChoiceOverlayAction, Editor, PermissionMode, PermissionStudioAction,
    PermissionStudioCatalogKind, PermissionStudioEditor, PermissionStudioEditorAction,
    PermissionStudioFocus, PermissionStudioModeTarget, PermissionStudioOverlay,
    PermissionStudioPage, PermissionStudioSectionId, PermissionStudioTextTarget,
    ToolPermissionRules, UiResult, apply_permission_studio_network_input,
    apply_permission_studio_text_input, editor_save_footer, parse_permission_studio_key_input,
    path_permission_mode_choice_items, permission_mode_choice_items,
    permission_studio_creator_input_text, permission_studio_creator_spec,
    permission_studio_mode_target_input_text, permission_studio_mode_target_label,
    permission_studio_read_only_message, permission_studio_text_target_input_text,
    permission_studio_text_target_label, settings_edit_title, ui_text,
};
