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
        self.overlay = Some(Overlay::UserInputReply(Self::build_user_input_overlay(
            session_id, request,
        )));
    }

    pub(crate) fn pending_user_input_overlay_target(&self) -> Option<(i64, UserInputRequest)> {
        let execution = self.transcript.execution.as_ref()?;
        let request = first_pending_interactive_request_by_kind(
            execution.pending_interactive_requests.as_slice(),
            PendingInteractiveKind::UserInput,
        )?;
        let session_id = request.session_id;
        let request = user_input_request_from_wire(request.request.as_user_input()?.clone());
        Some((session_id, request))
    }

    pub(crate) fn pending_permission_overlay_target(&self) -> Option<(i64, PermissionRequest)> {
        let execution = self.transcript.execution.as_ref()?;
        let request = first_pending_interactive_request_by_kind(
            execution.pending_interactive_requests.as_slice(),
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

    pub(crate) fn user_input_overlay_is_review(dialog: &UserInputOverlay) -> bool {
        dialog.presentation.is_review_decision()
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
            execution.pending_interactive_requests.as_slice(),
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
            execution.pending_interactive_requests.as_slice(),
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
                self.seen_user_input_request_ids
                    .insert(request.request_id.clone());
                self.present_pending_interactive_request(session_id, request.request_id.clone());
                self.overlay = Some(Overlay::UserInputReply(Self::build_user_input_overlay(
                    session_id, *request,
                )));
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
        let current_directory = if initial_path.is_dir() {
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
        Self::refresh_path_browser_overlay_with_root(workspace_root, &mut overlay);
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
    App, ChoiceItem, ChoiceOverlay, ChoiceOverlayAction, ConfirmAction, ConfirmDialogState,
    ConfirmOverlay, Editor, LineInputOverlay, Overlay, Path, PathBrowserMode, PathBrowserOverlay,
    PathBrowserTarget, PendingInteractiveKind, PendingInteractiveOverlayTarget,
    PendingInteractiveRequest, PermissionOverlay, PermissionPromptPresentation, PermissionRequest,
    PermissionScope, SearchPickerClearAction, SearchPickerConfig, SearchPickerPreviewMode,
    SearchPickerSearchMode, SelectionPickerOverlay, SelectionPickerQuery,
    SessionModelChooserOverlay, SessionModelChooserPurpose, SessionNavigationOverlay,
    SessionNavigationQuery, SessionSearchOverlay, TimelineOverlay, TimelinePresentation,
    UserInputOverlay, UserInputQuestion, UserInputRequest, choice_overlay_clear_detail,
    execution_pending_flash_key, first_auto_open_pending_interactive_request,
    first_pending_interactive_request_by_kind, path_browser_directory_input,
    pending_interactive_kind_for_execution, permission_prompt_content, settings_clear_label,
    ui_text,
};
use agena_tui_session::{session_search::SessionSearchPresentation, session_view::SessionViewMode};
