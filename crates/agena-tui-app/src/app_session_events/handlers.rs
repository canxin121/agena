impl App {
    pub(crate) fn handle_permission_replied(
        &mut self,
        session_id: i64,
        request_id: String,
        kind: PermissionReplyKind,
        label: String,
        result: UiResult<SessionExecutionResource>,
    ) {
        self.finish_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::PermissionReply,
        );
        match result {
            Ok(execution) => {
                // An auto-approve success resolves the request like a manual
                // allow/deny: close the open permission popup for this request
                // before the execution refresh (which would re-derive pending
                // state from a request that is no longer pending).
                if kind == PermissionReplyKind::AutoApprove
                    && matches!(
                        &self.overlay,
                        Some(Overlay::Permission(dialog))
                            if dialog.request.request_id == request_id
                    )
                {
                    self.overlay = None;
                }
                let transcript_is_target = self.transcript.session_id == Some(session_id);
                let transcript_contains_target =
                    self.transcript.execution.as_ref().is_some_and(|execution| {
                        execution
                            .pending_interactive_requests
                            .iter()
                            .any(|request| request.session_id == session_id)
                    });
                if transcript_is_target && self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
                if transcript_is_target {
                    self.request_refresh(session_id, true);
                } else if transcript_contains_target
                    && let Some(parent_session_id) = self.transcript.session_id
                {
                    self.request_refresh(parent_session_id, true);
                }
                self.request_sessions(false);
                self.flash_success(self.i18n.text_args(
                    "flash-permission-reply-sent",
                    &agena_tui::fl_args!("label" => label),
                ));
            }
            Err(error) => {
                if kind == PermissionReplyKind::AutoApprove {
                    // The classifier failed before the reply was recorded, so
                    // the pending request still exists. Keep the open popup and
                    // surface the failure on the auto-approve choice instead of
                    // closing and re-opening it.
                    if let Some(Overlay::Permission(dialog)) = self.overlay.as_mut()
                        && dialog.request.request_id == request_id
                    {
                        dialog.auto_approve =
                            Some(PermissionPromptAutoApproveStatus::Failed(error.to_string()));
                    } else {
                        self.flash_error(error);
                    }
                    return;
                }
                // The modal closes when a decision is submitted, but the
                // request is consumed only after backend acknowledgement. On
                // rejection make the same durable request unseen again and
                // immediately restore it instead of stranding it behind
                // Alt+A.
                self.seen_permission_request_ids.remove(&request_id);
                self.flash_error(error);
                if self.transcript.session_id == Some(session_id) {
                    self.maybe_auto_open_pending_interactive_overlay();
                    self.request_refresh(session_id, true);
                }
            }
        }
    }

    pub(crate) fn handle_user_input_replied(
        &mut self,
        session_id: i64,
        request_id: String,
        result: UiResult<SessionExecutionResource>,
    ) {
        self.finish_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::UserInputReply,
        );
        match result {
            Ok(execution) => {
                let transcript_contains_target =
                    self.transcript.execution.as_ref().is_some_and(|execution| {
                        execution
                            .pending_interactive_requests
                            .iter()
                            .any(|request| request.session_id == session_id)
                    });
                self.handle_session_execution_updated(session_id, execution, true);
                if transcript_contains_target
                    && self.transcript.session_id != Some(session_id)
                    && let Some(parent_session_id) = self.transcript.session_id
                {
                    self.request_refresh(parent_session_id, true);
                }
                self.flash_success(ui_text::t(&self.i18n, "flash-user-input-reply-sent"));
            }
            Err(error) => {
                self.seen_user_input_request_ids.remove(&request_id);
                self.flash_error(error);
                if self.transcript.session_id == Some(session_id) {
                    self.maybe_auto_open_pending_interactive_overlay();
                    self.request_refresh(session_id, true);
                }
            }
        }
    }

    pub(crate) fn handle_providers_loaded(
        &mut self,
        purpose: ProviderPickerPurpose,
        result: UiResult<Vec<ProviderSummaryResource>>,
    ) {
        let Some(mut dialog) = self.take_selection_picker_route() else {
            return;
        };
        let SelectionPickerQuery::Providers(current_purpose) = dialog.query;
        if current_purpose != purpose {
            self.restore_selection_picker_route(dialog);
            return;
        }

        dialog.presentation.set_loading(false);
        dialog.presentation.empty_message = ui_text::t(&self.i18n, "overlay-picker-empty");
        match result {
            Ok(providers) => {
                let fallback_adapter = settings_choice_adapter_fallback(&self.i18n);
                let mut rows = Vec::new();
                if purpose == ProviderPickerPurpose::Configure {
                    rows.push(provider_list_create_item(&self.i18n));
                }
                rows.extend(providers.into_iter().map(|provider| {
                    let detail = if purpose == ProviderPickerPurpose::Configure {
                        i18n_provider_list_detail(&self.i18n, &provider)
                    } else {
                        settings_choice_default_provider_detail(
                            &self.i18n,
                            provider
                                .defaults
                                .adapter
                                .as_deref()
                                .unwrap_or(fallback_adapter.as_str()),
                            provider.defaults.model.as_str(),
                        )
                    };
                    let label = provider.provider_id.clone();
                    (
                        agena_tui::selection_picker::SelectionPickerItem::new(
                            format!("provider:{}", provider.provider_id),
                            label.clone(),
                            detail.clone(),
                            format!("{label} {detail}"),
                        ),
                        SelectionPickerCommand::Provider {
                            provider_id: provider.provider_id,
                        },
                    )
                }));
                dialog.actions = rows
                    .iter()
                    .map(|(item, action)| (item.key.clone(), action.clone()))
                    .collect();
                dialog
                    .presentation
                    .replace_items(rows.into_iter().map(|(item, _)| item).collect());
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_selection_picker_route(dialog);
    }

    pub(crate) fn handle_session_search_page_loaded(
        &mut self,
        mode: SessionViewMode,
        query: String,
        page_index: usize,
        result: UiResult<PaginatedResponse<SessionResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_session_search_dialog() else {
            return;
        };
        if dialog.meta.mode != mode
            || dialog.meta.page_index != page_index
            || dialog.input.text().trim() != query
        {
            self.restore_session_search_dialog(host, dialog);
            return;
        }

        dialog.set_loading(false);
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-resume-empty");
        match result {
            Ok(page) => {
                let items = page
                    .items
                    .into_iter()
                    .map(|session| self.session_search_item(session))
                    .collect::<Vec<_>>();
                if page_index == 0 {
                    dialog.replace_items(items);
                } else {
                    dialog.append_items(items);
                }
                dialog
                    .meta
                    .apply_page(page.page.next_cursor, page.page.has_more);
                dialog.footer = self.session_search_footer(&dialog);
            }
            Err(error) => {
                dialog.meta.reject_page(page_index);
                self.flash_error(error);
            }
        }
        self.restore_session_search_dialog(host, dialog);
    }

    pub(crate) fn handle_session_search_subtree_loaded(
        &mut self,
        session_id: i64,
        query: String,
        result: UiResult<Vec<SessionResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_session_search_dialog() else {
            return;
        };
        if dialog.meta.mode != SessionViewMode::Subtree
            || dialog.meta.scope_session_id != Some(session_id)
            || dialog.input.text().trim() != query
        {
            self.restore_session_search_dialog(host, dialog);
            return;
        }

        dialog.set_loading(false);
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-resume-empty");
        match result {
            Ok(mut sessions) => {
                sessions.sort_by(|left, right| {
                    right
                        .updated_at
                        .cmp(&left.updated_at)
                        .then_with(|| right.id.cmp(&left.id))
                });
                dialog.meta.all_items = sessions
                    .into_iter()
                    .map(|session| self.session_search_item(session))
                    .collect();
                self.refresh_session_search_overlay_local(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_session_search_dialog(host, dialog);
    }

    pub(crate) fn handle_lineage_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    ) {
        match result {
            Ok(sessions) => {
                let sessions_by_id = sessions
                    .iter()
                    .cloned()
                    .map(|session| (session.id, session))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let items = agena_tui_session::session_navigation::build_lineage_items(
                    &sessions
                        .iter()
                        .map(
                            |session| agena_tui_session::session_navigation::SessionLineageNode {
                                session_id: session.id,
                                parent_session_id: session.parent_id,
                                updated_at_ms: session.updated_at.timestamp_millis(),
                            },
                        )
                        .collect::<Vec<_>>(),
                    session_id,
                );
                if self.transcript.session_id == Some(session_id)
                    && let Some(summary) =
                        agena_tui_session::session_navigation::summarize_lineage_items(
                            items.as_slice(),
                        )
                {
                    self.current_lineage = Some(CurrentLineageState {
                        session_id,
                        summary,
                    });
                }

                let Some(mut dialog) = self.take_session_navigation_route() else {
                    return;
                };
                let SessionNavigationQuery::Lineage {
                    session_id: current_session_id,
                } = dialog.query
                else {
                    self.restore_session_navigation_route(dialog);
                    return;
                };
                if current_session_id != session_id {
                    self.restore_session_navigation_route(dialog);
                    return;
                }

                dialog.presentation.set_loading(false);
                dialog.presentation.empty_message = ui_text::t(&self.i18n, "overlay-lineage-empty");
                let rows = items
                    .into_iter()
                    .filter_map(|item| {
                        sessions_by_id
                            .get(&item.session_id)
                            .cloned()
                            .map(|session| self.lineage_session_navigation_item(session, item))
                    })
                    .collect::<Vec<_>>();
                dialog.actions = rows
                    .iter()
                    .map(|(item, action)| (item.key.clone(), action.clone()))
                    .collect();
                dialog
                    .presentation
                    .replace_items(rows.into_iter().map(|(item, _)| item).collect());
                self.restore_session_navigation_route(dialog);
            }
            Err(error) => {
                if let Some(mut dialog) = self.take_session_navigation_route() {
                    if matches!(dialog.query, SessionNavigationQuery::Lineage { session_id: current_session_id } if current_session_id == session_id)
                    {
                        dialog.presentation.set_loading(false);
                        dialog.presentation.empty_message =
                            ui_text::t(&self.i18n, "overlay-lineage-empty");
                    }
                    self.restore_session_navigation_route(dialog);
                }
                self.flash_error(error);
            }
        }
    }

    pub(crate) fn handle_rewind_messages_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<Vec<crate::RewindTarget>>,
    ) {
        let Some(mut dialog) = self.take_session_navigation_route() else {
            return;
        };
        let SessionNavigationQuery::RewindMessages {
            session_id: current_session_id,
        } = dialog.query
        else {
            self.restore_session_navigation_route(dialog);
            return;
        };
        if current_session_id != session_id {
            self.restore_session_navigation_route(dialog);
            return;
        }

        dialog.presentation.set_loading(false);
        dialog.presentation.empty_message = ui_text::t(&self.i18n, "overlay-rewind-empty");
        match result {
            Ok(targets) => {
                let rows = targets
                    .into_iter()
                    .rev()
                    .map(|target| self.rewind_turn_navigation_item(session_id, target))
                    .collect::<Vec<_>>();
                dialog.actions = rows
                    .iter()
                    .map(|(item, action)| (item.key.clone(), action.clone()))
                    .collect();
                dialog
                    .presentation
                    .replace_items(rows.into_iter().map(|(item, _)| item).collect());
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_session_navigation_route(dialog);
    }

    pub(crate) fn handle_model_catalog_loaded(
        &mut self,
        query: String,
        offset: usize,
        result: UiResult<ModelCatalogListResponse>,
    ) {
        let Some((host, mut dialog)) = self.take_model_catalog_dialog() else {
            return;
        };
        if dialog.presentation.query() != query.as_str() || dialog.presentation.offset() != offset {
            self.restore_model_catalog_dialog(host, dialog);
            return;
        }

        match result {
            Ok(response) => {
                dialog.summary = response.summary;
                dialog.presentation.apply_page(
                    response
                        .items
                        .into_iter()
                        .map(|entry| model_catalog_presentation_item(&self.i18n, entry))
                        .collect(),
                    response.total,
                    response.offset,
                    response.limit,
                );
            }
            Err(error) => {
                dialog.presentation.reject_page();
                self.flash_error(error)
            }
        }
        self.restore_model_catalog_dialog(host, dialog);
    }

    pub(crate) fn handle_provider_studio_adapter_models_loaded(
        &mut self,
        request_key: String,
        result: UiResult<ProviderAdapterModelsResponse>,
    ) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            return;
        };
        if dialog.pending_adapter_models_key.as_deref() != Some(request_key.as_str()) {
            self.restore_provider_studio_dialog(host, dialog);
            return;
        }

        dialog.listing_adapter_models = false;
        dialog.pending_adapter_models_key = None;
        match result {
            Ok(response) => {
                let preserved_model_keys = dialog.selected_model_keys.clone();
                let previously_available =
                    provider_studio_available_model_keys(&dialog.adapter_models);
                dialog.adapter_models = provider_studio_merge_refreshed_adapter_models(
                    dialog.adapter_models,
                    response.adapters,
                    &dialog.selected_adapter_ids,
                );
                dialog
                    .selection
                    .clamp_left(dialog.adapter_candidate_ids.len());
                dialog.selection.set_right_selected(0);
                self.reload_provider_studio_catalog_matches(&mut dialog);
                dialog.selected_model_keys = preserved_model_keys;
                dialog
                    .selected_model_keys
                    .extend(provider_studio_new_selected_model_keys(
                        &dialog.adapter_models,
                        &dialog.selected_adapter_ids,
                        &previously_available,
                    ));
                provider_studio_restore_model_selection(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_provider_studio_dialog(host, dialog);
    }

    pub(crate) fn handle_provider_studio_auth_completed(
        &mut self,
        request_key: String,
        result: std::result::Result<
            agena_application::provider_studio::ProviderDraftAuthActionResult,
            agena_application::provider_studio::ProviderDraftAuthError,
        >,
    ) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            match result {
                Ok(action) => {
                    if !provider_draft_auth_message_is_pending(&action.message) {
                        self.flash_success(provider_draft_auth_action_message(
                            &self.i18n,
                            &action.message,
                        ));
                    }
                }
                Err(error) => {
                    self.flash_error(provider_draft_auth_error_message(&self.i18n, &error))
                }
            }
            return;
        };
        if dialog.pending_auth_key.as_deref() != Some(request_key.as_str()) {
            self.restore_provider_studio_dialog(host, dialog);
            return;
        }

        dialog.pending_auth_key = None;
        match result {
            Ok(action) => {
                dialog.draft = action.draft;
                self.sync_provider_studio_shape(&mut dialog);
                self.sync_provider_studio_auth_poll_deadline(&mut dialog, Instant::now(), true);
                let preferred_detail_field = provider_studio_preferred_detail_field_index(&dialog);
                if let Some(detail_page) = dialog.detail_page.as_mut() {
                    detail_page.selection.selected = preferred_detail_field;
                }
                if let Some(text) = action.clipboard_text {
                    self.request_clipboard_copy(
                        text,
                        "Copied provider authorization value.".to_string(),
                    );
                }
                if !provider_draft_auth_message_is_pending(&action.message) {
                    self.flash_success(provider_draft_auth_action_message(
                        &self.i18n,
                        &action.message,
                    ));
                }
            }
            Err(error) => {
                self.sync_provider_studio_auth_poll_deadline(&mut dialog, Instant::now(), true);
                self.flash_error(provider_draft_auth_error_message(&self.i18n, &error));
            }
        }
        self.restore_provider_studio_dialog(host, dialog);
    }

    pub(crate) fn handle_provider_studio_saved(
        &mut self,
        provider_id: String,
        result: std::result::Result<
            agena_application::provider_studio::ProviderStudioSaveResult,
            agena_application::provider_studio::ProviderStudioSaveError,
        >,
    ) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            match result {
                Ok(message) => {
                    self.flash_success(provider_studio_save_result_message(&self.i18n, &message))
                }
                Err(error) => {
                    self.flash_error(provider_studio_save_error_message(&self.i18n, &error))
                }
            }
            return;
        };
        dialog.saving = false;
        match result {
            Ok(message) => {
                let preserved_selected_adapter_ids = dialog.selected_adapter_ids.clone();
                let preserved_selected_adapter_id = provider_studio_selected_adapter_id(&dialog);
                let mut preserved_selected_model_keys = dialog.selected_model_keys.clone();
                let mut preserved_selected_adapter_ids = preserved_selected_adapter_ids;
                let mut preserved_selected_adapter_id = preserved_selected_adapter_id;
                match &message {
                    agena_application::provider_studio::ProviderStudioSaveResult::ModelDeleted {
                        adapter_id,
                        model_id,
                        ..
                    } => {
                        preserved_selected_model_keys
                            .remove(provider_studio_model_key(adapter_id, model_id).as_str());
                    }
                    agena_application::provider_studio::ProviderStudioSaveResult::AdapterDeleted {
                        adapter_id,
                        ..
                    } => {
                        preserved_selected_adapter_ids.remove(adapter_id.as_str());
                        if preserved_selected_adapter_id.as_deref() == Some(adapter_id.as_str()) {
                            preserved_selected_adapter_id = None;
                        }
                        let prefix = format!("{adapter_id}\u{1f}");
                        preserved_selected_model_keys
                            .retain(|key| !key.starts_with(prefix.as_str()));
                    }
                    agena_application::provider_studio::ProviderStudioSaveResult::ProviderDeleted { .. } => {}
                    agena_application::provider_studio::ProviderStudioSaveResult::ProviderDraftSaved { .. }
                    | agena_application::provider_studio::ProviderStudioSaveResult::AdapterMatchesSaved { .. }
                    | agena_application::provider_studio::ProviderStudioSaveResult::ConfiguredModelSaved {
                        ..
                    } => {}
                }
                self.flash_success(provider_studio_save_result_message(&self.i18n, &message));
                if matches!(
                    &message,
                    agena_application::provider_studio::ProviderStudioSaveResult::ProviderDeleted { .. }
                ) {
                    self.restore_provider_list_after_provider_delete();
                    return;
                }
                let providers = crate::app_backend::operations::list_configured_providers(
                    &self.application,
                );
                let provider_rows = provider_studio_provider_rows(&self.i18n, providers.as_slice());
                let selected_provider = provider_rows
                    .iter()
                    .position(|row| row.provider_id.as_deref() == Some(provider_id.as_str()))
                    .unwrap_or(0);
                dialog.providers = SelectableListState::new(provider_rows, selected_provider);
                self.load_provider_studio_draft(&mut dialog, Some(provider_id.as_str()), None);
                restore_provider_studio_adapter_selection(
                    &mut dialog,
                    &preserved_selected_adapter_ids,
                    preserved_selected_adapter_id.as_deref(),
                );
                dialog.selected_model_keys = preserved_selected_model_keys;
                match &message {
                    agena_application::provider_studio::ProviderStudioSaveResult::AdapterDeleted { .. } => {
                        dialog.selection.set_focus(ProviderStudioFocus::Adapters);
                    }
                    agena_application::provider_studio::ProviderStudioSaveResult::ModelDeleted { .. } => {
                        dialog.selection.set_focus(ProviderStudioFocus::Models);
                    }
                    _ => {}
                }
            }
            Err(error) => self.flash_error(provider_studio_save_error_message(&self.i18n, &error)),
        }
        self.restore_provider_studio_dialog(host, dialog);
    }

    pub(crate) fn restore_provider_list_after_provider_delete(&mut self) {
        let provider_picker = self.route_stack.last().and_then(|route| match route {
            Route::SelectionPicker(dialog)
                if dialog.query
                    == SelectionPickerQuery::Providers(ProviderPickerPurpose::Configure) =>
            {
                Some(dialog.presentation.input.text().to_string())
            }
            _ => None,
        });
        if provider_picker.is_some() {
            let _ = self.route_stack.pop();
        }
        self.current_route = Route::SelectionPicker(
            self.build_provider_list_overlay(provider_picker.as_deref().unwrap_or(""), false),
        );
        self.overlay = None;
    }

    pub(crate) fn handle_model_catalog_refreshed(&mut self, result: UiResult<()>) {
        let Some((host, mut dialog)) = self.take_model_catalog_dialog() else {
            match result {
                Ok(()) => self.flash_success(ui_text::t(
                    &self.i18n,
                    "flash-provider-studio-catalog-refreshed",
                )),
                Err(error) => self.flash_error(error),
            }
            return;
        };

        match result {
            Ok(()) => {
                self.flash_success(ui_text::t(
                    &self.i18n,
                    "flash-provider-studio-catalog-refreshed",
                ));
                if let agena_tui::model_catalog::ModelCatalogEffect::LoadPage { query, offset } =
                    dialog.presentation.request_first_page_after_refresh()
                {
                    self.request_model_catalog_page(query, offset);
                }
            }
            Err(error) => {
                dialog.presentation.reject_page();
                self.flash_error(error);
            }
        }
        self.restore_model_catalog_dialog(host, dialog);
    }

    pub(crate) fn handle_child_sessions_loaded(
        &mut self,
        parent_session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    ) {
        let Some(mut dialog) = self.take_session_navigation_route() else {
            return;
        };
        let SessionNavigationQuery::ChildSessions {
            parent_session_id: current_parent_id,
        } = dialog.query
        else {
            self.restore_session_navigation_route(dialog);
            return;
        };
        if current_parent_id != parent_session_id {
            self.restore_session_navigation_route(dialog);
            return;
        }

        dialog.presentation.set_loading(false);
        dialog.presentation.empty_message = ui_text::t(&self.i18n, "overlay-children-empty");
        match result {
            Ok(sessions) => {
                let rows = sessions
                    .into_iter()
                    .map(|session| {
                        let label = session.title;
                        let detail = format!(
                            "#{} | {} msg | {} child",
                            session.id, session.message_count, session.child_session_count
                        );
                        (
                            agena_tui_session::session_navigation::SessionNavigationItem::new(
                                format!("session:{}", session.id),
                                label.clone(),
                                detail.clone(),
                                format!("{label} {detail} #{}", session.id),
                            ),
                            SessionNavigationCommand::OpenSession {
                                session_id: session.id,
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                dialog.actions = rows
                    .iter()
                    .map(|(item, action)| (item.key.clone(), action.clone()))
                    .collect();
                dialog
                    .presentation
                    .replace_items(rows.into_iter().map(|(item, _)| item).collect());
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_session_navigation_route(dialog);
    }

    pub(crate) fn handle_timeline_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<Vec<crate::app_backend::SessionTimelineEntry>>,
    ) {
        let Some((host, mut dialog)) = self.take_timeline_dialog() else {
            return;
        };
        if dialog.meta.session_id != session_id {
            self.restore_timeline_dialog(host, dialog);
            return;
        }

        dialog.set_loading(false);
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-timeline-empty");
        match result {
            Ok(events) => {
                let items = events
                    .iter()
                    .map(|event| build_timeline_item(&self.i18n, event))
                    .collect();
                dialog.replace_items(items);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_timeline_dialog(host, dialog);
    }

    pub(crate) fn handle_session_rewound(
        &mut self,
        session_id: i64,
        message_text: String,
        target: String,
        result: UiResult<SessionExecutionResource>,
    ) {
        self.finish_run_operation(RunActivityTarget::Session(session_id), RunOperation::Rewind);
        match result {
            Ok(execution) => {
                let rewound_session_id = execution.session.id;
                let rewound_message_draft = ComposerDraft {
                    document: agena_domain::ComposerDocument(vec![
                        agena_domain::ComposerNode::Text { text: message_text },
                    ]),
                };
                self.set_draft_for_slot(
                    DraftSlot::Session(rewound_session_id),
                    rewound_message_draft.clone(),
                );
                self.open_session(rewound_session_id, execution.session.title.clone());
                self.replace_composer_draft(rewound_message_draft);
                self.sync_current_draft_slot();
                self.persist_draft_store_with_feedback(true);
                if self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(rewound_session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
                self.focus = Focus::Composer;
                self.request_sessions(false);
                self.flash_success(self.i18n.text_args(
                    "flash-session-rewound",
                    &agena_tui::fl_args!("target" => target),
                ));
            }
            Err(error) => {
                self.flash_error(error);
            }
        }
    }
}
use crate::view::model_catalog_presentation_item;
use crate::{
    App, ComposerDraft, CurrentLineageState, DraftSlot, Instant, ModelCatalogListResponse, Overlay,
    PaginatedResponse, PermissionReplyKind, ProviderAdapterModelsResponse, ProviderPickerPurpose,
    ProviderStudioFocus, ProviderSummaryResource, Route, RunActivityTarget, RunOperation,
    SelectableListState, SelectionPickerCommand, SelectionPickerQuery, SessionExecutionResource,
    SessionNavigationCommand, SessionNavigationQuery, SessionResource, UiResult,
    build_timeline_item, i18n_provider_list_detail, provider_draft_auth_action_message,
    provider_draft_auth_error_message, provider_draft_auth_message_is_pending,
    provider_list_create_item, provider_studio_available_model_keys,
    provider_studio_merge_refreshed_adapter_models, provider_studio_model_key,
    provider_studio_new_selected_model_keys, provider_studio_preferred_detail_field_index,
    provider_studio_provider_rows, provider_studio_restore_model_selection,
    provider_studio_save_error_message, provider_studio_save_result_message,
    provider_studio_selected_adapter_id, restore_provider_studio_adapter_selection,
    settings_choice_adapter_fallback, settings_choice_default_provider_detail, ui_text,
};
use agena_tui::main_focus::Focus;
use agena_tui::permission_prompt::PermissionPromptAutoApproveStatus;
use agena_tui_session::session_view::SessionViewMode;
