pub(in crate::app) fn provider_studio_request_key(
    draft: &ProviderConfigDraft,
    adapter_ids: &[String],
) -> String {
    draft.request_fingerprint(adapter_ids)
}

pub(in crate::app) fn provider_studio_auth_request_key(
    draft: &ProviderConfigDraft,
    action: &str,
) -> String {
    format!("{action}:{}", provider_studio_request_key(draft, &[]))
}

pub(in crate::app) fn provider_studio_candidate_adapter_ids(
    draft: &ProviderConfigDraft,
    configured_adapter_ids: BTreeSet<String>,
) -> Vec<String> {
    let mut adapter_ids = draft
        .auth_kind
        .adapter_rules()
        .iter()
        .map(|rule| rule.adapter_id.to_owned())
        .collect::<Vec<_>>();
    let mut configured_extras = configured_adapter_ids.into_iter().collect::<Vec<_>>();
    configured_extras.sort();
    for adapter_id in configured_extras {
        if !adapter_ids.iter().any(|candidate| candidate == &adapter_id) {
            adapter_ids.push(adapter_id);
        }
    }
    let default_adapter = draft.default_adapter.trim();
    if !default_adapter.is_empty()
        && !adapter_ids
            .iter()
            .any(|candidate| candidate.as_str() == default_adapter)
    {
        adapter_ids.push(default_adapter.to_owned());
    }
    adapter_ids
}

pub(in crate::app) fn provider_studio_effective_adapter_ids(
    dialog: &ProviderStudioOverlay,
) -> BTreeSet<String> {
    let mut adapter_ids = dialog.configured_adapter_ids.clone();
    adapter_ids.extend(dialog.selected_adapter_ids.iter().cloned());
    let default_adapter = dialog.draft.default_adapter.trim();
    if !default_adapter.is_empty() {
        adapter_ids.insert(default_adapter.to_owned());
    }
    adapter_ids
}

pub(in crate::app) fn provider_studio_adapter_selectable(
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
) -> bool {
    provider_studio_adapter_rule(dialog, adapter_id).is_some()
}

pub(in crate::app) fn provider_studio_request_adapter_ids(
    dialog: &ProviderStudioOverlay,
) -> Vec<String> {
    dialog
        .selected_adapter_ids
        .iter()
        .filter(|adapter_id| provider_studio_adapter_selectable(dialog, adapter_id.as_str()))
        .cloned()
        .collect()
}

pub(in crate::app) fn restore_provider_studio_adapter_selection(
    dialog: &mut ProviderStudioOverlay,
    selected_adapter_ids: &BTreeSet<String>,
    selected_adapter_id: Option<&str>,
) {
    dialog.selected_adapter_ids = selected_adapter_ids
        .iter()
        .filter(|adapter_id| {
            dialog
                .adapter_candidate_ids
                .iter()
                .any(|candidate| candidate == *adapter_id)
                && provider_studio_adapter_selectable(dialog, adapter_id.as_str())
        })
        .cloned()
        .collect();
    provider_studio_auto_select_single_adapter(dialog);
    if let Some(adapter_id) = selected_adapter_id
        && let Some(index) = dialog
            .adapter_candidate_ids
            .iter()
            .position(|candidate| candidate == adapter_id)
    {
        dialog.selection.set_left_selected(index);
    }
}

pub(in crate::app) fn provider_studio_auto_select_single_adapter(
    dialog: &mut ProviderStudioOverlay,
) {
    let mut selectable = dialog
        .adapter_candidate_ids
        .iter()
        .enumerate()
        .filter(|(_, adapter_id)| provider_studio_adapter_selectable(dialog, adapter_id.as_str()));
    let Some((index, adapter_id)) = selectable.next() else {
        return;
    };
    if selectable.next().is_some() {
        return;
    }
    dialog.selected_adapter_ids = BTreeSet::from([adapter_id.clone()]);
    dialog.selection.set_left_selected(index);
}

pub(in crate::app) fn provider_studio_adapter_rule(
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
) -> Option<&'static ProviderDraftAdapterRule> {
    dialog.draft.auth_kind.adapter_rule(adapter_id)
}

pub(in crate::app) fn provider_studio_base_url_visible(dialog: &ProviderStudioOverlay) -> bool {
    if !dialog.draft.auth.base_url.trim().is_empty() {
        return true;
    }
    match dialog.draft.auth_kind {
        ProviderDraftAuthKind::Unset => false,
        ProviderDraftAuthKind::ApiPending => false,
        ProviderDraftAuthKind::Api => {
            let effective = provider_studio_effective_adapter_ids(dialog);
            if effective.is_empty() {
                dialog
                    .draft
                    .auth_kind
                    .adapter_rules()
                    .iter()
                    .any(|rule| rule.requires_base_url)
            } else {
                effective
                    .iter()
                    .filter_map(|adapter_id| {
                        provider_studio_adapter_rule(dialog, adapter_id.as_str())
                    })
                    .any(|rule| rule.requires_base_url)
            }
        }
        ProviderDraftAuthKind::ClineApi => false,
        ProviderDraftAuthKind::Gitlab => false,
        ProviderDraftAuthKind::Credential(Some(issuer)) => issuer.uses_http_endpoint(),
        ProviderDraftAuthKind::Credential(None) => false,
        ProviderDraftAuthKind::BedrockSigv4 => true,
        ProviderDraftAuthKind::None => false,
    }
}

pub(in crate::app) fn provider_studio_selected_adapter_id(
    dialog: &ProviderStudioOverlay,
) -> Option<String> {
    dialog
        .adapter_candidate_ids
        .get(dialog.selection.left_selected())
        .cloned()
}

pub(in crate::app) fn provider_studio_selected_adapter_models(
    dialog: &ProviderStudioOverlay,
) -> Option<&ProviderAdapterModelsResource> {
    let adapter_id = dialog
        .adapter_candidate_ids
        .get(dialog.selection.left_selected())?;
    dialog
        .adapter_models
        .iter()
        .find(|adapter_models| adapter_models.adapter_id == *adapter_id)
}

pub(in crate::app) fn provider_studio_selected_model_target(
    dialog: &ProviderStudioOverlay,
) -> Option<(String, String, Option<ProviderModel>)> {
    let adapter_models = provider_studio_selected_adapter_models(dialog)?;
    let model = adapter_models
        .models
        .get(dialog.selection.right_selected())?
        .clone();
    Some((
        adapter_models.adapter_id.clone(),
        model.id.to_string(),
        Some(model),
    ))
}

pub(in crate::app) fn provider_studio_selected_adapter_models_for_save(
    dialog: &ProviderStudioOverlay,
) -> Option<ProviderAdapterModelsResource> {
    let adapter_models = provider_studio_selected_adapter_models(dialog)?.clone();
    let ProviderAdapterModelsResource {
        adapter_id,
        enabled,
        resolved_base_url,
        models,
        error,
    } = adapter_models;
    let selected_models = models
        .into_iter()
        .filter(|model| {
            provider_studio_model_selected(dialog, adapter_id.as_str(), model.id.as_ref())
        })
        .collect::<Vec<_>>();
    Some(ProviderAdapterModelsResource {
        adapter_id,
        enabled,
        resolved_base_url,
        models: selected_models,
        error,
    })
}

pub(in crate::app) fn provider_studio_model_selected(
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
    model_id: &str,
) -> bool {
    dialog
        .selected_model_keys
        .contains(provider_studio_model_key(adapter_id, model_id).as_str())
}

pub(in crate::app) fn provider_studio_available_model_keys(
    adapter_models: &[ProviderAdapterModelsResource],
) -> BTreeSet<String> {
    adapter_models
        .iter()
        .flat_map(|adapter_models| {
            adapter_models.models.iter().map(|model| {
                provider_studio_model_key(adapter_models.adapter_id.as_str(), model.id.as_ref())
            })
        })
        .collect()
}

pub(in crate::app) fn provider_studio_new_default_selected_model_keys(
    adapter_models: &[ProviderAdapterModelsResource],
    selected_adapter_ids: &BTreeSet<String>,
    previously_available: &BTreeSet<String>,
) -> BTreeSet<String> {
    adapter_models
        .iter()
        .filter(|adapter_models| {
            adapter_models.error.is_none()
                && selected_adapter_ids.contains(adapter_models.adapter_id.as_str())
        })
        .flat_map(|adapter_models| {
            adapter_models.models.iter().filter_map(|model| {
                let key = provider_studio_model_key(
                    adapter_models.adapter_id.as_str(),
                    model.id.as_ref(),
                );
                (!previously_available.contains(key.as_str())).then_some(key)
            })
        })
        .collect()
}

pub(in crate::app) fn provider_studio_restore_model_selection(dialog: &mut ProviderStudioOverlay) {
    let available = provider_studio_available_model_keys(&dialog.adapter_models);
    dialog
        .selected_model_keys
        .retain(|model_key| available.contains(model_key));
    for adapter_models in &dialog.adapter_models {
        let adapter_selected = dialog
            .selected_adapter_ids
            .contains(adapter_models.adapter_id.as_str());
        if !adapter_selected || adapter_models.error.is_some() {
            continue;
        }
        let has_any = adapter_models.models.iter().any(|model| {
            provider_studio_model_selected(
                dialog,
                adapter_models.adapter_id.as_str(),
                model.id.as_ref(),
            )
        });
        if !has_any {
            for model in &adapter_models.models {
                dialog.selected_model_keys.insert(provider_studio_model_key(
                    adapter_models.adapter_id.as_str(),
                    model.id.as_ref(),
                ));
            }
        }
    }
}

pub(in crate::app) fn provider_studio_first_selected_model<'a>(
    dialog: &'a ProviderStudioOverlay,
    adapter_id: &str,
) -> Option<&'a ProviderModel> {
    dialog
        .adapter_models
        .iter()
        .find(|adapter_models| adapter_models.adapter_id == adapter_id)
        .and_then(|adapter_models| {
            adapter_models
                .models
                .iter()
                .find(|model| provider_studio_model_selected(dialog, adapter_id, model.id.as_ref()))
        })
}

pub(in crate::app) fn provider_studio_ensure_default_selection(dialog: &mut ProviderStudioOverlay) {
    provider_studio_auto_select_single_adapter(dialog);

    let default_adapter_valid = dialog
        .selected_adapter_ids
        .contains(dialog.draft.default_adapter.as_str())
        && provider_studio_adapter_selectable(dialog, dialog.draft.default_adapter.as_str());
    if !default_adapter_valid {
        let replacement = provider_studio_selected_adapter_id(dialog)
            .filter(|adapter_id| dialog.selected_adapter_ids.contains(adapter_id.as_str()))
            .filter(|adapter_id| provider_studio_adapter_selectable(dialog, adapter_id.as_str()))
            .or_else(|| {
                dialog
                    .selected_adapter_ids
                    .iter()
                    .find(|adapter_id| {
                        provider_studio_adapter_selectable(dialog, adapter_id.as_str())
                    })
                    .cloned()
            });
        let Some(replacement) = replacement else {
            dialog.draft.default_adapter.clear();
            dialog.draft.default_model.clear();
            return;
        };
        dialog.draft.default_adapter = replacement;
    }

    let first_selected_model =
        provider_studio_first_selected_model(dialog, dialog.draft.default_adapter.as_str());
    let default_model_valid = first_selected_model
        .is_some_and(|model| model.id.as_ref() == dialog.draft.default_model.as_str());
    if !default_model_valid {
        dialog.draft.default_model = first_selected_model
            .map(|model| model.id.to_string())
            .unwrap_or_default();
    }
}

pub(in crate::app) fn provider_studio_supports_saved_model_listing(
    draft: &ProviderConfigDraft,
) -> bool {
    draft.supports_saved_model_listing()
}

pub(in crate::app) fn provider_studio_can_request_adapter_models(
    dialog: &ProviderStudioOverlay,
) -> bool {
    if dialog.draft.auth_kind.supports_draft_model_listing() {
        return true;
    }
    dialog.draft.source_provider_id.is_some()
        && provider_studio_supports_saved_model_listing(&dialog.draft)
}

pub(in crate::app) fn provider_studio_summary_value(
    value: &str,
    max_width: usize,
) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| truncate_display_width(value, max_width))
}

pub(in crate::app) enum ProviderStudioAuthStatus {
    Pending,
    Unset,
    None,
    SelectSubtype,
    SelectIssuer,
    Configured,
    Partial,
}
use super::{
    BTreeSet, ProviderAdapterModelsResource, ProviderConfigDraft, ProviderDraftAdapterRule,
    ProviderDraftAuthKind, ProviderModel, ProviderStudioOverlay, provider_studio_model_key,
    truncate_display_width,
};

#[cfg(test)]
mod tests {
    use super::{
        ProviderAdapterModelsResource, ProviderModel, provider_studio_available_model_keys,
        provider_studio_model_key, provider_studio_new_default_selected_model_keys,
    };
    use std::collections::BTreeSet;

    fn adapter_models(adapter_id: &str, model_ids: &[&str]) -> ProviderAdapterModelsResource {
        ProviderAdapterModelsResource {
            adapter_id: adapter_id.to_owned(),
            enabled: true,
            resolved_base_url: None,
            models: model_ids
                .iter()
                .map(|model_id| ProviderModel::new(adapter_id, *model_id))
                .collect(),
            error: None,
        }
    }

    #[test]
    fn newly_discovered_models_are_selected_without_reselecting_old_models() {
        let previous = vec![adapter_models("openai", &["old-a", "old-b"])];
        let refreshed = vec![adapter_models("openai", &["old-a", "old-b", "new-c"])];
        let previously_available = provider_studio_available_model_keys(previous.as_slice());

        let selected = provider_studio_new_default_selected_model_keys(
            refreshed.as_slice(),
            &BTreeSet::from(["openai".to_owned()]),
            &previously_available,
        );

        assert_eq!(
            selected,
            BTreeSet::from([provider_studio_model_key("openai", "new-c")]),
        );
    }

    #[test]
    fn models_from_unselected_adapters_are_not_auto_selected() {
        let refreshed = vec![adapter_models("openai", &["new-a"])];

        let selected = provider_studio_new_default_selected_model_keys(
            refreshed.as_slice(),
            &BTreeSet::new(),
            &BTreeSet::new(),
        );

        assert!(selected.is_empty());
    }
}
