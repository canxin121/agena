use super::{
    settings_source_rows_for_config_path, settings_source_rows_for_workspace_config_path,
    settings_studio_field_items,
};
use crate::{
    ConfigJsonSources, I18n, PermissionConfig, SessionPermissionStudioState, SettingsPickerAction,
    SettingsStudioItem, SettingsStudioSectionId, SettingsStudioSourceRow,
    permission_override_summary, ui_text,
};

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

pub(crate) fn settings_studio_mcp_items(
    i18n: &I18n,
    application: &crate::TuiBackend,
) -> Vec<SettingsStudioItem<SettingsPickerAction>> {
    let control = application.cached_mcp_server_control();
    let enabled = control
        .as_ref()
        .and_then(|value| value.get("enabled"))
        .and_then(serde_json::Value::as_bool);
    let auth_mode = control
        .as_ref()
        .and_then(|value| value.get("authMode"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            control
                .as_ref()
                .and_then(|value| value.get("authEnabled"))
                .and_then(serde_json::Value::as_bool)
                .map(|enabled| if enabled { "oauth" } else { "none" })
        })
        .unwrap_or("none");
    let auth_enabled = auth_mode != "none";
    let anonymous_access = control
        .as_ref()
        .and_then(|value| value.get("anonymousAccess"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let ready = control
        .as_ref()
        .and_then(|value| value.get("ready"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let warnings = control
        .as_ref()
        .and_then(|value| value.get("warnings"))
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();
    let configured_url = control
        .as_ref()
        .and_then(|value| value.get("publicUrl"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let resource_url = control
        .as_ref()
        .and_then(|value| value.get("resourceUrl"))
        .and_then(serde_json::Value::as_str);
    let configured_issuer = control
        .as_ref()
        .and_then(|value| value.get("oauthIssuerUrl"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let client_registration = control
        .as_ref()
        .and_then(|value| value.get("clientRegistration"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("cimd_only");
    let password_status = control
        .as_ref()
        .and_then(|value| value.get("oauth"))
        .and_then(|value| value.get("passwordConfigured"))
        .and_then(serde_json::Value::as_bool)
        .map(|configured| {
            if configured {
                ui_text::t(i18n, "settings-mcp-oauth-password-configured")
            } else if control
                .as_ref()
                .and_then(|value| value.get("oauth"))
                .and_then(|value| value.get("fallbackToUiPassword"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                ui_text::t(i18n, "settings-mcp-oauth-password-ui-fallback")
            } else {
                ui_text::t(i18n, "settings-mcp-oauth-password-not-configured")
            }
        })
        .unwrap_or_else(|| ui_text::t(i18n, "settings-mcp-status-unavailable"));
    let readiness_status = if ready {
        ui_text::t(i18n, "settings-mcp-ready")
    } else if control.is_some() {
        let label = ui_text::t(i18n, "settings-mcp-needs-attention");
        if warnings.is_empty() {
            label
        } else {
            format!("{label}: {warnings}")
        }
    } else {
        ui_text::t(i18n, "settings-mcp-status-unavailable")
    };

    let mut items = vec![
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-mcp-server-label"),
            enabled
                .map(|enabled| {
                    if enabled {
                        format!(
                            "{} · {readiness_status}",
                            ui_text::t(i18n, "settings-mcp-server-enabled")
                        )
                    } else {
                        format!(
                            "{} · {readiness_status}",
                            ui_text::t(i18n, "settings-mcp-server-disabled")
                        )
                    }
                })
                .unwrap_or_else(|| ui_text::t(i18n, "settings-mcp-status-unavailable")),
            format!(
                "{} {readiness_status}",
                ui_text::t(i18n, "settings-mcp-server-detail")
            ),
            SettingsPickerAction::ToggleMcpServer,
        ),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-mcp-auth-label"),
            match auth_mode {
                "oauth" => ui_text::t(i18n, "settings-mcp-auth-oauth"),
                "mixed" => ui_text::t(i18n, "settings-mcp-auth-mixed"),
                _ => ui_text::t(i18n, "settings-mcp-auth-none"),
            },
            if auth_enabled {
                let oauth = control.as_ref().and_then(|value| value.get("oauth"));
                let registrations = oauth
                    .and_then(|value| value.get("registrationMethods"))
                    .and_then(serde_json::Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" / ")
                    })
                    .unwrap_or_else(|| "—".to_owned());
                let pkce = oauth
                    .and_then(|value| value.get("pkceMethods"))
                    .and_then(serde_json::Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" / ")
                    })
                    .unwrap_or_else(|| "—".to_owned());
                format!(
                    "{} {}: {registrations}; {}: {pkce}",
                    ui_text::t(i18n, "settings-mcp-auth-detail"),
                    ui_text::t(i18n, "settings-mcp-registration-label"),
                    ui_text::t(i18n, "settings-mcp-pkce-label"),
                )
            } else {
                ui_text::t(i18n, "settings-mcp-auth-detail")
            },
            SettingsPickerAction::ToggleMcpAuth,
        ),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-mcp-anonymous-access-label"),
            if anonymous_access == "read_only" {
                ui_text::t(i18n, "settings-mcp-anonymous-access-read-only")
            } else {
                ui_text::t(i18n, "settings-mcp-anonymous-access-none")
            },
            if auth_mode != "mixed" {
                ui_text::t(i18n, "settings-mcp-anonymous-access-inactive-detail")
            } else if anonymous_access == "read_only" {
                ui_text::t(i18n, "settings-mcp-anonymous-access-read-only-detail")
            } else {
                ui_text::t(i18n, "settings-mcp-anonymous-access-none-detail")
            },
            SettingsPickerAction::ToggleMcpAnonymousAccess,
        ),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-mcp-client-registration-label"),
            if client_registration == "cimd_and_dcr" {
                ui_text::t(i18n, "settings-mcp-client-registration-dcr")
            } else {
                ui_text::t(i18n, "settings-mcp-client-registration-cimd")
            },
            if client_registration == "cimd_and_dcr" {
                ui_text::t(i18n, "settings-mcp-client-registration-dcr-detail")
            } else {
                ui_text::t(i18n, "settings-mcp-client-registration-cimd-detail")
            },
            SettingsPickerAction::ToggleMcpClientRegistration,
        ),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-mcp-public-url-label"),
            configured_url
                .map(str::to_owned)
                .unwrap_or_else(|| ui_text::t(i18n, "settings-mcp-public-url-auto")),
            resource_url
                .map(|value| {
                    format!(
                        "{} ({value})",
                        ui_text::t(i18n, "settings-mcp-public-url-detail")
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "settings-mcp-public-url-detail")),
            SettingsPickerAction::EditMcpPublicUrl,
        ),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-mcp-oauth-issuer-label"),
            configured_issuer
                .map(str::to_owned)
                .unwrap_or_else(|| ui_text::t(i18n, "settings-mcp-oauth-issuer-derived")),
            ui_text::t(i18n, "settings-mcp-oauth-issuer-detail"),
            SettingsPickerAction::EditMcpOAuthIssuerUrl,
        ),
    ];
    if auth_enabled {
        items.extend([
            SettingsStudioItem::new(
                ui_text::t(i18n, "settings-mcp-oauth-password-label"),
                password_status,
                ui_text::t(i18n, "settings-mcp-oauth-password-detail"),
                SettingsPickerAction::EditMcpOAuthPassword,
            ),
            SettingsStudioItem::new(
                ui_text::t(i18n, "settings-mcp-oauth-password-clear-label"),
                ui_text::t(i18n, "value-clear"),
                ui_text::t(i18n, "settings-mcp-oauth-password-clear-detail"),
                SettingsPickerAction::ClearMcpOAuthPassword,
            ),
        ]);
    }
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
            ui_text::t(i18n, "settings-permission-effective-detail"),
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
            ui_text::t(i18n, "settings-permission-current-detail"),
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
