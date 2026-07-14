pub(in crate::app) fn settings_field_display_description(
    i18n: &I18n,
    field: SettingsFieldSpec,
) -> String {
    ui_text::t(i18n, field.description_key)
}

pub(in crate::app) fn settings_field_display_label(
    i18n: &I18n,
    field: SettingsFieldSpec,
) -> String {
    ui_text::t(i18n, field.label_key)
}

pub(in crate::app) fn settings_field_edit_title(i18n: &I18n, field: SettingsFieldSpec) -> String {
    format!(
        "{} ({})",
        settings_field_display_label(i18n, field),
        field.path
    )
}

pub(in crate::app) fn runtime_setting_display_label(
    i18n: &I18n,
    field: RuntimeSettingSpec,
) -> String {
    let key = match field.id {
        RuntimeSettingId::ThinkingMode => "settings-runtime-thinking-label",
        RuntimeSettingId::SpeedMode => "settings-runtime-speed-label",
        RuntimeSettingId::Verbosity => "settings-runtime-verbosity-label",
        RuntimeSettingId::ParallelToolCalls => "settings-runtime-parallel-label",
        RuntimeSettingId::Temperature => "settings-runtime-temperature-label",
        RuntimeSettingId::MaxOutput => "settings-runtime-max-output-label",
        RuntimeSettingId::System => "settings-runtime-system-label",
    };
    ui_text::t(i18n, key)
}

pub(in crate::app) fn runtime_setting_display_description(
    i18n: &I18n,
    field: RuntimeSettingSpec,
) -> String {
    let key = match field.id {
        RuntimeSettingId::ThinkingMode => "settings-runtime-thinking-description",
        RuntimeSettingId::SpeedMode => "settings-runtime-speed-description",
        RuntimeSettingId::Verbosity => "settings-runtime-verbosity-description",
        RuntimeSettingId::ParallelToolCalls => "settings-runtime-parallel-description",
        RuntimeSettingId::Temperature => "settings-runtime-temperature-description",
        RuntimeSettingId::MaxOutput => "settings-runtime-max-output-description",
        RuntimeSettingId::System => "settings-runtime-system-description",
    };
    ui_text::t(i18n, key)
}

pub(in crate::app) fn session_model_variant_field(
    step: SessionModelVariantStep,
) -> RuntimeSettingSpec {
    match step {
        SessionModelVariantStep::ThinkingMode => RUNTIME_SETTINGS[0],
        SessionModelVariantStep::SpeedMode => RUNTIME_SETTINGS[1],
        SessionModelVariantStep::Verbosity => RUNTIME_SETTINGS[2],
    }
}

pub(in crate::app) fn settings_choice_adapter_fallback(i18n: &I18n) -> String {
    ui_text::t(i18n, "settings-choice-adapter-fallback")
}

pub(in crate::app) fn settings_choice_default_provider_detail(
    i18n: &I18n,
    adapter: &str,
    model: &str,
) -> String {
    i18n.text_args(
        "settings-choice-default-provider-detail",
        &crate::fl_args!("adapter" => adapter, "model" => model),
    )
}

pub(in crate::app) fn settings_choice_registered_agent_detail(i18n: &I18n) -> String {
    ui_text::t(i18n, "settings-choice-agent-profile-detail")
}

pub(in crate::app) fn settings_choice_bool_override_detail(i18n: &I18n) -> String {
    ui_text::t(i18n, "settings-choice-bool-override")
}

pub(in crate::app) fn runtime_setting_choice_supported_model_detail(i18n: &I18n) -> String {
    ui_text::t(i18n, "runtime-setting-choice-supported-model")
}

pub(in crate::app) fn runtime_setting_choice_parallel_detail(i18n: &I18n) -> String {
    ui_text::t(i18n, "runtime-setting-choice-parallel-detail")
}

pub(in crate::app) fn runtime_setting_override_summary(i18n: &I18n, value: &str) -> String {
    i18n.text_args(
        "runtime-setting-summary-override-value",
        &crate::fl_args!("value" => value),
    )
}

pub(in crate::app) fn settings_layers_summary(sources: &ConfigJsonSources) -> String {
    if sources.applied_layers.is_empty() {
        return "built-in defaults".to_owned();
    }
    sources.applied_layers.join(" -> ")
}

pub(in crate::app) fn settings_config_file_source_summary(
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
        &crate::fl_args!("path" => sources.config_path.display().to_string()),
    )
}

pub(in crate::app) fn settings_workspace_config_file_source_summary(
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
        &crate::fl_args!("path" => sources.project_config_path.display().to_string()),
    )
}

pub(in crate::app) fn settings_source_rows_for_config_path(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    path: &str,
    file_summary: impl Into<String>,
    effective_summary: impl Into<String>,
) -> Vec<SettingsSourceRow> {
    vec![
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-config-file"),
            settings_config_file_source_summary(i18n, sources),
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-file-value"),
            file_summary,
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-effective-value"),
            effective_summary,
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-write-target"),
            format!("{path} -> {}", sources.config_path.display()),
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-layers"),
            settings_layers_summary(sources),
        ),
    ]
}

pub(in crate::app) fn settings_source_rows_for_workspace_config_path(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    path: &str,
    workspace_summary: impl Into<String>,
    effective_summary: impl Into<String>,
) -> Vec<SettingsSourceRow> {
    vec![
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-workspace-config-file"),
            settings_workspace_config_file_source_summary(i18n, sources),
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-workspace-value"),
            workspace_summary,
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-effective-value"),
            effective_summary,
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-write-target"),
            format!("{path} -> {}", sources.project_config_path.display()),
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-layers"),
            settings_layers_summary(sources),
        ),
    ]
}

pub(in crate::app) fn settings_studio_field_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    section: SettingsStudioSectionId,
) -> Vec<SettingsStudioItem> {
    SETTINGS_FIELDS
        .iter()
        .filter(|field| field.section == section)
        .map(|field| {
            let file_value =
                get_json_path(&sources.file, Some(field.path)).unwrap_or(JsonValue::Null);
            let effective_value =
                get_json_path(&sources.effective, Some(field.path)).unwrap_or(JsonValue::Null);
            let effective_summary =
                settings_field_effective_summary(i18n, *field, &effective_value);
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

fn settings_field_effective_summary(
    _i18n: &I18n,
    _field: SettingsFieldSpec,
    value: &JsonValue,
) -> String {
    format_setting_value_inline(value)
}

pub(in crate::app) fn settings_studio_client_version_refresh_item(
    i18n: &I18n,
) -> SettingsStudioItem {
    let versions = agena::provider::provider_client_versions();
    SettingsStudioItem::new(
        ui_text::t(i18n, "settings-client-versions-refresh-label"),
        i18n.text_args(
            "settings-client-versions-refresh-value",
            &crate::fl_args!(
                "codex" => versions.codex,
                "claude" => versions.claude,
                "gemini" => versions.gemini,
            ),
        ),
        ui_text::t(i18n, "settings-client-versions-refresh-description"),
        SettingsPickerAction::RefreshProviderClientVersions,
    )
}

pub(in crate::app) fn settings_studio_provider_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    providers: &[ProviderSummaryResource],
) -> Vec<SettingsStudioItem> {
    let mut items =
        settings_studio_field_items(i18n, sources, SettingsStudioSectionId::ConfigProviders)
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

pub(in crate::app) fn settings_studio_provider_default_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    providers: &[ProviderSummaryResource],
) -> SettingsStudioItem {
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

pub(in crate::app) fn provider_default_selection_summary(
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

pub(in crate::app) fn provider_default_route_summary(
    i18n: &I18n,
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

    let mut parts = vec![route.join(" / ")];
    if let Some(thinking_mode) = provider
        .defaults
        .thinking_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "run-options-summary-thinking",
            &crate::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
        ));
    }
    if let Some(speed_mode) = provider
        .defaults
        .speed_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "run-options-summary-speed",
            &crate::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
        ));
    }
    if let Some(verbosity) = provider
        .defaults
        .verbosity
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "run-options-summary-verbosity",
            &crate::fl_args!("value" => verbosity.to_string()),
        ));
    }
    if let Some(parallel_tool_calls) = provider.defaults.parallel_tool_calls {
        parts.push(i18n.text_args(
            "run-options-summary-parallel-tools",
            &crate::fl_args!(
                "value" => ui_text::t(
                    i18n,
                    if parallel_tool_calls {
                        "value-on"
                    } else {
                        "value-off"
                    },
                )
            ),
        ));
    }
    join_inline_segments(parts)
}

pub(in crate::app) fn provider_defaults_settings_path(provider_id: &str) -> String {
    format!(
        "providers.{}.defaults",
        quoted_settings_segment(provider_id.trim())
    )
}

pub(in crate::app) fn settings_studio_harness_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem> {
    ["harnesses.browser", "harnesses.shell", "harnesses.editor"]
        .into_iter()
        .map(|path| settings_studio_config_path_item(i18n, sources, path))
        .collect()
}

pub(in crate::app) fn settings_studio_config_path_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    path: &str,
) -> SettingsStudioItem {
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

pub(in crate::app) fn settings_config_path_display_label(i18n: &I18n, path: &str) -> String {
    match path {
        "harnesses.browser" => ui_text::t(i18n, "settings-harness-browser-label"),
        "harnesses.shell" => ui_text::t(i18n, "settings-harness-shell-label"),
        "harnesses.editor" => ui_text::t(i18n, "settings-harness-editor-label"),
        _ => path.to_string(),
    }
}

pub(in crate::app) fn settings_studio_runtime_items(
    i18n: &I18n,
    run_options: &RunOptionsState,
) -> Vec<SettingsStudioItem> {
    let runtime_model = run_options
        .model
        .as_ref()
        .map(|model| format!("{}/{}", model.provider_id, model.model_id))
        .unwrap_or_else(|| ui_text::t(i18n, "value-default"));
    let runtime_provider = run_options
        .model
        .as_ref()
        .map(|model| model.provider_id.to_string())
        .unwrap_or_else(|| ui_text::t(i18n, "value-default"));
    let mut items = vec![
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-runtime-provider-override-label"),
            runtime_provider,
            ui_text::t(i18n, "settings-runtime-provider-override-detail"),
            SettingsPickerAction::OpenRuntimeProviderOverride,
        ),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-runtime-model-override-label"),
            runtime_model,
            ui_text::t(i18n, "settings-runtime-model-override-detail"),
            SettingsPickerAction::OpenRuntimeModelOverride,
        ),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-runtime-clear-stack-label"),
            ui_text::t(i18n, "value-reset"),
            ui_text::t(i18n, "settings-runtime-clear-stack-detail"),
            SettingsPickerAction::ClearRuntimeModelStack,
        ),
    ];
    for item in &mut items {
        item.source_rows = vec![SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-write-target"),
            ui_text::t(i18n, "settings-source-current-session-runtime"),
        )];
    }
    items.extend(RUNTIME_SETTINGS.iter().map(|field| {
        let summary = run_options.runtime_setting_summary(i18n, *field);
        SettingsStudioItem::from_parts(
            runtime_setting_display_label(i18n, *field),
            summary.clone(),
            runtime_setting_display_description(i18n, *field),
            None,
            Some(summary.clone()),
            Some(summary),
            vec![SettingsSourceRow::new(
                ui_text::t(i18n, "settings-source-row-write-target"),
                ui_text::t(i18n, "settings-source-current-session-runtime"),
            )],
            SettingsPickerAction::EditRuntimeSetting(*field),
        )
    }));
    items
}
use super::{
    ConfigJsonSources, I18n, JsonValue, ProviderSummaryResource, RUNTIME_SETTINGS, RunOptionsState,
    RuntimeSettingId, RuntimeSettingSpec, SETTINGS_FIELDS, SessionModelVariantStep,
    SettingsFieldSpec, SettingsPickerAction, SettingsSourceRow, SettingsStudioItem,
    SettingsStudioSectionId, get_json_path, join_inline_segments, ui_text,
};
use crate::app::quoted_settings_segment;
use crate::app::{format_setting_value_inline, settings_studio_provider_workbench_item};
