pub(in crate::app) fn permission_rule_draft_from_resource(
    rule: &PermissionRuleResource,
) -> PermissionRuleDraft {
    PermissionRuleDraft {
        subject_kind: match rule.subject_kind.as_str() {
            "path_access" => PermissionRuleSubjectKind::PathAccess,
            "network_access" => PermissionRuleSubjectKind::NetworkAccess,
            _ => PermissionRuleSubjectKind::Tool,
        },
        tool_name: rule.tool_name.clone().unwrap_or_default(),
        qualifier: rule.qualifier.clone().unwrap_or_default(),
        path_access_kind: rule
            .path_access_kind
            .clone()
            .unwrap_or_else(|| "read".to_string()),
        workspace_root: rule.workspace_root.clone().unwrap_or_default(),
        target_path: rule.target_path.clone().unwrap_or_default(),
        network_target: rule
            .network_target
            .clone()
            .or_else(|| rule.network_host.clone())
            .unwrap_or_default(),
        network_host: rule.network_host.clone().unwrap_or_default(),
        network_port: rule
            .network_port
            .map(|port| port.to_string())
            .unwrap_or_default(),
        scope: rule.scope.clone(),
        session_id: rule.session_id.map(|id| id.to_string()).unwrap_or_default(),
        mode: rule.mode,
    }
}

pub(in crate::app) fn permission_rule_draft_from_request(
    request: &PermissionRequest,
) -> PermissionRuleDraft {
    let scope = request
        .scope
        .map(|scope| scope.to_string())
        .unwrap_or_else(|| {
            if request.session_id.is_some() {
                "session".to_string()
            } else {
                "workspace".to_string()
            }
        });
    let session_id = request
        .session_id
        .map(|session_id| session_id.to_string())
        .unwrap_or_default();
    let (
        subject_kind,
        tool_name,
        qualifier,
        path_access_kind,
        workspace_root,
        target_path,
        network_target,
    ) = match &request.action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => (
            PermissionRuleSubjectKind::Tool,
            tool_name.clone(),
            qualifier.clone().unwrap_or_default(),
            "read".to_string(),
            String::new(),
            String::new(),
            String::new(),
        ),
        PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => (
            PermissionRuleSubjectKind::PathAccess,
            String::new(),
            String::new(),
            access_kind.clone(),
            workspace_root.clone(),
            target_path.clone(),
            String::new(),
        ),
        PermissionAction::NetworkAccess {
            target,
            host: _,
            port: _,
        } => (
            PermissionRuleSubjectKind::NetworkAccess,
            String::new(),
            String::new(),
            "read".to_string(),
            String::new(),
            String::new(),
            target.clone(),
        ),
    };
    PermissionRuleDraft {
        subject_kind,
        tool_name,
        qualifier,
        path_access_kind,
        workspace_root,
        target_path,
        network_target,
        network_host: String::new(),
        network_port: String::new(),
        scope,
        session_id,
        mode: PermissionMode::Allow,
    }
}

pub(in crate::app) fn permission_rule_label(i18n: &I18n, rule: &PermissionRuleResource) -> String {
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

pub(in crate::app) fn permission_rule_draft_label(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
) -> String {
    match draft.subject_kind {
        PermissionRuleSubjectKind::Tool => {
            let tool_name = draft.tool_name.trim();
            let qualifier = draft.qualifier.trim();
            if qualifier.is_empty() {
                tool_name.to_string()
            } else {
                format!("{tool_name} · {qualifier}")
            }
        }
        PermissionRuleSubjectKind::PathAccess => format!(
            "{} · {}",
            permission_rule_path_access_kind_display(i18n, draft.path_access_kind.trim()),
            draft.target_path.trim()
        ),
        PermissionRuleSubjectKind::NetworkAccess => {
            let target = draft.network_target.trim();
            if target.is_empty() {
                ui_text::t(i18n, "value-network")
            } else {
                i18n.text_args(
                    "permission-rule-label-network",
                    &crate::fl_args!("target" => target.to_string()),
                )
            }
        }
    }
}

pub(in crate::app) fn permission_rule_subject_kind_name(
    kind: PermissionRuleSubjectKind,
) -> &'static str {
    match kind {
        PermissionRuleSubjectKind::Tool => "tool",
        PermissionRuleSubjectKind::PathAccess => "path_access",
        PermissionRuleSubjectKind::NetworkAccess => "network_access",
    }
}

pub(in crate::app) fn permission_rule_mode_label(mode: PermissionMode) -> &'static str {
    permission_mode_name(mode)
}

pub(in crate::app) fn permission_mode_display(i18n: &I18n, mode: PermissionMode) -> String {
    ui_text::t(
        i18n,
        match mode {
            PermissionMode::Allow => "value-allow",
            PermissionMode::Ask => "value-ask",
            PermissionMode::Deny => "value-deny",
        },
    )
}

pub(in crate::app) fn permission_mode_token_display(i18n: &I18n, mode: &str) -> String {
    match mode.trim() {
        "allow" => ui_text::t(i18n, "value-allow"),
        "ask" => ui_text::t(i18n, "value-ask"),
        "deny" => ui_text::t(i18n, "value-deny"),
        other => other.to_string(),
    }
}

pub(in crate::app) fn permission_rule_subject_kind_display(
    i18n: &I18n,
    kind: PermissionRuleSubjectKind,
) -> String {
    ui_text::t(
        i18n,
        match kind {
            PermissionRuleSubjectKind::Tool => "value-permission-rule-subject-tool",
            PermissionRuleSubjectKind::PathAccess => "value-permission-rule-subject-path-access",
            PermissionRuleSubjectKind::NetworkAccess => {
                "value-permission-rule-subject-network-access"
            }
        },
    )
}

pub(in crate::app) fn permission_rule_path_access_kind_display(i18n: &I18n, kind: &str) -> String {
    match kind.trim() {
        "read" => ui_text::t(i18n, "value-read"),
        "write" => ui_text::t(i18n, "value-write"),
        "read_write" => ui_text::t(i18n, "value-read-write"),
        "path" => ui_text::t(i18n, "value-path"),
        other => other.to_string(),
    }
}

pub(in crate::app) fn permission_rule_scope_display(i18n: &I18n, scope: &str) -> String {
    match scope.trim() {
        "session" => ui_text::t(i18n, "value-session"),
        "workspace" => ui_text::t(i18n, "value-workspace"),
        "global" => ui_text::t(i18n, "value-global"),
        other => other.to_string(),
    }
}

pub(in crate::app) fn permission_rule_value_or(
    i18n: &I18n,
    value: &str,
    fallback_key: &str,
) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        ui_text::t(i18n, fallback_key)
    } else {
        value.to_string()
    }
}

pub(in crate::app) fn permission_rule_studio_item(
    i18n: &I18n,
    label_key: &str,
    value: String,
    detail_key: &str,
    action: PermissionRuleStudioAction,
) -> PermissionRuleStudioItem {
    PermissionRuleStudioItem {
        label: ui_text::t(i18n, label_key),
        value,
        detail: ui_text::t(i18n, detail_key),
        action,
    }
}

pub(in crate::app) fn permission_rule_choice_overlay_spec(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    field: PermissionRuleStudioChoiceField,
) -> (String, String, Editor, Vec<ChoiceItem>, bool) {
    match field {
        PermissionRuleStudioChoiceField::SubjectKind => (
            ui_text::t(i18n, "overlay-permission-rule-choice-subject-title"),
            ui_text::t(i18n, "overlay-permission-rule-choice-subject-prompt"),
            Editor::from_text(permission_rule_subject_kind_name(draft.subject_kind).to_string()),
            vec![
                choice_item(
                    "tool",
                    ui_text::t(i18n, "overlay-permission-rule-choice-subject-tool-detail"),
                ),
                choice_item(
                    "path_access",
                    ui_text::t(
                        i18n,
                        "overlay-permission-rule-choice-subject-path-access-detail",
                    ),
                ),
                choice_item(
                    "network_access",
                    ui_text::t(
                        i18n,
                        "overlay-permission-rule-choice-subject-network-access-detail",
                    ),
                ),
            ],
            false,
        ),
        PermissionRuleStudioChoiceField::PathAccessKind => (
            ui_text::t(i18n, "overlay-permission-rule-choice-access-title"),
            ui_text::t(i18n, "overlay-permission-rule-choice-access-prompt"),
            Editor::from_text(draft.path_access_kind.clone()),
            vec![
                choice_item(
                    "read",
                    ui_text::t(i18n, "overlay-permission-rule-choice-access-read-detail"),
                ),
                choice_item(
                    "write",
                    ui_text::t(i18n, "overlay-permission-rule-choice-access-write-detail"),
                ),
                choice_item(
                    "read_write",
                    ui_text::t(
                        i18n,
                        "overlay-permission-rule-choice-access-read-write-detail",
                    ),
                ),
            ],
            false,
        ),
        PermissionRuleStudioChoiceField::Scope => (
            ui_text::t(i18n, "overlay-permission-rule-choice-scope-title"),
            ui_text::t(i18n, "overlay-permission-rule-choice-scope-prompt"),
            Editor::from_text(draft.scope.clone()),
            vec![
                choice_item(
                    "session",
                    ui_text::t(i18n, "overlay-permission-rule-choice-scope-session-detail"),
                ),
                choice_item(
                    "workspace",
                    ui_text::t(
                        i18n,
                        "overlay-permission-rule-choice-scope-workspace-detail",
                    ),
                ),
                choice_item(
                    "global",
                    ui_text::t(i18n, "overlay-permission-rule-choice-scope-global-detail"),
                ),
            ],
            false,
        ),
        PermissionRuleStudioChoiceField::Mode => (
            ui_text::t(i18n, "overlay-permission-rule-choice-mode-title"),
            ui_text::t(i18n, "overlay-permission-rule-choice-mode-prompt"),
            Editor::from_text(permission_rule_mode_label(draft.mode).to_string()),
            vec![
                choice_item(
                    "allow",
                    ui_text::t(i18n, "overlay-permission-rule-choice-mode-allow-detail"),
                ),
                choice_item(
                    "ask",
                    ui_text::t(i18n, "overlay-permission-rule-choice-mode-ask-detail"),
                ),
                choice_item(
                    "deny",
                    ui_text::t(i18n, "overlay-permission-rule-choice-mode-deny-detail"),
                ),
            ],
            false,
        ),
    }
}

pub(in crate::app) fn permission_rule_editor_spec(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    field: PermissionRuleStudioEditField,
) -> (String, String, String, String) {
    let footer = ui_text::t(i18n, "overlay-permission-rule-editor-footer");
    match field {
        PermissionRuleStudioEditField::ToolName => (
            ui_text::t(i18n, "overlay-permission-rule-editor-tool-name-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-tool-name-prompt"),
            footer,
            draft.tool_name.clone(),
        ),
        PermissionRuleStudioEditField::Qualifier => (
            ui_text::t(i18n, "overlay-permission-rule-editor-qualifier-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-qualifier-prompt"),
            footer,
            draft.qualifier.clone(),
        ),
        PermissionRuleStudioEditField::WorkspaceRoot => (
            ui_text::t(i18n, "overlay-permission-rule-editor-workspace-root-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-workspace-root-prompt"),
            footer,
            draft.workspace_root.clone(),
        ),
        PermissionRuleStudioEditField::TargetPath => (
            ui_text::t(i18n, "overlay-permission-rule-editor-target-path-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-target-path-prompt"),
            footer,
            draft.target_path.clone(),
        ),
        PermissionRuleStudioEditField::NetworkTarget => (
            ui_text::t(i18n, "overlay-permission-rule-editor-network-target-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-network-target-prompt"),
            footer,
            draft.network_target.clone(),
        ),
        PermissionRuleStudioEditField::SessionId => (
            ui_text::t(i18n, "overlay-permission-rule-editor-session-id-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-session-id-prompt"),
            footer,
            draft.session_id.clone(),
        ),
    }
}

pub(in crate::app) fn permission_rule_path_browser_spec(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    field: PermissionRuleStudioPathField,
) -> (String, String, PathBrowserMode, String) {
    match field {
        PermissionRuleStudioPathField::WorkspaceRoot => (
            ui_text::t(i18n, "overlay-permission-rule-browser-workspace-root-title"),
            ui_text::t(
                i18n,
                "overlay-permission-rule-browser-workspace-root-prompt",
            ),
            PathBrowserMode::DirectoryOnly,
            draft.workspace_root.clone(),
        ),
        PermissionRuleStudioPathField::TargetPath => (
            ui_text::t(i18n, "overlay-permission-rule-browser-target-path-title"),
            ui_text::t(i18n, "overlay-permission-rule-browser-target-path-prompt"),
            PathBrowserMode::AnyPath,
            draft.target_path.clone(),
        ),
    }
}

pub(in crate::app) fn permission_rule_studio_items(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    _rule_id: Option<i64>,
) -> Vec<PermissionRuleStudioItem> {
    let mut items = vec![
        permission_rule_studio_item(
            i18n,
            "overlay-permission-rule-item-subject-kind",
            permission_rule_subject_kind_display(i18n, draft.subject_kind),
            "overlay-permission-rule-item-subject-kind-detail",
            PermissionRuleStudioAction::SubjectKind,
        ),
        permission_rule_studio_item(
            i18n,
            "overlay-permission-rule-item-mode",
            permission_mode_display(i18n, draft.mode),
            "overlay-permission-rule-item-mode-detail",
            PermissionRuleStudioAction::Mode,
        ),
        permission_rule_studio_item(
            i18n,
            "overlay-permission-rule-item-scope",
            permission_rule_scope_display(i18n, draft.scope.as_str()),
            "overlay-permission-rule-item-scope-detail",
            PermissionRuleStudioAction::Scope,
        ),
    ];

    if draft.scope == "session" {
        items.push(permission_rule_studio_item(
            i18n,
            "overlay-permission-rule-item-session-id",
            permission_rule_value_or(i18n, draft.session_id.as_str(), "value-unset"),
            "overlay-permission-rule-item-session-id-detail",
            PermissionRuleStudioAction::SessionId,
        ));
    }

    match draft.subject_kind {
        PermissionRuleSubjectKind::Tool => {
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-tool-name",
                permission_rule_value_or(i18n, draft.tool_name.as_str(), "value-unset"),
                "overlay-permission-rule-item-tool-name-detail",
                PermissionRuleStudioAction::ToolName,
            ));
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-qualifier",
                permission_rule_value_or(i18n, draft.qualifier.as_str(), "value-none"),
                "overlay-permission-rule-item-qualifier-detail",
                PermissionRuleStudioAction::Qualifier,
            ));
        }
        PermissionRuleSubjectKind::PathAccess => {
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-access-kind",
                permission_rule_path_access_kind_display(i18n, draft.path_access_kind.as_str()),
                "overlay-permission-rule-item-access-kind-detail",
                PermissionRuleStudioAction::PathAccessKind,
            ));
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-target-path",
                permission_rule_value_or(i18n, draft.target_path.as_str(), "value-unset"),
                "overlay-permission-rule-item-target-path-detail",
                PermissionRuleStudioAction::TargetPath,
            ));
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-workspace-root",
                permission_rule_value_or(
                    i18n,
                    draft.workspace_root.as_str(),
                    "value-runtime-default",
                ),
                "overlay-permission-rule-item-workspace-root-detail",
                PermissionRuleStudioAction::WorkspaceRoot,
            ));
        }
        PermissionRuleSubjectKind::NetworkAccess => {
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-network-target",
                permission_rule_value_or(i18n, draft.network_target.as_str(), "value-unset"),
                "overlay-permission-rule-item-network-target-detail",
                PermissionRuleStudioAction::NetworkTarget,
            ));
        }
    }

    items
}

pub(in crate::app) fn refresh_permission_rule_studio_dialog(
    i18n: &I18n,
    dialog: &mut PermissionRuleStudioOverlay,
) {
    let preferred_item = dialog
        .workbench
        .list
        .selected_item()
        .map(|item| item.label.as_str());
    let items = permission_rule_studio_items(i18n, &dialog.draft, dialog.rule_id);
    let selected = preferred_item
        .and_then(|label| items.iter().position(|item| item.label == label))
        .unwrap_or(0);
    dialog.workbench.list = SelectableListState::new(items, selected);
}

pub(in crate::app) fn permission_rule_studio_detail_text(
    i18n: &I18n,
    _draft: &PermissionRuleDraft,
    item: &PermissionRuleStudioItem,
) -> String {
    match item.action {
        PermissionRuleStudioAction::SubjectKind => {
            ui_text::t(i18n, "overlay-permission-rule-detail-subject-kind")
        }
        PermissionRuleStudioAction::ToolName => {
            ui_text::t(i18n, "overlay-permission-rule-detail-tool-name")
        }
        PermissionRuleStudioAction::Qualifier => {
            ui_text::t(i18n, "overlay-permission-rule-detail-qualifier")
        }
        PermissionRuleStudioAction::PathAccessKind => {
            ui_text::t(i18n, "overlay-permission-rule-detail-path-access-kind")
        }
        PermissionRuleStudioAction::WorkspaceRoot => {
            ui_text::t(i18n, "overlay-permission-rule-detail-workspace-root")
        }
        PermissionRuleStudioAction::TargetPath => {
            ui_text::t(i18n, "overlay-permission-rule-detail-target-path")
        }
        PermissionRuleStudioAction::NetworkTarget => {
            ui_text::t(i18n, "overlay-permission-rule-detail-network-target")
        }
        PermissionRuleStudioAction::Scope => {
            ui_text::t(i18n, "overlay-permission-rule-detail-scope")
        }
        PermissionRuleStudioAction::SessionId => {
            ui_text::t(i18n, "overlay-permission-rule-detail-session-id")
        }
        PermissionRuleStudioAction::Mode => ui_text::t(i18n, "overlay-permission-rule-detail-mode"),
    }
}
use crate::app::{
    ChoiceItem, Editor, I18n, PathBrowserMode, PermissionAction, PermissionMode, PermissionRequest,
    PermissionRuleDraft, PermissionRuleResource, PermissionRuleStudioAction,
    PermissionRuleStudioChoiceField, PermissionRuleStudioEditField, PermissionRuleStudioItem,
    PermissionRuleStudioOverlay, PermissionRuleStudioPathField, PermissionRuleSubjectKind,
    SelectableListState, choice_item, permission_mode_name, ui_text,
};
