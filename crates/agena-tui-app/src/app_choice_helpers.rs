use super::{
    model_mode_display_label, permission_mode_label, permission_mode_token,
    permission_studio_mode_target_label, settings_choice_adapter_fallback,
    settings_field_display_description,
};

pub(crate) fn permission_mode_choice_items(i18n: &I18n) -> Vec<ChoiceItem> {
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

pub(crate) fn settings_studio_provider_workbench_item(
    i18n: &I18n,
    providers: &[ProviderSummaryResource],
) -> SettingsStudioItem<SettingsPickerAction> {
    SettingsStudioItem::new(
        ui_text::t(i18n, "settings-provider-workbench-label"),
        i18n.text_args(
            "settings-provider-workbench-value",
            &agena_tui::fl_args!("count" => providers.len() as i64),
        ),
        ui_text::t(i18n, "settings-provider-workbench-detail"),
        SettingsPickerAction::OpenProviderList,
    )
}

pub(crate) fn settings_studio_model_catalog_items(
    i18n: &I18n,
    response: &ModelCatalogListResponse,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    vec![SettingsStudioItem::new(
        ui_text::t(i18n, "settings-model-catalog-open-label"),
        response.summary.model_count.to_string(),
        ui_text::t(i18n, "settings-model-catalog-open-detail"),
        SettingsPickerAction::OpenModelCatalogWorkbench,
    )]
}

pub(crate) fn settings_studio_file_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
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

pub(crate) fn format_setting_value_inline(value: &JsonValue) -> String {
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

pub(crate) fn setting_value_input_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => String::new(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => value.clone(),
        other => other.to_string(),
    }
}

pub(crate) fn settings_value_edit_prompt(
    i18n: &I18n,
    field: SettingsFieldSpec,
    file_value: &JsonValue,
    effective_value: &JsonValue,
) -> String {
    let mut lines = vec![
        settings_field_display_description(i18n, field),
        i18n.text_args(
            "overlay-settings-detail-path",
            &agena_tui::fl_args!("path" => field.path),
        ),
    ];
    if !file_value.is_null() {
        lines.push(i18n.text_args(
            "overlay-settings-edit-file-value",
            &agena_tui::fl_args!("value" => format_setting_value_inline(file_value)),
        ));
        if file_value != effective_value {
            lines.push(i18n.text_args(
                "overlay-settings-edit-effective-value",
                &agena_tui::fl_args!("value" => format_setting_value_inline(effective_value)),
            ));
        }
    } else {
        lines.push(i18n.text_args(
            "overlay-settings-edit-effective-value",
            &agena_tui::fl_args!("value" => format_setting_value_inline(effective_value)),
        ));
    }
    lines.push(settings_field_help_suffix(i18n, field.kind));
    lines.join("\n")
}

pub(crate) fn settings_field_help_suffix(i18n: &I18n, kind: SettingsFieldKind) -> String {
    match kind {
        SettingsFieldKind::String => ui_text::t(i18n, "overlay-settings-help-string"),
        SettingsFieldKind::Bool => ui_text::t(i18n, "overlay-settings-help-bool"),
        SettingsFieldKind::Integer => ui_text::t(i18n, "overlay-settings-help-integer"),
    }
}

pub(crate) fn choice_item(value: impl Into<String>, detail: impl Into<String>) -> ChoiceItem {
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

pub(crate) fn choice_item_with_value(
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

pub(crate) fn dedupe_choice_items(items: Vec<ChoiceItem>) -> Vec<ChoiceItem> {
    let mut deduped = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        if seen.insert(item.value.clone()) {
            deduped.push(item);
        }
    }
    deduped
}

pub(crate) fn inspector_rows_to_mode_choice_items(
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

pub(crate) fn boolean_choice_items(detail: &str) -> Vec<ChoiceItem> {
    vec![choice_item("true", detail), choice_item("false", detail)]
}

pub(crate) fn provider_studio_profile_choice_items(
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

pub(crate) fn provider_studio_api_key_env_choice_items(i18n: &I18n) -> Vec<ChoiceItem> {
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

pub(crate) fn provider_studio_field_allows_clear(field: ProviderStudioField) -> bool {
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
    )
}

pub(crate) fn choice_overlay_clear_detail(i18n: &I18n, action: &ChoiceOverlayAction) -> String {
    match action {
        ChoiceOverlayAction::SettingsField(field) => i18n.text_args(
            "overlay-choice-clear-settings-detail",
            &agena_tui::fl_args!("field" => field.path),
        ),
        ChoiceOverlayAction::SessionModelMode(step) => i18n.text_args(
            "overlay-choice-clear-runtime-detail",
            &agena_tui::fl_args!(
                "field" => model_mode_display_label(i18n, *step)
            ),
        ),
        ChoiceOverlayAction::ProviderDefaultModelMode { step, .. } => i18n.text_args(
            "overlay-choice-clear-runtime-detail",
            &agena_tui::fl_args!(
                "field" => model_mode_display_label(i18n, *step)
            ),
        ),
        ChoiceOverlayAction::ProviderStudioField(field) => i18n.text_args(
            "overlay-choice-clear-provider-detail",
            &agena_tui::fl_args!("field" => provider_studio_field_label(i18n, *field)),
        ),
        ChoiceOverlayAction::ProviderStudioModelField(field) => i18n.text_args(
            "overlay-choice-clear-provider-detail",
            &agena_tui::fl_args!("field" => provider_model_config_field_label(i18n, *field)),
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
            &agena_tui::fl_args!("field" => permission_studio_mode_target_label(i18n, target)),
        ),
        ChoiceOverlayAction::PermissionStudioAddEntries(_)
        | ChoiceOverlayAction::PermissionStudioAddEntriesMode { .. } => String::new(),
    }
}

pub(crate) fn parse_settings_field_input(
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
                        &agena_tui::fl_args!("field" => field.path),
                    ));
                }
            };
            Ok(Some(JsonValue::Bool(value)))
        }
        SettingsFieldKind::Integer => {
            let value = trimmed.parse::<u64>().map_err(|_| {
                i18n.text_args(
                    "settings-field-parse-integer",
                    &agena_tui::fl_args!("field" => field.path),
                )
            })?;
            Ok(Some(JsonValue::from(value)))
        }
    }
}

pub(crate) fn provider_studio_provider_rows(
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
            &agena_tui::fl_args!(
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

pub(crate) fn provider_list_create_item(
    i18n: &I18n,
) -> (
    agena_tui::selection_picker::SelectionPickerItem,
    SelectionPickerCommand,
) {
    let label = ui_text::t(i18n, "overlay-provider-list-create-label");
    let detail = ui_text::t(i18n, "overlay-provider-list-create-detail");
    (
        agena_tui::selection_picker::SelectionPickerItem::new(
            "action:create-provider",
            label.clone(),
            detail.clone(),
            format!("{label} {detail}"),
        )
        .always_visible(),
        SelectionPickerCommand::ProviderCreate,
    )
}

pub(crate) fn i18n_provider_list_detail(i18n: &I18n, provider: &ProviderSummaryResource) -> String {
    i18n.text_args(
        "overlay-provider-list-row-detail",
        &agena_tui::fl_args!(
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

pub(crate) fn session_model_choice_item(
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
    let identity = SessionModelIdentity::new(provider_id, adapter_id.clone(), model.id.as_ref());
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
    let context_window = model.metadata.limits.context_window_tokens.map(|value| {
        i18n.text_args(
            "session-model-context-window",
            &agena_tui::fl_args!("value" => format_compact_token_count(value as u64)),
        )
    });
    let mut detail_parts = vec![format!("{provider_id} / {adapter_label}")];
    if let Some(context_window) = context_window {
        detail_parts.push(context_window);
    }
    let search_text = format!(
        "{} {} {} {}",
        provider_id,
        identity.adapter_id.as_deref().unwrap_or_default(),
        model.id,
        display_name
    )
    .to_ascii_lowercase();
    SessionModelChoiceItem {
        label: if display_name.is_empty() {
            model.id.to_string()
        } else {
            display_name
        },
        detail: join_inline_segments(detail_parts),
        search_text,
        identity,
        current: false,
    }
}

fn format_compact_token_count(value: u64) -> String {
    fn format_unit(value: u64, unit: u64, suffix: char) -> String {
        if value.is_multiple_of(unit) {
            return format!("{}{suffix}", value / unit);
        }
        let scaled = value as f64 / unit as f64;
        let precision = if scaled < 10.0 {
            2
        } else if scaled < 100.0 {
            1
        } else {
            0
        };
        let formatted = format!("{scaled:.precision$}");
        format!(
            "{}{suffix}",
            formatted.trim_end_matches('0').trim_end_matches('.')
        )
    }

    if value >= 1_000_000 {
        format_unit(value, 1_000_000, 'M')
    } else if value >= 1_000 {
        format_unit(value, 1_000, 'K')
    } else {
        value.to_string()
    }
}

pub(crate) fn mark_current_session_model_choice(
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
        .position(|item| session_model_matches_current(&item.identity, current_model))
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
            label: current_model.model_id.to_string(),
            detail: join_inline_segments([
                format!("{} / {adapter_label}", current_model.provider_id),
                ui_text::t(i18n, "overlay-choice-current-unavailable-detail"),
            ]),
            search_text: format!(
                "{} {} {}",
                current_model.provider_id, adapter_label, current_model.model_id
            )
            .to_ascii_lowercase(),
            identity: SessionModelIdentity::new(
                current_model.provider_id.to_string(),
                current_model.adapter_id.as_ref().map(ToString::to_string),
                current_model.model_id.to_string(),
            ),
            current: true,
        },
    );
}

pub(crate) fn session_model_matches_current(
    candidate: &SessionModelIdentity,
    current: &ModelRef,
) -> bool {
    candidate.provider_id == current.provider_id.as_ref()
        && candidate.model_id == current.model_id.as_ref()
        && (candidate.adapter_id.as_deref()
            == current
                .adapter_id
                .as_ref()
                .map(|adapter_id| adapter_id.as_ref())
            || current.adapter_id.is_none())
}

pub(crate) fn provider_model_catalog_lookup_id(model: &ProviderModelResource) -> String {
    model
        .catalog_model_id
        .as_ref()
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| agena_provider::normalized_catalog_model_id(model.id.as_str()))
}

pub(crate) fn provider_studio_catalog_match_model<'a>(
    model: &ProviderModelResource,
    catalog_models: &'a [CatalogModelResource],
) -> Option<&'a CatalogModelResource> {
    let lookup_id = provider_model_catalog_lookup_id(model);
    catalog_models
        .iter()
        .filter(|catalog_model| {
            catalog_model.model_id == model.id || catalog_model.model_id == lookup_id
        })
        .min_by_key(|catalog_model| catalog_model.model_id.as_str())
}

use crate::{
    BTreeSet, Backend, CatalogModelResource, ChoiceItem, ChoiceOverlayAction, ConfigJsonSources,
    I18n, InspectorRow, JsonValue, ModelCatalogListResponse, ModelRef, PermissionMode,
    PermissionRuleStudioChoiceField, ProviderModel, ProviderModelResource, ProviderStudioField,
    ProviderStudioProviderRow, ProviderSummaryResource, SelectionPickerCommand,
    SessionModelChoiceItem, SessionModelIdentity, SettingsFieldKind, SettingsFieldSpec,
    SettingsPickerAction, SettingsStudioItem, join_inline_segments,
    provider_model_config_field_label, provider_studio_field_label, ui_text,
};

#[cfg(test)]
mod tests {
    use super::{
        format_compact_token_count, mark_current_session_model_choice, session_model_choice_item,
    };
    use crate::{ModelRef, ProviderModel, SessionModelIdentity};
    use agena_tui::i18n::I18n;
    use agena_tui_components::SearchPickerItem;

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
        assert_eq!(
            items[0].identity,
            SessionModelIdentity::new("provider-b", Some("adapter-b".to_owned()), "removed-model")
        );
        assert!(items[0].current);
        assert!(items[0].detail.contains("not in the available options"));
        assert!(!items[1].current);
    }

    #[test]
    fn session_model_rows_show_each_identity_once_and_compact_context() {
        let i18n = I18n::english();
        let mut model = ProviderModel::new("openai_responses", "gpt-5.6-sol");
        model.adapter_id = Some(agena_domain::AdapterId::new("openai_responses"));
        model.display_name = Some("GPT-5.6 Sol".to_owned());
        model.metadata.limits.context_window_tokens = Some(1_050_000);

        let item = session_model_choice_item(&i18n, "oai", None, model);

        assert_eq!(item.label, "GPT-5.6 Sol");
        assert_eq!(
            crate::sanitize_terminal_text(item.detail.as_str()),
            "oai / openai_responses · 1.05M ctx"
        );
        assert!(item.search_text.contains("gpt-5.6-sol"));
    }

    #[test]
    fn compact_token_counts_use_short_units_without_trailing_zeroes() {
        assert_eq!(format_compact_token_count(128_000), "128K");
        assert_eq!(format_compact_token_count(262_144), "262K");
        assert_eq!(format_compact_token_count(1_000_000), "1M");
        assert_eq!(format_compact_token_count(1_048_576), "1.05M");
    }
}
