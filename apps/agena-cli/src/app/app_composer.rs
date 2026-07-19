impl App {
    pub(in crate::app) fn enter_insert_mode(&mut self) {
        self.focus = Focus::Composer;
    }

    pub(in crate::app) fn toggle_transcript_cursor_node(&mut self) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        if !self.transcript.has_navigation_target() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-transcript-no-selection"));
            return;
        }
        let Some((kind, expanded)) = self.transcript.toggle_cursor_node_expansion(width, height)
        else {
            return;
        };
        self.flash_info(self.i18n.text_args(
            if expanded {
                "flash-transcript-node-expanded"
            } else {
                "flash-transcript-node-collapsed"
            },
            &crate::fl_args!("kind" => transcript_node_kind_label(&self.i18n, kind)),
        ));
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
        if let Some(text) = self.transcript.current_selected_line_text(width) {
            let text = text.replace(
                transcript_spinner_placeholder(),
                spinner_frame(current_spinner_millis()),
            );
            self.request_clipboard_copy(
                text,
                ui_text::t(&self.i18n, "flash-transcript-line-copied"),
            );
            return;
        }
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            if !self.transcript.has_navigation_target() {
                self.flash_warning(ui_text::t(&self.i18n, "flash-transcript-no-selection"));
            }
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
        if composer_slash_opens_command_palette(key, self.composer.text(), self.composer.cursor()) {
            self.reset_prompt_history_recall();
            self.open_command_palette();
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
        match handle_prompt_history_picker_key(&mut search, key) {
            PromptHistoryPickerOutcome::KeepOpen => {
                self.prompt_history_search = Some(search);
            }
            PromptHistoryPickerOutcome::Close => {}
            PromptHistoryPickerOutcome::Accept(text) => {
                self.replace_composer_draft(ComposerDraft {
                    text,
                    ..ComposerDraft::default()
                });
            }
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
            (),
        );
        Self::refresh_prompt_history_search(&self.prompt_history, &mut search);
        self.slash_command_suggestions = None;
        self.file_mention_suggestions = None;
        self.selected_composer_item = None;
        self.prompt_history_search = Some(search);
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
                if let Some(state) = self.file_mention_suggestions.as_mut() {
                    if matches!(key.code, crossterm::event::KeyCode::Up) {
                        let _ = state.handle_input_key(key);
                    } else {
                        state.move_selection(-1);
                    }
                }
                true
            }
            Some(KeyAction::Next) => {
                if let Some(state) = self.file_mention_suggestions.as_mut() {
                    if matches!(key.code, crossterm::event::KeyCode::Down) {
                        let _ = state.handle_input_key(key);
                    } else {
                        state.move_selection(1);
                    }
                }
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
                let before_cursor = state.input.cursor();
                let input_result = state.handle_input_key(key);
                let handled = match input_result {
                    SearchPickerInputResult::Navigated => true,
                    SearchPickerInputResult::Edited { changed } => {
                        if changed {
                            let items = self.file_mention_suggestion_items(state.input.text());
                            state.replace_items(items);
                        }
                        changed || state.input.cursor() != before_cursor
                    }
                    SearchPickerInputResult::Close => false,
                };
                self.file_mention_suggestions = Some(state);
                handled
            }
        }
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
                if let Some(state) = self.slash_command_suggestions.as_mut() {
                    if matches!(key.code, crossterm::event::KeyCode::Up) {
                        let _ = state.handle_input_key(key);
                    } else {
                        state.move_selection(-1);
                    }
                }
                true
            }
            Some(KeyAction::Next) => {
                if let Some(state) = self.slash_command_suggestions.as_mut() {
                    if matches!(key.code, crossterm::event::KeyCode::Down) {
                        let _ = state.handle_input_key(key);
                    } else {
                        state.move_selection(1);
                    }
                }
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
                let before_cursor = state.input.cursor();
                match state.handle_input_key(key) {
                    SearchPickerInputResult::Navigated => true,
                    SearchPickerInputResult::Edited { changed } => {
                        changed || state.input.cursor() != before_cursor
                    }
                    SearchPickerInputResult::Close => false,
                }
            }
        }
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
        let can_submit_without_arguments = match &item.value {
            SlashCommandSuggestionValue::Command(spec) => !spec.requires_arguments(),
            SlashCommandSuggestionValue::PluginCommand(_) => false,
        };

        self.apply_slash_command_completion(&item);
        if submit && can_submit_without_arguments {
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
            SlashCommandSuggestionValue::Command(spec) => spec.name.to_string(),
            SlashCommandSuggestionValue::PluginCommand(entry) => {
                let Some(name) = plugin_command_slash_name(entry) else {
                    return;
                };
                name
            }
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
            self.plugin_slash_commands()
                .into_iter()
                .filter(|entry| plugin_command_matches_slash_query(entry, &query))
                .map(|entry| {
                    let label = plugin_command_slash_name(&entry)
                        .expect("plugin slash commands have a normalized slash name");
                    SlashCommandSuggestionItem {
                        label: format!("/{label}"),
                        detail: plugin_command_detail(&entry),
                        value: SlashCommandSuggestionValue::PluginCommand(Box::new(entry)),
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
    PromptHistorySearchResult, PromptHistorySearchState, SlashCommandSuggestionContext,
    SlashCommandSuggestionItem, SlashCommandSuggestionMeta, SlashCommandSuggestionState,
    SlashCommandSuggestionValue, UiAction, commands, current_spinner_millis,
    file_mention_suggestion_context_for_text, min, plugin_command_detail,
    plugin_command_matches_slash_query, plugin_command_slash_name,
    slash_command_suggestion_context_for_text, spinner_frame, transcript_node_kind_label,
    transcript_spinner_placeholder, ui_text,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui_components::{SearchPickerConfig, SearchPickerInputResult, SearchPickerSearchMode};
use crossterm::event::{KeyCode, KeyModifiers};

fn composer_slash_opens_command_palette(key: KeyEvent, text: &str, cursor: usize) -> bool {
    key.code == KeyCode::Char('/')
        && key.modifiers == KeyModifiers::NONE
        && text.is_empty()
        && cursor == 0
}

fn composer_up_opens_prompt_history(key: KeyEvent, cursor: usize) -> bool {
    key.code == KeyCode::Up && key.modifiers == KeyModifiers::NONE && cursor == 0
}

#[derive(Debug, PartialEq, Eq)]
enum PromptHistoryPickerOutcome {
    KeepOpen,
    Close,
    Accept(String),
}

/// Keep picker navigation completely isolated from the composer. The caller
/// receives text only for an explicit accept action, so moving, filtering, and
/// closing the picker cannot accidentally preview or restore a history entry.
fn handle_prompt_history_picker_key(
    search: &mut PromptHistorySearchState,
    key: KeyEvent,
) -> PromptHistoryPickerOutcome {
    match resolve_tui_key(KeyContext::PromptHistory, key) {
        Some(KeyAction::Close) => PromptHistoryPickerOutcome::Close,
        Some(KeyAction::Accept) => search
            .selected_item()
            .map(|result| PromptHistoryPickerOutcome::Accept(result.text.clone()))
            .unwrap_or(PromptHistoryPickerOutcome::Close),
        Some(KeyAction::Previous | KeyAction::Next) => {
            let _ = search.handle_input_key(key);
            PromptHistoryPickerOutcome::KeepOpen
        }
        Some(KeyAction::Older) => {
            search.move_selection(1);
            PromptHistoryPickerOutcome::KeepOpen
        }
        Some(KeyAction::Newer) if search.selected == 0 => PromptHistoryPickerOutcome::Close,
        Some(KeyAction::Newer) => {
            search.move_selection(-1);
            PromptHistoryPickerOutcome::KeepOpen
        }
        Some(KeyAction::NewerKeepOpen) => {
            search.move_selection(-1);
            PromptHistoryPickerOutcome::KeepOpen
        }
        _ => {
            let _ = search.handle_input_key(key);
            PromptHistoryPickerOutcome::KeepOpen
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Editor, PromptHistoryPickerOutcome, PromptHistorySearchResult, PromptHistorySearchState,
        SearchPickerConfig, composer_slash_opens_command_palette, composer_up_opens_prompt_history,
        handle_prompt_history_picker_key,
    };
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

    #[test]
    fn only_plain_slash_in_an_empty_composer_opens_the_command_palette() {
        let slash = key(KeyCode::Char('/'), KeyModifiers::NONE);

        assert!(composer_slash_opens_command_palette(slash, "", 0));
        assert!(!composer_slash_opens_command_palette(slash, "draft", 0));
        assert!(!composer_slash_opens_command_palette(slash, "", 1));
        assert!(!composer_slash_opens_command_palette(
            key(KeyCode::Char('/'), KeyModifiers::CONTROL),
            "",
            0,
        ));
        assert!(!composer_slash_opens_command_palette(
            key(KeyCode::Char('a'), KeyModifiers::NONE),
            "",
            0,
        ));
    }

    #[test]
    fn prompt_history_only_returns_text_after_explicit_accept() {
        let mut search = PromptHistorySearchState::new(
            "History".to_string(),
            String::new(),
            String::new(),
            "No matches".to_string(),
            Editor::default(),
            SearchPickerConfig::searchable(),
            None,
            (),
        );
        search.replace_items(vec![
            PromptHistorySearchResult {
                history_index: 1,
                text: "new prompt".to_string(),
            },
            PromptHistorySearchResult {
                history_index: 0,
                text: "old prompt".to_string(),
            },
        ]);

        assert_eq!(
            handle_prompt_history_picker_key(&mut search, key(KeyCode::Down, KeyModifiers::NONE),),
            PromptHistoryPickerOutcome::KeepOpen
        );
        assert_eq!(
            handle_prompt_history_picker_key(&mut search, key(KeyCode::Down, KeyModifiers::NONE),),
            PromptHistoryPickerOutcome::KeepOpen
        );
        assert_eq!(search.selected, 1);

        assert_eq!(
            handle_prompt_history_picker_key(&mut search, key(KeyCode::Up, KeyModifiers::NONE),),
            PromptHistoryPickerOutcome::KeepOpen
        );
        assert_eq!(search.selected, 0);

        assert_eq!(
            handle_prompt_history_picker_key(
                &mut search,
                key(KeyCode::Char('o'), KeyModifiers::NONE),
            ),
            PromptHistoryPickerOutcome::KeepOpen
        );
        assert_eq!(search.input.text(), "o");
        let selected_text = search
            .selected_item()
            .expect("filtered history selection")
            .text
            .clone();

        assert_eq!(
            handle_prompt_history_picker_key(&mut search, key(KeyCode::Enter, KeyModifiers::NONE),),
            PromptHistoryPickerOutcome::Accept(selected_text)
        );
    }

    #[test]
    fn closing_prompt_history_never_returns_preview_text() {
        let mut search = PromptHistorySearchState::new(
            "History".to_string(),
            String::new(),
            String::new(),
            "No matches".to_string(),
            Editor::default(),
            SearchPickerConfig::searchable(),
            None,
            (),
        );
        search.replace_items(vec![PromptHistorySearchResult {
            history_index: 0,
            text: "history prompt".to_string(),
        }]);

        assert_eq!(
            handle_prompt_history_picker_key(&mut search, key(KeyCode::Esc, KeyModifiers::NONE),),
            PromptHistoryPickerOutcome::Close
        );
    }
}
