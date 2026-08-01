pub(crate) fn settings_field_display_description(i18n: &I18n, field: SettingsFieldSpec) -> String {
    ui_text::t(i18n, field.description_key)
}

pub(crate) fn settings_field_display_label(i18n: &I18n, field: SettingsFieldSpec) -> String {
    ui_text::t(i18n, field.label_key)
}

pub(crate) fn settings_field_edit_title(i18n: &I18n, field: SettingsFieldSpec) -> String {
    format!(
        "{} ({})",
        settings_field_display_label(i18n, field),
        field.path
    )
}

pub(crate) fn model_mode_display_label(i18n: &I18n, step: SessionModelModeStep) -> String {
    let key = match step {
        SessionModelModeStep::ThinkingMode => "settings-runtime-thinking-label",
        SessionModelModeStep::SpeedMode => "settings-runtime-speed-label",
        SessionModelModeStep::Verbosity => "settings-runtime-verbosity-label",
    };
    ui_text::t(i18n, key)
}

pub(crate) fn model_mode_display_description(i18n: &I18n, step: SessionModelModeStep) -> String {
    let key = match step {
        SessionModelModeStep::ThinkingMode => "settings-runtime-thinking-description",
        SessionModelModeStep::SpeedMode => "settings-runtime-speed-description",
        SessionModelModeStep::Verbosity => "settings-runtime-verbosity-description",
    };
    ui_text::t(i18n, key)
}

pub(crate) fn settings_choice_adapter_fallback(i18n: &I18n) -> String {
    ui_text::t(i18n, "settings-choice-adapter-fallback")
}

pub(crate) fn settings_choice_default_provider_detail(
    i18n: &I18n,
    adapter: &str,
    model: &str,
) -> String {
    i18n.text_args(
        "settings-choice-default-provider-detail",
        &agena_tui::fl_args!("adapter" => adapter, "model" => model),
    )
}

pub(crate) fn runtime_setting_choice_supported_model_detail(i18n: &I18n) -> String {
    ui_text::t(i18n, "runtime-setting-choice-supported-model")
}

pub(crate) fn settings_layers_summary(sources: &ConfigJsonSources) -> String {
    if sources.applied_layers.is_empty() {
        return "built-in defaults".to_owned();
    }
    sources.applied_layers.join(" -> ")
}

pub(crate) fn settings_config_file_source_summary(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> String {
    let status_key = if sources.config_found {
        "settings-source-file-found"
    } else {
        "settings-source-file-missing"
    };
    i18n.text_args(
        status_key,
        &agena_tui::fl_args!("path" => sources.config_path.display().to_string()),
    )
}

pub(crate) fn settings_workspace_config_file_source_summary(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> String {
    let status_key = if sources.project_config_found {
        "settings-source-file-found"
    } else {
        "settings-source-file-missing"
    };
    i18n.text_args(
        status_key,
        &agena_tui::fl_args!("path" => sources.project_config_path.display().to_string()),
    )
}

pub(crate) fn settings_source_rows_for_config_path(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    path: &str,
    file_summary: impl Into<String>,
    effective_summary: impl Into<String>,
) -> Vec<SettingsStudioSourceRow> {
    vec![
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-config-file"),
            settings_config_file_source_summary(i18n, sources),
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-file-value"),
            file_summary,
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-effective-value"),
            effective_summary,
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-write-target"),
            format!("{path} -> {}", sources.config_path.display()),
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-layers"),
            settings_layers_summary(sources),
        ),
    ]
}

pub(crate) fn settings_source_rows_for_workspace_config_path(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    path: &str,
    workspace_summary: impl Into<String>,
    effective_summary: impl Into<String>,
) -> Vec<SettingsStudioSourceRow> {
    vec![
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-workspace-config-file"),
            settings_workspace_config_file_source_summary(i18n, sources),
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-workspace-value"),
            workspace_summary,
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-effective-value"),
            effective_summary,
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-write-target"),
            format!("{path} -> {}", sources.project_config_path.display()),
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-source-row-layers"),
            settings_layers_summary(sources),
        ),
    ]
}

pub(crate) fn settings_studio_field_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    section: SettingsStudioSectionId,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    SETTINGS_FIELDS
        .iter()
        .filter(|field| field.section == section)
        .map(|field| {
            let file_value =
                get_json_path(&sources.file, Some(field.path)).unwrap_or(JsonValue::Null);
            let effective_value =
                get_json_path(&sources.effective, Some(field.path)).unwrap_or(JsonValue::Null);
            let effective_summary = settings_field_effective_summary(&effective_value);
            let current_summary = if file_value.is_null() {
                ui_text::t(i18n, "settings-source-unset")
            } else {
                format_setting_value_inline(&file_value)
            };
            let source_rows = settings_source_rows_for_config_path(
                i18n,
                sources,
                field.path,
                current_summary.clone(),
                effective_summary.clone(),
            );
            SettingsStudioItem::from_parts(
                settings_field_display_label(i18n, *field),
                effective_summary.clone(),
                settings_field_display_description(i18n, *field),
                Some(field.path.to_string()),
                Some(current_summary),
                Some(effective_summary),
                source_rows,
                SettingsPickerAction::EditField(*field),
            )
        })
        .collect()
}

fn settings_field_effective_summary(value: &JsonValue) -> String {
    format_setting_value_inline(value)
}

pub(crate) fn settings_studio_provider_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    providers: &[ProviderSummaryResource],
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let mut items =
        settings_studio_field_items(i18n, sources, SettingsStudioSectionId::ModelsProviders)
            .into_iter()
            .map(|item| {
                if item.path.as_deref() == Some("providers.default") {
                    settings_studio_provider_default_item(i18n, sources, providers)
                } else {
                    item
                }
            })
            .collect::<Vec<_>>();
    items.push(settings_studio_provider_workbench_item(i18n, providers));
    items
}

pub(crate) fn settings_studio_provider_default_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    providers: &[ProviderSummaryResource],
) -> SettingsStudioItem<SettingsPickerAction> {
    let field = SETTINGS_FIELDS
        .iter()
        .find(|field| field.path == "providers.default")
        .copied()
        .expect("providers.default settings field must exist");
    let file_value = get_json_path(&sources.file, Some(field.path)).unwrap_or(JsonValue::Null);
    let effective_value =
        get_json_path(&sources.effective, Some(field.path)).unwrap_or(JsonValue::Null);
    let effective_summary = provider_default_selection_summary(i18n, providers, &effective_value);
    let current_summary = if file_value.is_null() {
        ui_text::t(i18n, "settings-source-unset")
    } else {
        provider_default_selection_summary(i18n, providers, &file_value)
    };
    let source_rows = settings_source_rows_for_config_path(
        i18n,
        sources,
        field.path,
        current_summary.clone(),
        effective_summary.clone(),
    );
    SettingsStudioItem::from_parts(
        settings_field_display_label(i18n, field),
        effective_summary.clone(),
        settings_field_display_description(i18n, field),
        Some(field.path.to_string()),
        Some(current_summary),
        Some(effective_summary),
        source_rows,
        SettingsPickerAction::OpenProviderDefaultModelChooser,
    )
}

pub(crate) fn provider_default_selection_summary(
    i18n: &I18n,
    providers: &[ProviderSummaryResource],
    value: &JsonValue,
) -> String {
    let Some(provider_id) = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return format_setting_value_inline(value);
    };
    providers
        .iter()
        .find(|provider| provider.provider_id == provider_id)
        .map(|provider| provider_default_route_summary(i18n, provider))
        .unwrap_or_else(|| provider_id.to_owned())
}

pub(crate) fn provider_default_route_summary(
    _i18n: &I18n,
    provider: &ProviderSummaryResource,
) -> String {
    let mut route = vec![provider.provider_id.clone()];
    if let Some(adapter) = provider
        .defaults
        .adapter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        route.push(adapter.trim().to_owned());
    }
    if !provider.defaults.model.trim().is_empty() {
        route.push(provider.defaults.model.trim().to_owned());
    }

    join_inline_segments(vec![route.join(" / ")])
}

pub(crate) fn provider_defaults_settings_path(provider_id: &str) -> String {
    format!(
        "providers.{}.defaults",
        quoted_settings_segment(provider_id.trim())
    )
}

pub(crate) fn settings_studio_harness_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    ["harnesses.browser", "harnesses.shell", "harnesses.editor"]
        .into_iter()
        .map(|path| settings_studio_config_path_item(i18n, sources, path))
        .collect()
}

pub(crate) fn settings_studio_config_path_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    path: &str,
) -> SettingsStudioItem<SettingsPickerAction> {
    let file_value = get_json_path(&sources.file, Some(path)).unwrap_or(JsonValue::Null);
    let effective_value = get_json_path(&sources.effective, Some(path)).unwrap_or(JsonValue::Null);
    let effective_summary = format_setting_value_inline(&effective_value);
    let current_summary = if file_value.is_null() {
        ui_text::t(i18n, "settings-source-unset")
    } else {
        format_setting_value_inline(&file_value)
    };
    let source_rows = settings_source_rows_for_config_path(
        i18n,
        sources,
        path,
        current_summary.clone(),
        effective_summary.clone(),
    );
    SettingsStudioItem::from_parts(
        settings_config_path_display_label(i18n, path),
        effective_summary.clone(),
        ui_text::t(i18n, "settings-config-open-file-detail"),
        Some(path.to_string()),
        Some(current_summary),
        Some(effective_summary),
        source_rows,
        SettingsPickerAction::OpenConfigFile,
    )
}

pub(crate) fn settings_config_path_display_label(i18n: &I18n, path: &str) -> String {
    match path {
        "harnesses.browser" => ui_text::t(i18n, "settings-harness-browser-label"),
        "harnesses.shell" => ui_text::t(i18n, "settings-harness-shell-label"),
        "harnesses.editor" => ui_text::t(i18n, "settings-harness-editor-label"),
        _ => path.to_string(),
    }
}

use super::{
    ConfigJsonSources, I18n, JsonValue, ProviderSummaryResource, SETTINGS_FIELDS,
    SessionModelModeStep, SettingsFieldSpec, SettingsPickerAction, SettingsStudioItem,
    SettingsStudioSectionId, SettingsStudioSourceRow, get_json_path, join_inline_segments,
};
use crate::quoted_settings_segment;
use crate::ui_text;
use crate::{format_setting_value_inline, settings_studio_provider_workbench_item};
