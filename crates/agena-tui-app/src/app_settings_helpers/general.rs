use super::{
    settings_source_rows_for_config_path, settings_source_rows_for_workspace_config_path,
    settings_studio_field_items,
};
use crate::{
    ConfigJsonSources, I18n, PermissionConfig, SessionPermissionStudioState, SettingsPickerAction,
    SettingsStudioItem, SettingsStudioSectionId, SettingsStudioSourceRow,
    permission_override_summary, ui_text,
};

pub(crate) fn quoted_settings_segment(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn settings_studio_plugin_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let mut items =
        settings_studio_field_items(i18n, sources, SettingsStudioSectionId::PluginsTools);
    items.push(SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-plugin-workbench-label"),
        ui_text::t(i18n, "value-open"),
        ui_text::t(i18n, "settings-plugin-workbench-detail"),
        None,
        None,
        None,
        Vec::new(),
        SettingsPickerAction::OpenPluginWorkbench,
    ));
    items
}

pub(crate) fn permission_layer_source_rows(
    i18n: &I18n,
    global_permission: &PermissionConfig,
    workspace_permission: &PermissionConfig,
    session: Option<&SessionPermissionStudioState>,
) -> Vec<SettingsStudioSourceRow> {
    let mut rows = vec![
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-global"),
            permission_override_summary(i18n, global_permission),
        ),
        SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-workspace"),
            permission_override_summary(i18n, workspace_permission),
        ),
    ];
    if let Some(session) = session {
        rows.push(SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-session"),
            permission_override_summary(i18n, &session.permission),
        ));
        rows.push(SettingsStudioSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-effective"),
            permission_override_summary(i18n, &session.effective_permission),
        ));
    }
    rows
}

pub(crate) fn settings_studio_permission_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    global_permission: &PermissionConfig,
    workspace_permission: &PermissionConfig,
    effective_permission: &PermissionConfig,
    current_session: Option<&SessionPermissionStudioState>,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let mut items = Vec::new();
    if let Some(session) = current_session {
        let effective_summary = permission_override_summary(i18n, &session.effective_permission);
        items.push(SettingsStudioItem::from_parts(
            ui_text::t(i18n, "settings-permission-effective-label"),
            effective_summary.clone(),
            i18n.text_args(
                "settings-permission-effective-detail",
                &agena_tui::fl_args!("session" => session.session_title.clone()),
            ),
            None,
            Some(effective_summary.clone()),
            Some(effective_summary),
            permission_layer_source_rows(
                i18n,
                global_permission,
                workspace_permission,
                Some(session),
            ),
            SettingsPickerAction::OpenSessionEffectivePermissionView(session.session_id),
        ));
        let session_summary = permission_override_summary(i18n, &session.permission);
        let session_effective_summary =
            permission_override_summary(i18n, &session.effective_permission);
        let session_source_rows = {
            let mut rows = permission_layer_source_rows(
                i18n,
                global_permission,
                workspace_permission,
                Some(session),
            );
            rows.push(SettingsStudioSourceRow::new(
                ui_text::t(i18n, "settings-source-row-write-target"),
                ui_text::t(i18n, "settings-source-current-session"),
            ));
            rows
        };
        items.push(SettingsStudioItem::from_parts(
            ui_text::t(i18n, "settings-permission-current-label"),
            session_summary.clone(),
            i18n.text_args(
                "settings-permission-current-detail",
                &agena_tui::fl_args!("session" => session.session_title.clone()),
            ),
            None,
            Some(session_summary.clone()),
            Some(session_effective_summary),
            session_source_rows,
            SettingsPickerAction::OpenCurrentSessionPermissionWorkbench,
        ));
    }

    let global_summary = permission_override_summary(i18n, global_permission);
    let workspace_summary = permission_override_summary(i18n, workspace_permission);
    let effective_summary = permission_override_summary(i18n, effective_permission);
    let global_source_rows = settings_source_rows_for_config_path(
        i18n,
        sources,
        "permission",
        global_summary.clone(),
        effective_summary.clone(),
    );
    items.push(SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-permission-global-label"),
        global_summary.clone(),
        ui_text::t(i18n, "settings-permission-global-detail"),
        Some("permission".to_string()),
        Some(global_summary),
        Some(effective_summary),
        global_source_rows,
        SettingsPickerAction::OpenGlobalPermissionWorkbench,
    ));
    let workspace_effective_summary = permission_override_summary(i18n, effective_permission);
    let workspace_source_rows = settings_source_rows_for_workspace_config_path(
        i18n,
        sources,
        "permission",
        workspace_summary.clone(),
        workspace_effective_summary.clone(),
    );
    items.push(SettingsStudioItem::from_parts(
        ui_text::t(i18n, "settings-permission-workspace-label"),
        workspace_summary.clone(),
        ui_text::t(i18n, "settings-permission-workspace-detail"),
        Some("permission".to_string()),
        Some(workspace_summary),
        Some(workspace_effective_summary),
        workspace_source_rows,
        SettingsPickerAction::OpenWorkspacePermissionWorkbench,
    ));
    items
}
