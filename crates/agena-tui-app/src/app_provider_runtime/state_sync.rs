impl App {
    pub(crate) fn sync_provider_studio_shape(&mut self, dialog: &mut ProviderStudioOverlay) {
        dialog.draft.normalize_shape();
        dialog.adapter_candidate_ids = provider_studio_candidate_adapter_ids(
            &dialog.draft,
            dialog.configured_adapter_ids.clone(),
        );
        let selectable_adapter_ids = dialog
            .adapter_candidate_ids
            .iter()
            .filter(|adapter_id| provider_studio_adapter_selectable(dialog, adapter_id.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>();
        dialog
            .selection
            .clamp_top(provider_studio_visible_fields(dialog).len());
        let detail_field_count = provider_studio_detail_fields(dialog).len();
        if let Some(detail_page) = dialog.detail_page.as_mut() {
            detail_page.selection.clamp(detail_field_count);
        }
        dialog.selected_adapter_ids.retain(|adapter_id| {
            dialog
                .adapter_candidate_ids
                .iter()
                .any(|candidate| candidate == adapter_id)
                && selectable_adapter_ids.contains(adapter_id)
        });
        dialog
            .selection
            .clamp_left(dialog.adapter_candidate_ids.len());
        dialog.selection.clamp_right(
            provider_studio_selected_adapter_models(dialog)
                .map(|adapter| adapter.models.len())
                .unwrap_or_default(),
        );
        if !dialog.adapter_models.is_empty() {
            provider_studio_restore_model_selection(dialog);
        }
        self.sync_provider_studio_auth_poll_deadline(dialog, Instant::now(), false);
    }

    pub(crate) fn sync_provider_studio_auth_poll_deadline(
        &self,
        dialog: &mut ProviderStudioOverlay,
        now: Instant,
        reset: bool,
    ) {
        match provider_studio_auth_poll_interval(dialog) {
            Some(interval) if reset || dialog.next_auth_poll_at.is_none() => {
                dialog.next_auth_poll_at = now.checked_add(interval).or(Some(now));
            }
            Some(_) => {}
            None => {
                dialog.next_auth_poll_at = None;
            }
        }
    }

    pub(crate) fn reload_provider_studio_catalog_matches(
        &self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let lookup_ids = dialog
            .adapter_models
            .iter()
            .flat_map(|adapter| {
                adapter.models.iter().flat_map(|model| {
                    [
                        model.id.to_string(),
                        provider_model_catalog_lookup_id(model),
                    ]
                })
            })
            .collect::<Vec<_>>();
        let catalog_entries = self.backend.lookup_model_catalog_models(&lookup_ids);
        dialog.catalog_matches = dialog
            .adapter_models
            .iter()
            .flat_map(|adapter| {
                adapter.models.iter().filter_map(|provider_model| {
                    provider_studio_catalog_match_model(provider_model, &catalog_entries).map(
                        |catalog_model| {
                            (
                                provider_studio_model_key(
                                    adapter.adapter_id.as_str(),
                                    provider_model.id.as_ref(),
                                ),
                                catalog_model.clone(),
                            )
                        },
                    )
                })
            })
            .collect();
    }

    pub(crate) fn refresh_provider_studio_adapter_state(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        dialog.adapter_models.clear();
        dialog.selected_model_keys.clear();
        dialog.catalog_matches.clear();
        self.sync_provider_studio_shape(dialog);
        dialog.selection.set_right_selected(0);
        dialog.pending_adapter_models_key = None;
        dialog.listing_adapter_models = false;
    }
}
use crate::{
    App, BTreeSet, Instant, ProviderStudioOverlay, provider_model_catalog_lookup_id,
    provider_studio_adapter_selectable, provider_studio_auth_poll_interval,
    provider_studio_candidate_adapter_ids, provider_studio_catalog_match_model,
    provider_studio_detail_fields, provider_studio_model_key,
    provider_studio_restore_model_selection, provider_studio_selected_adapter_models,
    provider_studio_visible_fields,
};
