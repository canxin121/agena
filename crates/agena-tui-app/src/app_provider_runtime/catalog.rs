impl App {
    pub(crate) fn open_provider_list(&mut self, query: &str) {
        let dialog = self.build_provider_list_overlay(query, true);
        self.current_route = Route::SelectionPicker(dialog);
        self.request_providers(ProviderPickerPurpose::Configure);
    }

    pub(crate) fn build_provider_list_overlay(
        &self,
        query: &str,
        loading: bool,
    ) -> SelectionPickerOverlay {
        let rows = if loading {
            Vec::new()
        } else {
            let mut rows = vec![provider_list_create_item(&self.i18n)];
            rows.extend(
                self.backend
                    .list_configured_providers()
                    .into_iter()
                    .map(|provider| {
                        let label = provider.provider_id.clone();
                        let detail = i18n_provider_list_detail(&self.i18n, &provider);
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
                    }),
            );
            rows
        };
        let mut dialog = self.build_selection_picker_overlay(
            ui_text::t(&self.i18n, "overlay-provider-list-title"),
            ui_text::t(&self.i18n, "overlay-provider-list-prompt"),
            ui_text::t(&self.i18n, "overlay-provider-list-footer"),
            ui_text::t(
                &self.i18n,
                if loading {
                    "overlay-picker-loading"
                } else {
                    "overlay-picker-empty"
                },
            ),
            query.trim().to_string(),
            SelectionPickerQuery::Providers(ProviderPickerPurpose::Configure),
            loading,
        );
        dialog.actions = rows
            .iter()
            .map(|(item, action)| (item.key.clone(), action.clone()))
            .collect();
        dialog
            .presentation
            .replace_items(rows.into_iter().map(|(item, _)| item).collect());
        dialog
    }

    pub(crate) fn open_session_model_chooser(&mut self) {
        self.open_model_chooser(SessionModelChooserPurpose::RuntimeOverride);
    }

    pub(crate) fn open_provider_default_model_chooser(&mut self) {
        self.open_model_chooser(SessionModelChooserPurpose::ProviderDefault);
    }

    fn open_model_chooser(&mut self, purpose: SessionModelChooserPurpose) {
        if purpose == SessionModelChooserPurpose::RuntimeOverride {
            let Some(session_id) = self.transcript.session_id else {
                self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
                return;
            };
            if self.session_is_busy(session_id) {
                self.flash_warning(ui_text::t(&self.i18n, "flash-session-busy"));
                return;
            }
        }
        let mut dialog = self.build_session_model_chooser_overlay(purpose);
        dialog.set_loading(false);
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-picker-empty");
        match self.session_model_chooser_items() {
            Ok(mut items) => {
                let current_model = match purpose {
                    SessionModelChooserPurpose::RuntimeOverride => self.current_session_model_ref(),
                    SessionModelChooserPurpose::ProviderDefault => {
                        self.current_provider_default_model_ref()
                    }
                };
                mark_current_session_model_choice(&self.i18n, &mut items, current_model.as_ref());
                dialog.replace_items(items);
                agena_tui::model_chooser::refresh(&mut dialog, true);
            }
            Err(error) => self.flash_error(error),
        }
        self.current_route = Route::SessionModelChooser(dialog);
    }

    pub(crate) fn open_provider_studio(&mut self, initial_provider: Option<&str>) {
        let providers = self.backend.list_configured_providers();
        let provider_rows = provider_studio_provider_rows(&self.i18n, providers.as_slice());
        let selected_provider = initial_provider
            .and_then(|provider_id| {
                provider_rows
                    .iter()
                    .position(|row| row.provider_id.as_deref() == Some(provider_id.trim()))
            })
            .unwrap_or(0);
        let draft_prefill = initial_provider
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|provider_id| {
                !providers
                    .iter()
                    .any(|provider| provider.provider_id == *provider_id)
            })
            .map(str::to_owned);
        let mut overlay = ProviderStudioOverlay {
            title: ui_text::t(&self.i18n, "overlay-provider-studio-title"),
            footer: ui_text::t(&self.i18n, "overlay-provider-studio-footer"),
            show_provider_list: false,
            providers: SelectableListState::new(provider_rows, selected_provider),
            selection: DashboardSelectionState::new(
                [
                    ProviderStudioFocus::Fields,
                    ProviderStudioFocus::Adapters,
                    ProviderStudioFocus::Models,
                ],
                ProviderStudioFocus::Fields,
                0,
                0,
                0,
            ),
            draft: self
                .backend
                .provider_config_draft(None)
                .unwrap_or_else(|_| ProviderConfigDraft::new_empty()),
            adapter_models: Vec::new(),
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: Vec::new(),
            selected_adapter_ids: BTreeSet::new(),
            selected_model_keys: BTreeSet::new(),
            catalog_matches: BTreeMap::new(),
            listing_adapter_models: false,
            saving: false,
            pending_adapter_models_key: None,
            pending_auth_key: None,
            next_auth_poll_at: None,
            detail_page: None,
            model_page: None,
            editor: None,
        };
        let selected_id = overlay
            .providers
            .selected_item()
            .and_then(|row| row.provider_id.clone());
        self.load_provider_studio_draft(&mut overlay, selected_id.as_deref(), draft_prefill);
        self.current_route = Route::ProviderStudio(Box::new(overlay));
    }

    pub(crate) fn load_provider_studio_draft(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        provider_id: Option<&str>,
        prefill_new_id: Option<String>,
    ) {
        match self.backend.provider_config_draft(provider_id) {
            Ok(mut draft) => {
                if provider_id.is_none()
                    && let Some(prefill) = prefill_new_id
                {
                    draft.provider_id = prefill;
                    draft.normalize_shape();
                }
                dialog.draft = draft;
                dialog.selection.set_top_selected(0);
                dialog.adapter_models =
                    self.backend.configured_provider_adapter_models(provider_id);
                let configured_adapters = provider_id
                    .and_then(|id| {
                        self.backend
                            .list_configured_providers()
                            .into_iter()
                            .find(|provider| provider.provider_id == id)
                    })
                    .map(|provider| provider.adapters)
                    .unwrap_or_default();
                let configured_adapter_ids = configured_adapters
                    .iter()
                    .map(|adapter| adapter.adapter_id.clone())
                    .collect::<BTreeSet<_>>();
                let enabled_adapter_ids = configured_adapters
                    .into_iter()
                    .filter(|adapter| adapter.enabled)
                    .map(|adapter| adapter.adapter_id)
                    .collect::<BTreeSet<_>>();
                dialog.configured_adapter_ids = configured_adapter_ids.clone();
                dialog.selection.set_left_selected(0);
                dialog.selection.set_right_selected(0);
                dialog.pending_adapter_models_key = None;
                dialog.pending_auth_key = None;
                dialog.detail_page = None;
                dialog.model_page = None;
                dialog.listing_adapter_models = false;
                dialog.selected_adapter_ids = enabled_adapter_ids;
                dialog.selected_model_keys = self
                    .backend
                    .configured_provider_model_routes(provider_id)
                    .into_iter()
                    .map(|(adapter_id, model_id)| {
                        provider_studio_model_key(adapter_id.as_str(), model_id.as_str())
                    })
                    .collect();
                dialog.catalog_matches.clear();
                self.reload_provider_studio_catalog_matches(dialog);
                self.sync_provider_studio_shape(dialog);
                if let Some(first_selected) = dialog
                    .adapter_candidate_ids
                    .iter()
                    .position(|candidate| dialog.selected_adapter_ids.contains(candidate.as_str()))
                {
                    dialog.selection.set_left_selected(first_selected);
                }
            }
            Err(error) => self.flash_error(error.to_string()),
        }
    }

    pub(crate) fn open_model_catalog_studio(&mut self) {
        let dialog = ModelCatalogStudioOverlay {
            summary: ModelCatalogResponse {
                refreshing: false,
                last_refresh_at: None,
                last_successful_source: None,
                last_error: None,
                model_count: 0,
            },
            presentation: ModelCatalogPresentation::new(
                ui_text::t(&self.i18n, "overlay-model-catalog-title"),
                ui_text::t(&self.i18n, "overlay-model-catalog-footer"),
                50,
            ),
            editor: None,
        };
        self.request_model_catalog_page(String::new(), 0);
        self.current_route = Route::ModelCatalogStudio(dialog.clone());
    }

    pub(crate) fn request_model_catalog_page(&mut self, query: String, offset: usize) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_model_catalog_models(query.as_str(), offset, 50)
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ModelCatalogLoaded {
                query,
                offset,
                result,
            });
        });
    }

    pub(crate) fn request_model_catalog_refresh(&mut self) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .refresh_model_catalog()
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ModelCatalogRefreshed { result });
        });
    }

    pub(crate) fn request_provider_studio_adapter_models(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let adapter_ids = provider_studio_request_adapter_ids(dialog);
        if adapter_ids.is_empty() {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-required",
            ));
            return;
        }
        if !provider_studio_can_request_adapter_models(dialog) {
            self.flash_warning(provider_studio_live_listing_unavailable_message(
                &self.i18n,
                &dialog.draft.auth_kind,
            ));
            return;
        }

        if dialog.draft.auth_kind.supports_draft_model_listing() {
            let unsupported = adapter_ids
                .iter()
                .filter(|adapter_id| {
                    provider_studio_adapter_rule(dialog, adapter_id.as_str())
                        .map(|rule| !rule.supports_draft_model_listing)
                        .unwrap_or(true)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !unsupported.is_empty() {
                self.flash_error(provider_studio_draft_listing_unsupported_message(
                    &self.i18n,
                    unsupported.as_slice(),
                ));
                return;
            }
        }

        let request_key = provider_studio_request_key(&dialog.draft, &adapter_ids);
        dialog.pending_adapter_models_key = Some(request_key.clone());
        dialog.listing_adapter_models = true;
        let backend = self.backend.clone();
        let i18n = self.i18n.clone();
        let tx = self.tx.clone();
        let draft = dialog.draft.clone();
        tokio::spawn(async move {
            let result = if draft.auth_kind.supports_draft_model_listing() {
                backend
                    .list_draft_provider_adapter_models(&draft, &adapter_ids)
                    .await
                    .map_err(|error| error.to_string())
            } else if let Some(provider_id) = draft.source_provider_id.as_deref() {
                backend
                    .list_saved_provider_adapter_models(provider_id, &adapter_ids)
                    .await
                    .map_err(|error| error.to_string())
            } else {
                Err(provider_studio_listing_auth_required_message(
                    &i18n,
                    &draft.auth_kind,
                ))
            };
            let _ = tx.send(AppMessage::ProviderStudioAdapterModelsLoaded {
                request_key,
                result,
            });
        });
    }

    pub(crate) fn request_provider_studio_start_auth(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        if dialog.pending_auth_key.is_some() {
            return;
        }
        let request_key = provider_studio_auth_request_key(&dialog.draft, "start");
        dialog.pending_auth_key = Some(request_key.clone());
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let draft = dialog.draft.clone();
        tokio::spawn(async move {
            let result = backend.start_provider_draft_auth(draft).await;
            let _ = tx.send(AppMessage::ProviderStudioAuthCompleted {
                request_key,
                result,
            });
        });
    }

    pub(crate) fn request_provider_studio_continue_auth(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        if dialog.pending_auth_key.is_some() {
            return;
        }
        let request_key = provider_studio_auth_request_key(&dialog.draft, "continue");
        dialog.pending_auth_key = Some(request_key.clone());
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let draft = dialog.draft.clone();
        tokio::spawn(async move {
            let result = backend.continue_provider_draft_auth(draft).await;
            let _ = tx.send(AppMessage::ProviderStudioAuthCompleted {
                request_key,
                result,
            });
        });
    }
}
use crate::{
    App, AppMessage, BTreeMap, BTreeSet, DashboardSelectionState, ModelCatalogResponse,
    ModelCatalogStudioOverlay, ProviderConfigDraft, ProviderPickerPurpose, ProviderStudioFocus,
    ProviderStudioOverlay, Route, SelectableListState, SelectionPickerCommand,
    SelectionPickerOverlay, SelectionPickerQuery, SessionModelChooserPurpose,
    i18n_provider_list_detail, mark_current_session_model_choice, provider_list_create_item,
    provider_studio_adapter_rule, provider_studio_auth_request_key,
    provider_studio_can_request_adapter_models, provider_studio_draft_listing_unsupported_message,
    provider_studio_listing_auth_required_message,
    provider_studio_live_listing_unavailable_message, provider_studio_model_key,
    provider_studio_provider_rows, provider_studio_request_adapter_ids,
    provider_studio_request_key, ui_text,
};
use agena_tui::model_catalog::ModelCatalogPresentation;
