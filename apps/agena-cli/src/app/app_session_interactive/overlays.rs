impl App {
    pub(in crate::app) fn open_user_input_overlay(&mut self) {
        let Some((session_id, request)) = self.pending_user_input_overlay_target() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-user-input-request"));
            return;
        };
        self.seen_user_input_request_ids
            .insert(request.request_id.clone());
        self.overlay = Some(Overlay::UserInputReply(Self::build_user_input_overlay(
            session_id, request,
        )));
    }

    pub(in crate::app) fn pending_user_input_overlay_target(
        &self,
    ) -> Option<(i64, UserInputRequest)> {
        let execution = self.transcript.execution.as_ref()?;
        let request = first_pending_interactive_request_by_kind(
            execution.pending_interactive_requests.as_slice(),
            PendingInteractiveKind::UserInput,
        )?;
        let session_id = request.session_id;
        let request = request.request.as_user_input()?.clone();
        Some((session_id, request))
    }

    pub(in crate::app) fn pending_permission_overlay_target(
        &self,
    ) -> Option<(i64, PermissionRequest)> {
        let execution = self.transcript.execution.as_ref()?;
        let request = first_pending_interactive_request_by_kind(
            execution.pending_interactive_requests.as_slice(),
            PendingInteractiveKind::Permission,
        )?;
        let session_id = request.session_id;
        let request = request.request.as_permission()?.clone();
        Some((session_id, request))
    }

    pub(in crate::app) fn build_user_input_overlay(
        session_id: i64,
        request: UserInputRequest,
    ) -> UserInputOverlay {
        UserInputOverlay {
            session_id,
            request,
            answers: BTreeMap::new(),
            state: QuestionFlowState::default(),
            editing_custom: false,
            custom_input: Editor::default(),
            review_option: 0,
            review_scroll: 0,
        }
    }

    pub(in crate::app) fn user_input_overlay_is_review(dialog: &UserInputOverlay) -> bool {
        user_input_review_question(&dialog.request).is_some()
    }

    pub(in crate::app) fn build_permission_overlay(
        session_id: i64,
        request: PermissionRequest,
    ) -> PermissionOverlay {
        PermissionOverlay {
            session_id,
            request,
            page: PermissionOverlayPage::Action,
            selection: SelectionCursor::default(),
        }
    }

    pub(in crate::app) fn next_pending_interactive_overlay_target(
        &self,
    ) -> Option<PendingInteractiveOverlayTarget> {
        let execution = self.transcript.execution.as_ref()?;
        let resource = first_unseen_pending_interactive_request(
            execution.pending_interactive_requests.as_slice(),
            &self.seen_permission_request_ids,
            &self.seen_user_input_request_ids,
        )?;
        let session_id = resource.session_id;
        match &resource.request {
            PendingInteractiveRequest::Permission { request } => {
                Some(PendingInteractiveOverlayTarget::Permission {
                    session_id,
                    request: Box::new(request.clone()),
                })
            }
            PendingInteractiveRequest::UserInput { request } => {
                Some(PendingInteractiveOverlayTarget::UserInput {
                    session_id,
                    request: Box::new(request.clone()),
                })
            }
        }
    }

    pub(in crate::app) fn current_session_pending_interactive_kind(
        &self,
    ) -> Option<PendingInteractiveKind> {
        self.transcript
            .execution
            .as_ref()
            .and_then(pending_interactive_kind_for_execution)
    }

    pub(in crate::app) fn pending_interactive_kind_for_session(
        &self,
        session_id: i64,
    ) -> Option<PendingInteractiveKind> {
        (self.transcript.session_id == Some(session_id))
            .then_some(())
            .and(self.current_session_pending_interactive_kind())
    }

    pub(in crate::app) fn open_pending_interactive_overlay_for_kind(
        &mut self,
        kind: PendingInteractiveKind,
    ) {
        match kind {
            PendingInteractiveKind::Permission => self.open_permission_overlay(),
            PendingInteractiveKind::UserInput => self.open_user_input_overlay(),
        }
    }

    pub(in crate::app) fn prompt_for_pending_interactive_on_session(
        &mut self,
        session_id: i64,
    ) -> bool {
        let Some(kind) = self.pending_interactive_kind_for_session(session_id) else {
            return false;
        };
        let key = self
            .transcript
            .execution
            .as_ref()
            .and_then(execution_pending_flash_key)
            .unwrap_or(match kind {
                PendingInteractiveKind::Permission => "flash-session-awaiting-approval",
                PendingInteractiveKind::UserInput => "flash-session-awaiting-user-input",
            });
        self.flash_warning(ui_text::t(&self.i18n, key));
        self.open_pending_interactive_overlay_for_kind(kind);
        true
    }

    pub(in crate::app) fn has_unseen_pending_interactive_request(&self) -> bool {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return false;
        };
        first_unseen_pending_interactive_request(
            execution.pending_interactive_requests.as_slice(),
            &self.seen_permission_request_ids,
            &self.seen_user_input_request_ids,
        )
        .is_some()
    }

    pub(in crate::app) fn should_suppress_pending_interactive_overlay(&self) -> bool {
        if !self.current_route_is_main() {
            return true;
        }
        composer_input_is_active(
            self.focus,
            !self.composer.text().trim().is_empty() || !self.composer_items.is_empty(),
            self.prompt_history_search.is_some()
                || self.file_mention_suggestions.is_some()
                || self.slash_command_suggestions.is_some()
                || self.selected_composer_item.is_some(),
        )
    }

    pub(in crate::app) fn has_suppressed_pending_interactive_overlay(&self) -> bool {
        self.has_unseen_pending_interactive_request()
            && self.should_suppress_pending_interactive_overlay()
    }

    pub(in crate::app) fn maybe_auto_open_pending_interactive_overlay(&mut self) {
        if self.overlay.is_some()
            || !self.current_route_is_main()
            || self.should_suppress_pending_interactive_overlay()
        {
            return;
        }
        match self.next_pending_interactive_overlay_target() {
            Some(PendingInteractiveOverlayTarget::Permission {
                session_id,
                request,
            }) => {
                self.seen_permission_request_ids
                    .insert(request.request_id.clone());
                self.overlay = Some(Overlay::Permission(Self::build_permission_overlay(
                    session_id, *request,
                )));
            }
            Some(PendingInteractiveOverlayTarget::UserInput {
                session_id,
                request,
            }) => {
                self.seen_user_input_request_ids
                    .insert(request.request_id.clone());
                self.overlay = Some(Overlay::UserInputReply(Self::build_user_input_overlay(
                    session_id, *request,
                )));
            }
            None => {}
        }
    }

    pub(in crate::app) fn open_permission_overlay(&mut self) {
        let Some((session_id, request)) = self.pending_permission_overlay_target() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-permission-request"));
            return;
        };
        self.seen_permission_request_ids
            .insert(request.request_id.clone());
        self.overlay = Some(Overlay::Permission(Self::build_permission_overlay(
            session_id, request,
        )));
    }

    pub(in crate::app) fn standard_choice_overlay_config() -> SearchPickerConfig {
        SearchPickerConfig {
            input_mode: SearchPickerInputMode::SearchWithCustomValue,
            ..SearchPickerConfig::searchable()
        }
    }

    pub(in crate::app) fn searchable_select_choice_overlay_config() -> SearchPickerConfig {
        SearchPickerConfig {
            input_mode: SearchPickerInputMode::Search,
            ..Self::standard_choice_overlay_config()
        }
    }

    pub(in crate::app) fn select_only_choice_overlay_config() -> SearchPickerConfig {
        SearchPickerConfig::select_only()
    }

    pub(in crate::app) fn standard_picker_overlay_config() -> SearchPickerConfig {
        SearchPickerConfig::searchable()
    }

    pub(in crate::app) fn path_browser_overlay_config() -> SearchPickerConfig {
        SearchPickerConfig {
            input_mode: SearchPickerInputMode::EditableValue,
            search_mode: SearchPickerSearchMode::External,
            fill_selected_into_input: true,
            ..SearchPickerConfig::searchable()
        }
    }

    pub(in crate::app) fn file_attach_overlay_config() -> SearchPickerConfig {
        SearchPickerConfig {
            input_mode: SearchPickerInputMode::EditableValue,
            search_mode: SearchPickerSearchMode::External,
            fill_selected_into_input: true,
            ..SearchPickerConfig::searchable()
        }
    }

    pub(in crate::app) fn session_search_overlay_config() -> SearchPickerConfig {
        SearchPickerConfig {
            search_mode: SearchPickerSearchMode::External,
            ..SearchPickerConfig::searchable()
        }
    }

    pub(in crate::app) fn session_model_chooser_overlay_config() -> SearchPickerConfig {
        SearchPickerConfig::searchable()
    }

    pub(in crate::app) fn choice_overlay_config(style: ChoiceOverlayStyle) -> SearchPickerConfig {
        match style {
            ChoiceOverlayStyle::Searchable => Self::standard_choice_overlay_config(),
            ChoiceOverlayStyle::SearchableSelect => Self::searchable_select_choice_overlay_config(),
            ChoiceOverlayStyle::SelectOnly => Self::select_only_choice_overlay_config(),
        }
    }

    pub(in crate::app) fn choice_overlay_footer(&self, style: ChoiceOverlayStyle) -> String {
        match style {
            ChoiceOverlayStyle::Searchable | ChoiceOverlayStyle::SearchableSelect => {
                ui_text::t(&self.i18n, "overlay-choice-footer")
            }
            ChoiceOverlayStyle::SelectOnly => {
                ui_text::t(&self.i18n, "overlay-choice-footer-select")
            }
        }
    }

    pub(in crate::app) fn choice_overlay_clear_action(
        &self,
        action: ChoiceOverlayAction,
    ) -> SearchPickerClearAction {
        SearchPickerClearAction {
            label: settings_clear_label(&self.i18n),
            detail: choice_overlay_clear_detail(&self.i18n, &action),
            current: false,
        }
    }

    pub(in crate::app) fn build_choice_overlay(
        &self,
        title: String,
        prompt: String,
        current_value: Option<String>,
        mut all_items: Vec<ChoiceItem>,
        action: ChoiceOverlayAction,
        allow_clear: bool,
        style: ChoiceOverlayStyle,
    ) -> ChoiceOverlay {
        mark_current_choice_item(&self.i18n, &mut all_items, current_value.as_deref());
        let clear_action = allow_clear.then(|| {
            let mut clear_action = self.choice_overlay_clear_action(action.clone());
            clear_action.current = current_value.is_none();
            clear_action
        });
        let mut overlay = ChoiceOverlay::new(
            title,
            prompt,
            self.choice_overlay_footer(style),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            Editor::default(),
            Self::choice_overlay_config(style),
            clear_action,
            ChoiceOverlayMeta {
                i18n: self.i18n.clone(),
                action,
                current_value,
            },
        );
        overlay.replace_items(all_items);
        overlay
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::app) fn build_picker_overlay(
        &self,
        title: String,
        prompt: String,
        footer: String,
        empty_message: String,
        input: Editor,
        all_items: Vec<PickerItem>,
        kind: PickerKind,
        loading: bool,
    ) -> PickerOverlay {
        let mut overlay = PickerOverlay::new(
            title,
            prompt,
            footer,
            empty_message,
            input,
            Self::standard_picker_overlay_config(),
            None,
            PickerOverlayMeta { kind },
        );
        overlay.replace_items(all_items);
        overlay.set_loading(loading);
        overlay
    }

    pub(in crate::app) fn build_path_browser_overlay(
        &self,
        title: String,
        prompt: String,
        mode: PathBrowserMode,
        initial: String,
        target: PathBrowserTarget,
    ) -> PathBrowserOverlay {
        let mut overlay = PathBrowserOverlay::new(
            title,
            prompt,
            ui_text::t(&self.i18n, "overlay-permission-rule-browser-footer"),
            ui_text::t(&self.i18n, "overlay-permission-rule-browser-empty"),
            Editor::from_text(initial),
            Self::path_browser_overlay_config(),
            None,
            PathBrowserOverlayMeta {
                i18n: self.i18n.clone(),
                mode,
                target,
            },
        );
        Self::refresh_path_browser_overlay_with_root(self.backend.workspace_root(), &mut overlay);
        overlay
    }

    pub(in crate::app) fn build_file_attach_overlay(&self) -> FileAttachOverlay {
        let mut overlay = FileAttachOverlay::new(
            ui_text::t(&self.i18n, "overlay-attach-title"),
            ui_text::t(&self.i18n, "overlay-attach-prompt"),
            ui_text::t(&self.i18n, "overlay-attach-footer"),
            ui_text::t(&self.i18n, "overlay-attach-no-match"),
            Editor::default(),
            Self::file_attach_overlay_config(),
            None,
            FileAttachOverlayMeta {
                i18n: self.i18n.clone(),
            },
        );
        self.refresh_file_attach_overlay(&mut overlay);
        overlay
    }

    pub(in crate::app) fn build_session_search_overlay(
        &self,
        input: Editor,
        mode: SessionViewMode,
        scope_session_id: Option<i64>,
    ) -> SessionSearchOverlay {
        let mut dialog = SessionSearchOverlay::new(
            ui_text::t(&self.i18n, "overlay-resume-title"),
            ui_text::t(&self.i18n, "overlay-resume-prompt"),
            String::new(),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            input,
            Self::session_search_overlay_config(),
            None,
            SessionSearchOverlayMeta {
                all_items: Vec::new(),
                mode,
                scope_session_id,
                page_index: 0,
                next_cursor: None,
                has_more: false,
            },
        );
        dialog.set_loading(true);
        dialog.footer = self.session_search_footer(&dialog);
        dialog
    }

    pub(in crate::app) fn build_session_model_chooser_overlay(
        &self,
        purpose: SessionModelChooserPurpose,
    ) -> SessionModelChooserOverlay {
        let mut dialog = SessionModelChooserOverlay::new(
            ui_text::t(&self.i18n, "overlay-session-model-title"),
            ui_text::t(&self.i18n, "overlay-session-model-prompt"),
            ui_text::t(&self.i18n, "overlay-session-model-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            Editor::default(),
            Self::session_model_chooser_overlay_config(),
            None,
            SessionModelChooserOverlayMeta { purpose },
        );
        dialog.set_loading(true);
        dialog
    }

    pub(in crate::app) fn build_line_input_overlay(
        &self,
        title: String,
        prompt: String,
        input: Editor,
    ) -> LineInputOverlay {
        LineInputOverlay::new(title, prompt, input, ())
    }

    pub(in crate::app) fn build_transcript_search_overlay(&self) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-transcript-search-title"),
            ui_text::t(&self.i18n, "overlay-transcript-search-prompt"),
            Editor::from_text(self.transcript.search_query.clone()),
        )
    }

    pub(in crate::app) fn build_model_catalog_search_overlay(
        &self,
        query: &str,
    ) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-model-catalog-search-title"),
            ui_text::t(&self.i18n, "overlay-model-catalog-search-prompt"),
            Editor::from_text(query.to_string()),
        )
    }

    pub(in crate::app) fn build_session_rename_overlay(&self, title: String) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-rename-title"),
            ui_text::t(&self.i18n, "overlay-rename-prompt"),
            Editor::from_text(title),
        )
    }

    pub(in crate::app) fn build_agent_create_overlay(&self) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-agent-list-create-title"),
            ui_text::t(&self.i18n, "overlay-agent-list-create-prompt"),
            Editor::default(),
        )
    }

    pub(in crate::app) fn build_confirm_overlay(
        &self,
        title: String,
        body_lines: Vec<String>,
        action: ConfirmAction,
    ) -> ConfirmOverlay {
        ConfirmDialogState::new(
            title,
            body_lines,
            ui_text::t(&self.i18n, "overlay-confirm-footer"),
            action,
        )
    }

    pub(in crate::app) fn build_timeline_overlay(&self, session_id: i64) -> TimelineOverlay {
        let mut overlay = TimelineOverlay::new(
            self.i18n.text_args(
                "overlay-timeline-title",
                &crate::fl_args!("session" => session_id),
            ),
            ui_text::t(&self.i18n, "overlay-timeline-prompt"),
            ui_text::t(&self.i18n, "overlay-timeline-footer"),
            ui_text::t(&self.i18n, "overlay-timeline-empty"),
            Editor::default(),
            SearchPickerConfig {
                preview_mode: SearchPickerPreviewMode::Responsive {
                    min_total_width: 100,
                    left_min_width: 40,
                    right_min_width: 46,
                },
                ..SearchPickerConfig::searchable()
            },
            None,
            TimelineOverlayMeta { session_id },
        );
        overlay.set_loading(true);
        overlay
    }
}
use crate::app::{
    App, BTreeMap, ChoiceItem, ChoiceOverlay, ChoiceOverlayAction, ChoiceOverlayMeta,
    ChoiceOverlayStyle, ConfirmAction, ConfirmDialogState, ConfirmOverlay, Editor,
    FileAttachOverlay, FileAttachOverlayMeta, LineInputOverlay, Overlay, PathBrowserMode,
    PathBrowserOverlay, PathBrowserOverlayMeta, PathBrowserTarget, PendingInteractiveKind,
    PendingInteractiveOverlayTarget, PendingInteractiveRequest, PermissionOverlay,
    PermissionOverlayPage, PermissionRequest, PickerItem, PickerKind, PickerOverlay,
    PickerOverlayMeta, QuestionFlowState, SearchPickerClearAction, SearchPickerConfig,
    SearchPickerInputMode, SearchPickerPreviewMode, SearchPickerSearchMode, SelectionCursor,
    SessionModelChooserOverlay, SessionModelChooserOverlayMeta, SessionModelChooserPurpose,
    SessionSearchOverlay, SessionSearchOverlayMeta, SessionViewMode, TimelineOverlay,
    TimelineOverlayMeta, UserInputOverlay, UserInputRequest, choice_overlay_clear_detail,
    composer_input_is_active, execution_pending_flash_key,
    first_pending_interactive_request_by_kind, first_unseen_pending_interactive_request,
    mark_current_choice_item, pending_interactive_kind_for_execution, settings_clear_label,
    ui_text, user_input_review_question,
};
