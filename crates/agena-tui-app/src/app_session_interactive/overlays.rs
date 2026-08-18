impl App {
    pub(crate) fn open_user_input_overlay(&mut self) {
        let Some((session_id, request)) = self.pending_user_input_overlay_target() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-user-input-request"));
            return;
        };
        self.seen_user_input_request_ids
            .insert(request.request_id.clone());
        // Persist the presentation: a presented-but-unanswered request is not
        // forced open again after a restart or on another client; it stays
        // reachable through the awaiting-input hint.
        self.present_pending_interactive_request(session_id, request.request_id.clone());
        let request_id = request.request_id.clone();
        self.user_input_interactions.insert(
            request_id.clone(),
            Self::build_user_input_overlay(session_id, request),
        );
        self.sync_interaction_documents();
        // Reveal-and-expand the pending interaction part instead of opening a
        // modal: the expanded part is the interaction surface.
        self.reveal_pending_user_input_interaction(&request_id);
    }

    /// Rebuild the inline interaction views from the live presentations and
    /// stage them on the transcript. The renderer draws these inside expanded
    /// pending interaction parts (the plan body + decision rows come from the
    /// wire request, which the renderer already has in scope; the view carries
    /// only the live selection state). Rebuilding on every presentation
    /// mutation keeps selection, highlight, and custom-feedback state in sync
    /// ("everything is a part").
    pub(crate) fn sync_interaction_documents(&mut self) {
        let width = self.layout.transcript_body.width.max(1);
        let views = self
            .user_input_interactions
            .iter()
            .map(|(request_id, dialog)| {
                (
                    request_id.clone(),
                    Self::interaction_view_for(&dialog.request, &dialog.presentation, width),
                )
            })
            .collect();
        self.transcript.interaction_views = views;
        self.transcript.invalidate_render();
    }

    /// Projects one live presentation into the [`PendingInteractionView`] the
    /// renderer draws. `plan_body_lines` is measured with the EXACT renderer
    /// path at `width`, so the App's key routing and the rendered body can
    /// never drift. Selection is a starting point derived from the
    /// presentation; [`App::refresh_interaction_selection`] overwrites it from
    /// the transcript cursor on every draw (the cursor IS the review cursor).
    pub(crate) fn interaction_view_for(
        request: &UserInputRequest,
        presentation: &agena_tui::user_input::UserInputPresentation,
        width: u16,
    ) -> agena_tui_transcript::PendingInteractionView {
        let plan_body_lines =
            agena_tui_transcript::interaction_plan_body_lines(&request.body_markdown, width);
        if Self::user_input_review_question(request).is_some() {
            let review = presentation.review();
            let custom_index = request
                .questions
                .first()
                .map(|question| question.options.len())
                .unwrap_or(0);
            let selected_option = if review.is_editing_custom() {
                Some(custom_index)
            } else {
                // The presentation's selected_option is the last decision row
                // the old cursor sat on; the transcript cursor corrects it on
                // the next draw via refresh_interaction_selection.
                Some(review.selected_option().min(custom_index))
            };
            agena_tui_transcript::PendingInteractionView {
                selected_option,
                custom_text: review.custom_text(),
                custom_draft: review.custom_input().text().to_owned(),
                editing_custom: review.is_editing_custom(),
                custom_cursor: review.custom_input().cursor(),
                editing_question: None,
                answers: std::collections::BTreeMap::new(),
                plan_body_lines,
                plan_width: width,
            }
        } else {
            let answers = presentation
                .answers()
                .iter()
                .map(|(index, draft)| {
                    (
                        *index,
                        agena_tui_transcript::PendingInteractionAnswerView {
                            picked: draft.option_indexes.iter().copied().collect(),
                            custom_values: draft.custom_values.clone(),
                        },
                    )
                })
                .collect();
            // Ask-user is one continuous body; the whole-line cursor is the
            // transcript cursor. The only presentation state the renderer needs
            // is the per-question answers and which question's custom slot is
            // showing the inline editor.
            let editing_question = if presentation.is_editing_custom() {
                Some(presentation.selected_question())
            } else {
                None
            };
            agena_tui_transcript::PendingInteractionView {
                selected_option: None,
                custom_text: String::new(),
                custom_draft: presentation.custom_input().text().to_owned(),
                editing_custom: presentation.is_editing_custom(),
                custom_cursor: presentation.custom_input().cursor(),
                editing_question,
                answers,
                plan_body_lines,
                plan_width: width,
            }
        }
    }

    /// Move the transcript cursor onto the pending interaction part for
    /// `request_id` and force it expanded, so the expanded part is the
    /// interaction surface ("everything is a part"). The part always
    /// auto-expands on arrival regardless of the configured default, then
    /// falls back to that default once the interaction completes. No-op when
    /// the part has not been rendered yet (for example the execution snapshot
    /// arrived before the transcript was populated); the outstanding retry
    /// covers that case.
    pub(crate) fn reveal_pending_user_input_interaction(&mut self, request_id: &str) {
        let Some(key) = self.pending_interaction_part_node_key(request_id) else {
            return;
        };
        self.revealed_user_input_request_ids
            .insert(request_id.to_string());
        self.transcript.node_expansions.insert(key.clone(), true);
        self.transcript.invalidate_render();
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        // Fit-scroll so the ENTIRE expanded interaction part is visible (the
        // whole ask-user question page with its footer keys, not just the
        // headline) and land the cursor on the part.
        self.transcript.reveal_node_fully(&key, width, height);
    }

    /// Retry the auto-reveal for every outstanding request whose part was not
    /// present when the request arrived (the execution snapshot can land before
    /// the transcript parts populate). Each request is revealed at most once:
    /// the `revealed_user_input_request_ids` guard records a successful reveal
    /// so a later execution refresh cannot re-yank the cursor.
    pub(crate) fn reveal_outstanding_pending_user_input_interactions(&mut self) {
        let outstanding: BTreeSet<String> = self
            .transcript
            .execution
            .as_ref()
            .map(|execution| {
                execution
                    .session
                    .state
                    .pending_interactive_requests()
                    .iter()
                    .filter_map(|request| request.request.as_user_input())
                    .map(|request| request.request_id.clone())
                    .collect()
            })
            .unwrap_or_default();
        for request_id in outstanding {
            if !self.revealed_user_input_request_ids.contains(&request_id) {
                self.reveal_pending_user_input_interaction(&request_id);
            }
        }
    }

    /// The transcript node key of the pending user-input interaction part
    /// matching `request_id`, if the part is present in the transcript parts.
    /// The canonical single-activity shape renders the ask on the `tool_call`
    /// **Operation** activity, so the node key is resolved through the same
    /// shared pending-part predicate the renderer and key router use
    /// ([`agena_tui_transcript::interaction_request_id_for_part`], which reads
    /// the awaiting `user_input` record off the operation). The legacy
    /// `Request` activity arm is covered by that predicate too. The reply-clear
    /// path also resolves through this function: it runs against the still
    /// pending snapshot before the answered execution is applied, so the
    /// awaiting record is still present.
    pub(crate) fn pending_interaction_part_node_key(
        &self,
        request_id: &str,
    ) -> Option<TranscriptNodeKey> {
        agena_tui_transcript::parts_entries(&self.transcript.parts)
            .iter()
            .find_map(|entry| {
                entry.parts.iter().find_map(|part| {
                    (agena_tui_transcript::interaction_request_id_for_part(part)
                        == Some(request_id))
                    .then_some(TranscriptNodeKey::Activity {
                        entry_id: entry.id,
                        content_id: part.id,
                    })
                })
            })
    }

    pub(crate) fn pending_user_input_overlay_target(&self) -> Option<(i64, UserInputRequest)> {
        let execution = self.transcript.execution.as_ref()?;
        let request = first_pending_interactive_request_by_kind(
            execution.session.state.pending_interactive_requests(),
            PendingInteractiveKind::UserInput,
        )?;
        let session_id = request.session_id;
        let request = user_input_request_from_wire(request.request.as_user_input()?.clone());
        Some((session_id, request))
    }

    pub(crate) fn pending_permission_overlay_target(&self) -> Option<(i64, PermissionRequest)> {
        let execution = self.transcript.execution.as_ref()?;
        let request = first_pending_interactive_request_by_kind(
            execution.session.state.pending_interactive_requests(),
            PendingInteractiveKind::Permission,
        )?;
        let session_id = request.session_id;
        let request = permission_request_from_wire(request.request.as_permission()?.clone());
        Some((session_id, request))
    }

    pub(crate) fn build_user_input_overlay(
        session_id: i64,
        request: UserInputRequest,
    ) -> UserInputOverlay {
        let review_decision = Self::user_input_review_question(&request).is_some();
        let presentation = agena_tui::user_input::UserInputPresentation::new(
            agena_tui::user_input::UserInputOverlayPresentation {
                request_id: request.request_id.clone(),
                title: request.title.clone(),
                auto_resolution_ms: request.auto_resolution_ms,
                created_at_ms: request.created_at.timestamp_millis(),
                review_decision,
            },
            request
                .questions
                .iter()
                .map(
                    |question| agena_tui::user_input::UserInputQuestionPresentation {
                        header: question.header.clone(),
                        question: question.question.clone(),
                        options: question
                            .options
                            .iter()
                            .map(
                                |option| agena_tui::user_input::UserInputOptionPresentation {
                                    label: option.label.clone(),
                                    description: option.description.clone(),
                                },
                            )
                            .collect(),
                        multiple: question.multiple,
                        allow_custom: question.allow_custom,
                    },
                )
                .collect(),
        );
        UserInputOverlay {
            session_id,
            request,
            presentation,
        }
    }

    pub(crate) fn user_input_review_question(
        request: &UserInputRequest,
    ) -> Option<&UserInputQuestion> {
        let question = request.questions.first()?;
        if !matches!(request.kind, agena_domain::UserInputKind::Review)
            || request.questions.len() != 1
            || question.multiple
        {
            return None;
        }
        (!question.options.is_empty()).then_some(question)
    }

    pub(crate) fn build_permission_overlay(
        &self,
        session_id: i64,
        request: PermissionRequest,
    ) -> PermissionOverlay {
        PermissionOverlay {
            session_id,
            presentation: PermissionPromptPresentation::new(permission_prompt_content(
                &self.i18n, &request,
            )),
            request,
            auto_approve: None,
        }
    }

    pub(crate) fn next_auto_open_pending_interactive_overlay_target(
        &self,
    ) -> Option<PendingInteractiveOverlayTarget> {
        let execution = self.transcript.execution.as_ref()?;
        let resource = first_auto_open_pending_interactive_request(
            execution.session.state.pending_interactive_requests(),
            &self.seen_permission_request_ids,
            &self.seen_user_input_request_ids,
        )?;
        let session_id = resource.session_id;
        match &resource.request {
            PendingInteractiveRequest::Permission { request } => {
                Some(PendingInteractiveOverlayTarget::Permission {
                    session_id,
                    request: Box::new(permission_request_from_wire(request.clone())),
                })
            }
            PendingInteractiveRequest::UserInput { request } => {
                Some(PendingInteractiveOverlayTarget::UserInput {
                    session_id,
                    request: Box::new(user_input_request_from_wire(request.clone())),
                })
            }
        }
    }

    pub(crate) fn current_session_pending_interactive_kind(
        &self,
    ) -> Option<PendingInteractiveKind> {
        self.transcript
            .execution
            .as_ref()
            .and_then(pending_interactive_kind_for_execution)
    }

    pub(crate) fn pending_interactive_kind_for_session(
        &self,
        session_id: i64,
    ) -> Option<PendingInteractiveKind> {
        (self.transcript.session_id == Some(session_id))
            .then_some(())
            .and(self.current_session_pending_interactive_kind())
    }

    pub(crate) fn open_pending_interactive_overlay_for_kind(
        &mut self,
        kind: PendingInteractiveKind,
    ) {
        match kind {
            PendingInteractiveKind::Permission => self.open_permission_overlay(),
            PendingInteractiveKind::UserInput => self.open_user_input_overlay(),
        }
    }

    pub(crate) fn prompt_for_pending_interactive_on_session(&mut self, session_id: i64) -> bool {
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

    pub(crate) fn has_unseen_pending_interactive_request(&self) -> bool {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return false;
        };
        first_auto_open_pending_interactive_request(
            execution.session.state.pending_interactive_requests(),
            &self.seen_permission_request_ids,
            &self.seen_user_input_request_ids,
        )
        .is_some()
    }

    pub(crate) fn should_suppress_pending_interactive_overlay(&self) -> bool {
        // A pending permission or user-input request is a foreground
        // interaction, not a transcript hint. Keeping a composer, slash
        // picker, or mention picker open must never silently hide it behind
        // Alt+A/Alt+U; the draft remains intact underneath the modal.
        !self.current_route_is_main()
    }

    pub(crate) fn has_suppressed_pending_interactive_overlay(&self) -> bool {
        self.has_unseen_pending_interactive_request()
            && self.should_suppress_pending_interactive_overlay()
    }

    pub(crate) fn maybe_auto_open_pending_interactive_overlay(&mut self) {
        if self.overlay.is_some()
            || !self.current_route_is_main()
            || self.should_suppress_pending_interactive_overlay()
        {
            return;
        }
        match self.next_auto_open_pending_interactive_overlay_target() {
            Some(PendingInteractiveOverlayTarget::Permission {
                session_id,
                request,
            }) => {
                self.seen_permission_request_ids
                    .insert(request.request_id.clone());
                self.overlay = Some(Overlay::Permission(
                    self.build_permission_overlay(session_id, *request),
                ));
                self.queue_permission_notification();
            }
            Some(PendingInteractiveOverlayTarget::UserInput {
                session_id,
                request,
            }) => {
                let request_id = request.request_id.clone();
                self.seen_user_input_request_ids.insert(request_id.clone());
                self.present_pending_interactive_request(session_id, request_id.clone());
                // A pending user-input request lives in the per-request
                // interaction map instead of a modal: the transcript part is
                // the interaction surface, and expanding it renders the live
                // native body (plan + decision rows) built from this view.
                self.user_input_interactions.insert(
                    request_id.clone(),
                    Self::build_user_input_overlay(session_id, *request),
                );
                self.sync_interaction_documents();
                // Auto-expand the part and focus the cursor on it on arrival,
                // regardless of the configured default expand/collapse setting.
                self.reveal_pending_user_input_interaction(&request_id);
                self.queue_user_input_notification();
            }
            None => {}
        }
    }

    /// Fire-and-forget durable presentation acknowledgement for an interactive
    /// user-input request. Best effort: a failed acknowledgement (for example
    /// a race with the request being resolved) is logged and never surfaced,
    /// because the request will simply auto-popup again on the next sync.
    pub(crate) fn present_pending_interactive_request(
        &mut self,
        session_id: i64,
        request_id: String,
    ) {
        let application = self.application.clone();
        tokio::spawn(async move {
            if let Err(error) = crate::app_backend::operations::present_interactive_request(
                &application,
                session_id,
                request_id,
            )
            .await
            {
                tracing::debug!(
                    target: "agena::tui::interactive",
                    %error,
                    "failed to mark interactive request presented"
                );
            }
        });
    }

    /// Queues a terminal attention notification for an incoming permission
    /// request. The notification method is selected per terminal family; BEL
    /// is the universal fallback.
    pub(crate) fn queue_permission_notification(&mut self) {
        use agena_tui_platform::terminal::integration::NotificationMethod;
        if let Some(method) = crate::current_notification_method(self) {
            self.terminal_integration.queue_notification(method);
        } else {
            self.terminal_integration
                .queue_notification(NotificationMethod::Bell);
        }
    }

    /// Queues a terminal attention notification for an incoming user-input
    /// request.
    pub(crate) fn queue_user_input_notification(&mut self) {
        self.queue_permission_notification();
    }

    pub(crate) fn open_permission_overlay(&mut self) {
        let Some((session_id, request)) = self.pending_permission_overlay_target() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-permission-request"));
            return;
        };
        self.seen_permission_request_ids
            .insert(request.request_id.clone());
        self.overlay = Some(Overlay::Permission(
            self.build_permission_overlay(session_id, request),
        ));
    }

    pub(crate) fn session_search_overlay_config() -> SearchPickerConfig {
        SearchPickerConfig {
            search_mode: SearchPickerSearchMode::External,
            ..SearchPickerConfig::searchable()
        }
    }

    pub(crate) fn choice_overlay_footer(
        &self,
        style: agena_tui::choice::ChoicePresentationStyle,
    ) -> String {
        match style {
            agena_tui::choice::ChoicePresentationStyle::Searchable
            | agena_tui::choice::ChoicePresentationStyle::SearchableSelect => {
                ui_text::t(&self.i18n, "overlay-choice-footer")
            }
            agena_tui::choice::ChoicePresentationStyle::SelectOnly => {
                ui_text::t(&self.i18n, "overlay-choice-footer-select")
            }
        }
    }

    pub(crate) fn choice_overlay_clear_action(
        &self,
        action: ChoiceOverlayAction,
    ) -> SearchPickerClearAction {
        SearchPickerClearAction {
            label: settings_clear_label(&self.i18n),
            detail: choice_overlay_clear_detail(&self.i18n, &action),
            current: false,
        }
    }

    pub(crate) fn build_choice_overlay(
        &self,
        title: String,
        prompt: String,
        current_value: Option<String>,
        all_items: Vec<ChoiceItem>,
        action: ChoiceOverlayAction,
        allow_clear: bool,
        style: agena_tui::choice::ChoicePresentationStyle,
    ) -> ChoiceOverlay {
        let clear_action = allow_clear.then(|| {
            let mut clear_action = self.choice_overlay_clear_action(action.clone());
            clear_action.current = current_value.is_none();
            clear_action
        });
        let custom_marker = "__agena_choice_custom_value__";
        let custom_detail = self.i18n.text_args(
            "search-picker-custom-value-detail",
            &agena_tui::fl_args!("value" => custom_marker),
        );
        let (custom_detail_prefix, custom_detail_suffix) = custom_detail
            .split_once(custom_marker)
            .map(|(prefix, suffix)| (prefix.to_owned(), suffix.to_owned()))
            .unwrap_or_else(|| (custom_detail, String::new()));
        let presentation = agena_tui::choice::new_presentation(
            title,
            prompt,
            self.choice_overlay_footer(style),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            all_items,
            current_value,
            clear_action,
            style,
            ui_text::t(&self.i18n, "search-picker-custom-value-label"),
            custom_detail_prefix,
            custom_detail_suffix,
        );
        ChoiceOverlay {
            presentation,
            action,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_selection_picker_overlay(
        &self,
        title: String,
        prompt: String,
        footer: String,
        empty_message: String,
        initial_query: String,
        query: SelectionPickerQuery,
        loading: bool,
    ) -> SelectionPickerOverlay {
        let mut presentation = agena_tui::selection_picker::new_presentation(
            title,
            prompt,
            footer,
            empty_message,
            initial_query,
        );
        presentation.set_loading(loading);
        SelectionPickerOverlay {
            presentation,
            query,
            actions: Default::default(),
        }
    }

    pub(crate) fn build_session_navigation_overlay(
        &self,
        title: String,
        prompt: String,
        footer: String,
        empty_message: String,
        mode: agena_tui_session::session_navigation::SessionNavigationMode,
        query: SessionNavigationQuery,
    ) -> SessionNavigationOverlay {
        let mut presentation = agena_tui_session::session_navigation::new_presentation(
            title,
            prompt,
            footer,
            empty_message,
            mode,
        );
        presentation.set_loading(true);
        SessionNavigationOverlay {
            presentation,
            query,
            actions: Default::default(),
        }
    }

    pub(crate) fn build_path_browser_overlay(
        &self,
        title: String,
        prompt: String,
        footer: String,
        empty_message: String,
        mode: PathBrowserMode,
        initial: String,
        target: PathBrowserTarget,
    ) -> PathBrowserOverlay {
        let workspace_root = self.application.workspace_root();
        let initial_path = App::resolve_browser_input_path_with_root(workspace_root, &initial);
        let current_directory = if self
            .application
            .workspace_path_metadata(initial_path.as_path())
            .is_some_and(|metadata| metadata.is_directory)
        {
            initial_path
        } else {
            initial_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| workspace_root.to_path_buf())
        };
        let input = if matches!(target, PathBrowserTarget::FileAttachment { .. }) {
            path_browser_directory_input(current_directory.as_path())
        } else {
            initial
        };
        let presentation = agena_tui::path_browser::new_presentation(
            title,
            prompt,
            footer,
            empty_message,
            input,
            self.i18n.clone(),
            mode,
        );
        let mut overlay = PathBrowserOverlay {
            presentation,
            target,
            path_actions: Default::default(),
            current_directory,
        };
        Self::refresh_path_browser_overlay(&self.application, &mut overlay);
        overlay
    }

    pub(crate) fn build_session_search_overlay(
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
            SessionSearchPresentation::new(mode, scope_session_id),
        );
        dialog.set_loading(true);
        dialog.footer = self.session_search_footer(&dialog);
        dialog
    }

    pub(crate) fn build_session_model_chooser_overlay(
        &self,
        purpose: SessionModelChooserPurpose,
    ) -> SessionModelChooserOverlay {
        agena_tui::model_chooser::new_presentation(
            ui_text::t(&self.i18n, "overlay-session-model-title"),
            ui_text::t(&self.i18n, "overlay-session-model-prompt"),
            ui_text::t(&self.i18n, "overlay-session-model-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            purpose,
        )
    }

    pub(crate) fn build_line_input_overlay(
        &self,
        title: String,
        prompt: String,
        input: Editor,
    ) -> LineInputOverlay {
        LineInputOverlay::new(title, prompt, input, ())
    }

    pub(crate) fn build_transcript_search_overlay(&self) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-transcript-search-title"),
            ui_text::t(&self.i18n, "overlay-transcript-search-prompt"),
            Editor::from_text(self.transcript.search_query.clone()),
        )
    }

    pub(crate) fn open_transcript_search_overlay(&mut self, forward: bool) {
        self.transcript_search_forward = forward;
        self.overlay = Some(Overlay::TranscriptSearch(
            self.build_transcript_search_overlay(),
        ));
    }

    pub(crate) fn build_model_catalog_search_overlay(&self, query: &str) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-model-catalog-search-title"),
            ui_text::t(&self.i18n, "overlay-model-catalog-search-prompt"),
            Editor::from_text(query.to_string()),
        )
    }

    pub(crate) fn build_session_rename_overlay(&self, title: String) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-rename-title"),
            ui_text::t(&self.i18n, "overlay-rename-prompt"),
            Editor::from_text(title),
        )
    }

    pub(crate) fn build_confirm_overlay(
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

    pub(crate) fn build_timeline_overlay(&self, session_id: i64) -> TimelineOverlay {
        let mut overlay = TimelineOverlay::new(
            self.i18n.text_args(
                "overlay-timeline-title",
                &agena_tui::fl_args!("session" => session_id),
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
            TimelinePresentation::new(session_id),
        );
        overlay.set_loading(true);
        overlay
    }
}

fn user_input_request_from_wire(value: agena_api::resource::UserInputRequest) -> UserInputRequest {
    UserInputRequest {
        request_id: value.request_id,
        session_id: value.session_id,
        title: value.title,
        body_markdown: value.body_markdown,
        kind: value.kind.into(),
        source: value.source,
        auto_resolution_ms: value.auto_resolution_ms,
        presented_at: value.presented_at,
        questions: value
            .questions
            .into_iter()
            .map(|question| UserInputQuestion {
                header: question.header,
                question: question.question,
                options: question
                    .options
                    .into_iter()
                    .map(|option| agena_domain::UserInputOption {
                        label: option.label,
                        description: option.description,
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect(),
        created_at: value.created_at,
    }
}

fn permission_request_from_wire(
    value: agena_api::resource::PermissionRequest,
) -> PermissionRequest {
    PermissionRequest {
        request_id: value.request_id,
        session_id: value.session_id,
        action: permission_action_from_wire(value.action),
        related_actions: value
            .related_actions
            .into_iter()
            .map(permission_action_from_wire)
            .collect(),
        requested_actions: value
            .requested_actions
            .into_iter()
            .map(permission_action_from_wire)
            .collect(),
        reason: value.reason,
        explanation: value.explanation,
        source: value.source,
        scope: value.scope.map(permission_scope_from_wire),
        operator: value.operator,
        trace: value
            .trace
            .into_iter()
            .map(|step| agena_domain::DecisionTraceStep {
                source_kind: match step.source_kind {
                    agena_api::resource::PolicySourceKind::StaticPolicy => {
                        agena_domain::PolicySourceKind::StaticPolicy
                    }
                    agena_api::resource::PolicySourceKind::PersistedRule => {
                        agena_domain::PolicySourceKind::PersistedRule
                    }
                    agena_api::resource::PolicySourceKind::PluginAdvice => {
                        agena_domain::PolicySourceKind::PluginAdvice
                    }
                    agena_api::resource::PolicySourceKind::ManagedPolicy => {
                        agena_domain::PolicySourceKind::ManagedPolicy
                    }
                },
                summary: step.summary,
                source: step.source,
                scope: step.scope.map(permission_scope_from_wire),
                operator: step.operator,
            })
            .collect(),
        created_at: value.created_at,
    }
}

fn permission_action_from_wire(
    value: agena_api::resource::PermissionActionResource,
) -> agena_domain::PermissionAction {
    match value {
        agena_api::resource::PermissionActionResource::Tool {
            tool_name,
            qualifier,
        } => agena_domain::PermissionAction::Tool {
            tool_name,
            qualifier,
        },
        agena_api::resource::PermissionActionResource::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => agena_domain::PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        },
        agena_api::resource::PermissionActionResource::NetworkAccess { target, host, port } => {
            agena_domain::PermissionAction::NetworkAccess { target, host, port }
        }
    }
}

const fn permission_scope_from_wire(
    value: agena_api::resource::PermissionScope,
) -> PermissionScope {
    match value {
        agena_api::resource::PermissionScope::Session => PermissionScope::Session,
        agena_api::resource::PermissionScope::Workspace => PermissionScope::Workspace,
        agena_api::resource::PermissionScope::Global => PermissionScope::Global,
    }
}
use crate::{
    App, BTreeSet, ChoiceItem, ChoiceOverlay, ChoiceOverlayAction, ConfirmAction,
    ConfirmDialogState, ConfirmOverlay, Editor, LineInputOverlay, Overlay, Path, PathBrowserMode,
    PathBrowserOverlay, PathBrowserTarget, PendingInteractiveKind, PendingInteractiveOverlayTarget,
    PendingInteractiveRequest, PermissionOverlay, PermissionPromptPresentation, PermissionRequest,
    PermissionScope, SearchPickerClearAction, SearchPickerConfig, SearchPickerPreviewMode,
    SearchPickerSearchMode, SelectionPickerOverlay, SelectionPickerQuery,
    SessionModelChooserOverlay, SessionModelChooserPurpose, SessionNavigationOverlay,
    SessionNavigationQuery, SessionSearchOverlay, TimelineOverlay, TimelinePresentation,
    TranscriptNodeKey, UserInputOverlay, UserInputQuestion, UserInputRequest,
    choice_overlay_clear_detail, execution_pending_flash_key,
    first_auto_open_pending_interactive_request, first_pending_interactive_request_by_kind,
    path_browser_directory_input, pending_interactive_kind_for_execution,
    permission_prompt_content, settings_clear_label, ui_text,
};
use agena_tui_session::{session_search::SessionSearchPresentation, session_view::SessionViewMode};

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{
        UserInputKind, UserInputOption as DomainOption, UserInputQuestion as DomainQuestion,
    };

    fn review_request(body: &str) -> UserInputRequest {
        agena_domain::UserInputRequest {
            request_id: "host-input:1:2:0".to_owned(),
            session_id: Some(1),
            title: "Approve New Plan".to_owned(),
            body_markdown: body.to_owned(),
            kind: UserInputKind::Review,
            source: agena_domain::UserInputSource::Host,
            auto_resolution_ms: None,
            presented_at: None,
            questions: vec![DomainQuestion {
                header: "Decision".to_owned(),
                question: "Choose whether this plan should move to active.".to_owned(),
                options: vec![
                    DomainOption {
                        label: "Approve".to_owned(),
                        description: String::new(),
                    },
                    DomainOption {
                        label: "Request changes".to_owned(),
                        description: String::new(),
                    },
                ],
                multiple: false,
                allow_custom: true,
            }],
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn review_overlay_projects_selection_state_into_the_interaction_view() {
        let request = review_request("## Proposed Plan\n\n1. Step one\n2. Step two");
        let overlay = App::build_user_input_overlay(1, request.clone());
        assert!(
            overlay.presentation.is_review_decision(),
            "a single-option review request is a review decision"
        );
        // The plan body is NOT pre-rendered into the presentation anymore —
        // the renderer draws it natively — but the view still measures its
        // row count with the exact renderer path at the given width.
        let view = App::interaction_view_for(&request, &overlay.presentation, 80);
        assert!(
            view.plan_body_lines > 1,
            "plan body must measure more than one row, got {}",
            view.plan_body_lines
        );
        assert_eq!(view.plan_width, 80);
        assert!(!view.editing_custom);
        assert!(view.custom_text.is_empty());
        assert!(view.custom_draft.is_empty());
    }

    #[test]
    fn non_review_overlay_projects_an_empty_ask_user_view() {
        let mut request = review_request("");
        request.kind = UserInputKind::AskUser;
        request.body_markdown.clear();
        let overlay = App::build_user_input_overlay(1, request.clone());
        assert!(!overlay.presentation.is_review_decision());
        let view = App::interaction_view_for(&request, &overlay.presentation, 80);
        assert!(view.answers.is_empty());
        assert_eq!(view.selected_option, None);
        assert_eq!(view.editing_question, None, "no inline editor is open");
        assert!(!view.editing_custom);
        assert!(view.custom_text.is_empty());
        assert!(view.custom_draft.is_empty());
        assert_eq!(view.plan_width, 80);
    }
}
