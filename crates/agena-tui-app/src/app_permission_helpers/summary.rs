use super::{permission_mode_label, permission_mode_token};
use agena_api::resource::PermissionConfigResource;

pub(crate) fn permission_override_summary(i18n: &I18n, permission: &PermissionConfig) -> String {
    let mut parts = Vec::new();
    if permission.path.is_some() {
        parts.push(path_permission_summary(i18n, permission.path.as_ref()));
    }
    if permission.network.is_some() {
        parts.push(network_permission_summary(
            i18n,
            permission.network.as_ref(),
        ));
    }
    if permission.tools.is_some() {
        parts.push(tool_permission_summary(i18n, permission.tools.as_ref()));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
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
        if !tools.capabilities.is_empty() {
            labels.push(i18n.text_args(
                "value-tag-count",
                &agena_tui::fl_args!("count" => tools.capabilities.len() as i64),
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

pub(crate) fn path_permission_summary(i18n: &I18n, path: Option<&PathPermissionConfig>) -> String {
    let Some(path) = path else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if path.workspace.is_some() {
        parts.push(ui_text::t(i18n, "value-workspace"));
    }
    if path.external.is_some() {
        parts.push(ui_text::t(i18n, "value-external"));
    }
    if !path.rules.is_empty() {
        parts.push(i18n.text_args(
            "value-rule-count",
            &agena_tui::fl_args!("count" => path.rules.len() as i64),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-custom")
    } else {
        join_inline_segments(parts)
    }
}

pub(crate) fn network_permission_summary(
    i18n: &I18n,
    network: Option<&NetworkPermissionConfig>,
) -> String {
    let Some(network) = network else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if network.internet.is_some() {
        parts.push(ui_text::t(i18n, "value-internet"));
    }
    if network.private.is_some() {
        parts.push(ui_text::t(i18n, "value-private"));
    }
    if network.loopback.is_some() {
        parts.push(ui_text::t(i18n, "value-loopback"));
    }
    if !network.rules.is_empty() {
        parts.push(i18n.text_args(
            "value-rule-count",
            &agena_tui::fl_args!("count" => network.rules.len() as i64),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-custom")
    } else {
        join_inline_segments(parts)
    }
}

pub(crate) fn tool_permission_summary(i18n: &I18n, tools: Option<&ToolPermissionConfig>) -> String {
    let Some(tools) = tools else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if let Some(mode) = tools.default {
        parts.push(i18n.text_args(
            "permission-studio-tool-default-summary",
            &agena_tui::fl_args!("value" => permission_mode_label(i18n, mode)),
        ));
    }
    if !tools.names.is_empty() {
        parts.push(i18n.text_args(
            "value-name-count",
            &agena_tui::fl_args!("count" => tools.names.len() as i64),
        ));
    }
    if !tools.rules.is_empty() {
        parts.push(i18n.text_args(
            "value-rule-set-count",
            &agena_tui::fl_args!("count" => tools.rules.len() as i64),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-custom")
    } else {
        join_inline_segments(parts)
    }
}

pub(crate) fn network_defaults_summary(
    i18n: &I18n,
    network: Option<&NetworkPermissionConfig>,
) -> String {
    let Some(network) = network else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if let Some(mode) = network.internet {
        parts.push(i18n.text_args(
            "permission-studio-network-default",
            &agena_tui::fl_args!(
                "label" => ui_text::t(i18n, "value-internet"),
                "value" => permission_mode_label(i18n, mode),
            ),
        ));
    }
    if let Some(mode) = network.private {
        parts.push(i18n.text_args(
            "permission-studio-network-default",
            &agena_tui::fl_args!(
                "label" => ui_text::t(i18n, "value-private"),
                "value" => permission_mode_label(i18n, mode),
            ),
        ));
    }
    if let Some(mode) = network.loopback {
        parts.push(i18n.text_args(
            "permission-studio-network-default",
            &agena_tui::fl_args!(
                "label" => ui_text::t(i18n, "value-loopback"),
                "value" => permission_mode_label(i18n, mode),
            ),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

pub(crate) fn permission_rule_count_summary(i18n: &I18n, count: usize) -> String {
    match count {
        0 => ui_text::t(i18n, "value-unset"),
        count => i18n.text_args(
            "value-rule-count",
            &agena_tui::fl_args!("count" => count as i64),
        ),
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
    I18n, JsonValue, NetworkPermissionConfig, PathPermissionConfig, PermissionConfig,
    PermissionMode, PermissionStudioSource, ToolPermissionConfig, UiResult, join_inline_segments,
    ui_text,
};
