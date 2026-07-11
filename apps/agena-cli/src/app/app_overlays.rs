impl App {
    pub(in crate::app) fn handle_runtime_setting_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut RuntimeSettingEditOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(action, input) => {
                match self.run_options.apply_runtime_setting_input(
                    &self.i18n,
                    action,
                    input.as_str(),
                ) {
                    Ok(message) => {
                        self.flash_success(message);
                        self.refresh_current_route_after_local_edit();
                        true
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            InputDialogKeyResult::Continue => false,
        }
    }

    pub(in crate::app) fn handle_choice_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ChoiceOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Tab => {
                if dialog.fill_input_from_selected() {
                    Self::sync_choice_overlay_input(dialog, true);
                }
                false
            }
            KeyCode::Enter => self.commit_choice_overlay(dialog),
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::sync_choice_overlay_input(dialog, true);
                    }
                    false
                }
            },
        }
    }

    pub(in crate::app) fn handle_file_attach_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut FileAttachOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Tab => {
                dialog.fill_input_from_selected();
                false
            }
            KeyCode::Enter => {
                let Some(path) = dialog.selected_row().and_then(|selection| match selection {
                    SearchListRow::Clear(_) => None,
                    SearchListRow::Custom(value) => Some(PathBuf::from(value.raw)),
                    SearchListRow::Item(path) => Some(path),
                }) else {
                    return false;
                };
                match self.stage_attachment_from_path(path.as_path(), false) {
                    Ok(()) => true,
                    Err(error) => {
                        self.flash_error(error);
                        false
                    }
                }
            }
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        self.refresh_file_attach_overlay(dialog);
                    }
                    false
                }
            },
        }
    }

    pub(in crate::app) fn handle_help_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut HelpOverlay,
    ) -> bool {
        let max_scroll = ui_text::help_lines(&self.i18n)
            .len()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16;
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => true,
            _ => dialog.handle_navigation_key(key, max_scroll, 8),
        }
    }

    pub(in crate::app) fn handle_session_search_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SessionSearchOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                if dialog.loading || dialog.meta.page_index == 0 {
                    return false;
                }
                dialog.meta.page_index = dialog.meta.page_index.saturating_sub(1);
                dialog.selected = 0;
                match dialog.meta.mode {
                    SessionViewMode::Subtree => {
                        self.refresh_session_search_overlay_local(dialog);
                    }
                    SessionViewMode::All | SessionViewMode::Roots => {
                        let cursor = dialog
                            .meta
                            .cursors
                            .get(dialog.meta.page_index)
                            .cloned()
                            .flatten();
                        dialog.loading = true;
                        dialog.footer = self.session_search_footer(dialog);
                        self.request_session_search_page(
                            dialog.meta.mode,
                            dialog.input.text().trim().to_string(),
                            dialog.meta.page_index,
                            cursor,
                        );
                    }
                }
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if dialog.loading || !dialog.meta.has_more {
                    return false;
                }
                match dialog.meta.mode {
                    SessionViewMode::Subtree => {
                        dialog.meta.page_index = dialog.meta.page_index.saturating_add(1);
                        dialog.selected = 0;
                        self.refresh_session_search_overlay_local(dialog);
                    }
                    SessionViewMode::All | SessionViewMode::Roots => {
                        let Some(cursor) = dialog.meta.next_cursor.clone() else {
                            return false;
                        };
                        dialog.meta.page_index = dialog.meta.page_index.saturating_add(1);
                        if dialog.meta.cursors.len() <= dialog.meta.page_index {
                            dialog.meta.cursors.resize(dialog.meta.page_index + 1, None);
                        }
                        dialog.meta.cursors[dialog.meta.page_index] = Some(cursor.clone());
                        dialog.selected = 0;
                        dialog.loading = true;
                        dialog.footer = self.session_search_footer(dialog);
                        self.request_session_search_page(
                            dialog.meta.mode,
                            dialog.input.text().trim().to_string(),
                            dialog.meta.page_index,
                            Some(cursor),
                        );
                    }
                }
                false
            }
            KeyCode::Tab => {
                if let Some(session) = dialog.items.get(dialog.selected) {
                    let title = session.session.title.clone();
                    if dialog.input.text() != title {
                        dialog.input.set_text(title.clone());
                        self.reset_session_search_query(dialog, title);
                    }
                }
                false
            }
            KeyCode::Enter => {
                let Some(session) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                self.open_session(session.session.id, session.session.title);
                self.focus = Focus::Composer;
                true
            }
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        self.reset_session_search_query(
                            dialog,
                            dialog.input.text().trim().to_string(),
                        );
                    }
                    false
                }
            },
        }
    }

    pub(in crate::app) fn reset_session_search_query(
        &mut self,
        dialog: &mut SessionSearchOverlay,
        query: String,
    ) {
        dialog.meta.page_index = 0;
        dialog.selected = 0;
        dialog.meta.offset = 0;
        dialog.meta.cursors.clear();
        dialog.meta.cursors.push(None);
        dialog.meta.next_cursor = None;
        dialog.meta.has_more = false;
        dialog.loading = true;
        dialog.footer = self.session_search_footer(dialog);
        dialog.meta.page_index = 0;
        match dialog.meta.mode {
            SessionViewMode::Subtree => {
                if let Some(session_id) = dialog.meta.scope_session_id {
                    self.request_session_search_subtree(session_id, query);
                }
            }
            SessionViewMode::All | SessionViewMode::Roots => {
                self.request_session_search_page(dialog.meta.mode, query, 0, None);
            }
        }
    }

    pub(in crate::app) fn refresh_session_search_overlay_local(
        &self,
        dialog: &mut SessionSearchOverlay,
    ) {
        let query = dialog.input.text().trim();
        let filtered = dialog
            .meta
            .all_items
            .iter()
            .filter(|session| session.search_list_matches_query(query))
            .cloned()
            .collect::<Vec<_>>();
        let total = filtered.len();
        let page_limit = dialog.meta.page_limit.max(1);
        let max_page_index = total.saturating_sub(1) / page_limit;
        dialog.meta.page_index = min(dialog.meta.page_index, max_page_index);
        dialog.meta.offset = dialog.meta.page_index.saturating_mul(page_limit);
        dialog.items = filtered
            .into_iter()
            .skip(dialog.meta.offset)
            .take(page_limit)
            .collect();
        dialog.meta.has_more = dialog.meta.offset + dialog.items.len() < total;
        dialog.meta.next_cursor = None;
        dialog.clamp_selection();
        dialog.loading = false;
        dialog.footer = self.session_search_footer(dialog);
    }

    pub(in crate::app) fn session_search_footer(&self, dialog: &SessionSearchOverlay) -> String {
        let scope = match dialog.meta.mode {
            SessionViewMode::All => ui_text::t(&self.i18n, "overlay-session-search-scope-all"),
            SessionViewMode::Roots => ui_text::t(&self.i18n, "overlay-session-search-scope-roots"),
            SessionViewMode::Subtree => {
                ui_text::t(&self.i18n, "overlay-session-search-scope-subtree")
            }
        };
        let start = if dialog.items.is_empty() {
            0
        } else {
            dialog.meta.offset.saturating_add(1)
        };
        let end = dialog.meta.offset.saturating_add(dialog.items.len());
        if dialog.meta.mode == SessionViewMode::Subtree {
            let total = dialog
                .meta
                .all_items
                .iter()
                .filter(|session| session.search_list_matches_query(dialog.input.text().trim()))
                .count();
            let page_total = if total == 0 {
                0
            } else {
                (total + dialog.meta.page_limit.saturating_sub(1)) / dialog.meta.page_limit.max(1)
            };
            return self.i18n.text_args(
                "overlay-session-search-footer-local",
                &crate::fl_args!(
                    "scope" => scope,
                    "start" => start as i64,
                    "end" => end as i64,
                    "total" => total as i64,
                    "page" => dialog.meta.page_index.saturating_add(1) as i64,
                    "pages" => page_total.max(1) as i64,
                ),
            );
        }

        let end_state = if dialog.meta.has_more {
            ui_text::t(&self.i18n, "overlay-session-search-tail-more")
        } else {
            ui_text::t(&self.i18n, "overlay-session-search-tail-end")
        };
        self.i18n.text_args(
            "overlay-session-search-footer-remote",
            &crate::fl_args!(
                "scope" => scope,
                "start" => start as i64,
                "end" => end as i64,
                "page" => dialog.meta.page_index.saturating_add(1) as i64,
                "tail" => end_state,
            ),
        )
    }

    pub(in crate::app) fn handle_picker_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PickerOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Tab => {
                if dialog.fill_input_from_selected() {
                    Self::refresh_picker_overlay(dialog);
                }
                false
            }
            KeyCode::Char('n') if matches!(dialog.meta.kind, PickerKind::Agents) => {
                self.open_agent_create_overlay();
                false
            }
            KeyCode::Char('n')
                if matches!(
                    dialog.meta.kind,
                    PickerKind::Providers(ProviderPickerPurpose::Configure)
                ) =>
            {
                self.route_stack.push(Route::Picker(dialog.clone()));
                self.open_provider_studio(None);
                false
            }
            KeyCode::Char('n') if matches!(dialog.meta.kind, PickerKind::PermissionRules) => {
                self.route_stack.push(Route::Picker(dialog.clone()));
                self.open_permission_rule_studio(None, None);
                false
            }
            KeyCode::Char('d') if matches!(dialog.meta.kind, PickerKind::PermissionRules) => {
                let Some(item) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                if let PickerValue::PermissionRule(rule) = item.value {
                    self.open_revoke_permission_rule_confirm(&rule, dialog.input.text());
                    false
                } else {
                    false
                }
            }
            KeyCode::Enter => {
                let Some(item) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                if matches!(dialog.meta.kind, PickerKind::Agents) {
                    match item.value {
                        PickerValue::AgentCreate => {
                            self.open_agent_create_overlay();
                            return false;
                        }
                        PickerValue::Agent(agent) => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_agent_studio(agent.name.as_str());
                            return false;
                        }
                        _ => {}
                    }
                }
                if matches!(
                    dialog.meta.kind,
                    PickerKind::Providers(ProviderPickerPurpose::Configure)
                ) {
                    match item.value {
                        PickerValue::ProviderCreate => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_provider_studio(None);
                            return false;
                        }
                        PickerValue::Provider(provider) => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_provider_studio(Some(provider.provider_id.as_str()));
                            return false;
                        }
                        _ => {}
                    }
                }
                if matches!(dialog.meta.kind, PickerKind::PermissionRules) {
                    match item.value {
                        PickerValue::PermissionRuleCreate => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_permission_rule_studio(None, None);
                            return false;
                        }
                        PickerValue::PermissionRule(rule) => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_permission_rule_studio(Some(&rule), None);
                            return false;
                        }
                        _ => {}
                    }
                }
                self.handle_picker_selection(dialog.meta.kind.clone(), item);
                true
            }
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::refresh_picker_overlay(dialog);
                    }
                    false
                }
            },
        }
    }

    pub(in crate::app) fn handle_session_model_chooser_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SessionModelChooserOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Left => {
                dialog.move_selection_page(-1, dialog.meta.page_size);
                false
            }
            KeyCode::Right => {
                dialog.move_selection_page(1, dialog.meta.page_size);
                false
            }
            KeyCode::Enter => {
                let Some(item) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                self.apply_model_override(item.model);
                true
            }
            _ => match dialog.handle_filter_input_key(key, dialog.meta.page_size) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::refresh_session_model_chooser_overlay(dialog, false, None);
                    }
                    false
                }
            },
        }
    }

    pub(in crate::app) fn handle_timeline_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut TimelineOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Enter => {
                if let Some(item) = dialog.selected_item()
                    && let Some(message_id) = item.linked_message_id
                {
                    self.jump_to_message(message_id);
                    return true;
                }
                false
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(item) = dialog.selected_item() {
                    match set_clipboard_text(item.copy_text.as_str()) {
                        Ok(method) => self.flash_clipboard_copy_success(
                            method,
                            ui_text::t(&self.i18n, "flash-timeline-event-copied"),
                        ),
                        Err(error) => self.flash_error(self.i18n.text_args(
                            "flash-clipboard-copy-failed",
                            &crate::fl_args!("error" => error.to_string()),
                        )),
                    }
                }
                false
            }
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::refresh_timeline_overlay(dialog);
                    }
                    false
                }
            },
        }
    }

    pub(in crate::app) fn handle_provider_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ProviderStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => return false,
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                    return false;
                }
                EditorDialogKeyResult::Submit(action, input) => match action {
                    ProviderStudioEditorAction::Field(field) => {
                        let value = input.trim().to_string();
                        if let Err(error) = self.commit_provider_studio_field(dialog, field, value)
                        {
                            self.flash_error(error);
                            return false;
                        }
                        dialog.editor = None;
                        return false;
                    }
                    ProviderStudioEditorAction::NewModel { adapter_id } => {
                        let value = input.trim().to_string();
                        match self.add_provider_studio_manual_model(dialog, adapter_id, value) {
                            Ok(()) => dialog.editor = None,
                            Err(error) => self.flash_error(error),
                        }
                        return false;
                    }
                    ProviderStudioEditorAction::ModelField(field) => {
                        let value = input.trim().to_string();
                        if let Err(error) =
                            self.commit_provider_studio_model_field(dialog, field, value)
                        {
                            self.flash_error(error);
                            return false;
                        }
                        dialog.editor = None;
                        return false;
                    }
                },
            }
        }

        if dialog.model_page.is_some() {
            return self.handle_provider_studio_model_page_key(key, dialog);
        }

        if dialog.detail_page.is_some() {
            return self.handle_provider_studio_detail_page_key(key, dialog);
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Tab => {
                dialog.selection.next_focus();
                false
            }
            KeyCode::BackTab => {
                dialog.selection.prev_focus();
                false
            }
            KeyCode::Char('n') => {
                self.load_provider_studio_draft(dialog, None, Some(String::new()));
                false
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                self.request_provider_studio_start_auth(dialog);
                false
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.request_provider_studio_continue_auth(dialog);
                false
            }
            KeyCode::Char('r') => {
                self.request_provider_studio_adapter_models(dialog);
                false
            }
            KeyCode::Char('+') if dialog.selection.focus() == ProviderStudioFocus::Models => {
                self.open_provider_studio_new_model_editor(dialog);
                false
            }
            KeyCode::Delete | KeyCode::Backspace
                if dialog.selection.focus() == ProviderStudioFocus::Adapters =>
            {
                self.open_provider_studio_delete_selected_adapter_confirm(dialog);
                false
            }
            KeyCode::Delete | KeyCode::Backspace
                if dialog.selection.focus() == ProviderStudioFocus::Models =>
            {
                self.open_provider_studio_delete_selected_model_confirm(dialog);
                false
            }
            KeyCode::Char('D') if dialog.draft.source_provider_id.is_some() => {
                if let Some(provider_id) = dialog.draft.source_provider_id.clone() {
                    self.open_provider_studio_delete_provider_confirm(provider_id);
                }
                false
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                dialog.saving = true;
                self.request_provider_studio_save_draft(dialog.clone());
                false
            }
            KeyCode::Char('a') => {
                if provider_studio_selected_adapter_models(dialog).is_none() {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-provider-studio-adapter-required",
                    ));
                    return false;
                }
                dialog.saving = true;
                self.request_provider_studio_save_selected_adapter(dialog.clone());
                false
            }
            KeyCode::Char('A') if dialog.selection.focus() == ProviderStudioFocus::Adapters => {
                Self::select_all_provider_studio_adapters(dialog);
                false
            }
            KeyCode::Char('A') if dialog.selection.focus() == ProviderStudioFocus::Models => {
                Self::select_all_provider_studio_models(dialog);
                false
            }
            KeyCode::Char('c') | KeyCode::Char('C')
                if dialog.selection.focus() == ProviderStudioFocus::Adapters =>
            {
                Self::clear_provider_studio_selected_adapters(dialog);
                false
            }
            KeyCode::Char('c') | KeyCode::Char('C')
                if dialog.selection.focus() == ProviderStudioFocus::Models =>
            {
                Self::clear_provider_studio_selected_models(dialog);
                false
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                if provider_studio_selected_model_target(dialog).is_none() {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-provider-studio-model-required",
                    ));
                    return false;
                }
                dialog.saving = true;
                self.request_provider_studio_save_selected_model(dialog.clone());
                false
            }
            KeyCode::Char(' ') if dialog.selection.focus() == ProviderStudioFocus::Adapters => {
                self.toggle_provider_studio_selected_adapter(dialog);
                false
            }
            KeyCode::Char(' ') if dialog.selection.focus() == ProviderStudioFocus::Models => {
                self.toggle_provider_studio_selected_model(dialog);
                false
            }
            KeyCode::PageUp => {
                self.move_provider_studio_selection_page(dialog, -1, 10);
                false
            }
            KeyCode::PageDown => {
                self.move_provider_studio_selection_page(dialog, 1, 10);
                false
            }
            KeyCode::Home => {
                self.move_provider_studio_selection_home(dialog);
                false
            }
            KeyCode::End => {
                self.move_provider_studio_selection_end(dialog);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_provider_studio_selection(dialog, -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_provider_studio_selection(dialog, 1);
                false
            }
            KeyCode::Enter => {
                self.activate_provider_studio_focus(dialog);
                false
            }
            _ => false,
        }
    }

    pub(in crate::app) fn handle_model_catalog_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ModelCatalogStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.workbench.editor.as_mut() {
            return match drive_input_dialog_key(editor, key) {
                InputDialogKeyResult::Close => {
                    dialog.workbench.editor = None;
                    false
                }
                InputDialogKeyResult::Submit(_, value) => {
                    dialog.query = value.trim().to_string();
                    dialog.offset = 0;
                    dialog.workbench.list.selected = 0;
                    dialog.loading = true;
                    dialog.workbench.editor = None;
                    self.request_model_catalog_page(dialog.query.clone(), 0);
                    false
                }
                InputDialogKeyResult::Continue => false,
            };
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('/') => {
                dialog.workbench.editor =
                    Some(self.build_model_catalog_search_overlay(dialog.query.as_str()));
                false
            }
            KeyCode::Char('R') => {
                dialog.loading = true;
                self.request_model_catalog_refresh();
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if dialog.offset == 0 {
                    return false;
                }
                let offset = dialog.offset.saturating_sub(dialog.limit.max(1));
                dialog.offset = offset;
                dialog.workbench.list.selected = 0;
                dialog.loading = true;
                self.request_model_catalog_page(dialog.query.clone(), offset);
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if dialog.offset + dialog.workbench.list.items.len() >= dialog.total {
                    return false;
                }
                dialog.offset += dialog.limit.max(1);
                dialog.workbench.list.selected = 0;
                dialog.loading = true;
                self.request_model_catalog_page(dialog.query.clone(), dialog.offset);
                false
            }
            _ if dialog.workbench.list.handle_navigation_key(key, 10) => false,
            _ => false,
        }
    }
}
use crate::app::{
    App, ChoiceOverlay, EditorDialogKeyResult, FileAttachOverlay, Focus, HelpOverlay,
    InputDialogKeyResult, KeyCode, KeyEvent, KeyModifiers, ModelCatalogStudioOverlay, PathBuf,
    PickerKind, PickerOverlay, PickerValue, ProviderPickerPurpose, ProviderStudioEditorAction,
    ProviderStudioFocus, ProviderStudioOverlay, Route, RuntimeSettingEditOverlay,
    SearchInputKeyResult, SearchListRow, SessionModelChooserOverlay, SessionSearchOverlay,
    SessionViewMode, TimelineOverlay, drive_editor_dialog_key, drive_input_dialog_key, min,
    provider_studio_selected_adapter_models, provider_studio_selected_model_target,
    set_clipboard_text, ui_text,
};
use agena_tui_components::SearchListItem;
