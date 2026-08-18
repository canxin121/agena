pub(crate) fn settings_field_display_description(i18n: &I18n, field: &SettingsFieldSpec) -> String {
    field
        .description_override
        .clone()
        .unwrap_or_else(|| ui_text::t(i18n, field.description_key))
}

pub(crate) fn settings_field_display_label(i18n: &I18n, field: &SettingsFieldSpec) -> String {
    field
        .label_override
        .clone()
        .unwrap_or_else(|| ui_text::t(i18n, field.label_key))
}

pub(crate) fn settings_field_edit_title(i18n: &I18n, field: &SettingsFieldSpec) -> String {
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

/// Dynamic Interface items for each activity kind in the catalog (built-in
/// plus plugin-contributed). Each kind toggles its default expansion state at
/// `ui.tui.transcript.activity_kinds.<id>`.
pub(crate) fn settings_studio_activity_kind_items(
    i18n: &I18n,
    application: &crate::TuiBackend,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let mut items = Vec::new();
    let kinds = application.activity_kinds();
    for kind in kinds {
        let path = agena_domain::format_json_path(&[
            "ui".to_owned(),
            "tui".to_owned(),
            "transcript".to_owned(),
            "activity_kinds".to_owned(),
            kind.id.clone(),
        ]);
        let label_key = format!("settings-activity-kind-{}-label", kind.id);
        let description_key = format!("settings-activity-kind-{}-description", kind.id);
        // Missing locale keys fall back to the key itself, so resolve now and
        // use the human label (or kind label) as the dynamic override.
        let label = ui_text::t(i18n, &label_key);
        let description = ui_text::t(i18n, &description_key);
        let label_override = if label != label_key {
            Some(label)
        } else {
            Some(kind.label.clone())
        };
        let description_override = if description != description_key {
            Some(description)
        } else {
            Some(kind.label.clone())
        };
        let field = SettingsFieldSpec {
            section: SettingsStudioSectionId::Interface,
            path: path.clone(),
            label_key: "settings-activity-kind-label",
            description_key: "settings-activity-kind-description",
            kind: SettingsFieldKind::Bool,
            label_override,
            description_override,
        };
        let file_value =
            get_json_path(&sources.file, Some(path.as_str())).unwrap_or(JsonValue::Null);
        let effective_value =
            get_json_path(&sources.effective, Some(path.as_str())).unwrap_or(JsonValue::Null);
        let effective_summary = settings_field_effective_summary(&effective_value);
        let current_summary = if file_value.is_null() {
            ui_text::t(i18n, "settings-source-unset")
        } else {
            format_setting_value_inline(&file_value)
        };
        let source_rows = settings_source_rows_for_config_path(
            i18n,
            sources,
            path.as_str(),
            current_summary.clone(),
            effective_summary.clone(),
        );
        items.push(SettingsStudioItem::from_parts(
            settings_field_display_label(i18n, &field),
            effective_summary.clone(),
            settings_field_display_description(i18n, &field),
            Some(path.clone()),
            Some(current_summary),
            Some(effective_summary),
            source_rows,
            SettingsPickerAction::EditField(field.clone()),
        ));
    }
    items
}

/// Dynamic Interface items for every concrete tool in the live registry.
///
/// Tool overrides share the open-ended `activity_kinds` map using a `tool:`
/// selector prefix. Quoted JSON-path segments preserve dotted tool names as a
/// single map key instead of accidentally creating nested configuration.
pub(crate) fn settings_studio_activity_tool_items(
    i18n: &I18n,
    application: &crate::TuiBackend,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let mut tools = agena_domain::ToolApiFunction::ALL
        .into_iter()
        .map(|function| {
            let summary_key = match function {
                agena_domain::ToolApiFunction::List => "settings-tool-api-list-description",
                agena_domain::ToolApiFunction::Search => "settings-tool-api-search-description",
                agena_domain::ToolApiFunction::Help => "settings-tool-api-help-description",
                agena_domain::ToolApiFunction::Tags => "settings-tool-api-tags-description",
                agena_domain::ToolApiFunction::Call => "settings-tool-api-call-description",
                agena_domain::ToolApiFunction::PluginsList => {
                    "settings-tool-api-plugins-list-description"
                }
                agena_domain::ToolApiFunction::PluginsSearch => {
                    "settings-tool-api-plugins-search-description"
                }
                agena_domain::ToolApiFunction::PluginsTags => {
                    "settings-tool-api-plugins-tags-description"
                }
            };
            (function.function_name().to_owned(), i18n.text(summary_key))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for tool in application.permission_tools() {
        let name = tool.name.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        tools.insert(name, tool.summary);
    }

    tools
        .into_iter()
        .map(|(name, summary)| {
            let selector = format!("tool:{name}");
            let path = agena_domain::format_json_path(&[
                "ui".to_owned(),
                "tui".to_owned(),
                "transcript".to_owned(),
                "activity_kinds".to_owned(),
                selector,
            ]);
            let description = if summary.trim().is_empty() {
                ui_text::t(i18n, "settings-field-activity-tool-description")
            } else {
                summary
            };
            let field = SettingsFieldSpec {
                section: SettingsStudioSectionId::Interface,
                path: path.clone(),
                label_key: "settings-field-activity-tool-label",
                description_key: "settings-field-activity-tool-description",
                kind: SettingsFieldKind::Bool,
                label_override: Some(name),
                description_override: Some(description),
            };
            let file_value =
                get_json_path(&sources.file, Some(path.as_str())).unwrap_or(JsonValue::Null);
            let effective_value =
                get_json_path(&sources.effective, Some(path.as_str())).unwrap_or(JsonValue::Null);
            let effective_summary = settings_field_effective_summary(&effective_value);
            let current_summary = if file_value.is_null() {
                ui_text::t(i18n, "settings-source-unset")
            } else {
                format_setting_value_inline(&file_value)
            };
            let source_rows = settings_source_rows_for_config_path(
                i18n,
                sources,
                path.as_str(),
                current_summary.clone(),
                effective_summary.clone(),
            );
            SettingsStudioItem::from_parts(
                settings_field_display_label(i18n, &field),
                effective_summary.clone(),
                settings_field_display_description(i18n, &field),
                Some(path.clone()),
                Some(current_summary),
                Some(effective_summary),
                source_rows,
                SettingsPickerAction::EditField(field),
            )
        })
        .collect()
}

pub(crate) fn settings_studio_field_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    section: SettingsStudioSectionId,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    settings_fields()
        .into_iter()
        .filter(|field| field.section == section)
        .map(|field| {
            let file_value =
                get_json_path(&sources.file, Some(field.path.as_str())).unwrap_or(JsonValue::Null);
            let effective_value = get_json_path(&sources.effective, Some(field.path.as_str()))
                .unwrap_or(JsonValue::Null);
            let effective_summary = settings_field_effective_summary(&effective_value);
            let current_summary = if file_value.is_null() {
                ui_text::t(i18n, "settings-source-unset")
            } else {
                format_setting_value_inline(&file_value)
            };
            let source_rows = settings_source_rows_for_config_path(
                i18n,
                sources,
                field.path.as_str(),
                current_summary.clone(),
                effective_summary.clone(),
            );
            SettingsStudioItem::from_parts(
                settings_field_display_label(i18n, &field),
                effective_summary.clone(),
                settings_field_display_description(i18n, &field),
                Some(field.path.clone()),
                Some(current_summary),
                Some(effective_summary),
                source_rows,
                SettingsPickerAction::EditField(field.clone()),
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
    let mut items = vec![settings_studio_provider_workbench_item(i18n, providers)];
    items.extend(
        settings_studio_field_items(i18n, sources, SettingsStudioSectionId::ModelsProviders)
            .into_iter()
            .map(|item| {
                if item.path.as_deref() == Some("providers.default") {
                    settings_studio_provider_default_item(i18n, sources, providers)
                } else {
                    item
                }
            }),
    );
    items
}

pub(crate) fn settings_studio_provider_default_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    providers: &[ProviderSummaryResource],
) -> SettingsStudioItem<SettingsPickerAction> {
    let field = settings_fields()
        .into_iter()
        .find(|field| field.path == "providers.default")
        .expect("providers.default settings field must exist");
    let file_provider_value =
        get_json_path(&sources.file, Some(field.path.as_str())).unwrap_or(JsonValue::Null);
    let effective_provider_value =
        get_json_path(&sources.effective, Some(field.path.as_str())).unwrap_or(JsonValue::Null);
    let file_value = get_json_path(&sources.file, Some("providers.default_selection"))
        .ok()
        .filter(|value| !value.is_null())
        .unwrap_or_else(|| file_provider_value.clone());
    let effective_value = get_json_path(&sources.effective, Some("providers.default_selection"))
        .ok()
        .filter(|value| !value.is_null())
        .unwrap_or(effective_provider_value);
    let effective_summary = provider_default_selection_summary(i18n, providers, &effective_value);
    let current_summary = if file_provider_value.is_null() && file_value.is_null() {
        ui_text::t(i18n, "settings-source-unset")
    } else {
        provider_default_selection_summary(i18n, providers, &file_value)
    };
    let source_rows = settings_source_rows_for_config_path(
        i18n,
        sources,
        field.path.as_str(),
        current_summary.clone(),
        effective_summary.clone(),
    );
    SettingsStudioItem::from_parts(
        settings_field_display_label(i18n, &field),
        effective_summary.clone(),
        settings_field_display_description(i18n, &field),
        Some(field.path.clone()),
        Some(current_summary),
        Some(effective_summary),
        source_rows,
        SettingsPickerAction::OpenProviderDefaultModelChooser,
    )
}

pub(crate) fn settings_studio_provider_approval_model_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    global_permission: &agena_domain::PermissionConfig,
    effective_permission: &agena_domain::PermissionConfig,
) -> SettingsStudioItem<SettingsPickerAction> {
    let current_summary = global_permission
        .approval_model
        .as_ref()
        .map(approval_model_selection_summary)
        .unwrap_or_else(|| ui_text::t(i18n, "settings-source-unset"));
    let effective_summary = effective_permission
        .approval_model
        .as_ref()
        .map(approval_model_selection_summary)
        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"));
    let source_rows = settings_source_rows_for_config_path(
        i18n,
        sources,
        "permission.approval_model",
        current_summary.clone(),
        effective_summary.clone(),
    );
    SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-field-permission-approval-model-label"),
        effective_summary.clone(),
        ui_text::t(i18n, "settings-field-permission-approval-model-description"),
        Some("permission.approval_model".to_owned()),
        Some(current_summary),
        Some(effective_summary),
        source_rows,
        SettingsPickerAction::OpenPermissionApprovalModelChooser,
    )
}

fn approval_model_selection_summary(selection: &agena_domain::ApprovalModelSelection) -> String {
    let mut route = vec![selection.provider_id.clone()];
    if let Some(adapter_id) = selection
        .adapter_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        route.push(adapter_id.trim().to_owned());
    }
    route.push(selection.model_id.clone());
    let mut variants = Vec::new();
    if let Some(value) = selection.thinking_mode.as_deref() {
        variants.push(format!("think={value}"));
    }
    if let Some(value) = selection.speed_mode.as_deref() {
        variants.push(format!("speed={value}"));
    }
    if let Some(value) = selection.verbosity.as_deref() {
        variants.push(format!("verbosity={value}"));
    }
    if !variants.is_empty() {
        route.push(variants.join(", "));
    }
    route.join(" / ")
}

pub(crate) fn provider_default_selection_summary(
    i18n: &I18n,
    providers: &[ProviderSummaryResource],
    value: &JsonValue,
) -> String {
    if let Ok(selection) =
        serde_json::from_value::<agena_domain::ModelSelectionConfig>(value.clone())
        && selection
            .provider
            .as_deref()
            .is_some_and(|provider| !provider.trim().is_empty())
        && selection
            .model
            .as_deref()
            .is_some_and(|model| !model.trim().is_empty())
    {
        return model_selection_summary(&selection);
    }
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

fn model_selection_summary(selection: &agena_domain::ModelSelectionConfig) -> String {
    let mut route = Vec::new();
    if let Some(provider) = selection
        .provider
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        route.push(provider.trim().to_owned());
    }
    if let Some(adapter) = selection
        .adapter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        route.push(adapter.trim().to_owned());
    }
    if let Some(model) = selection
        .model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        route.push(model.trim().to_owned());
    }
    let mut variants = Vec::new();
    if let Some(value) = selection.thinking_mode.as_deref() {
        variants.push(format!("think={value}"));
    }
    if let Some(value) = selection.speed_mode.as_deref() {
        variants.push(format!("speed={value}"));
    }
    if let Some(value) = selection.verbosity.as_deref() {
        variants.push(format!("verbosity={value}"));
    }
    if !variants.is_empty() {
        route.push(variants.join(", "));
    }
    join_inline_segments(vec![route.join(" / ")])
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
    ConfigJsonSources, I18n, JsonValue, ProviderSummaryResource, SessionModelModeStep,
    SettingsFieldKind, SettingsFieldSpec, SettingsPickerAction, SettingsStudioItem,
    SettingsStudioSectionId, SettingsStudioSourceRow, get_json_path, join_inline_segments,
    settings_fields,
};
use crate::ui_text;
use crate::{format_setting_value_inline, settings_studio_provider_workbench_item};
