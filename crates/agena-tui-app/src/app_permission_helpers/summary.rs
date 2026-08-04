use super::{permission_mode_label, permission_mode_token};
use agena_api::resource::PermissionConfigResource;

pub(crate) fn permission_override_summary(i18n: &I18n, permission: &PermissionConfig) -> String {
    // The list row only shows which domains have configuration; details such
    // as rules and default modes live in the inspector panel.
    let mut labels = Vec::new();
    if permission.path.is_some() {
        labels.push(ui_text::t(i18n, "value-permission-filesystem"));
    }
    if permission.network.is_some() {
        labels.push(ui_text::t(i18n, "value-permission-network"));
    }
    if permission.tools.is_some() {
        labels.push(ui_text::t(i18n, "value-permission-tools"));
    }
    if labels.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(labels)
    }
}

pub(crate) fn permission_resource_override_summary(
    i18n: &I18n,
    permission: &PermissionConfigResource,
) -> String {
    let mut parts = Vec::new();
    if let Some(path) = permission.path.as_ref() {
        let mut labels = Vec::new();
        if path.workspace.is_some() {
            labels.push(ui_text::t(i18n, "value-workspace"));
        }
        if path.external.is_some() {
            labels.push(ui_text::t(i18n, "value-external"));
        }
        if !path.rules.is_empty() {
            labels.push(i18n.text_args(
                "value-rule-count",
                &agena_tui::fl_args!("count" => path.rules.len() as i64),
            ));
        }
        parts.push(if labels.is_empty() {
            ui_text::t(i18n, "value-custom")
        } else {
            join_inline_segments(labels)
        });
    }
    if let Some(network) = permission.network.as_ref() {
        let mut labels = Vec::new();
        if network.internet.is_some() {
            labels.push(ui_text::t(i18n, "value-internet"));
        }
        if network.private.is_some() {
            labels.push(ui_text::t(i18n, "value-private"));
        }
        if network.loopback.is_some() {
            labels.push(ui_text::t(i18n, "value-loopback"));
        }
        if !network.rules.is_empty() {
            labels.push(i18n.text_args(
                "value-rule-count",
                &agena_tui::fl_args!("count" => network.rules.len() as i64),
            ));
        }
        parts.push(if labels.is_empty() {
            ui_text::t(i18n, "value-custom")
        } else {
            join_inline_segments(labels)
        });
    }
    if let Some(tools) = permission.tools.as_ref() {
        let mut labels = Vec::new();
        if let Some(mode) = tools.default {
            labels.push(i18n.text_args(
                "permission-studio-tool-default-summary",
                &agena_tui::fl_args!("value" => permission_mode_label(i18n, match mode {
                    agena_api::resource::PermissionMode::Allow => PermissionMode::Allow,
                    agena_api::resource::PermissionMode::Auto => PermissionMode::Auto,
                    agena_api::resource::PermissionMode::Ask => PermissionMode::Ask,
                    agena_api::resource::PermissionMode::Deny => PermissionMode::Deny,
                })),
            ));
        }
        if !tools.names.is_empty() {
            labels.push(i18n.text_args(
                "value-name-count",
                &agena_tui::fl_args!("count" => tools.names.len() as i64),
            ));
        }
        if !tools.rules.is_empty() {
            labels.push(i18n.text_args(
                "value-rule-set-count",
                &agena_tui::fl_args!("count" => tools.rules.len() as i64),
            ));
        }
        parts.push(if labels.is_empty() {
            ui_text::t(i18n, "value-custom")
        } else {
            join_inline_segments(labels)
        });
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

pub(crate) fn permission_studio_read_only_message(
    i18n: &I18n,
    source: &PermissionStudioSource,
) -> String {
    match source {
        PermissionStudioSource::EffectiveSession { .. } => {
            ui_text::t(i18n, "settings-permission-effective-read-only")
        }
        PermissionStudioSource::GlobalConfig
        | PermissionStudioSource::WorkspaceConfig
        | PermissionStudioSource::Session { .. } => {
            ui_text::t(i18n, "permission-studio-detail-read-only")
        }
    }
}

pub(crate) fn permission_mode_input_text(mode: Option<PermissionMode>, i18n: &I18n) -> String {
    mode.map(|mode| permission_mode_label(i18n, mode))
        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
}

pub(crate) fn permission_mode_token_text(mode: Option<PermissionMode>) -> String {
    mode.map(permission_mode_token)
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn permission_config_from_json_value(value: &JsonValue) -> UiResult<PermissionConfig> {
    if value.is_null() {
        Ok(PermissionConfig::default())
    } else {
        serde_json::from_value(value.clone()).map_err(crate::UiFailure::internal)
    }
}
use crate::{
    I18n, JsonValue, PermissionConfig, PermissionMode, PermissionStudioSource, UiResult,
    join_inline_segments, ui_text,
};
