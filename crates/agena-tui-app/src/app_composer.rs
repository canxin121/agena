impl App {
    pub(crate) fn enter_insert_mode(&mut self) {
        self.focus = Focus::Composer;
    }

    /// Opens the unified, cursor-preserving insertion palette. Every option
    /// stages an atomic user message part at the current composer cursor.
    pub(crate) fn open_insert_content_picker(&mut self) {
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, "overlay-insert-content-title"),
            ui_text::t(&self.i18n, "overlay-insert-content-prompt"),
            None,
            vec![
                ChoiceItem {
                    label: ui_text::t(&self.i18n, "insert-content-skill-label"),
                    detail: ui_text::t(&self.i18n, "insert-content-skill-detail"),
                    value: "skill".to_owned(),
                    search_text: "skill instructions".to_owned(),
                    current: false,
                },
                ChoiceItem {
                    label: ui_text::t(&self.i18n, "insert-content-file-label"),
                    detail: ui_text::t(&self.i18n, "insert-content-file-detail"),
                    value: "file".to_owned(),
                    search_text: "file folder path attachment document image".to_owned(),
                    current: false,
                },
            ],
            ChoiceOverlayAction::InsertContent,
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        ));
    }

    pub(crate) fn toggle_transcript_cursor_node(&mut self) {
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
            &agena_tui::fl_args!("kind" => transcript_node_kind_label(&self.i18n, kind)),
        ));
    }

    pub(crate) fn flash_clipboard_copy_success(
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

    pub(crate) fn request_clipboard_copy(&mut self, text: String, success: String) {
        self.pending_ui_action = Some(UiAction::CopyText { text, success });
    }

    pub(crate) fn copy_transcript_cursor_node(&mut self) {
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
            &agena_tui::fl_args!("kind" => transcript_node_kind_label(&self.i18n, node.kind)),
        );
        self.request_clipboard_copy(node.copy_text, success);
    }

    pub(crate) fn handle_composer_key(&mut self, key: KeyEvent) {
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
                    // No pending message leaves Ctrl+P as a no-op. Bare Up
                    // keeps its normal cursor movement except at position
                    // zero, where it opens prompt history.
                }
                ComposerAction::CancelPending => {
                    self.cancel_pending_message();
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
                ComposerAction::InsertContent => {
                    self.open_insert_content_picker();
                    return;
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
                ComposerAction::AttachImage => {
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
        if composer_shell_edit_shortcut_is_disabled(key) {
            return;
        }
        self.reset_prompt_history_recall();
        self.composer.handle_multiline_input_key(key);
        self.after_composer_text_mutated();
    }

    pub(crate) fn after_composer_text_mutated(&mut self) {
        // Editing invalidates any mouse selection over the composer editor;
        // the stored cell range no longer maps to the same text.
        self.cancel_surface_selection();
        self.sync_composer_items_with_editor();
        self.clamp_selected_composer_item();
        self.sync_composer_suggestions();
    }

    pub(crate) fn sync_composer_suggestions(&mut self) {
        if self.prompt_history_search.is_some() {
            self.slash_command_suggestions = None;
            self.slash_command_suggestion_actions.clear();
            self.file_mention_suggestions = None;
            self.file_mention_suggestion_actions.clear();
            return;
        }
        self.sync_file_mention_suggestions();
        self.sync_slash_command_suggestions();
    }

    pub(crate) fn toggle_composer_item_selection(&mut self) -> bool {
        self.composer_item_selection
            .toggle(self.composer_items.len())
    }

    pub(crate) fn clamp_selected_composer_item(&mut self) {
        self.composer_item_selection
            .clamp(self.composer_items.len());
    }

    pub(crate) fn handle_selected_composer_item_key(&mut self, key: KeyEvent) -> bool {
        let action = match resolve_tui_key(KeyContext::ComposerItem, key) {
            Some(KeyAction::Close) => ComposerItemAction::Close,
            Some(KeyAction::Previous) => ComposerItemAction::Previous,
            Some(KeyAction::Next) => ComposerItemAction::Next,
            Some(KeyAction::Delete) => ComposerItemAction::Delete,
            Some(KeyAction::Open) => ComposerItemAction::Open,
            _ => return false,
        };
        match self
            .composer_item_selection
            .reduce(action, self.composer_items.len())
        {
            ComposerItemEffect::Ignored => false,
            ComposerItemEffect::Consumed => true,
            ComposerItemEffect::Remove(index) => {
                self.remove_composer_item(index);
                true
            }
            ComposerItemEffect::Open(index) => {
                self.open_selected_composer_item(index);
                true
            }
        }
    }

    pub(crate) fn remove_composer_item(&mut self, index: usize) {
        let Some(range) = self.composer.draft_elements().get(index).cloned() else {
            return;
        };
        self.composer.remove_range(range.start, range.end);
        self.after_composer_text_mutated();
    }

    pub(crate) fn open_selected_composer_item(&mut self, index: usize) {
        let Some(item) = self.composer_items.get(index) else {
            return;
        };
        match item.payload() {
            agena_domain::ActivityPayload::Resource(resource) => {
                if let agena_domain::ResourceReference::WorkspacePath { path } = &resource.reference
                {
                    self.pending_ui_action = Some(UiAction::OpenPath {
                        path: self
                            .resolve_workspace_path(std::path::Path::new(path)),
                    });
                } else {
                    self.flash_info(ui_text::t(&self.i18n, "flash-large-paste-no-file-view"));
                }
            }
            agena_domain::ActivityPayload::TextArtifact(_) => {
                self.flash_info(ui_text::t(&self.i18n, "flash-large-paste-no-file-view"));
            }
            agena_domain::ActivityPayload::SkillReference(_) => {
                self.flash_info(ui_text::t(&self.i18n, "flash-skill-no-file-view"));
            }
            _ => self.flash_info(ui_text::t(&self.i18n, "flash-large-paste-no-file-view")),
        }
    }

    pub(crate) fn handle_prompt_history_search_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut search) = self.prompt_history_search.take() else {
            return false;
        };
        match agena_tui::prompt_history::handle_key(&mut search, key) {
            agena_tui::prompt_history::PromptHistoryPickerEffect::KeepOpen => {
                self.prompt_history_search = Some(search);
            }
            agena_tui::prompt_history::PromptHistoryPickerEffect::Close => {}
            agena_tui::prompt_history::PromptHistoryPickerEffect::UseText(text) => {
                self.replace_composer_draft(ComposerDraft {
                    document: agena_domain::ComposerDocument(vec![
                        agena_domain::ComposerNode::Text { text },
                    ]),
                });
            }
        }
        true
    }

    pub(crate) fn open_prompt_history_search(&mut self) {
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
        self.slash_command_suggestion_actions.clear();
        self.file_mention_suggestions = None;
        self.file_mention_suggestion_actions.clear();
        self.composer_item_selection.clear();
        self.prompt_history_search = Some(search);
    }

    pub(crate) fn refresh_prompt_history_search(
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

    pub(crate) fn handle_file_mention_suggestion_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut state) = self.file_mention_suggestions.take() else {
            return false;
        };

        match agena_tui::file_mentions::handle_key(&mut state, key) {
            agena_tui::file_mentions::FileMentionSuggestionEffect::KeepOpen => {
                self.file_mention_suggestions = Some(state);
                true
            }
            agena_tui::file_mentions::FileMentionSuggestionEffect::Dismiss => {
                self.file_mention_suggestions = Some(state);
                self.dismiss_file_mention_suggestions();
                true
            }
            agena_tui::file_mentions::FileMentionSuggestionEffect::Refresh { query } => {
                let (items, actions) = self.file_mention_suggestion_items(query.as_str());
                state.replace_items(items);
                self.file_mention_suggestion_actions = actions;
                self.file_mention_suggestions = Some(state);
                true
            }
            agena_tui::file_mentions::FileMentionSuggestionEffect::Select { key } => {
                self.complete_file_mention_suggestion(key);
                true
            }
            agena_tui::file_mentions::FileMentionSuggestionEffect::Unhandled => {
                self.file_mention_suggestions = Some(state);
                false
            }
        }
    }

    pub(crate) fn dismiss_file_mention_suggestions(&mut self) {
        if let Some(state) = self.file_mention_suggestions.take() {
            self.dismissed_file_mention_suggestions_for = Some(state.meta.fingerprint);
        }
        self.file_mention_suggestion_actions.clear();
    }

    pub(crate) fn complete_file_mention_suggestion(&mut self, key: String) {
        let Some(action) = self
            .file_mention_suggestion_actions
            .get(key.as_str())
            .cloned()
        else {
            self.file_mention_suggestions = None;
            self.file_mention_suggestion_actions.clear();
            return;
        };
        let Some(context) = self.file_mention_suggestion_context() else {
            self.file_mention_suggestions = None;
            self.file_mention_suggestion_actions.clear();
            return;
        };

        self.file_mention_suggestions = None;
        self.file_mention_suggestion_actions.clear();
        self.dismissed_file_mention_suggestions_for = None;
        self.composer
            .remove_range(context.mention_range.start, context.mention_range.end);
        if let Err(error) = self.stage_attachment_from_path(action.path.as_path()) {
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

    pub(crate) fn sync_file_mention_suggestions(&mut self) {
        let Some(context) = self.file_mention_suggestion_context() else {
            self.file_mention_suggestions = None;
            self.file_mention_suggestion_actions.clear();
            return;
        };
        if self.dismissed_file_mention_suggestions_for.as_deref()
            == Some(context.fingerprint.as_str())
        {
            self.file_mention_suggestions = None;
            self.file_mention_suggestion_actions.clear();
            return;
        }

        if self
            .file_mention_suggestions
            .as_ref()
            .is_some_and(|state| state.meta.fingerprint == context.fingerprint)
        {
            return;
        }

        let (items, actions) = self.file_mention_suggestion_items(context.query.as_str());
        let mut state = agena_tui::file_mentions::new_state(
            ui_text::t(&self.i18n, "overlay-attach-title"),
            ui_text::t(&self.i18n, "overlay-attach-prompt"),
            ui_text::t(&self.i18n, "overlay-attach-footer"),
            ui_text::t(&self.i18n, "overlay-attach-no-match"),
            context.query,
            context.fingerprint,
        );
        state.replace_items(items);
        self.file_mention_suggestions = Some(state);
        self.file_mention_suggestion_actions = actions;
    }

    pub(crate) fn file_mention_suggestion_context(&self) -> Option<FileMentionSuggestionContext> {
        if self.focus != Focus::Composer || self.overlay.is_some() || !self.current_route_is_main()
        {
            return None;
        }
        if self.prompt_history_search.is_some() {
            return None;
        }
        file_mention_suggestion_context_for_text(self.composer.text(), self.composer.cursor())
    }

    pub(crate) fn file_mention_suggestion_items(
        &self,
        query: &str,
    ) -> (
        Vec<FileMentionSuggestionItem>,
        BTreeMap<String, FileMentionSuggestionAction>,
    ) {
        let mut items = Vec::new();
        let mut actions = BTreeMap::new();
        for (index, path) in crate::app_backend::file_index::search_workspace_files(
            &self.application,
            &self.file_index,
            query,
            MAX_FILE_MENTION_SUGGESTIONS,
        )
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        {
            let key = format!("path:{index}");
            let label = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| path.display().to_string());
            actions.insert(
                key.clone(),
                FileMentionSuggestionAction { path: path.clone() },
            );
            items.push(FileMentionSuggestionItem {
                key,
                detail: path.display().to_string(),
                label,
            });
        }
        (items, actions)
    }

    pub(crate) fn handle_slash_command_suggestion_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut state) = self.slash_command_suggestions.take() else {
            return false;
        };

        match agena_tui::slash_commands::handle_key(&mut state, key) {
            agena_tui::slash_commands::SlashCommandSuggestionEffect::KeepOpen => {
                self.slash_command_suggestions = Some(state);
                true
            }
            agena_tui::slash_commands::SlashCommandSuggestionEffect::Dismiss => {
                self.slash_command_suggestions = Some(state);
                self.dismiss_slash_command_suggestions();
                true
            }
            agena_tui::slash_commands::SlashCommandSuggestionEffect::Fill { key } => {
                self.complete_slash_command_suggestion(key, false);
                true
            }
            agena_tui::slash_commands::SlashCommandSuggestionEffect::Accept { key } => {
                self.complete_slash_command_suggestion(key, true);
                true
            }
            agena_tui::slash_commands::SlashCommandSuggestionEffect::Unhandled => {
                self.slash_command_suggestions = Some(state);
                false
            }
        }
    }

    pub(crate) fn dismiss_slash_command_suggestions(&mut self) {
        if let Some(state) = self.slash_command_suggestions.take() {
            self.dismissed_slash_command_suggestions_for = Some(state.meta.fingerprint);
        }
        self.slash_command_suggestion_actions.clear();
    }

    pub(crate) fn complete_slash_command_suggestion(&mut self, key: String, submit: bool) {
        let Some(action) = self
            .slash_command_suggestion_actions
            .get(key.as_str())
            .cloned()
        else {
            // A catalog refresh can invalidate a picker key between key
            // handling and completion. Drop both halves together so the App
            // never retains action metadata for a vanished TUI row.
            self.slash_command_suggestions = None;
            self.slash_command_suggestion_actions.clear();
            return;
        };

        self.apply_slash_command_completion(&action);
        if submit && action.can_submit_without_arguments {
            self.submit_composer();
        } else {
            self.sync_composer_suggestions();
        }
    }

    pub(crate) fn apply_slash_command_completion(&mut self, action: &SlashCommandSuggestionAction) {
        let Some(context) = self.slash_command_suggestion_context() else {
            return;
        };

        let replacement = format!("/{}", action.slash_name);
        self.slash_command_suggestions = None;
        self.slash_command_suggestion_actions.clear();
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

    pub(crate) fn sync_slash_command_suggestions(&mut self) {
        let Some(context) = self.slash_command_suggestion_context() else {
            self.slash_command_suggestions = None;
            self.slash_command_suggestion_actions.clear();
            return;
        };
        if self.dismissed_slash_command_suggestions_for.as_deref()
            == Some(context.fingerprint.as_str())
        {
            self.slash_command_suggestions = None;
            self.slash_command_suggestion_actions.clear();
            return;
        }

        if self
            .slash_command_suggestions
            .as_ref()
            .is_some_and(|state| state.meta.fingerprint == context.fingerprint)
        {
            return;
        }

        let (items, actions) = self.slash_command_suggestion_items("");
        if items.is_empty() {
            self.slash_command_suggestions = None;
            self.slash_command_suggestion_actions.clear();
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
        self.slash_command_suggestion_actions = actions;
    }

    pub(crate) fn slash_command_suggestion_context(&self) -> Option<SlashCommandSuggestionContext> {
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

    pub(crate) fn slash_command_suggestion_items(
        &self,
        query: &str,
    ) -> (
        Vec<SlashCommandSuggestionItem>,
        BTreeMap<String, SlashCommandSuggestionAction>,
    ) {
        let query = query.trim().to_ascii_lowercase();
        let mut items = Vec::new();
        let mut actions = BTreeMap::new();
        for spec in commands::command_suggestions_for_prefix(query.as_str()) {
            let key = format!("command:{}", spec.name);
            actions.insert(
                key.clone(),
                SlashCommandSuggestionAction {
                    slash_name: spec.name.to_owned(),
                    can_submit_without_arguments: !spec.requires_arguments(),
                },
            );
            items.push(SlashCommandSuggestionItem {
                key,
                label: format!("/{}", spec.name),
                detail: ui_text::t(&self.i18n, spec.summary_key),
            });
        }
        for entry in self
            .plugin_slash_commands()
            .into_iter()
            .filter(|entry| plugin_command_matches_slash_query(entry, &query))
        {
            let Some(slash_name) = plugin_command_slash_name(&entry) else {
                // A plugin may legally declare a command without a slash
                // name; skip rather than assume the suggestion source
                // pre-filtered it out.
                continue;
            };
            let key = format!("plugin-command:{}:{}", entry.plugin_id, entry.command.id);
            actions.insert(
                key.clone(),
                SlashCommandSuggestionAction {
                    slash_name: slash_name.clone(),
                    can_submit_without_arguments: plugin_command_accepts_empty_arguments(&entry),
                },
            );
            items.push(SlashCommandSuggestionItem {
                key,
                label: format!("/{slash_name}"),
                detail: plugin_command_detail(&entry),
            });
        }
        (items, actions)
    }

    /// Ctrl+P / EditQueue binding: pull the single pending message back into
    /// the composer for editing from any cursor position. Returns true if
    /// anything was pulled (so the caller skips the default cursor-up
    /// behavior).
    pub(crate) fn try_pop_queue_into_editor(&mut self) -> bool {
        let Some(combined) = self.queue.take() else {
            return false;
        };
        // Merge the queued draft on top of whatever's already in the
        // editor.
        let mut existing = self.take_composer_draft();
        let existing_render = existing.render_text();
        if !existing_render.is_empty() && !existing_render.ends_with('\n') {
            existing.document.0.push(agena_domain::ComposerNode::Text {
                text: "\n\n".to_owned(),
            });
        }
        existing.document.0.extend(combined.document.0);
        self.restore_composer_draft(existing);
        true
    }

    /// Cancel the single pending message (Ctrl+X). Shows a hint when there
    /// is nothing to cancel so the key never silently disappears.
    pub(crate) fn cancel_pending_message(&mut self) {
        if self.queue.is_empty() {
            self.flash_info(ui_text::t(&self.i18n, "flash-no-pending-message"));
            return;
        }
        self.queue.clear();
        self.flash_success(ui_text::t(&self.i18n, "flash-pending-cancelled"));
    }
}
use crate::{
    App, BTreeMap, ChoiceItem, ChoiceOverlayAction, ClipboardCopyMethod, ComposerDraft, Editor,
    FileMentionSuggestionAction, FileMentionSuggestionContext, FileMentionSuggestionItem, KeyEvent,
    MAX_FILE_MENTION_SUGGESTIONS, PromptHistory, PromptHistorySearchResult,
    PromptHistorySearchState, SlashCommandSuggestionAction, SlashCommandSuggestionContext,
    SlashCommandSuggestionItem, SlashCommandSuggestionMeta, SlashCommandSuggestionState, UiAction,
    commands, current_spinner_millis, file_mention_suggestion_context_for_text,
    plugin_command_accepts_empty_arguments, plugin_command_detail,
    plugin_command_matches_slash_query, plugin_command_slash_name,
    slash_command_suggestion_context_for_text, spinner_frame, transcript_node_kind_label,
    transcript_spinner_placeholder, ui_text,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
use agena_tui::{
    composer::{ComposerItemAction, ComposerItemEffect},
    input::ComposerAction,
};
use agena_tui_components::SearchPickerConfig;
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

/// The composer deliberately uses standard terminal/editor movement and
/// deletion keys rather than the shell-style Ctrl chord family. Keep these
/// chords inert here so the help surface and actual editing behavior agree.
fn composer_shell_edit_shortcut_is_disabled(key: KeyEvent) -> bool {
    key.modifiers == KeyModifiers::CONTROL
        && matches!(
            key.code,
            KeyCode::Char('a' | 'e' | 'b' | 'f' | 'p' | 'n' | 'd' | 'w' | 'u' | 'k' | 'y')
        )
}

#[cfg(test)]
mod tests {
    use super::{
        Editor, PromptHistorySearchResult, PromptHistorySearchState, SearchPickerConfig,
        composer_slash_opens_command_palette, composer_up_opens_prompt_history,
    };
    use agena_tui::prompt_history::{PromptHistoryPickerEffect, handle_key};
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
            handle_key(&mut search, key(KeyCode::Down, KeyModifiers::NONE),),
            PromptHistoryPickerEffect::KeepOpen
        );
        assert_eq!(
            handle_key(&mut search, key(KeyCode::Down, KeyModifiers::NONE),),
            PromptHistoryPickerEffect::KeepOpen
        );
        assert_eq!(search.selected, 1);

        assert_eq!(
            handle_key(&mut search, key(KeyCode::Up, KeyModifiers::NONE),),
            PromptHistoryPickerEffect::KeepOpen
        );
        assert_eq!(search.selected, 0);

        assert_eq!(
            handle_key(&mut search, key(KeyCode::Char('o'), KeyModifiers::NONE),),
            PromptHistoryPickerEffect::KeepOpen
        );
        assert_eq!(search.input.text(), "o");
        let selected_text = search
            .selected_item()
            .expect("filtered history selection")
            .text
            .clone();

        assert_eq!(
            handle_key(&mut search, key(KeyCode::Enter, KeyModifiers::NONE),),
            PromptHistoryPickerEffect::UseText(selected_text)
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
            handle_key(&mut search, key(KeyCode::Esc, KeyModifiers::NONE),),
            PromptHistoryPickerEffect::Close
        );
    }
}
