impl App {
    pub(in crate::app) fn enter_insert_mode(&mut self) {
        self.focus = Focus::Composer;
    }

    pub(in crate::app) fn toggle_transcript_cursor_node(&mut self) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return;
        };
        if !node.toggleable {
            return;
        }
        self.transcript
            .node_expansions
            .insert(node.key, !node.expanded);
        self.transcript.invalidate_render();
        self.transcript.clamp_scroll(width, height);
        if node.toggleable {
            self.flash_info(self.i18n.text_args(
                if node.expanded {
                    "flash-transcript-node-collapsed"
                } else {
                    "flash-transcript-node-expanded"
                },
                &crate::fl_args!("kind" => transcript_node_kind_label(&self.i18n, node.kind)),
            ));
        }
    }

    pub(in crate::app) fn flash_clipboard_copy_success(
        &mut self,
        method: ClipboardCopyMethod,
        success: String,
    ) {
        if method.is_unconfirmed_terminal_request() {
            self.flash_info("Clipboard request sent to terminal (OSC 52).".to_string());
        } else {
            self.flash_success(success);
        }
    }

    pub(in crate::app) fn request_clipboard_copy(&mut self, text: String, success: String) {
        self.pending_ui_action = Some(UiAction::CopyText { text, success });
    }

    pub(in crate::app) fn copy_transcript_cursor_node(&mut self) {
        let width = self.layout.transcript_body.width;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return;
        };
        let success = self.i18n.text_args(
            "flash-transcript-node-copied",
            &crate::fl_args!("kind" => transcript_node_kind_label(&self.i18n, node.kind)),
        );
        self.request_clipboard_copy(node.copy_text, success);
    }

    pub(in crate::app) fn handle_composer_key(&mut self, key: KeyEvent) {
        if self.handle_prompt_history_search_key(key) {
            return;
        }
        if self.handle_file_mention_suggestion_key(key) {
            return;
        }
        if self.handle_slash_command_suggestion_key(key) {
            return;
        }
        if self.handle_selected_composer_item_key(key) {
            return;
        }
        if composer_up_opens_prompt_history(key, self.composer.cursor()) {
            self.open_prompt_history_search();
            return;
        }
        // Esc handling is special — double-tap clears the input. We track
        // it before consulting the configurable bindings.
        let composer_key = resolve_tui_key(KeyContext::Composer, key);
        if composer_key == Some(KeyAction::EnterView) {
            self.focus = Focus::Transcript;
            self.sync_composer_suggestions();
            return;
        }
        // Configurable bindings define the composer map. The defaults preserve
        // the user's stated preference:
        // Enter = queue, Ctrl+Enter = submit, Shift+Enter / Ctrl+J = newline.
        if let Some(action) = self.keybindings.match_action(&key) {
            match action {
                ComposerAction::Submit => {
                    self.submit_or_steer();
                    return;
                }
                ComposerAction::Queue => {
                    self.queue_or_submit();
                    return;
                }
                ComposerAction::Newline => {
                    self.reset_prompt_history_recall();
                    self.composer.insert_explicit_newline();
                    self.after_composer_text_mutated();
                    return;
                }
                ComposerAction::EditQueue => {
                    if self.try_pop_queue_into_editor() {
                        self.reset_prompt_history_recall();
                        self.after_composer_text_mutated();
                        return;
                    }
                    // An empty queue leaves Ctrl+Up as a no-op. Bare Up keeps
                    // its normal cursor movement except at position zero,
                    // where it opens prompt history.
                }
                ComposerAction::ClearInput => {
                    self.reset_prompt_history_recall();
                    self.clear_composer_state();
                    return;
                }
                ComposerAction::FocusItems => {
                    if self.toggle_composer_item_selection() {
                        return;
                    }
                }
                ComposerAction::AttachFile => {
                    self.reset_prompt_history_recall();
                    self.request_file_attachment(false);
                    return;
                }
                ComposerAction::ExternalEditor => {
                    self.reset_prompt_history_recall();
                    self.pending_ui_action = Some(UiAction::EditComposerExternally);
                    return;
                }
                ComposerAction::AttachClipboardImage => {
                    self.reset_prompt_history_recall();
                    self.request_file_attachment(true);
                    return;
                }
                ComposerAction::OpenPendingUserInput => {
                    self.open_user_input_overlay();
                    return;
                }
                ComposerAction::OpenPendingPermission => {
                    self.open_permission_overlay();
                    return;
                }
            }
        }
        self.reset_prompt_history_recall();
        self.composer.handle_multiline_input_key(key);
        self.after_composer_text_mutated();
    }

    pub(in crate::app) fn after_composer_text_mutated(&mut self) {
        self.sync_composer_items_with_editor();
        self.clamp_selected_composer_item();
        self.sync_composer_suggestions();
    }

    pub(in crate::app) fn sync_composer_suggestions(&mut self) {
        if self.prompt_history_search.is_some() {
            self.slash_command_suggestions = None;
            self.file_mention_suggestions = None;
            return;
        }
        self.sync_file_mention_suggestions();
        self.sync_slash_command_suggestions();
    }

    pub(in crate::app) fn toggle_composer_item_selection(&mut self) -> bool {
        if self.composer_items.is_empty() {
            self.selected_composer_item = None;
            return false;
        }
        self.selected_composer_item = match self.selected_composer_item {
            Some(_) => None,
            None => Some(0),
        };
        true
    }

    pub(in crate::app) fn clamp_selected_composer_item(&mut self) {
        self.selected_composer_item = self
            .selected_composer_item
            .and_then(|index| (!self.composer_items.is_empty()).then_some(index))
            .map(|index| min(index, self.composer_items.len().saturating_sub(1)));
    }

    pub(in crate::app) fn handle_selected_composer_item_key(&mut self, key: KeyEvent) -> bool {
        let Some(index) = self.selected_composer_item else {
            return false;
        };
        match resolve_tui_key(KeyContext::ComposerItem, key) {
            Some(KeyAction::Close) => {
                self.selected_composer_item = None;
                true
            }
            Some(KeyAction::Previous) => {
                self.selected_composer_item = Some(index.saturating_sub(1));
                true
            }
            Some(KeyAction::Next) => {
                self.selected_composer_item =
                    Some(min(index + 1, self.composer_items.len().saturating_sub(1)));
                true
            }
            Some(KeyAction::Delete) => {
                self.remove_composer_item(index);
                true
            }
            Some(KeyAction::Open) => {
                self.open_selected_composer_item(index);
                true
            }
            _ => false,
        }
    }

    pub(in crate::app) fn remove_composer_item(&mut self, index: usize) {
        let Some(range) = self.composer.draft_elements().get(index).cloned() else {
            return;
        };
        self.composer.remove_range(range.start, range.end);
        self.after_composer_text_mutated();
    }

    pub(in crate::app) fn open_selected_composer_item(&mut self, index: usize) {
        let Some(item) = self.composer_items.get(index) else {
            return;
        };
        match item {
            ComposerItem::Attachment(attachment) => {
                self.pending_ui_action = Some(UiAction::OpenPath {
                    path: attachment.path.clone(),
                });
            }
            ComposerItem::LargePaste(_) => {
                self.flash_info(ui_text::t(&self.i18n, "flash-large-paste-no-file-view"));
            }
        }
    }

    pub(in crate::app) fn handle_prompt_history_search_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut search) = self.prompt_history_search.take() else {
            return false;
        };
        let close = match resolve_tui_key(KeyContext::PromptHistory, key) {
            Some(KeyAction::Close) => {
                self.replace_composer_draft(search.meta.original.clone());
                true
            }
            Some(KeyAction::Accept) => {
                if let Some(result) = search.selected_item().cloned() {
                    self.replace_composer_draft(ComposerDraft {
                        text: result.text,
                        ..ComposerDraft::default()
                    });
                } else {
                    self.replace_composer_draft(search.meta.original.clone());
                }
                true
            }
            Some(KeyAction::Previous) => {
                search.move_selection(-1);
                false
            }
            Some(KeyAction::Next) => {
                search.move_selection(1);
                false
            }
            Some(KeyAction::Older) => {
                search.move_selection(1);
                false
            }
            Some(KeyAction::Newer) => {
                if search.selected == 0 {
                    self.replace_composer_draft(search.meta.original.clone());
                    true
                } else {
                    search.move_selection(-1);
                    false
                }
            }
            Some(KeyAction::NewerKeepOpen) => {
                search.move_selection(-1);
                false
            }
            _ => {
                let before = search.input.text().to_string();
                search.input.handle_line_input_key(key);
                if search.input.text() != before {
                    search.refresh_results();
                }
                false
            }
        };

        if !close {
            self.preview_prompt_history_search_selection(&search);
            self.prompt_history_search = Some(search);
        }
        true
    }

    pub(in crate::app) fn open_prompt_history_search(&mut self) {
        self.sync_composer_items_with_editor();
        if !self.composer_items.is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-prompt-history-items"));
            return;
        }
        if self.prompt_history.is_empty() {
            self.flash_info(ui_text::t(&self.i18n, "flash-prompt-history-empty"));
            return;
        }
        self.after_composer_text_mutated();
        let config = SearchPickerConfig::searchable();
        let mut search = PromptHistorySearchState::new(
            ui_text::t(&self.i18n, "composer-prompt-history-title"),
            String::new(),
            String::new(),
            ui_text::t(&self.i18n, "composer-prompt-history-no-matches"),
            Editor::default(),
            config,
            None,
            PromptHistorySearchMeta {
                original: self.current_composer_draft(),
            },
        );
        Self::refresh_prompt_history_search(&self.prompt_history, &mut search);
        self.slash_command_suggestions = None;
        self.file_mention_suggestions = None;
        self.selected_composer_item = None;
        self.prompt_history_search = Some(search);
    }

    /// History owns its query independently from the composer and previews the
    /// selected newest-to-oldest entry directly in the editor. Esc restores the
    /// untouched draft; Enter accepts the current selection.
    pub(in crate::app) fn preview_prompt_history_search_selection(
        &mut self,
        search: &PromptHistorySearchState,
    ) {
        let preview = search.selected_item().map(|result| result.text.as_str());
        match preview {
            Some(text) => self.set_composer_text_for_history(text, false),
            None => self.set_composer_text_for_history(search.meta.original.text.as_str(), false),
        }
    }

    pub(in crate::app) fn set_composer_text_for_history(
        &mut self,
        text: &str,
        cursor_at_start: bool,
    ) {
        self.composer.set_text(text.to_string());
        if cursor_at_start {
            self.composer.set_cursor(0);
        }
        self.composer_items.clear();
        self.selected_composer_item = None;
        self.slash_command_suggestions = None;
        self.file_mention_suggestions = None;
    }

    pub(in crate::app) fn refresh_prompt_history_search(
        prompt_history: &PromptHistory,
        search: &mut PromptHistorySearchState,
    ) {
        search.replace_items(
            prompt_history
                .items
                .iter()
                .enumerate()
                .rev()
                .map(|(history_index, text)| PromptHistorySearchResult {
                    history_index,
                    text: text.clone(),
                })
                .collect(),
        );
    }

    pub(in crate::app) fn handle_file_mention_suggestion_key(&mut self, key: KeyEvent) -> bool {
        if self.file_mention_suggestions.is_none() {
            return false;
        }

        match resolve_tui_key(KeyContext::Suggestion, key) {
            Some(KeyAction::Previous) => {
                self.move_file_mention_suggestion(-1);
                true
            }
            Some(KeyAction::Next) => {
                self.move_file_mention_suggestion(1);
                true
            }
            Some(KeyAction::Close) => {
                self.dismiss_file_mention_suggestions();
                true
            }
            Some(KeyAction::Fill | KeyAction::Accept) => {
                self.complete_selected_file_mention();
                true
            }
            _ => {
                let Some(mut state) = self.file_mention_suggestions.take() else {
                    return false;
                };
                let before = state.input.text().to_string();
                let before_cursor = state.input.cursor();
                state.input.handle_line_input_key(key);
                let handled = if state.input.text() != before {
                    let items = self.file_mention_suggestion_items(state.input.text());
                    state.replace_items(items);
                    true
                } else {
                    state.input.cursor() != before_cursor
                };
                self.file_mention_suggestions = Some(state);
                handled
            }
        }
    }

    pub(in crate::app) fn move_file_mention_suggestion(&mut self, delta: isize) {
        let Some(state) = self.file_mention_suggestions.as_mut() else {
            return;
        };
        state.move_selection(delta);
    }

    pub(in crate::app) fn dismiss_file_mention_suggestions(&mut self) {
        if let Some(state) = self.file_mention_suggestions.take() {
            self.dismissed_file_mention_suggestions_for = Some(state.meta.fingerprint);
        }
    }

    pub(in crate::app) fn complete_selected_file_mention(&mut self) {
        let Some((item, mention_range)) =
            self.file_mention_suggestions.as_ref().and_then(|state| {
                state
                    .selected_item()
                    .cloned()
                    .map(|item| (item, state.meta.mention_range.clone()))
            })
        else {
            return;
        };

        self.file_mention_suggestions = None;
        self.dismissed_file_mention_suggestions_for = None;
        self.composer
            .remove_range(mention_range.start, mention_range.end);
        if let Err(error) = self.stage_attachment_from_path(item.path.as_path(), false) {
            self.flash_error(error);
            return;
        }
        let after_cursor_is_space = self.composer.text()[self.composer.cursor()..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace);
        if !after_cursor_is_space {
            self.composer.insert_char(' ');
        }
        self.after_composer_text_mutated();
    }

    pub(in crate::app) fn sync_file_mention_suggestions(&mut self) {
        let Some(context) = self.file_mention_suggestion_context() else {
            self.file_mention_suggestions = None;
            return;
        };
        if self.dismissed_file_mention_suggestions_for.as_deref()
            == Some(context.fingerprint.as_str())
        {
            self.file_mention_suggestions = None;
            return;
        }

        if self
            .file_mention_suggestions
            .as_ref()
            .is_some_and(|state| state.meta.fingerprint == context.fingerprint)
        {
            return;
        }

        let items = self.file_mention_suggestion_items(context.query.as_str());
        let mut config = SearchPickerConfig::searchable();
        config.search_mode = SearchPickerSearchMode::External;
        let mut state = FileMentionSuggestionState::new(
            ui_text::t(&self.i18n, "overlay-attach-title"),
            ui_text::t(&self.i18n, "overlay-attach-prompt"),
            ui_text::t(&self.i18n, "overlay-attach-footer"),
            ui_text::t(&self.i18n, "overlay-attach-no-match"),
            Editor::from_text(context.query),
            config,
            None,
            FileMentionSuggestionMeta {
                fingerprint: context.fingerprint,
                mention_range: context.mention_range,
            },
        );
        state.replace_items(items);
        self.file_mention_suggestions = Some(state);
    }

    pub(in crate::app) fn file_mention_suggestion_context(
        &self,
    ) -> Option<FileMentionSuggestionContext> {
        if self.focus != Focus::Composer || self.overlay.is_some() || !self.current_route_is_main()
        {
            return None;
        }
        if self.prompt_history_search.is_some() {
            return None;
        }
        file_mention_suggestion_context_for_text(self.composer.text(), self.composer.cursor())
    }

    pub(in crate::app) fn file_mention_suggestion_items(
        &self,
        query: &str,
    ) -> Vec<FileMentionSuggestionItem> {
        self.backend
            .search_workspace_files(query, MAX_FILE_MENTION_SUGGESTIONS)
            .unwrap_or_default()
            .into_iter()
            .map(|path| {
                let label = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| path.display().to_string());
                FileMentionSuggestionItem {
                    detail: path.display().to_string(),
                    label,
                    path,
                }
            })
            .collect()
    }

    pub(in crate::app) fn handle_slash_command_suggestion_key(&mut self, key: KeyEvent) -> bool {
        if self.slash_command_suggestions.is_none() {
            return false;
        }

        match resolve_tui_key(KeyContext::Suggestion, key) {
            Some(KeyAction::Previous) => {
                self.move_slash_command_suggestion(-1);
                true
            }
            Some(KeyAction::Next) => {
                self.move_slash_command_suggestion(1);
                true
            }
            Some(KeyAction::Close) => {
                self.dismiss_slash_command_suggestions();
                true
            }
            Some(KeyAction::Fill) => {
                self.complete_selected_slash_command_suggestion(false);
                true
            }
            Some(KeyAction::Accept) => {
                self.complete_selected_slash_command_suggestion(true);
                true
            }
            _ => {
                let Some(state) = self.slash_command_suggestions.as_mut() else {
                    return false;
                };
                let before = state.input.text().to_string();
                let before_cursor = state.input.cursor();
                state.input.handle_line_input_key(key);
                if state.input.text() != before {
                    state.refresh_results();
                    true
                } else {
                    state.input.cursor() != before_cursor
                }
            }
        }
    }

    pub(in crate::app) fn move_slash_command_suggestion(&mut self, delta: isize) {
        let Some(state) = self.slash_command_suggestions.as_mut() else {
            return;
        };
        state.move_selection(delta);
    }

    pub(in crate::app) fn dismiss_slash_command_suggestions(&mut self) {
        if let Some(state) = self.slash_command_suggestions.take() {
            self.dismissed_slash_command_suggestions_for = Some(state.meta.fingerprint);
        }
    }

    pub(in crate::app) fn complete_selected_slash_command_suggestion(&mut self, submit: bool) {
        let Some(item) = self.selected_slash_command_suggestion().cloned() else {
            return;
        };

        self.apply_slash_command_completion(&item);
        if submit {
            self.submit_composer();
        } else {
            self.sync_composer_suggestions();
        }
    }

    pub(in crate::app) fn selected_slash_command_suggestion(
        &self,
    ) -> Option<&SlashCommandSuggestionItem> {
        let state = self.slash_command_suggestions.as_ref()?;
        state.selected_item()
    }

    pub(in crate::app) fn apply_slash_command_completion(
        &mut self,
        item: &SlashCommandSuggestionItem,
    ) {
        let Some(context) = self.slash_command_suggestion_context() else {
            return;
        };

        let name = match &item.value {
            SlashCommandSuggestionValue::Command(spec) => spec.name,
            SlashCommandSuggestionValue::RuntimeTool(name) => name.as_str(),
        };
        let replacement = format!("/{name}");
        self.slash_command_suggestions = None;
        self.dismissed_slash_command_suggestions_for = None;

        self.composer
            .remove_range(context.name_range.start, context.name_range.end);
        self.composer
            .insert_str_at(context.name_range.start, replacement.as_str());

        let after_cursor_is_space = self.composer.text()[self.composer.cursor()..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace);
        if !after_cursor_is_space {
            self.composer.insert_char(' ');
        }
        self.after_composer_text_mutated();
    }

    pub(in crate::app) fn sync_slash_command_suggestions(&mut self) {
        let Some(context) = self.slash_command_suggestion_context() else {
            self.slash_command_suggestions = None;
            return;
        };
        if self.dismissed_slash_command_suggestions_for.as_deref()
            == Some(context.fingerprint.as_str())
        {
            self.slash_command_suggestions = None;
            return;
        }

        if self
            .slash_command_suggestions
            .as_ref()
            .is_some_and(|state| state.meta.fingerprint == context.fingerprint)
        {
            return;
        }

        let items = self.slash_command_suggestion_items("");
        if items.is_empty() {
            self.slash_command_suggestions = None;
            return;
        }

        let config = SearchPickerConfig::searchable();
        let mut state = SlashCommandSuggestionState::new(
            ui_text::t(&self.i18n, "overlay-commands-title"),
            ui_text::t(&self.i18n, "overlay-commands-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            Editor::from_text(context.query),
            config,
            None,
            SlashCommandSuggestionMeta {
                fingerprint: context.fingerprint,
            },
        );
        state.replace_items(items);
        self.slash_command_suggestions = Some(state);
    }

    pub(in crate::app) fn slash_command_suggestion_context(
        &self,
    ) -> Option<SlashCommandSuggestionContext> {
        if self.focus != Focus::Composer || self.overlay.is_some() || !self.current_route_is_main()
        {
            return None;
        }

        let context = slash_command_suggestion_context_for_text(
            self.composer.text(),
            self.composer.cursor(),
        )?;
        Some(context)
    }

    pub(in crate::app) fn slash_command_suggestion_items(
        &self,
        query: &str,
    ) -> Vec<SlashCommandSuggestionItem> {
        let query = query.trim().to_ascii_lowercase();
        let mut items = commands::command_suggestions_for_prefix(query.as_str())
            .into_iter()
            .map(|spec| SlashCommandSuggestionItem {
                label: format!("/{}", spec.name),
                detail: ui_text::t(&self.i18n, spec.summary_key),
                value: SlashCommandSuggestionValue::Command(spec),
            })
            .collect::<Vec<_>>();

        items.extend(
            self.runtime_tool_command_rows()
                .into_iter()
                .filter(|entry| runtime_tool_matches_slash_query(entry.label.as_str(), &query))
                .map(|entry| {
                    let label = entry.label;
                    SlashCommandSuggestionItem {
                        label: format!("/{label}"),
                        detail: entry.detail,
                        value: SlashCommandSuggestionValue::RuntimeTool(label),
                    }
                }),
        );
        items
    }

    /// UP / EditQueue binding: pull every editable queued message back into
    /// the editor for editing. Returns true if anything was pulled (so the
    /// caller skips the default cursor-up behavior).
    pub(in crate::app) fn try_pop_queue_into_editor(&mut self) -> bool {
        // Only pull the queue when the cursor is at the top line of the
        // editor — otherwise UP is a normal cursor movement.
        if !self.composer.cursor_on_first_line() {
            return false;
        }
        let Some(combined) = self.queue.pop_all_editable() else {
            return false;
        };
        // Merge the queued draft on top of whatever's already in the
        // editor.
        let mut existing = self.take_composer_draft();
        if !existing.text.is_empty() && !existing.text.ends_with('\n') {
            existing.text.push_str("\n\n");
        }
        let prev_len = existing.text.len();
        existing.text.push_str(combined.text.as_str());
        for mut element in combined.elements {
            element.range = (element.range.start + prev_len)..(element.range.end + prev_len);
            existing.elements.push(element);
        }
        existing.items.extend(combined.items);
        self.restore_composer_draft(existing);
        true
    }
}
use crate::app::{
    App, ClipboardCopyMethod, ComposerAction, ComposerDraft, ComposerItem, Editor,
    FileMentionSuggestionContext, FileMentionSuggestionItem, FileMentionSuggestionMeta,
    FileMentionSuggestionState, Focus, KeyEvent, MAX_FILE_MENTION_SUGGESTIONS, PromptHistory,
    PromptHistorySearchMeta, PromptHistorySearchResult, PromptHistorySearchState,
    SlashCommandSuggestionContext, SlashCommandSuggestionItem, SlashCommandSuggestionMeta,
    SlashCommandSuggestionState, SlashCommandSuggestionValue, UiAction, commands,
    file_mention_suggestion_context_for_text, min, runtime_tool_matches_slash_query,
    slash_command_suggestion_context_for_text, transcript_node_kind_label, ui_text,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui_components::{SearchPickerConfig, SearchPickerSearchMode};
use crossterm::event::{KeyCode, KeyModifiers};

fn composer_up_opens_prompt_history(key: KeyEvent, cursor: usize) -> bool {
    key.code == KeyCode::Up && key.modifiers == KeyModifiers::NONE && cursor == 0
}

#[cfg(test)]
mod tests {
    use super::composer_up_opens_prompt_history;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn only_plain_up_at_the_composer_start_opens_history() {
        assert!(composer_up_opens_prompt_history(
            key(KeyCode::Up, KeyModifiers::NONE),
            0,
        ));
        assert!(!composer_up_opens_prompt_history(
            key(KeyCode::Up, KeyModifiers::NONE),
            1,
        ));
        assert!(!composer_up_opens_prompt_history(
            key(KeyCode::Up, KeyModifiers::ALT),
            0,
        ));
        assert!(!composer_up_opens_prompt_history(
            key(KeyCode::Char('r'), KeyModifiers::CONTROL),
            0,
        ));
    }
}
