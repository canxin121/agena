use super::*;

pub(super) fn permission_rule_label(i18n: &I18n, rule: &PermissionRuleResource) -> String {
    match rule.subject_kind.as_str() {
        "tool" => match (rule.tool_name.as_deref(), rule.qualifier.as_deref()) {
            (Some(tool_name), Some(qualifier)) if !qualifier.trim().is_empty() => {
                format!("{tool_name} · {qualifier}")
            }
            (Some(tool_name), _) => tool_name.to_string(),
            _ => rule.action_key.clone(),
        },
        "path_access" => i18n.text_args(
            "permission-rule-label-path",
            &crate::fl_args!(
                "access" => permission_rule_path_access_kind_display(
                    i18n,
                    rule.path_access_kind.as_deref().unwrap_or("path"),
                ),
                "path" => rule
                    .target_path
                    .as_deref()
                    .unwrap_or(rule.action_key.as_str())
                    .to_string(),
            ),
        ),
        "network_access" => {
            let host = rule
                .network_host
                .as_deref()
                .or(rule.network_target.as_deref())
                .unwrap_or(rule.action_key.as_str());
            let target = match rule.network_port {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            };
            i18n.text_args(
                "permission-rule-label-network",
                &crate::fl_args!("target" => target),
            )
        }
        _ => rule.action_key.clone(),
    }
}

pub(super) fn permission_rule_scope_label(i18n: &I18n, rule: &PermissionRuleResource) -> String {
    match rule.scope.as_str() {
        "session" => rule
            .session_id
            .map(|id| {
                i18n.text_args(
                    "permission-rule-scope-session",
                    &crate::fl_args!("id" => id),
                )
            })
            .unwrap_or_else(|| ui_text::t(i18n, "permission-rule-scope-session-generic")),
        "workspace" => rule
            .workspace_id
            .map(|id| {
                i18n.text_args(
                    "permission-rule-scope-workspace",
                    &crate::fl_args!("id" => id),
                )
            })
            .unwrap_or_else(|| ui_text::t(i18n, "permission-rule-scope-workspace-generic")),
        "global" => ui_text::t(i18n, "value-global"),
        other => other.to_string(),
    }
}

pub(super) fn permission_rule_detail(i18n: &I18n, rule: &PermissionRuleResource) -> String {
    let mut facts = vec![
        i18n.text_args(
            "permission-rule-detail-mode",
            &crate::fl_args!("mode" => permission_mode_display(i18n, rule.mode)),
        ),
        i18n.text_args(
            "permission-rule-detail-scope",
            &crate::fl_args!("scope" => permission_rule_scope_label(i18n, rule)),
        ),
        i18n.text_args(
            "permission-rule-detail-source",
            &crate::fl_args!("source" => rule.source.clone()),
        ),
    ];
    if let Some(operator) = rule
        .operator
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        facts.push(i18n.text_args(
            "permission-rule-detail-operator",
            &crate::fl_args!("operator" => operator.to_string()),
        ));
    }
    if let Some(reason) = rule
        .reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        facts.push(i18n.text_args(
            "permission-rule-detail-reason",
            &crate::fl_args!("reason" => reason.to_string()),
        ));
    }
    facts.push(i18n.text_args(
        "permission-rule-detail-updated",
        &crate::fl_args!("updated" => rule.updated_at.to_string()),
    ));
    join_inline_segments(facts)
}
