impl App {
    pub(in crate::app) fn activate_permission_studio_selection(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
    ) -> bool {
        if dialog.pane_focus == PermissionStudioPaneFocus::Navigation {
            set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Content);
            return false;
        }
        let Some(item) = dialog.state.selected_item().cloned() else {
            return false;
        };
        match item.action {
            PermissionStudioAction::Noop => return false,
            PermissionStudioAction::EditMode(target) => {
                if !dialog.editable {
                    self.flash_warning(permission_studio_read_only_message(
                        &self.i18n,
                        &dialog.source,
                    ));
                    return false;
                }
                self.open_permission_studio_mode_overlay(dialog, target);
                return false;
            }
            PermissionStudioAction::AddToolCommandPattern { tool_name } => {
                if !dialog.editable {
                    self.flash_warning(permission_studio_read_only_message(
                        &self.i18n,
                        &dialog.source,
                    ));
                    return false;
                }
                self.open_permission_studio_creator(
                    dialog,
                    PermissionStudioEditorAction::AddToolCommandPattern { tool_name },
                );
                return false;
            }
        }
    }

    pub(in crate::app) fn apply_permission_studio_nav_selection(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
    ) {
        let Some(item) = dialog.nav.selected_item().cloned() else {
            return;
        };
        if !item.selectable {
            return;
        };
        self.set_permission_studio_page_with_section(
            dialog,
            item.page,
            item.section,
            dialog.state.focus(),
        );
    }

    pub(in crate::app) fn open_permission_studio_mode_overlay(
        &mut self,
        dialog: &PermissionStudioOverlay,
        target: PermissionStudioModeTarget,
    ) {
        self.open_choice_overlay(self.build_choice_overlay(
            settings_edit_title(
                &self.i18n,
                permission_studio_mode_target_label(&self.i18n, &target).as_str(),
            ),
            String::new(),
            Editor::from_text(permission_studio_mode_target_input_text(dialog, &target)),
            permission_mode_choice_items(&self.i18n),
            ChoiceOverlayAction::PermissionStudioMode(target),
            true,
            ChoiceOverlayStyle::SelectOnly,
        ));
    }

    pub(in crate::app) fn open_permission_studio_text_editor(
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

    pub(in crate::app) fn open_permission_studio_rename_current(
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
        let target = match action {
            PermissionStudioAction::EditMode(PermissionStudioModeTarget::PathRuleRead {
                pattern,
            })
            | PermissionStudioAction::EditMode(PermissionStudioModeTarget::PathRuleWrite {
                pattern,
            }) => PermissionStudioTextTarget::PathRulePattern { pattern },
            PermissionStudioAction::EditMode(PermissionStudioModeTarget::NetworkRule {
                target,
            }) => PermissionStudioTextTarget::NetworkRuleTarget { target },
            PermissionStudioAction::EditMode(PermissionStudioModeTarget::ToolTag { key }) => {
                PermissionStudioTextTarget::ToolTagKey { key }
            }
            PermissionStudioAction::EditMode(PermissionStudioModeTarget::ToolName { key }) => {
                PermissionStudioTextTarget::ToolNameKey { key }
            }
            PermissionStudioAction::EditMode(PermissionStudioModeTarget::ToolRule {
                tool_name,
            }) => PermissionStudioTextTarget::ToolRuleName { tool_name },
            _ => {
                self.flash_warning("This entry cannot be renamed; delete and recreate it instead.");
                return;
            }
        };
        self.open_permission_studio_text_editor(dialog, target);
    }

    pub(in crate::app) fn open_permission_studio_creator(
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

    pub(in crate::app) fn commit_permission_studio_editor(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        action: PermissionStudioEditorAction,
        input: String,
    ) -> UiResult<()> {
        match action {
            PermissionStudioEditorAction::Text(target) => {
                let mut permission = dialog.permission.clone();
                let next_page = apply_permission_studio_text_input(
                    &self.i18n,
                    &mut permission,
                    &target,
                    input.as_str(),
                )?;
                self.persist_permission_studio(dialog, permission)?;
                let next_section = match next_page {
                    PermissionStudioPage::PathRules => Some(PermissionStudioSectionId::PathRules),
                    PermissionStudioPage::NetworkRules => {
                        Some(PermissionStudioSectionId::NetworkRules)
                    }
                    PermissionStudioPage::ToolTags => Some(PermissionStudioSectionId::ToolTags),
                    PermissionStudioPage::ToolNames => Some(PermissionStudioSectionId::ToolNames),
                    PermissionStudioPage::ToolCommandRules => {
                        Some(PermissionStudioSectionId::ToolCommandRules)
                    }
                    PermissionStudioPage::PathDefaults
                    | PermissionStudioPage::NetworkZones
                    | PermissionStudioPage::Overview => None,
                };
                self.set_permission_studio_page_with_section(
                    dialog,
                    next_page,
                    next_section,
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddPathRule { duplicate_from } => {
                let pattern = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-path-rules").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let rule = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .path
                            .as_ref()
                            .and_then(|path| path.rules.get(from.as_str()))
                            .cloned()
                    })
                    .unwrap_or_else(|| {
                        PathAccessRuleConfig::Modes(PathAccessModes {
                            read: Some(PermissionMode::Ask),
                            write: Some(PermissionMode::Ask),
                        })
                    });
                permission
                    .path
                    .get_or_insert_with(Default::default)
                    .rules
                    .insert(pattern.clone(), rule);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::PathRules,
                    Some(PermissionStudioSectionId::PathRules),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddNetworkRule { duplicate_from } => {
                let target = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-network-rules").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let mode = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .network
                            .as_ref()
                            .and_then(|network| network.rules.get(from.as_str()).copied())
                    })
                    .unwrap_or(PermissionMode::Ask);
                permission
                    .network
                    .get_or_insert_with(Default::default)
                    .rules
                    .insert(target.clone(), mode);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::NetworkRules,
                    Some(PermissionStudioSectionId::NetworkRules),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddToolTag { duplicate_from } => {
                let key = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-tool-tags").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let mode = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.tags.get(from.as_str()).copied())
                    })
                    .unwrap_or(PermissionMode::Ask);
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .tags
                    .insert(key.clone(), mode);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::ToolTags,
                    Some(PermissionStudioSectionId::ToolTags),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddToolName { duplicate_from } => {
                let key = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-tool-names").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let mode = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.names.get(from.as_str()).copied())
                    })
                    .unwrap_or(PermissionMode::Ask);
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .names
                    .insert(key.clone(), mode);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::ToolNames,
                    Some(PermissionStudioSectionId::ToolNames),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddToolRule { duplicate_from } => {
                let tool_name = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-tool-rules").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let rule = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.rules.get(from.as_str()).cloned())
                    })
                    .unwrap_or(ToolPermissionRules::Mode(PermissionMode::Ask));
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .rules
                    .insert(tool_name.clone(), rule);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::ToolCommandRules,
                    Some(PermissionStudioSectionId::ToolCommandRules),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddToolCommandPattern { tool_name } => {
                let pattern = parse_permission_studio_key_input(
                    &self.i18n,
                    "command pattern",
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
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
                entries.insert(pattern, PermissionMode::Ask);
                tools
                    .rules
                    .insert(tool_name, ToolPermissionRules::Ordered(entries));
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::ToolCommandRules,
                    Some(PermissionStudioSectionId::ToolCommandRules),
                    PermissionStudioFocus::Items,
                );
            }
        }
        Ok(())
    }
}
use crate::app::{
    App, ChoiceOverlayAction, ChoiceOverlayStyle, Editor, PathAccessModes, PathAccessRuleConfig,
    PermissionMode, PermissionStudioAction, PermissionStudioEditor, PermissionStudioEditorAction,
    PermissionStudioFocus, PermissionStudioModeTarget, PermissionStudioOverlay,
    PermissionStudioPage, PermissionStudioPaneFocus, PermissionStudioSectionId,
    PermissionStudioTextTarget, ToolPermissionRules, UiResult, apply_permission_studio_text_input,
    editor_save_footer, parse_permission_studio_key_input, permission_mode_choice_items,
    permission_studio_creator_input_text, permission_studio_creator_spec,
    permission_studio_mode_target_input_text, permission_studio_mode_target_label,
    permission_studio_read_only_message, permission_studio_text_target_input_text,
    permission_studio_text_target_label, set_permission_studio_pane_focus, settings_edit_title,
    ui_text,
};
