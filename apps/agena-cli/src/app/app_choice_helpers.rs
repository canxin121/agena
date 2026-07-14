use super::{
    model_variant_display_label, permission_mode_label, permission_mode_token,
    permission_studio_mode_target_label, quoted_settings_segment, settings_choice_adapter_fallback,
    settings_field_display_description,
};

pub(in crate::app) fn permission_mode_choice_items(i18n: &I18n) -> Vec<ChoiceItem> {
    [
        PermissionMode::Allow,
        PermissionMode::Ask,
        PermissionMode::Deny,
    ]
    .into_iter()
    .map(|mode| ChoiceItem {
        label: permission_mode_label(i18n, mode),
        detail: String::new(),
        value: permission_mode_token(mode).to_string(),
        search_text: format!(
            "{} {}",
            permission_mode_label(i18n, mode),
            permission_mode_token(mode)
        ),
        current: false,
    })
    .collect()
}

pub(in crate::app) fn agent_config_path(agent_name: &str, suffix: &str) -> String {
    format!("agents.{}.{}", quoted_settings_segment(agent_name), suffix)
}

pub(in crate::app) fn settings_studio_provider_workbench_item(
    i18n: &I18n,
    providers: &[ProviderSummaryResource],
) -> SettingsStudioItem {
    SettingsStudioItem::new(
        ui_text::t(i18n, "settings-provider-workbench-label"),
        i18n.text_args(
            "settings-provider-workbench-value",
            &crate::fl_args!("count" => providers.len() as i64),
        ),
        ui_text::t(i18n, "settings-provider-workbench-detail"),
        SettingsPickerAction::OpenProviderList,
    )
}

pub(in crate::app) fn settings_studio_model_catalog_items(
    i18n: &I18n,
    response: &ModelCatalogListResponse,
) -> Vec<SettingsStudioItem> {
    vec![SettingsStudioItem::new(
        ui_text::t(i18n, "settings-model-catalog-open-label"),
        response.summary.model_count.to_string(),
        ui_text::t(i18n, "settings-model-catalog-open-detail"),
        SettingsPickerAction::OpenModelCatalogWorkbench,
    )]
}

pub(in crate::app) fn settings_studio_file_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem> {
    vec![SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-files-open-config-label"),
        if sources.config_found {
            ui_text::t(i18n, "settings-files-open-config-present")
        } else {
            ui_text::t(i18n, "settings-files-open-config-create")
        },
        sources.config_path.display().to_string(),
        Some(sources.config_path.display().to_string()),
        None,
        None,
        Vec::new(),
        SettingsPickerAction::OpenConfigFile,
    )]
}

pub(in crate::app) fn format_setting_value_inline(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "unset".to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                "\"\"".to_string()
            } else if trimmed.chars().count() > 64 {
                format!("\"{}…\"", trimmed.chars().take(64).collect::<String>())
            } else {
                format!("\"{trimmed}\"")
            }
        }
        other => {
            let rendered = other.to_string();
            if rendered.chars().count() > 72 {
                format!("{}…", rendered.chars().take(72).collect::<String>())
            } else {
                rendered
            }
        }
    }
}

pub(in crate::app) fn setting_value_input_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => String::new(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => value.clone(),
        other => other.to_string(),
    }
}

pub(in crate::app) fn settings_value_edit_prompt(
    i18n: &I18n,
    field: SettingsFieldSpec,
    file_value: &JsonValue,
    effective_value: &JsonValue,
) -> String {
    let mut lines = vec![
        settings_field_display_description(i18n, field),
        i18n.text_args(
            "overlay-settings-detail-path",
            &crate::fl_args!("path" => field.path),
        ),
    ];
    if !file_value.is_null() {
        lines.push(i18n.text_args(
            "overlay-settings-edit-file-value",
            &crate::fl_args!("value" => format_setting_value_inline(file_value)),
        ));
        if file_value != effective_value {
            lines.push(i18n.text_args(
                "overlay-settings-edit-effective-value",
                &crate::fl_args!("value" => format_setting_value_inline(effective_value)),
            ));
        }
    } else {
        lines.push(i18n.text_args(
            "overlay-settings-edit-effective-value",
            &crate::fl_args!("value" => format_setting_value_inline(effective_value)),
        ));
    }
    lines.push(settings_field_help_suffix(i18n, field.kind));
    lines.join("\n")
}

pub(in crate::app) fn settings_field_help_suffix(i18n: &I18n, kind: SettingsFieldKind) -> String {
    match kind {
        SettingsFieldKind::String => ui_text::t(i18n, "overlay-settings-help-string"),
    }
}

pub(in crate::app) fn choice_item(
    value: impl Into<String>,
    detail: impl Into<String>,
) -> ChoiceItem {
    let value = value.into();
    let detail = detail.into();
    let search_text = format!("{} {}", value.to_lowercase(), detail.to_lowercase());
    ChoiceItem {
        label: value.clone(),
        detail,
        value,
        search_text,
        current: false,
    }
}

pub(in crate::app) fn choice_item_with_value(
    label: impl Into<String>,
    value: impl Into<String>,
    detail: impl Into<String>,
) -> ChoiceItem {
    let label = label.into();
    let value = value.into();
    let detail = detail.into();
    let search_text = format!(
        "{} {} {}",
        label.to_lowercase(),
        value.to_lowercase(),
        detail.to_lowercase()
    );
    ChoiceItem {
        label,
        detail,
        value,
        search_text,
        current: false,
    }
}

pub(in crate::app) fn mark_current_choice_item(
    i18n: &I18n,
    items: &mut Vec<ChoiceItem>,
    current_value: Option<&str>,
) {
    let Some(current_value) = current_value else {
        return;
    };
    let current_value = current_value.trim();
    if let Some(item) = items.iter_mut().find(|item| {
        item.value.eq_ignore_ascii_case(current_value)
            || item.label.eq_ignore_ascii_case(current_value)
    }) {
        item.current = true;
        return;
    }
    if current_value.is_empty() {
        return;
    }

    let mut current = choice_item(
        current_value,
        ui_text::t(i18n, "overlay-choice-current-unavailable-detail"),
    );
    current.current = true;
    items.insert(0, current);
}

pub(in crate::app) fn dedupe_choice_items(items: Vec<ChoiceItem>) -> Vec<ChoiceItem> {
    let mut deduped = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        if seen.insert(item.value.clone()) {
            deduped.push(item);
        }
    }
    deduped
}

pub(in crate::app) fn inspector_rows_to_mode_choice_items(
    rows: Vec<InspectorRow>,
    display_value: fn(&str) -> String,
) -> Vec<ChoiceItem> {
    rows.into_iter()
        .map(|row| {
            let label = display_value(row.label.as_str());
            let detail = if label == row.label {
                row.detail
            } else if row.detail.trim().is_empty() {
                row.label.clone()
            } else {
                format!("{} · {}", row.label, row.detail)
            };
            choice_item_with_value(label, row.label, detail)
        })
        .collect()
}

pub(in crate::app) fn boolean_choice_items(detail: &str) -> Vec<ChoiceItem> {
    vec![choice_item("true", detail), choice_item("false", detail)]
}

pub(in crate::app) fn provider_studio_default_model_choice_items(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> Vec<ChoiceItem> {
    let mut preferred_adapter_id = dialog.draft.default_adapter.trim().to_owned();
    if preferred_adapter_id.is_empty() {
        preferred_adapter_id = provider_studio_selected_adapter_id(dialog).unwrap_or_default();
    }
    let mut items = Vec::new();
    let mut adapter_models = dialog.adapter_models.iter().collect::<Vec<_>>();
    adapter_models.sort_by_key(|adapter_models| {
        (
            adapter_models.adapter_id != preferred_adapter_id,
            adapter_models.adapter_id.clone(),
        )
    });
    for adapter_models in adapter_models {
        for model in &adapter_models.models {
            if !dialog.selected_model_keys.is_empty()
                && !provider_studio_model_selected(
                    dialog,
                    adapter_models.adapter_id.as_str(),
                    model.id.as_ref(),
                )
            {
                continue;
            }
            let mut detail_parts = vec![adapter_models.adapter_id.clone()];
            if let Some(display_name) = model
                .display_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                detail_parts.push(display_name.trim().to_owned());
            }
            let key =
                provider_studio_model_key(adapter_models.adapter_id.as_str(), model.id.as_ref());
            detail_parts.push(provider_studio_catalog_match_label(
                i18n,
                dialog
                    .catalog_matches
                    .get(key.as_str())
                    .map(|entry| entry.model_id.as_str()),
            ));
            items.push(choice_item(
                model.id.to_string(),
                join_inline_segments(detail_parts),
            ));
        }
    }
    dedupe_choice_items(items)
}

pub(in crate::app) fn provider_studio_profile_choice_items(
    i18n: &I18n,
    backend: &Backend,
) -> Vec<ChoiceItem> {
    let mut items = backend
        .list_aws_profile_names()
        .into_iter()
        .map(|profile| choice_item(profile, ui_text::t(i18n, "provider-profile-choice-detail")))
        .collect::<Vec<_>>();
    if !items.iter().any(|item| item.value == "default") {
        items.insert(
            0,
            choice_item(
                "default",
                ui_text::t(i18n, "provider-profile-default-detail"),
            ),
        );
    }
    dedupe_choice_items(items)
}

pub(in crate::app) fn provider_studio_api_key_env_choice_items(i18n: &I18n) -> Vec<ChoiceItem> {
    let items = vec![
        choice_item(
            "OPENAI_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-openai-detail"),
        ),
        choice_item(
            "ANTHROPIC_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-anthropic-detail"),
        ),
        choice_item(
            "GEMINI_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-gemini-detail"),
        ),
        choice_item(
            "GITLAB_TOKEN",
            ui_text::t(i18n, "provider-api-key-env-gitlab-detail"),
        ),
        choice_item(
            "GOOGLE_VERTEX_ACCESS_TOKEN",
            ui_text::t(i18n, "provider-api-key-env-vertex-detail"),
        ),
        choice_item(
            "SHARED_GATEWAY_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-shared-gateway-detail"),
        ),
        choice_item(
            "OPENCODE_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-opencode-detail"),
        ),
    ];
    dedupe_choice_items(items)
}

pub(in crate::app) fn provider_studio_field_allows_clear(field: ProviderStudioField) -> bool {
    matches!(
        field,
        ProviderStudioField::AuthMode
            | ProviderStudioField::AuthSubtype
            | ProviderStudioField::BaseUrl
            | ProviderStudioField::InstanceUrl
            | ProviderStudioField::ApiKeySource
            | ProviderStudioField::ApiKeyValue
            | ProviderStudioField::RedirectUri
            | ProviderStudioField::CallbackUrl
            | ProviderStudioField::RefreshToken
            | ProviderStudioField::AccessToken
            | ProviderStudioField::ExpiresAtMs
            | ProviderStudioField::AccountId
            | ProviderStudioField::EnterpriseDomain
            | ProviderStudioField::Region
            | ProviderStudioField::Profile
            | ProviderStudioField::AccessKeyId
            | ProviderStudioField::SecretAccessKey
            | ProviderStudioField::SessionToken
            | ProviderStudioField::ServiceKeyEnv
            | ProviderStudioField::DefaultAdapter
            | ProviderStudioField::DefaultModel
    )
}

pub(in crate::app) fn choice_overlay_clear_detail(
    i18n: &I18n,
    action: &ChoiceOverlayAction,
) -> String {
    match action {
        ChoiceOverlayAction::SettingsField(field) => i18n.text_args(
            "overlay-choice-clear-settings-detail",
            &crate::fl_args!("field" => field.path),
        ),
        ChoiceOverlayAction::SessionModelVariant(step) => i18n.text_args(
            "overlay-choice-clear-runtime-detail",
            &crate::fl_args!(
                "field" => model_variant_display_label(i18n, *step)
            ),
        ),
        ChoiceOverlayAction::ProviderStudioField(field) => i18n.text_args(
            "overlay-choice-clear-provider-detail",
            &crate::fl_args!("field" => provider_studio_field_label(i18n, *field)),
        ),
        ChoiceOverlayAction::ProviderStudioModelField(field) => i18n.text_args(
            "overlay-choice-clear-provider-detail",
            &crate::fl_args!("field" => provider_model_config_field_label(i18n, *field)),
        ),
        ChoiceOverlayAction::PermissionRuleStudio(field) => match field {
            PermissionRuleStudioChoiceField::SubjectKind => {
                ui_text::t(i18n, "overlay-choice-clear-permission-subject")
            }
            PermissionRuleStudioChoiceField::PathAccessKind => {
                ui_text::t(i18n, "overlay-choice-clear-permission-access-kind")
            }
            PermissionRuleStudioChoiceField::Scope => {
                ui_text::t(i18n, "overlay-choice-clear-permission-scope")
            }
            PermissionRuleStudioChoiceField::Mode => {
                ui_text::t(i18n, "overlay-choice-clear-permission-mode")
            }
        },
        ChoiceOverlayAction::PermissionStudioMode(target) => i18n.text_args(
            "overlay-choice-clear-permission-override-detail",
            &crate::fl_args!("field" => permission_studio_mode_target_label(i18n, target)),
        ),
    }
}

pub(in crate::app) fn parse_settings_field_input(
    _i18n: &I18n,
    field: SettingsFieldSpec,
    input: &str,
) -> std::result::Result<Option<JsonValue>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("clear") {
        return Ok(None);
    }
    match field.kind {
        SettingsFieldKind::String => Ok(Some(JsonValue::String(trimmed.to_string()))),
    }
}

pub(in crate::app) fn provider_studio_provider_rows(
    i18n: &I18n,
    providers: &[ProviderSummaryResource],
) -> Vec<ProviderStudioProviderRow> {
    let mut rows = vec![ProviderStudioProviderRow {
        provider_id: None,
        label: ui_text::t(i18n, "settings-provider-new-label"),
        detail: ui_text::t(i18n, "overlay-provider-studio-new-provider-detail"),
    }];
    rows.extend(providers.iter().map(|provider| ProviderStudioProviderRow {
        provider_id: Some(provider.provider_id.clone()),
        label: provider.provider_id.clone(),
        detail: i18n.text_args(
            "overlay-provider-studio-provider-row-detail",
            &crate::fl_args!(
                "adapter" => provider
                    .defaults
                    .adapter
                    .clone()
                    .unwrap_or_else(|| settings_choice_adapter_fallback(i18n)),
                "model" => provider.defaults.model.clone(),
                "count" => provider.adapters.len() as i64,
            ),
        ),
    }));
    rows
}

pub(in crate::app) fn provider_list_create_item(i18n: &I18n) -> PickerItem {
    PickerItem {
        label: ui_text::t(i18n, "overlay-provider-list-create-label"),
        detail: ui_text::t(i18n, "overlay-provider-list-create-detail"),
        value: PickerValue::ProviderCreate,
    }
}

pub(in crate::app) fn i18n_provider_list_detail(
    i18n: &I18n,
    provider: &ProviderSummaryResource,
) -> String {
    i18n.text_args(
        "overlay-provider-list-row-detail",
        &crate::fl_args!(
            "adapter" => provider
                .defaults
                .adapter
                .clone()
                .unwrap_or_else(|| settings_choice_adapter_fallback(i18n)),
            "model" => provider.defaults.model.clone(),
            "count" => provider.adapters.len() as i64,
        ),
    )
}

pub(in crate::app) fn session_model_choice_item(
    i18n: &I18n,
    provider_id: &str,
    default_adapter: Option<&str>,
    model: ProviderModel,
) -> SessionModelChoiceItem {
    let adapter_id = model
        .adapter_id
        .as_ref()
        .map(ToString::to_string)
        .or_else(|| default_adapter.map(str::to_owned));
    let model_ref = adapter_id
        .as_deref()
        .map(|adapter_id| ModelRef::new_with_adapter(provider_id, adapter_id, model.id.as_ref()))
        .unwrap_or_else(|| ModelRef::new(provider_id, model.id.as_ref()));
    let display_name = model
        .display_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    let adapter_label = adapter_id
        .clone()
        .unwrap_or_else(|| ui_text::t(i18n, "value-default"));
    let context_window = model
        .metadata
        .limits
        .context_window_tokens
        .map(|value| {
            i18n.text_args(
                "session-model-context-window",
                &crate::fl_args!("value" => value as i64),
            )
        })
        .unwrap_or_else(|| {
            i18n.text_args(
                "session-model-context-window",
                &crate::fl_args!("value" => ui_text::t(i18n, "value-unknown")),
            )
        });
    let mut detail_parts = vec![provider_id.to_owned(), adapter_label, context_window];
    if !display_name.is_empty() && display_name != model.id.as_ref() {
        detail_parts.push(display_name.clone());
    }
    let search_text = format!(
        "{} {} {} {}",
        provider_id,
        model_ref
            .adapter_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        model.id,
        display_name
    )
    .to_ascii_lowercase();
    SessionModelChoiceItem {
        label: format!(
            "{provider_id} / {} / {}",
            model_ref
                .adapter_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            model.id
        ),
        detail: join_inline_segments(detail_parts),
        search_text,
        model: model_ref,
        current: false,
    }
}

pub(in crate::app) fn mark_current_session_model_choice(
    i18n: &I18n,
    items: &mut Vec<SessionModelChoiceItem>,
    current_model: Option<&ModelRef>,
) {
    items.iter_mut().for_each(|item| item.current = false);
    let Some(current_model) = current_model else {
        return;
    };
    if let Some(current_item) = items
        .iter()
        .position(|item| session_model_matches_current(&item.model, current_model))
        .and_then(|index| items.get_mut(index))
    {
        current_item.current = true;
        return;
    }

    let adapter_label = current_model
        .adapter_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| ui_text::t(i18n, "value-default"));
    items.insert(
        0,
        SessionModelChoiceItem {
            label: format!(
                "{} / {} / {}",
                current_model.provider_id, adapter_label, current_model.model_id
            ),
            detail: ui_text::t(i18n, "overlay-choice-current-unavailable-detail"),
            search_text: format!(
                "{} {} {}",
                current_model.provider_id, adapter_label, current_model.model_id
            )
            .to_ascii_lowercase(),
            model: current_model.clone(),
            current: true,
        },
    );
}

pub(in crate::app) fn session_model_matches_current(
    candidate: &ModelRef,
    current: &ModelRef,
) -> bool {
    candidate.provider_id == current.provider_id
        && candidate.model_id == current.model_id
        && (candidate.adapter_id == current.adapter_id || current.adapter_id.is_none())
}

pub(in crate::app) fn provider_model_catalog_lookup_id(model: &ProviderModel) -> String {
    model
        .catalog_model_id
        .as_ref()
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| agena::model_catalog::canonical_model_catalog_id(model.id.as_ref()))
}

pub(in crate::app) fn provider_studio_catalog_match_model<'a>(
    model: &ProviderModel,
    catalog_models: &'a [CatalogModelResource],
) -> Option<&'a CatalogModelResource> {
    let lookup_id = provider_model_catalog_lookup_id(model);
    catalog_models
        .iter()
        .filter(|catalog_model| {
            catalog_model.model_id == model.id.as_ref() || catalog_model.model_id == lookup_id
        })
        .min_by_key(|catalog_model| catalog_model.model_id.as_str())
}

use crate::app::{
    BTreeSet, Backend, CatalogModelResource, ChoiceItem, ChoiceOverlayAction, ConfigJsonSources,
    I18n, InspectorRow, JsonValue, ModelCatalogListResponse, ModelRef, PermissionMode,
    PermissionRuleStudioChoiceField, PickerItem, PickerValue, ProviderModel, ProviderStudioField,
    ProviderStudioOverlay, ProviderStudioProviderRow, ProviderSummaryResource,
    SessionModelChoiceItem, SettingsFieldKind, SettingsFieldSpec, SettingsPickerAction,
    SettingsStudioItem, join_inline_segments, provider_model_config_field_label,
    provider_studio_catalog_match_label, provider_studio_field_label, provider_studio_model_key,
    provider_studio_model_selected, provider_studio_selected_adapter_id, ui_text,
};

#[cfg(test)]
mod tests {
    use super::{
        choice_item, mark_current_choice_item, mark_current_session_model_choice,
        session_model_choice_item,
    };
    use crate::app::{ModelRef, ProviderModel};
    use crate::i18n::I18n;
    use agena_tui_components::SearchPickerItem;

    #[test]
    fn current_choice_is_marked_without_filtering_the_catalog() {
        let mut items = vec![
            choice_item("build", "agent"),
            choice_item("review", "agent"),
        ];

        mark_current_choice_item(&I18n::english(), &mut items, Some("build"));

        assert_eq!(items.len(), 2);
        assert!(items[0].current);
        assert!(!items[1].current);
    }

    #[test]
    fn unavailable_current_choice_is_preserved_as_a_visible_row() {
        let mut items = vec![choice_item("review", "agent")];

        mark_current_choice_item(&I18n::english(), &mut items, Some("removed-agent"));

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].value, "removed-agent");
        assert!(items[0].current);
        assert!(items[0].detail.contains("not in the available options"));
    }

    #[test]
    fn current_model_is_marked_on_exactly_one_picker_row() {
        let i18n = I18n::english();
        let mut items = vec![
            session_model_choice_item(
                &i18n,
                "provider-a",
                None,
                ProviderModel::new("adapter-a", "model-a"),
            ),
            session_model_choice_item(
                &i18n,
                "provider-a",
                None,
                ProviderModel::new("adapter-b", "model-a"),
            ),
            session_model_choice_item(
                &i18n,
                "provider-a",
                None,
                ProviderModel::new("adapter-a", "model-b"),
            ),
        ];

        mark_current_session_model_choice(
            &i18n,
            &mut items,
            Some(&ModelRef::new("provider-a", "model-a")),
        );

        assert!(items[0].current);
        assert!(items[1..].iter().all(|item| !item.current));
        assert_eq!(items[0].search_picker_prefix().as_deref(), Some("✓ "),);
    }

    #[test]
    fn unavailable_current_model_remains_visible_and_marked() {
        let i18n = I18n::english();
        let mut items = vec![session_model_choice_item(
            &i18n,
            "provider-a",
            None,
            ProviderModel::new("adapter-a", "model-a"),
        )];
        let current = ModelRef::new_with_adapter("provider-b", "adapter-b", "removed-model");

        mark_current_session_model_choice(&i18n, &mut items, Some(&current));

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].model, current);
        assert!(items[0].current);
        assert!(items[0].detail.contains("not in the available options"));
        assert!(!items[1].current);
    }
}
