impl App {
    pub(in crate::app) fn handle_permission_replied(
        &mut self,
        session_id: i64,
        label: String,
        result: UiResult<SessionExecutionResource>,
    ) {
        self.finish_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::PermissionReply,
        );
        match result {
            Ok(execution) => {
                let transcript_is_target = self.transcript.session_id == Some(session_id);
                if transcript_is_target && self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
                if transcript_is_target {
                    self.request_refresh(session_id, true);
                }
                self.request_sessions(false);
                self.flash_success(self.i18n.text_args(
                    "flash-permission-reply-sent",
                    &crate::fl_args!("label" => label),
                ));
            }
            Err(error) => {
                self.pending_permission_replay = None;
                self.flash_error(error);
            }
        }
    }

    pub(in crate::app) fn handle_user_input_replied(
        &mut self,
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    ) {
        self.finish_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::UserInputReply,
        );
        match result {
            Ok(execution) => {
                self.handle_session_execution_updated(session_id, execution, true);
                self.flash_success(ui_text::t(&self.i18n, "flash-user-input-reply-sent"));
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(in crate::app) fn handle_providers_loaded(
        &mut self,
        purpose: ProviderPickerPurpose,
        result: UiResult<Vec<ProviderSummaryResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_picker_dialog() else {
            return;
        };
        let PickerKind::Providers(current_purpose) = &dialog.meta.kind else {
            self.restore_picker_dialog(host, dialog);
            return;
        };
        if *current_purpose != purpose {
            self.restore_picker_dialog(host, dialog);
            return;
        }

        dialog.set_loading(false);
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-picker-empty");
        match result {
            Ok(providers) => {
                let fallback_adapter = settings_choice_adapter_fallback(&self.i18n);
                let mut items = Vec::new();
                if purpose == ProviderPickerPurpose::Configure {
                    items.push(provider_list_create_item(&self.i18n));
                }
                items.extend(providers.into_iter().map(|provider| {
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
                    PickerItem {
                        label: provider.provider_id.clone(),
                        detail,
                        value: PickerValue::Provider(provider),
                    }
                }));
                dialog.replace_items(items);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_picker_dialog(host, dialog);
    }

    pub(in crate::app) fn handle_agents_loaded(&mut self, result: UiResult<Vec<AgentDescriptor>>) {
        let Some((host, mut dialog)) = self.take_picker_dialog() else {
            return;
        };
        if !matches!(dialog.meta.kind, PickerKind::Agents) {
            self.restore_picker_dialog(host, dialog);
            return;
        }

        dialog.set_loading(false);
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-picker-empty");
        match result {
            Ok(agents) => {
                let items = agent_list_items(
                    &self.i18n,
                    agents,
                    self.backend.default_agent_name().as_deref(),
                    &self.backend.config_agent_names(),
                );
                dialog.replace_items(items);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_picker_dialog(host, dialog);
    }

    pub(in crate::app) fn handle_session_search_page_loaded(
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
                dialog.meta.next_cursor = page.page.next_cursor;
                dialog.meta.has_more = page.page.has_more;
                dialog.footer = self.session_search_footer(&dialog);
            }
            Err(error) => {
                if page_index > 0 {
                    dialog.meta.page_index = page_index.saturating_sub(1);
                }
                self.flash_error(error);
            }
        }
        self.restore_session_search_dialog(host, dialog);
    }

    pub(in crate::app) fn handle_session_search_subtree_loaded(
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

    pub(in crate::app) fn handle_lineage_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    ) {
        match result {
            Ok(sessions) => {
                let items = build_lineage_session_items(sessions.as_slice(), session_id);
                if self.transcript.session_id == Some(session_id)
                    && let Some(summary) = summarize_lineage_session_items(items.as_slice())
                {
                    self.current_lineage = Some(CurrentLineageState {
                        session_id,
                        summary,
                    });
                }

                let Some((host, mut dialog)) = self.take_picker_dialog() else {
                    return;
                };
                let PickerKind::Lineage {
                    session_id: current_session_id,
                } = &dialog.meta.kind
                else {
                    self.restore_picker_dialog(host, dialog);
                    return;
                };
                if *current_session_id != session_id {
                    self.restore_picker_dialog(host, dialog);
                    return;
                }

                dialog.set_loading(false);
                dialog.empty_message = ui_text::t(&self.i18n, "overlay-lineage-empty");
                let picker_items = items
                    .into_iter()
                    .map(|item| self.lineage_session_picker_item(item))
                    .collect();
                dialog.replace_items(picker_items);
                self.restore_picker_dialog(host, dialog);
            }
            Err(error) => {
                if let Some((host, mut dialog)) = self.take_picker_dialog() {
                    if matches!(dialog.meta.kind, PickerKind::Lineage { session_id: current_session_id } if current_session_id == session_id)
                    {
                        dialog.set_loading(false);
                        dialog.empty_message = ui_text::t(&self.i18n, "overlay-lineage-empty");
                    }
                    self.restore_picker_dialog(host, dialog);
                }
                self.flash_error(error);
            }
        }
    }

    pub(in crate::app) fn handle_rewind_messages_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<Vec<MessageResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_picker_dialog() else {
            return;
        };
        let PickerKind::RewindMessages {
            session_id: current_session_id,
        } = &dialog.meta.kind
        else {
            self.restore_picker_dialog(host, dialog);
            return;
        };
        if *current_session_id != session_id {
            self.restore_picker_dialog(host, dialog);
            return;
        }

        dialog.set_loading(false);
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-rewind-empty");
        match result {
            Ok(messages) => {
                let items = messages
                    .into_iter()
                    .filter(is_rewind_target_message)
                    .rev()
                    .map(|message| self.rewind_message_picker_item(message))
                    .collect();
                dialog.replace_items(items);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_picker_dialog(host, dialog);
    }

    pub(in crate::app) fn handle_model_catalog_loaded(
        &mut self,
        query: String,
        offset: usize,
        result: UiResult<ModelCatalogListResponse>,
    ) {
        let Some((host, mut dialog)) = self.take_model_catalog_dialog() else {
            return;
        };
        if dialog.query != query || dialog.offset != offset {
            self.restore_model_catalog_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        match result {
            Ok(response) => {
                dialog.workbench.list.items = response.items;
                dialog.summary = response.summary;
                dialog.total = response.total;
                dialog.offset = response.offset;
                dialog.limit = response.limit;
                dialog.workbench.list.clamp_selection();
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_model_catalog_dialog(host, dialog);
    }

    pub(in crate::app) fn handle_provider_studio_adapter_models_loaded(
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
                dialog.adapter_models = response.adapters;
                dialog
                    .selection
                    .clamp_left(dialog.adapter_candidate_ids.len());
                dialog.selection.set_right_selected(0);
                self.reload_provider_studio_catalog_matches(&mut dialog);
                dialog.selected_model_keys = preserved_model_keys;
                provider_studio_restore_model_selection(&mut dialog);
                provider_studio_ensure_default_selection(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_provider_studio_dialog(host, dialog);
    }

    pub(in crate::app) fn handle_provider_studio_auth_completed(
        &mut self,
        request_key: String,
        result: std::result::Result<
            crate::backend::ProviderDraftAuthActionResult,
            crate::backend::ProviderDraftAuthError,
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

    pub(in crate::app) fn handle_provider_studio_saved(
        &mut self,
        provider_id: String,
        result: std::result::Result<
            crate::backend::ProviderStudioSaveResult,
            crate::backend::ProviderStudioSaveError,
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
                    crate::backend::ProviderStudioSaveResult::ModelDeleted {
                        adapter_id,
                        model_id,
                        ..
                    } => {
                        preserved_selected_model_keys
                            .remove(provider_studio_model_key(adapter_id, model_id).as_str());
                    }
                    crate::backend::ProviderStudioSaveResult::AdapterDeleted {
                        adapter_id, ..
                    } => {
                        preserved_selected_adapter_ids.remove(adapter_id.as_str());
                        if preserved_selected_adapter_id.as_deref() == Some(adapter_id.as_str()) {
                            preserved_selected_adapter_id = None;
                        }
                        let prefix = format!("{adapter_id}\u{1f}");
                        preserved_selected_model_keys
                            .retain(|key| !key.starts_with(prefix.as_str()));
                    }
                    crate::backend::ProviderStudioSaveResult::ProviderDeleted { .. } => {}
                    crate::backend::ProviderStudioSaveResult::ProviderDraftSaved { .. }
                    | crate::backend::ProviderStudioSaveResult::AdapterMatchesSaved { .. }
                    | crate::backend::ProviderStudioSaveResult::ConfiguredModelSaved { .. } => {}
                }
                self.flash_success(provider_studio_save_result_message(&self.i18n, &message));
                if matches!(
                    &message,
                    crate::backend::ProviderStudioSaveResult::ProviderDeleted { .. }
                ) {
                    self.restore_provider_list_after_provider_delete();
                    return;
                }
                let providers = self.backend.list_configured_providers();
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
                    crate::backend::ProviderStudioSaveResult::AdapterDeleted { .. } => {
                        dialog.selection.set_focus(ProviderStudioFocus::Adapters);
                    }
                    crate::backend::ProviderStudioSaveResult::ModelDeleted { .. } => {
                        dialog.selection.set_focus(ProviderStudioFocus::Models);
                    }
                    _ => {}
                }
                provider_studio_ensure_default_selection(&mut dialog);
            }
            Err(error) => self.flash_error(provider_studio_save_error_message(&self.i18n, &error)),
        }
        self.restore_provider_studio_dialog(host, dialog);
    }

    pub(in crate::app) fn restore_provider_list_after_provider_delete(&mut self) {
        let provider_picker = self.route_stack.last().and_then(|route| match route {
            Route::Picker(dialog)
                if matches!(
                    dialog.meta.kind,
                    PickerKind::Providers(ProviderPickerPurpose::Configure)
                ) =>
            {
                Some(dialog.input.text().to_string())
            }
            _ => None,
        });
        if provider_picker.is_some() {
            let _ = self.route_stack.pop();
        }
        self.current_route = Route::Picker(
            self.build_provider_list_overlay(provider_picker.as_deref().unwrap_or(""), false),
        );
        self.overlay = None;
    }

    pub(in crate::app) fn handle_model_catalog_refreshed(&mut self, result: UiResult<()>) {
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
                dialog.loading = true;
                dialog.offset = 0;
                dialog.workbench.list.selected = 0;
                self.request_model_catalog_page(dialog.query.clone(), 0);
            }
            Err(error) => {
                dialog.loading = false;
                self.flash_error(error);
            }
        }
        self.restore_model_catalog_dialog(host, dialog);
    }

    pub(in crate::app) fn handle_child_sessions_loaded(
        &mut self,
        parent_session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_picker_dialog() else {
            return;
        };
        let PickerKind::ChildSessions {
            parent_session_id: current_parent_id,
        } = &dialog.meta.kind
        else {
            self.restore_picker_dialog(host, dialog);
            return;
        };
        if *current_parent_id != parent_session_id {
            self.restore_picker_dialog(host, dialog);
            return;
        }

        dialog.set_loading(false);
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-children-empty");
        match result {
            Ok(sessions) => {
                let items = sessions
                    .into_iter()
                    .map(|session| PickerItem {
                        label: session.title.clone(),
                        detail: format!(
                            "#{} | {} msg | {} child",
                            session.id, session.message_count, session.child_session_count
                        ),
                        value: PickerValue::Session(session.id),
                    })
                    .collect();
                dialog.replace_items(items);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_picker_dialog(host, dialog);
    }

    pub(in crate::app) fn handle_timeline_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<Vec<DomainEvent>>,
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

    pub(in crate::app) fn handle_session_rewound(
        &mut self,
        session_id: i64,
        target: String,
        result: UiResult<SessionExecutionResource>,
    ) {
        self.finish_run_operation(RunActivityTarget::Session(session_id), RunOperation::Rewind);
        match result {
            Ok(execution) => {
                let rewound_session_id = execution.session.id;
                let preserved_draft = (self.transcript.session_id == Some(session_id))
                    .then(|| self.current_composer_draft());
                if let Some(draft) = preserved_draft.clone() {
                    self.set_draft_for_slot(DraftSlot::Session(rewound_session_id), draft);
                }
                self.open_session(rewound_session_id, execution.session.title.clone());
                if let Some(draft) = preserved_draft {
                    self.replace_composer_draft(draft);
                    self.persist_draft_store_with_feedback(true);
                }
                if self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(rewound_session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
                self.focus = Focus::Composer;
                self.request_sessions(false);
                self.flash_success(self.i18n.text_args(
                    "flash-session-rewound",
                    &crate::fl_args!("target" => target),
                ));
            }
            Err(error) => {
                self.flash_error(error);
            }
        }
    }
}
use crate::app::{
    AgentDescriptor, App, CurrentLineageState, DomainEvent, DraftSlot, Focus, Instant,
    MessageResource, ModelCatalogListResponse, PaginatedResponse, PickerItem, PickerKind,
    PickerValue, ProviderAdapterModelsResponse, ProviderPickerPurpose, ProviderStudioFocus,
    ProviderSummaryResource, Route, RunActivityTarget, RunOperation, SelectableListState,
    SessionExecutionResource, SessionResource, SessionViewMode, UiResult, agent_list_items,
    build_lineage_session_items, build_timeline_item, i18n_provider_list_detail,
    is_rewind_target_message, provider_draft_auth_action_message,
    provider_draft_auth_error_message, provider_draft_auth_message_is_pending,
    provider_list_create_item, provider_studio_ensure_default_selection, provider_studio_model_key,
    provider_studio_preferred_detail_field_index, provider_studio_provider_rows,
    provider_studio_restore_model_selection, provider_studio_save_error_message,
    provider_studio_save_result_message, provider_studio_selected_adapter_id,
    restore_provider_studio_adapter_selection, settings_choice_adapter_fallback,
    settings_choice_default_provider_detail, summarize_lineage_session_items, ui_text,
};
