use super::{
    path_rule_summary, permission_mode_label, permission_mode_token, tool_permission_rules_summary,
};
use agena_api::resource::PermissionConfigResource;

pub(in crate::app) fn permission_override_summary(
    i18n: &I18n,
    permission: &PermissionConfig,
) -> String {
    let mut parts = Vec::new();
    if permission.path.is_some() {
        parts.push(agent_path_permission_summary(
            i18n,
            permission.path.as_ref(),
        ));
    }
    if permission.network.is_some() {
        parts.push(agent_network_permission_summary(
            i18n,
            permission.network.as_ref(),
        ));
    }
    if permission.tools.is_some() {
        parts.push(agent_tool_permission_summary(
            i18n,
            permission.tools.as_ref(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

pub(in crate::app) fn permission_resource_override_summary(
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
                    agena_api::resource::PermissionMode::Ask => PermissionMode::Ask,
                    agena_api::resource::PermissionMode::Deny => PermissionMode::Deny,
                })),
            ));
        }
        if !tools.tags.is_empty() {
            labels.push(i18n.text_args(
                "value-tag-count",
                &agena_tui::fl_args!("count" => tools.tags.len() as i64),
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

pub(in crate::app) fn permission_studio_read_only_message(
    i18n: &I18n,
    source: &PermissionStudioSource,
) -> String {
    match source {
        PermissionStudioSource::Agent { .. } => agent_read_only_permissions_message(i18n),
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

pub(in crate::app) fn agent_path_permission_summary(
    i18n: &I18n,
    path: Option<&PathPermissionConfig>,
) -> String {
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

pub(in crate::app) fn agent_network_permission_summary(
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

pub(in crate::app) fn agent_tool_permission_summary(
    i18n: &I18n,
    tools: Option<&ToolPermissionConfig>,
) -> String {
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
    if !tools.tags.is_empty() {
        parts.push(i18n.text_args(
            "value-tag-count",
            &agena_tui::fl_args!("count" => tools.tags.len() as i64),
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

pub(in crate::app) fn path_access_modes_summary(
    i18n: &I18n,
    modes: Option<&PathAccessModes>,
) -> String {
    let Some(modes) = modes else {
        return ui_text::t(i18n, "value-unset");
    };
    match (modes.read, modes.write) {
        (Some(read), Some(write)) if read == write => permission_mode_label(i18n, read),
        (read, write) => join_inline_segments(vec![
            i18n.text_args(
                "permission-studio-mode-read",
                &agena_tui::fl_args!(
                    "value" => read
                        .map(|mode| permission_mode_label(i18n, mode))
                        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
                ),
            ),
            i18n.text_args(
                "permission-studio-mode-write",
                &agena_tui::fl_args!(
                    "value" => write
                        .map(|mode| permission_mode_label(i18n, mode))
                        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
                ),
            ),
        ]),
    }
}

pub(in crate::app) fn agent_permission_document_detail_lines(
    i18n: &I18n,
    permission: &PermissionConfig,
) -> Vec<DetailTextLine<'static>> {
    if permission.is_empty() {
        return vec![app_detail_plain_line(ui_text::t(
            i18n,
            "overlay-agent-permission-document-unset",
        ))];
    }

    let mut lines = Vec::new();
    if let Some(path) = permission.path.as_ref() {
        push_agent_permission_section_gap(&mut lines);
        push_path_permission_detail_lines(i18n, &mut lines, path);
    }
    if let Some(network) = permission.network.as_ref() {
        push_agent_permission_section_gap(&mut lines);
        push_network_permission_detail_lines(i18n, &mut lines, network);
    }
    if let Some(tools) = permission.tools.as_ref() {
        push_agent_permission_section_gap(&mut lines);
        push_tool_permission_detail_lines(i18n, &mut lines, tools);
    }
    lines
}

pub(in crate::app) fn push_agent_permission_section_gap(lines: &mut Vec<DetailTextLine<'static>>) {
    if !lines.is_empty() {
        lines.push(app_detail_plain_line(String::new()));
    }
}

pub(in crate::app) fn push_path_permission_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    path: &PathPermissionConfig,
) {
    lines.push(app_detail_heading_line(ui_text::t(
        i18n,
        "agent-permission-field-path-section",
    )));
    if path.workspace.is_some() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-workspace"),
            path_access_modes_summary(i18n, path.workspace.as_ref()),
        ));
    }
    if path.external.is_some() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-external"),
            path_access_modes_summary(i18n, path.external.as_ref()),
        ));
    }
    if !path.rules.is_empty() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "permission-studio-section-rules"),
            permission_rule_count_summary(i18n, path.rules.len()),
        ));
        for (pattern, rule) in &path.rules {
            lines.push(app_detail_labeled_line(
                pattern.clone(),
                path_rule_summary(i18n, Some(rule)),
            ));
        }
    }
}

pub(in crate::app) fn push_network_permission_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    network: &NetworkPermissionConfig,
) {
    lines.push(app_detail_heading_line(ui_text::t(
        i18n,
        "agent-permission-field-network-section",
    )));
    if let Some(mode) = network.internet {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-internet"),
            permission_mode_label(i18n, mode),
        ));
    }
    if let Some(mode) = network.private {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-private"),
            permission_mode_label(i18n, mode),
        ));
    }
    if let Some(mode) = network.loopback {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-loopback"),
            permission_mode_label(i18n, mode),
        ));
    }
    push_permission_mode_entries(
        i18n,
        lines,
        ui_text::t(i18n, "permission-studio-section-rules"),
        network.rules.iter(),
    );
}

pub(in crate::app) fn push_tool_permission_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    tools: &ToolPermissionConfig,
) {
    lines.push(app_detail_heading_line(ui_text::t(
        i18n,
        "agent-permission-field-tool-section",
    )));
    if let Some(mode) = tools.default {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "permission-studio-tool-default"),
            permission_mode_label(i18n, mode),
        ));
    }
    push_permission_mode_entries(
        i18n,
        lines,
        ui_text::t(i18n, "permission-studio-field-tool-tags"),
        tools.tags.iter(),
    );
    push_permission_mode_entries(
        i18n,
        lines,
        ui_text::t(i18n, "permission-studio-field-tool-names"),
        tools.names.iter(),
    );
    push_permission_mode_entries(
        i18n,
        lines,
        ui_text::t(i18n, "value-plugin-tools"),
        tools.plugin.iter(),
    );
    if !tools.rules.is_empty() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "permission-studio-page-tool-rules"),
            i18n.text_args(
                "value-rule-set-count",
                &agena_tui::fl_args!("count" => tools.rules.len() as i64),
            ),
        ));
        for (tool_name, rules) in &tools.rules {
            lines.push(app_detail_labeled_line(
                tool_name.clone(),
                tool_permission_rules_summary(i18n, Some(rules)),
            ));
        }
    }
}

pub(in crate::app) fn push_permission_mode_entries<'a, I>(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    label: String,
    entries: I,
) where
    I: IntoIterator<Item = (&'a String, &'a PermissionMode)>,
{
    let entries = entries.into_iter().collect::<Vec<_>>();
    if entries.is_empty() {
        return;
    }
    lines.push(app_detail_labeled_line(
        label,
        i18n.text_args(
            "value-item-count",
            &agena_tui::fl_args!("count" => entries.len() as i64),
        ),
    ));
    for (name, mode) in entries {
        lines.push(app_detail_labeled_line(
            name.clone(),
            permission_mode_label(i18n, *mode),
        ));
    }
}

pub(in crate::app) fn network_defaults_summary(
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

pub(in crate::app) fn permission_rule_count_summary(i18n: &I18n, count: usize) -> String {
    match count {
        0 => ui_text::t(i18n, "value-unset"),
        count => i18n.text_args(
            "value-rule-count",
            &agena_tui::fl_args!("count" => count as i64),
        ),
    }
}

pub(in crate::app) fn permission_mode_input_text(
    mode: Option<PermissionMode>,
    i18n: &I18n,
) -> String {
    mode.map(|mode| permission_mode_label(i18n, mode))
        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
}

pub(in crate::app) fn permission_mode_token_text(mode: Option<PermissionMode>) -> String {
    mode.map(permission_mode_token)
        .unwrap_or_default()
        .to_string()
}

pub(in crate::app) fn permission_config_from_json_value(
    value: &JsonValue,
) -> UiResult<PermissionConfig> {
    if value.is_null() {
        Ok(PermissionConfig::default())
    } else {
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }
}
use crate::app::{
    DetailTextLine, I18n, JsonValue, NetworkPermissionConfig, PathAccessModes,
    PathPermissionConfig, PermissionConfig, PermissionMode, PermissionStudioSource,
    ToolPermissionConfig, UiResult, agent_read_only_permissions_message, app_detail_heading_line,
    app_detail_labeled_line, app_detail_plain_line, join_inline_segments, ui_text,
};
