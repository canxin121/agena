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

    pub(in crate::app) fn copy_transcript_cursor_node(&mut self) {
        let width = self.layout.transcript_body.width;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return;
        };
        match set_clipboard_text(node.copy_text.as_str()) {
            Ok(method) => self.flash_clipboard_copy_success(
                method,
                self.i18n.text_args(
                    "flash-transcript-node-copied",
                    &crate::fl_args!("kind" => transcript_node_kind_label(&self.i18n, node.kind)),
                ),
            ),
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-clipboard-copy-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
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
        // Esc handling is special — double-tap clears the input. We track
        // it before consulting the configurable bindings.
        if matches!(key.code, KeyCode::Esc) && key.modifiers.is_empty() {
            self.focus = Focus::Transcript;
            self.sync_composer_suggestions();
            return;
        }
        if matches!(key.code, KeyCode::Up) && key.modifiers.contains(KeyModifiers::ALT) {
            self.open_prompt_history_search();
            return;
        }
        // Match shell/OpenCode history navigation without stealing normal
        // multiline editing: bare Up opens history only at the start of the
        // input. Once open, Up/Down move through the floating newest-first
        // list. Queued message editing remains higher priority.
        if matches!(key.code, KeyCode::Up)
            && key.modifiers.is_empty()
            && self.composer.cursor() == 0
        {
            if self.try_pop_queue_into_editor() {
                self.reset_prompt_history_recall();
                self.after_composer_text_mutated();
            } else {
                self.open_prompt_history_search();
            }
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
                    // Fall through to normal cursor-up behavior when queue
                    // is empty.
                }
                ComposerAction::HistorySearch => {
                    self.open_prompt_history_search();
                    return;
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
                    self.composer.flush_all_pending_input();
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
        match key {
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.selected_composer_item = None;
                true
            }
            KeyEvent {
                code: KeyCode::BackTab,
                ..
            }
            | KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.selected_composer_item = Some(index.saturating_sub(1));
                true
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            }
            | KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('l'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.selected_composer_item =
                    Some(min(index + 1, self.composer_items.len().saturating_sub(1)));
                true
            }
            KeyEvent {
                code: KeyCode::Delete | KeyCode::Backspace,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.remove_composer_item(index);
                true
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('o'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
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
        let close = match key {
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.replace_composer_draft(search.meta.original.clone());
                true
            }
            KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.replace_composer_draft(search.meta.original.clone());
                true
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
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
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE | KeyModifiers::ALT,
                ..
            } => {
                if search.selected + 1 >= search.items.len() && search.meta.has_more {
                    search.meta.loaded_count = search
                        .meta
                        .loaded_count
                        .saturating_add(PROMPT_HISTORY_PAGE_SIZE);
                    Self::refresh_prompt_history_search(&self.prompt_history, &mut search);
                }
                move_selected_index(&mut search.selected, search.items.len(), 1);
                false
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE | KeyModifiers::ALT,
                ..
            } => {
                if search.selected == 0 {
                    self.replace_composer_draft(search.meta.original.clone());
                    true
                } else {
                    move_selected_index(&mut search.selected, search.items.len(), -1);
                    false
                }
            }
            KeyEvent {
                code: KeyCode::Char('s'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                move_selected_index(&mut search.selected, search.items.len(), -1);
                false
            }
            _ => {
                let before = search.query.text().to_string();
                search.query.handle_line_input_key(key);
                search.query.flush_all_pending_input();
                if search.query.text() != before {
                    search.selected = 0;
                    search.meta.loaded_count = PROMPT_HISTORY_PAGE_SIZE;
                    Self::refresh_prompt_history_search(&self.prompt_history, &mut search);
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
        self.composer.flush_all_pending_input();
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
        let mut search = PromptHistorySearchState::new(
            Editor::default(),
            0,
            PromptHistorySearchMeta {
                original: self.current_composer_draft(),
                loaded_count: PROMPT_HISTORY_PAGE_SIZE,
                total_matches: 0,
                has_more: false,
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
        search.query.flush_all_pending_input();
        let query = search.query.text().trim().to_ascii_lowercase();
        let mut total_matches = 0_usize;
        let mut loaded = Vec::with_capacity(search.meta.loaded_count);
        for (history_index, text) in prompt_history.items.iter().enumerate().rev() {
            if !query.is_empty() && !text.to_ascii_lowercase().contains(&query) {
                continue;
            }
            total_matches = total_matches.saturating_add(1);
            if loaded.len() < search.meta.loaded_count {
                loaded.push(PromptHistorySearchResult {
                    history_index,
                    text: text.clone(),
                });
            }
        }
        search.meta.total_matches = total_matches;
        search.meta.has_more = total_matches > loaded.len();
        search.items = loaded;
        search.clamp_selection();
    }

    pub(in crate::app) fn handle_file_mention_suggestion_key(&mut self, key: KeyEvent) -> bool {
        if self.file_mention_suggestions.is_none() {
            return false;
        }

        match key {
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_file_mention_suggestion(-1);
                true
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_file_mention_suggestion(1);
                true
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.dismiss_file_mention_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.complete_selected_file_mention();
                true
            }
            _ => false,
        }
    }

    pub(in crate::app) fn move_file_mention_suggestion(&mut self, delta: isize) {
        let Some(state) = self.file_mention_suggestions.as_mut() else {
            return;
        };
        state.move_selection_cycle(delta);
    }

    pub(in crate::app) fn dismiss_file_mention_suggestions(&mut self) {
        if let Some(state) = self.file_mention_suggestions.take() {
            self.dismissed_file_mention_suggestions_for = Some(state.fingerprint);
        }
    }

    pub(in crate::app) fn complete_selected_file_mention(&mut self) {
        let Some(state) = self.file_mention_suggestions.clone() else {
            return;
        };
        let Some(item) = state.items.get(state.selected).cloned() else {
            return;
        };

        self.file_mention_suggestions = None;
        self.dismissed_file_mention_suggestions_for = None;
        self.composer
            .remove_range(state.meta.mention_range.start, state.meta.mention_range.end);
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

        let items = self.file_mention_suggestion_items(context.query.as_str());
        if items.is_empty() {
            self.file_mention_suggestions = None;
            return;
        }

        let selected = self
            .file_mention_suggestions
            .as_ref()
            .filter(|state| state.query == context.query)
            .map(|state| min(state.selected, items.len().saturating_sub(1)))
            .unwrap_or(0);
        self.file_mention_suggestions = Some(FileMentionSuggestionState::new(
            context.query,
            context.fingerprint,
            items,
            selected,
            FileMentionSuggestionMeta {
                mention_range: context.mention_range,
            },
        ));
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

        match key {
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_slash_command_suggestion(-1);
                true
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_slash_command_suggestion(1);
                true
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.dismiss_slash_command_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                self.complete_selected_slash_command_suggestion(false);
                true
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.complete_selected_slash_command_suggestion(true);
                true
            }
            _ => false,
        }
    }

    pub(in crate::app) fn move_slash_command_suggestion(&mut self, delta: isize) {
        let Some(state) = self.slash_command_suggestions.as_mut() else {
            return;
        };
        state.move_selection_cycle(delta);
    }

    pub(in crate::app) fn dismiss_slash_command_suggestions(&mut self) {
        if let Some(state) = self.slash_command_suggestions.take() {
            self.dismissed_slash_command_suggestions_for = Some(state.fingerprint);
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

        let items = self.slash_command_suggestion_items(context.query.as_str());
        if items.is_empty() {
            self.slash_command_suggestions = None;
            return;
        }

        let selected = self
            .slash_command_suggestions
            .as_ref()
            .filter(|state| state.query == context.query)
            .map(|state| min(state.selected, items.len().saturating_sub(1)))
            .unwrap_or(0);
        self.slash_command_suggestions = Some(SlashCommandSuggestionState::new(
            context.query,
            context.fingerprint,
            items,
            selected,
            SlashCommandSuggestionMeta,
        ));
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
        if !context.query.is_empty()
            && self
                .slash_command_suggestion_items(context.query.as_str())
                .is_empty()
        {
            return None;
        }
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
    FileMentionSuggestionState, Focus, KeyCode, KeyEvent, KeyModifiers,
    MAX_FILE_MENTION_SUGGESTIONS, PROMPT_HISTORY_PAGE_SIZE, PromptHistory, PromptHistorySearchMeta,
    PromptHistorySearchResult, PromptHistorySearchState, SlashCommandSuggestionContext,
    SlashCommandSuggestionItem, SlashCommandSuggestionMeta, SlashCommandSuggestionState,
    SlashCommandSuggestionValue, UiAction, commands, file_mention_suggestion_context_for_text, min,
    move_selected_index, runtime_tool_matches_slash_query, set_clipboard_text,
    slash_command_suggestion_context_for_text, transcript_node_kind_label, ui_text,
};
