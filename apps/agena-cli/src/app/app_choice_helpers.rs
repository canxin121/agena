use super::{
    permission_mode_label, permission_mode_token, permission_studio_mode_target_label,
    quoted_settings_segment, runtime_setting_display_description, runtime_setting_display_label,
    session_model_variant_field, settings_choice_adapter_fallback,
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

pub(in crate::app) fn runtime_setting_edit_prompt(
    i18n: &I18n,
    field: RuntimeSettingSpec,
    current_summary: &str,
) -> String {
    [
        runtime_setting_display_description(i18n, field),
        i18n.text_args(
            "overlay-runtime-setting-current-value",
            &crate::fl_args!("value" => current_summary.to_string()),
        ),
        settings_field_help_suffix(i18n, field.kind),
    ]
    .join("\n")
}

pub(in crate::app) fn settings_field_help_suffix(i18n: &I18n, kind: SettingsFieldKind) -> String {
    match kind {
        SettingsFieldKind::String => ui_text::t(i18n, "overlay-settings-help-string"),
        SettingsFieldKind::Bool => ui_text::t(i18n, "overlay-settings-help-bool"),
        SettingsFieldKind::Integer => ui_text::t(i18n, "overlay-settings-help-integer"),
        SettingsFieldKind::Float => ui_text::t(i18n, "overlay-settings-help-float"),
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
    }
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

pub(in crate::app) fn inspector_rows_to_choice_items(rows: Vec<InspectorRow>) -> Vec<ChoiceItem> {
    rows.into_iter()
        .map(|row| choice_item(row.label, row.detail))
        .collect()
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
        ChoiceOverlayAction::RuntimeSetting(field) => i18n.text_args(
            "overlay-choice-clear-runtime-detail",
            &crate::fl_args!("field" => runtime_setting_display_label(i18n, *field)),
        ),
        ChoiceOverlayAction::SessionModelVariant(step) => i18n.text_args(
            "overlay-choice-clear-runtime-detail",
            &crate::fl_args!(
                "field" => runtime_setting_display_label(i18n, session_model_variant_field(*step))
            ),
        ),
        ChoiceOverlayAction::ProviderDefaultWizard(_, _) => {
            ui_text::t(i18n, "overlay-choice-clear-provider-default-detail")
        }
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
    i18n: &I18n,
    field: SettingsFieldSpec,
    input: &str,
) -> std::result::Result<Option<JsonValue>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("clear") {
        return Ok(None);
    }
    match field.kind {
        SettingsFieldKind::String => Ok(Some(JsonValue::String(trimmed.to_string()))),
        SettingsFieldKind::Bool => {
            let value = match trimmed.to_ascii_lowercase().as_str() {
                "true" | "on" | "yes" | "1" => true,
                "false" | "off" | "no" | "0" => false,
                _ => {
                    return Err(i18n.text_args(
                        "settings-field-parse-bool",
                        &crate::fl_args!("field" => field.path),
                    ));
                }
            };
            Ok(Some(JsonValue::Bool(value)))
        }
        SettingsFieldKind::Integer => {
            let value = trimmed.parse::<u64>().map_err(|_| {
                i18n.text_args(
                    "settings-field-parse-integer",
                    &crate::fl_args!("field" => field.path),
                )
            })?;
            Ok(Some(JsonValue::from(value)))
        }
        SettingsFieldKind::Float => {
            let value = trimmed.parse::<f64>().map_err(|_| {
                i18n.text_args(
                    "settings-field-parse-float",
                    &crate::fl_args!("field" => field.path),
                )
            })?;
            Ok(Some(JsonValue::from(value)))
        }
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
    }
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
    ProviderStudioOverlay, ProviderStudioProviderRow, ProviderSummaryResource, RuntimeSettingSpec,
    SessionModelChoiceItem, SettingsFieldKind, SettingsFieldSpec, SettingsPickerAction,
    SettingsStudioItem, join_inline_segments, provider_model_config_field_label,
    provider_studio_catalog_match_label, provider_studio_field_label, provider_studio_model_key,
    provider_studio_model_selected, provider_studio_selected_adapter_id, ui_text,
};
