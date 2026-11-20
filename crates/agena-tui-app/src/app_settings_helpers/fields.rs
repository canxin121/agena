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
    let projection = super::SettingsFieldProjection::at_path(sources, path, true);
    vec![
        SettingsStudioSourceRow::new(
            i18n.text("settings-layer-source-title"),
            projection.source_label(i18n),
        ),
        SettingsStudioSourceRow::new(
            projection.fallback_label(i18n),
            super::settings_layer_value_label(i18n, projection.inherited.as_ref()),
        ),
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
/// `ui.transcript.activity.kinds.<id>`.
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
            "transcript".to_owned(),
            "activity".to_owned(),
            "kinds".to_owned(),
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
            explicit_local: false,
            workspace: false,
            section: SettingsStudioSectionId::Interface,
            path: path.clone(),
            label_key: "settings-activity-kind-label",
            description_key: "settings-activity-kind-description",
            kind: SettingsFieldKind::Bool,
            label_override,
            description_override,
        };
        let projection = super::SettingsFieldProjection::new(sources, &field);
        let effective_summary =
            super::settings_layer_value_label(i18n, projection.effective.as_ref());
        let current_summary = super::settings_layer_value_label(i18n, projection.local.as_ref());
        let source_rows = settings_source_rows_for_config_path(
            i18n,
            sources,
            path.as_str(),
            current_summary.clone(),
            effective_summary.clone(),
        );
        items.push(
            SettingsStudioItem::from_parts(
                settings_field_display_label(i18n, &field),
                super::SettingsFieldProjection::new(sources, &field).row_value(i18n),
                settings_field_display_description(i18n, &field),
                Some(path.clone()),
                Some(current_summary),
                Some(effective_summary),
                source_rows,
                SettingsPickerAction::EditField(field.clone()),
            )
            .with_control(super::SettingsFieldProjection::new(sources, &field).control(i18n)),
        );
    }
    items
}

/// Dynamic Interface items for every concrete tool in the live registry.
///
/// Tool overrides live beside activity-kind defaults in the shared UI
/// transcript preference object. Quoted JSON-path segments preserve dotted
/// tool names as a single map key.
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
                agena_domain::ToolApiFunction::Help => "settings-tool-api-help-description",
                agena_domain::ToolApiFunction::Call => "settings-tool-api-call-description",
                agena_domain::ToolApiFunction::PluginsList => {
                    "settings-tool-api-plugins-list-description"
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
            let path = agena_domain::format_json_path(&[
                "ui".to_owned(),
                "transcript".to_owned(),
                "tools".to_owned(),
                "overrides".to_owned(),
                name.clone(),
            ]);
            let description = if summary.trim().is_empty() {
                ui_text::t(i18n, "settings-field-activity-tool-description")
            } else {
                summary
            };
            let field = SettingsFieldSpec {
                explicit_local: false,
                workspace: false,
                section: SettingsStudioSectionId::Interface,
                path: path.clone(),
                label_key: "settings-field-activity-tool-label",
                description_key: "settings-field-activity-tool-description",
                kind: SettingsFieldKind::Bool,
                label_override: Some(name),
                description_override: Some(description),
            };
            let projection = super::SettingsFieldProjection::new(sources, &field);
            let effective_summary =
                super::settings_layer_value_label(i18n, projection.effective.as_ref());
            let current_summary =
                super::settings_layer_value_label(i18n, projection.local.as_ref());
            let source_rows = settings_source_rows_for_config_path(
                i18n,
                sources,
                path.as_str(),
                current_summary.clone(),
                effective_summary.clone(),
            );
            SettingsStudioItem::from_parts(
                settings_field_display_label(i18n, &field),
                super::SettingsFieldProjection::new(sources, &field).row_value(i18n),
                settings_field_display_description(i18n, &field),
                Some(path.clone()),
                Some(current_summary),
                Some(effective_summary),
                source_rows,
                SettingsPickerAction::EditField(field.clone()),
            )
            .with_control(super::SettingsFieldProjection::new(sources, &field).control(i18n))
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
            let projection = super::SettingsFieldProjection::new(sources, &field);
            let effective_summary =
                super::settings_layer_value_label(i18n, projection.effective.as_ref());
            let display_value = projection.row_value(i18n);
            let current_summary =
                super::settings_layer_value_label(i18n, projection.local.as_ref());
            let source_rows = if field.workspace {
                settings_source_rows_for_workspace_config_path(
                    i18n,
                    sources,
                    field.path.as_str(),
                    current_summary.clone(),
                    effective_summary.clone(),
                )
            } else {
                settings_source_rows_for_config_path(
                    i18n,
                    sources,
                    field.path.as_str(),
                    current_summary.clone(),
                    effective_summary.clone(),
                )
            };
            SettingsStudioItem::from_parts(
                settings_field_display_label(i18n, &field),
                display_value,
                settings_field_display_description(i18n, &field),
                Some(field.path.clone()),
                Some(current_summary),
                Some(effective_summary),
                source_rows,
                SettingsPickerAction::EditField(field.clone()),
            )
            .with_control(super::SettingsFieldProjection::new(sources, &field).control(i18n))
        })
        .collect()
}

pub(crate) fn settings_studio_provider_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    providers: &[ProviderSummaryResource],
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let mut items = vec![settings_studio_provider_workbench_item(i18n, providers)];
    items.push(settings_studio_global_default_model_item(i18n, sources));
    items.push(settings_studio_workspace_default_model_item(i18n, sources));
    for workspace in [false, true] {
        let path = "runtime.auxiliary_model";
        let source = if workspace {
            &sources.project_file
        } else {
            &sources.file
        };
        let value = get_json_path(source, Some(path)).unwrap_or(JsonValue::Null);
        let effective = get_json_path(&sources.effective, Some(path)).unwrap_or(JsonValue::Null);
        let current = if value.is_null() {
            ui_text::t(i18n, "settings-source-unset")
        } else {
            global_default_selection_summary(i18n, &value)
        };
        let summary = global_default_selection_summary(i18n, &effective);
        let rows = if workspace {
            settings_source_rows_for_workspace_config_path(
                i18n,
                sources,
                path,
                current.clone(),
                summary.clone(),
            )
        } else {
            settings_source_rows_for_config_path(
                i18n,
                sources,
                path,
                current.clone(),
                summary.clone(),
            )
        };
        items.push(
            SettingsStudioItem::from_parts(
                ui_text::t(
                    i18n,
                    if workspace {
                        "settings-workspace-auxiliary-model-label"
                    } else {
                        "settings-auxiliary-model-label"
                    },
                ),
                settings_model_layer_summary(i18n, sources, path, workspace),
                ui_text::t(i18n, "settings-auxiliary-model-description"),
                Some(path.into()),
                Some(current),
                Some(summary),
                rows,
                SettingsPickerAction::OpenAuxiliaryModelChooser { workspace },
            )
            .with_control(
                super::SettingsFieldProjection::at_path(sources, path, workspace).control(i18n),
            ),
        );
    }
    items.extend(
        settings_studio_field_items(i18n, sources, SettingsStudioSectionId::ModelsProviders)
            .into_iter()
            .collect::<Vec<_>>(),
    );
    items
}

pub(crate) fn settings_studio_global_default_model_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> SettingsStudioItem<SettingsPickerAction> {
    let file_value = get_json_path(&sources.file, Some("providers.default_selection"))
        .unwrap_or(JsonValue::Null);
    let effective_value = get_json_path(&sources.effective, Some("providers.default_selection"))
        .unwrap_or(JsonValue::Null);
    let effective_summary = global_default_selection_summary(i18n, &effective_value);
    let current_summary = if file_value.is_null() {
        ui_text::t(i18n, "settings-source-unset")
    } else {
        global_default_selection_summary(i18n, &file_value)
    };
    let source_rows = settings_source_rows_for_config_path(
        i18n,
        sources,
        "providers.default_selection",
        current_summary.clone(),
        effective_summary.clone(),
    );
    SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-global-default-model-label"),
        settings_model_layer_summary(i18n, sources, "providers.default_selection", false),
        ui_text::t(i18n, "settings-global-default-model-description"),
        Some("providers.default_selection".to_owned()),
        Some(current_summary),
        Some(effective_summary),
        source_rows,
        SettingsPickerAction::OpenGlobalDefaultModelChooser,
    )
    .with_control(
        super::SettingsFieldProjection::at_path(sources, "providers.default_selection", false)
            .control(i18n),
    )
}

/// Workspace (project) layer of the default model. Kept beside the global
/// item so both layers are visible and independently editable, mirroring the
/// permission studio's global/workspace entries.
pub(crate) fn settings_studio_workspace_default_model_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> SettingsStudioItem<SettingsPickerAction> {
    let workspace_value = get_json_path(&sources.project_file, Some("providers.default_selection"))
        .unwrap_or(JsonValue::Null);
    let effective_value = get_json_path(&sources.effective, Some("providers.default_selection"))
        .unwrap_or(JsonValue::Null);
    let effective_summary = global_default_selection_summary(i18n, &effective_value);
    let workspace_summary = if workspace_value.is_null() {
        ui_text::t(i18n, "settings-source-unset")
    } else {
        global_default_selection_summary(i18n, &workspace_value)
    };
    let source_rows = settings_source_rows_for_workspace_config_path(
        i18n,
        sources,
        "providers.default_selection",
        workspace_summary.clone(),
        effective_summary.clone(),
    );
    SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-workspace-default-model-label"),
        settings_model_layer_summary(i18n, sources, "providers.default_selection", true),
        ui_text::t(i18n, "settings-workspace-default-model-description"),
        Some("providers.default_selection".to_owned()),
        Some(workspace_summary),
        Some(effective_summary),
        source_rows,
        SettingsPickerAction::OpenWorkspaceDefaultModelChooser,
    )
    .with_control(
        super::SettingsFieldProjection::at_path(sources, "providers.default_selection", true)
            .control(i18n),
    )
}

fn settings_model_layer_summary(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    path: &str,
    workspace: bool,
) -> String {
    let projection = super::SettingsFieldProjection::at_path(sources, path, workspace);
    global_default_selection_summary(i18n, projection.value.as_ref().unwrap_or(&JsonValue::Null))
}

fn global_default_selection_summary(i18n: &I18n, value: &JsonValue) -> String {
    let Ok(selection) = serde_json::from_value::<agena_domain::ModelSelectionConfig>(value.clone())
    else {
        return format_setting_value_inline(value);
    };
    let Some(provider) = selection
        .provider
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return if value.is_null() {
            ui_text::t(i18n, "value-unset")
        } else {
            format_setting_value_inline(value)
        };
    };
    let Some(model) = selection
        .model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return format_setting_value_inline(value);
    };
    let mut route = vec![provider.trim().to_owned()];
    if let Some(adapter) = selection
        .adapter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        route.push(adapter.trim().to_owned());
    }
    route.push(model.trim().to_owned());
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
        settings_model_layer_summary(i18n, sources, "permission.approval_model", false),
        ui_text::t(i18n, "settings-field-permission-approval-model-description"),
        Some("permission.approval_model".to_owned()),
        Some(current_summary),
        Some(effective_summary),
        source_rows,
        SettingsPickerAction::OpenPermissionApprovalModelChooser,
    )
    .with_control(
        super::SettingsFieldProjection::at_path(sources, "permission.approval_model", false)
            .control(i18n),
    )
}

/// Workspace (project) layer of the automatic approval model.
pub(crate) fn settings_studio_workspace_provider_approval_model_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    workspace_permission: &agena_domain::PermissionConfig,
    effective_permission: &agena_domain::PermissionConfig,
) -> SettingsStudioItem<SettingsPickerAction> {
    let workspace_summary = workspace_permission
        .approval_model
        .as_ref()
        .map(approval_model_selection_summary)
        .unwrap_or_else(|| ui_text::t(i18n, "settings-source-unset"));
    let effective_summary = effective_permission
        .approval_model
        .as_ref()
        .map(approval_model_selection_summary)
        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"));
    let source_rows = settings_source_rows_for_workspace_config_path(
        i18n,
        sources,
        "permission.approval_model",
        workspace_summary.clone(),
        effective_summary.clone(),
    );
    SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-workspace-approval-model-label"),
        settings_model_layer_summary(i18n, sources, "permission.approval_model", true),
        ui_text::t(i18n, "settings-workspace-approval-model-description"),
        Some("permission.approval_model".to_owned()),
        Some(workspace_summary),
        Some(effective_summary),
        source_rows,
        SettingsPickerAction::OpenWorkspacePermissionApprovalModelChooser,
    )
    .with_control(
        super::SettingsFieldProjection::at_path(sources, "permission.approval_model", true)
            .control(i18n),
    )
}

fn approval_model_selection_summary(selection: &agena_domain::ApprovalModelSelection) -> String {
    let mut route = vec![selection.provider.clone()];
    if let Some(adapter_id) = selection
        .adapter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        route.push(adapter_id.trim().to_owned());
    }
    route.push(selection.model.clone());
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

use super::{
    ConfigJsonSources, I18n, JsonValue, ProviderSummaryResource, SessionModelModeStep,
    SettingsFieldKind, SettingsFieldSpec, SettingsPickerAction, SettingsStudioItem,
    SettingsStudioSectionId, SettingsStudioSourceRow, get_json_path, settings_fields,
};
use crate::ui_text;
use crate::{format_setting_value_inline, settings_studio_provider_workbench_item};
