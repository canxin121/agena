impl App {
    pub(in crate::app) fn open_plugin_workbench(&mut self, query: &str) {
        match self.build_plugin_workbench(query) {
            Ok(dialog) => {
                self.route_stack.clear();
                self.current_route = Route::PluginWorkbench(Box::new(dialog));
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(in crate::app) fn build_plugin_policy_studio(&self) -> UiResult<PluginPolicyStudioOverlay> {
        let sources = self
            .backend
            .config_json_sources()
            .map_err(|error| error.to_string())?;
        let locale = self.i18n.locale_tag();
        let statuses = self.backend.plugin_statuses();
        let sections =
            build_plugin_policy_sections(&sources, locale.as_str(), statuses, |plugin_id| {
                self.backend.plugin_inspect(&plugin_id.to_string())
            });
        Ok(PluginPolicyStudioOverlay {
            title: "Plugin Policy Studio".to_owned(),
            footer: "Tab/Alt+Tab switches plugin list and rows. Left/Right picks Prompt or UI. Enter cycles the selected value; Delete clears it.".to_owned(),
            config_path: sources.config_path.display().to_string(),
            config_found: sources.config_found,
            selected_column: PluginPolicyColumn::Prompt,
            visible_section_page_size: Cell::new(1),
            visible_item_page_size: Cell::new(1),
            state: SectionedListState::new(sections, 0, 0, SectionedListFocus::Navigation),
        })
    }

    pub(in crate::app) fn refresh_plugin_policy_studio(
        &mut self,
        dialog: &mut PluginPolicyStudioOverlay,
    ) {
        let selected_plugin_id = dialog
            .state
            .selected_section()
            .map(|section| section.plugin_id.clone());
        let selected_item_key = dialog.state.selected_item().map(|item| item.key.clone());
        let focus = dialog.state.focus();
        let selected_column = dialog.selected_column;
        match self.build_plugin_policy_studio() {
            Ok(mut refreshed) => {
                refreshed.selected_column = selected_column;
                refreshed.state.set_focus(focus);
                if let Some(plugin_id) = selected_plugin_id.as_deref()
                    && let Some(section_index) = refreshed
                        .state
                        .sections()
                        .iter()
                        .position(|section| section.plugin_id == plugin_id)
                {
                    let item_index = selected_item_key
                        .as_deref()
                        .and_then(|key| {
                            refreshed.state.sections()[section_index]
                                .items
                                .iter()
                                .position(|item| item.key == key)
                        })
                        .unwrap_or_default();
                    refreshed.state.set_indices(section_index, item_index);
                }
                *dialog = refreshed;
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(in crate::app) fn refresh_restored_plugin_policy_studio(
        &self,
        dialog: PluginPolicyStudioOverlay,
    ) -> PluginPolicyStudioOverlay {
        let selected_plugin_id = dialog
            .state
            .selected_section()
            .map(|section| section.plugin_id.clone());
        let selected_item_key = dialog.state.selected_item().map(|item| item.key.clone());
        let focus = dialog.state.focus();
        let selected_column = dialog.selected_column;
        let Ok(mut refreshed) = self.build_plugin_policy_studio() else {
            return dialog;
        };
        refreshed.selected_column = selected_column;
        refreshed.state.set_focus(focus);
        if let Some(plugin_id) = selected_plugin_id.as_deref()
            && let Some(section_index) = refreshed
                .state
                .sections()
                .iter()
                .position(|section| section.plugin_id == plugin_id)
        {
            let item_index = selected_item_key
                .as_deref()
                .and_then(|key| {
                    refreshed.state.sections()[section_index]
                        .items
                        .iter()
                        .position(|item| item.key == key)
                })
                .unwrap_or_default();
            refreshed.state.set_indices(section_index, item_index);
        }
        refreshed
    }

    pub(in crate::app) fn handle_plugin_policy_studio_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginPolicyStudioOverlay,
    ) -> bool {
        match resolve_tui_key(KeyContext::PluginPolicy, key) {
            Some(KeyAction::Close) => true,
            Some(KeyAction::NextTab | KeyAction::PreviousTab) => {
                dialog.state.set_focus(match dialog.state.focus() {
                    SectionedListFocus::Navigation => SectionedListFocus::Items,
                    SectionedListFocus::Items => SectionedListFocus::Navigation,
                });
                false
            }
            Some(KeyAction::Activate) if dialog.state.focus() == SectionedListFocus::Navigation => {
                false
            }
            Some(KeyAction::MoveUp) => {
                dialog.state.move_selection(-1);
                false
            }
            Some(KeyAction::MoveDown) => {
                dialog.state.move_selection(1);
                false
            }
            Some(KeyAction::MoveLeft) if dialog.state.focus() == SectionedListFocus::Items => {
                dialog.selected_column = PluginPolicyColumn::Prompt;
                false
            }
            Some(KeyAction::MoveRight) if dialog.state.focus() == SectionedListFocus::Items => {
                dialog.selected_column = PluginPolicyColumn::Ui;
                false
            }
            Some(KeyAction::Activate) if dialog.state.focus() == SectionedListFocus::Items => {
                self.cycle_plugin_policy_override(dialog);
                false
            }
            Some(KeyAction::Delete) if dialog.state.focus() == SectionedListFocus::Items => {
                self.clear_plugin_policy_override(dialog);
                false
            }
            _ => false,
        }
    }

    fn cycle_plugin_policy_override(&mut self, dialog: &mut PluginPolicyStudioOverlay) {
        let Some(item) = dialog.selected_item() else {
            return;
        };
        match dialog.selected_column {
            PluginPolicyColumn::Prompt => {
                let next = match item
                    .prompt_file_override
                    .unwrap_or(agena::plugin::ToolDescriptionOverride::ToolDefault)
                {
                    agena::plugin::ToolDescriptionOverride::ToolDefault => {
                        Some(agena::plugin::ToolDescriptionOverride::Detailed)
                    }
                    agena::plugin::ToolDescriptionOverride::Detailed => {
                        Some(agena::plugin::ToolDescriptionOverride::Brief)
                    }
                    agena::plugin::ToolDescriptionOverride::Brief => None,
                };
                self.set_plugin_policy_prompt_override(dialog, next);
            }
            PluginPolicyColumn::Ui => {
                let next = match item
                    .ui_file_override
                    .unwrap_or(agena::plugin::UiPresentationOverride::Default)
                {
                    agena::plugin::UiPresentationOverride::Default => {
                        Some(agena::plugin::UiPresentationOverride::Detailed)
                    }
                    agena::plugin::UiPresentationOverride::Detailed => {
                        Some(agena::plugin::UiPresentationOverride::Summary)
                    }
                    agena::plugin::UiPresentationOverride::Summary => None,
                };
                self.set_plugin_policy_ui_override(dialog, next);
            }
        }
    }

    fn clear_plugin_policy_override(&mut self, dialog: &mut PluginPolicyStudioOverlay) {
        match dialog.selected_column {
            PluginPolicyColumn::Prompt => self.set_plugin_policy_prompt_override(dialog, None),
            PluginPolicyColumn::Ui => self.set_plugin_policy_ui_override(dialog, None),
        }
    }

    fn set_plugin_policy_prompt_override(
        &mut self,
        dialog: &mut PluginPolicyStudioOverlay,
        mode: Option<agena::plugin::ToolDescriptionOverride>,
    ) {
        let Some(item) = dialog.selected_item().cloned() else {
            return;
        };
        let op = match mode {
            Some(agena::plugin::ToolDescriptionOverride::Detailed) => {
                self.block_on_async(self.backend.set_config_setting(
                    item.prompt_path.as_str(),
                    JsonValue::String("detailed".to_owned()),
                ))
            }
            Some(agena::plugin::ToolDescriptionOverride::Brief) => {
                self.block_on_async(self.backend.set_config_setting(
                    item.prompt_path.as_str(),
                    JsonValue::String("brief".to_owned()),
                ))
            }
            _ => self.block_on_async(
                self.backend
                    .delete_config_setting(item.prompt_path.as_str()),
            ),
        };
        match op {
            Ok(_) => {
                self.flash_success(format!("Updated prompt policy for {}.", item.scope_label));
                self.refresh_plugin_policy_studio(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn set_plugin_policy_ui_override(
        &mut self,
        dialog: &mut PluginPolicyStudioOverlay,
        mode: Option<agena::plugin::UiPresentationOverride>,
    ) {
        let Some(item) = dialog.selected_item().cloned() else {
            return;
        };
        let op = match mode {
            Some(agena::plugin::UiPresentationOverride::Detailed) => {
                self.block_on_async(self.backend.set_config_setting(
                    item.ui_path.as_str(),
                    JsonValue::String("detailed".to_owned()),
                ))
            }
            Some(agena::plugin::UiPresentationOverride::Summary) => {
                self.block_on_async(self.backend.set_config_setting(
                    item.ui_path.as_str(),
                    JsonValue::String("summary".to_owned()),
                ))
            }
            _ => self.block_on_async(self.backend.delete_config_setting(item.ui_path.as_str())),
        };
        match op {
            Ok(_) => {
                self.flash_success(format!("Updated UI policy for {}.", item.scope_label));
                self.refresh_plugin_policy_studio(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }
}
use super::{
    App, Cell, JsonValue, KeyEvent, PluginPolicyColumn, PluginPolicyStudioOverlay, Route,
    SectionedListFocus, SectionedListState, UiResult, build_plugin_policy_sections,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
