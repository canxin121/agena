pub(crate) fn permission_rule_draft_from_resource(
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
        mode: permission_mode_from_wire(rule.mode),
    }
}

const fn permission_mode_from_wire(mode: agena_api::resource::PermissionMode) -> PermissionMode {
    match mode {
        agena_api::resource::PermissionMode::Allow => PermissionMode::Allow,
        agena_api::resource::PermissionMode::Auto => PermissionMode::Auto,
        agena_api::resource::PermissionMode::Ask => PermissionMode::Ask,
        agena_api::resource::PermissionMode::Deny => PermissionMode::Deny,
    }
}

pub(crate) fn permission_rule_draft_from_request(
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
        mode: PermissionMode::Auto,
    }
}

pub(crate) fn permission_rule_label(i18n: &I18n, rule: &PermissionRuleResource) -> String {
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
            &agena_tui::fl_args!(
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
                &agena_tui::fl_args!("target" => target),
            )
        }
        _ => rule.action_key.clone(),
    }
}

pub(crate) fn permission_rule_draft_label(i18n: &I18n, draft: &PermissionRuleDraft) -> String {
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
                    &agena_tui::fl_args!("target" => target.to_string()),
                )
            }
        }
    }
}

pub(crate) fn permission_rule_subject_kind_name(kind: PermissionRuleSubjectKind) -> &'static str {
    match kind {
        PermissionRuleSubjectKind::Tool => "tool",
        PermissionRuleSubjectKind::PathAccess => "path_access",
        PermissionRuleSubjectKind::NetworkAccess => "network_access",
    }
}

pub(crate) fn permission_rule_mode_label(mode: PermissionMode) -> &'static str {
    permission_mode_name(mode)
}

pub(crate) fn permission_mode_display(i18n: &I18n, mode: PermissionMode) -> String {
    ui_text::t(
        i18n,
        match mode {
            PermissionMode::Allow => "value-allow",
            PermissionMode::Auto => "value-auto",
            PermissionMode::Ask => "value-ask",
            PermissionMode::Deny => "value-deny",
        },
    )
}

pub(crate) fn permission_rule_subject_kind_display(
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

pub(crate) fn permission_rule_path_access_kind_display(i18n: &I18n, kind: &str) -> String {
    match kind.trim() {
        "read" => ui_text::t(i18n, "value-read"),
        "write" => ui_text::t(i18n, "value-write"),
        "read_write" => ui_text::t(i18n, "value-read-write"),
        "path" => ui_text::t(i18n, "value-path"),
        other => other.to_string(),
    }
}

pub(crate) fn permission_rule_scope_display(i18n: &I18n, scope: &str) -> String {
    match scope.trim() {
        "session" => ui_text::t(i18n, "value-session"),
        "workspace" => ui_text::t(i18n, "value-workspace"),
        "global" => ui_text::t(i18n, "value-global"),
        other => other.to_string(),
    }
}

pub(crate) fn permission_rule_value_or(i18n: &I18n, value: &str, fallback_key: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        ui_text::t(i18n, fallback_key)
    } else {
        value.to_string()
    }
}

pub(crate) fn permission_rule_studio_item(
    i18n: &I18n,
    label_key: &str,
    value: String,
    detail_key: &str,
    action: PermissionRuleStudioAction,
) -> PermissionRuleStudioItem<PermissionRuleStudioAction> {
    PermissionRuleStudioItem {
        label: ui_text::t(i18n, label_key),
        value,
        detail: ui_text::t(i18n, detail_key),
        action,
    }
}

pub(crate) fn permission_rule_choice_overlay_spec(
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
                    "auto",
                    ui_text::t(i18n, "overlay-permission-rule-choice-mode-auto-detail"),
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

pub(crate) fn permission_rule_editor_spec(
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

pub(crate) fn permission_rule_path_browser_spec(
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

pub(crate) fn permission_rule_studio_items(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    _rule_id: Option<i64>,
) -> Vec<PermissionRuleStudioItem<PermissionRuleStudioAction>> {
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

pub(crate) fn refresh_permission_rule_studio_dialog(
    i18n: &I18n,
    dialog: &mut PermissionRuleStudioOverlay,
) {
    let preferred_item = dialog
        .presentation
        .list
        .selected_item()
        .map(|item| item.label.as_str());
    let items = permission_rule_studio_items(i18n, &dialog.draft, dialog.rule_id);
    let selected = preferred_item
        .and_then(|label| items.iter().position(|item| item.label == label))
        .unwrap_or(0);
    dialog.presentation.list = SelectableListState::new(items, selected);
}

pub(crate) fn permission_rule_studio_detail_text(
    i18n: &I18n,
    _draft: &PermissionRuleDraft,
    item: &PermissionRuleStudioItem<PermissionRuleStudioAction>,
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
use crate::{
    ChoiceItem, Editor, I18n, PathBrowserMode, PermissionAction, PermissionMode, PermissionRequest,
    PermissionRuleDraft, PermissionRuleResource, PermissionRuleStudioAction,
    PermissionRuleStudioChoiceField, PermissionRuleStudioEditField, PermissionRuleStudioItem,
    PermissionRuleStudioOverlay, PermissionRuleStudioPathField, PermissionRuleSubjectKind,
    PermissionScope, SelectableListState, choice_item, format_key_value_segment,
    join_inline_segments, permission_action_label, permission_mode_name,
    permission_related_actions_for_display, permission_requested_actions_for_display, ui_text,
};

/// Performs the one-way Domain-to-display projection for the terminal
/// permission prompt. The returned value contains terminal text only; the
/// original request stays with the App for reply validation, rule editing, and
/// Runtime submission.
pub(crate) fn permission_prompt_content(
    i18n: &I18n,
    request: &PermissionRequest,
) -> agena_tui::permission_prompt::PermissionPromptContent {
    use agena_tui::permission_prompt::{PermissionPromptContent, PermissionPromptLine};

    // Overview shows only what is needed to decide: the action, the reason
    // auto-approval did not resolve (or why approval is requested), and the
    // risk. Everything else lives behind the Details page.
    let mut overview = Vec::new();

    // The action, rendered as Markdown: a shell command gets a fenced code
    // block so what would execute is immediately legible, paths/network stay
    // inline. Full per-field expansion moves to Details.
    overview.push(PermissionPromptLine::markdown(permission_action_markdown(
        i18n,
        &request.action,
    )));

    // Secondary actions that also need approval this call. Keep it to a
    // single summary line in Overview; the exact list is in Details.
    let requested_actions =
        permission_requested_actions_for_display(Some(&request.action), &request.requested_actions);
    let requested_extra = requested_actions.len();
    if requested_extra > 0 {
        overview.push(PermissionPromptLine::muted(i18n.text_args(
            "overlay-permission-summary-more-approvals",
            &agena_tui::fl_args!("count" => requested_extra as i64),
        )));
    }

    // The primary reason to ask. Prefer `reason` (which carries the concrete
    // auto-approval failure); fall back to the policy explanation only when
    // no reason is present.
    let reason = permission_request_reason(request);
    append_prompt_field(
        i18n,
        &mut overview,
        "overlay-permission-field-reason",
        reason,
        PromptFieldTone::Normal,
    );

    // Details carries everything that is not needed to decide.
    let mut details = Vec::new();

    // Full primary action fields (tool / target / path / workspace / network).
    details.push(PermissionPromptLine::markdown(permission_action_markdown(
        i18n,
        &request.action,
    )));
    append_prompt_primary_action(i18n, &mut details, &request.action);

    // Requested actions (still need approval) vs related (already allowed).
    let requested_actions =
        permission_requested_actions_for_display(Some(&request.action), &request.requested_actions);
    if !requested_actions.is_empty() {
        details.push(PermissionPromptLine::muted(i18n.text(
            "overlay-permission-detail-requested-actions",
        )));
        details.extend(requested_actions.iter().map(|action| {
            PermissionPromptLine::markdown(format!(
                "- {}",
                permission_action_markdown_single_line(i18n, action)
            ))
        }));
    }
    let related_actions = permission_related_actions_for_display(
        Some(&request.action),
        request.related_actions.as_slice(),
        request.requested_actions.as_slice(),
    );
    if !related_actions.is_empty() {
        details.push(PermissionPromptLine::muted(i18n.text(
            "overlay-permission-detail-related-actions",
        )));
        details.extend(related_actions.iter().map(|action| {
            PermissionPromptLine::markdown(format!(
                "- {}",
                permission_action_markdown_single_line(i18n, action)
            ))
        }));
    }

    append_prompt_field(
        i18n,
        &mut details,
        "overlay-permission-detail-request-id",
        request.request_id.as_str(),
        PromptFieldTone::Muted,
    );
    if let Some(source) = request.source.as_deref() {
        append_prompt_field(
            i18n,
            &mut details,
            "overlay-permission-detail-source",
            source,
            PromptFieldTone::Muted,
        );
    }
    if let Some(scope) = request.scope {
        append_prompt_field(
            i18n,
            &mut details,
            "overlay-permission-detail-scope",
            permission_request_scope_label(i18n, scope),
            PromptFieldTone::Muted,
        );
    }
    if let Some(operator) = request.operator.as_deref() {
        append_prompt_field(
            i18n,
            &mut details,
            "overlay-permission-detail-operator",
            operator,
            PromptFieldTone::Muted,
        );
    }
    if !request.trace.is_empty() {
        details.push(PermissionPromptLine::muted(
            i18n.text("overlay-permission-detail-trace"),
        ));
        details.extend(request.trace.iter().map(|step| {
            PermissionPromptLine::muted(format!("  {}", permission_trace_step_label(i18n, step)))
        }));
    }

    PermissionPromptContent { overview, details }
}

/// The primary reason shown to the user. `reason` is authoritative: it carries
/// the concrete auto-approval failure or policy reason. `explanation` (the
/// policy decision trace summary) is only a fallback so a bare policy Ask does
/// not render an empty field.
fn permission_request_reason(request: &PermissionRequest) -> &str {
    let reason = request.reason.trim();
    if !reason.is_empty() {
        return reason;
    }
    request.explanation.trim()
}

/// Markdown projection of the primary action: a tool shell command becomes a
/// fenced code block, paths and network targets stay inline, so what would
/// execute is immediately legible.
fn permission_action_markdown(i18n: &I18n, action: &PermissionAction) -> String {
    match action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => {
            let command = qualifier.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let title = i18n.text_args(
                "overlay-permission-action-tool",
                &agena_tui::fl_args!("tool" => tool_name.clone()),
            );
            match command {
                Some(command) if command.lines().count() > 1 || command.len() > 60 => {
                    format!("**{title}**\n\n```sh\n{command}\n```")
                }
                Some(command) => format!("**{title}**\n\n`{command}`"),
                None => format!("**{title}**"),
            }
        }
        PermissionAction::PathAccess {
            access_kind,
            target_path,
            ..
        } => {
            let label = i18n.text_args(
                "overlay-permission-action-path",
                &agena_tui::fl_args!(
                    "access" => access_kind.clone(),
                    "path" => target_path.clone(),
                ),
            );
            format!("**{label}**")
        }
        PermissionAction::NetworkAccess { target, host, port } => {
            let endpoint = if target.trim().is_empty() {
                match port {
                    Some(port) => format!("{host}:{port}"),
                    None => host.clone(),
                }
            } else {
                target.clone()
            };
            let label = i18n.text_args(
                "overlay-permission-action-network",
                &agena_tui::fl_args!("target" => endpoint),
            );
            format!("**{label}**")
        }
    }
}

/// Single-line Markdown projection for a secondary (requested/related) action.
/// Long shell commands are kept inline with inline-code styling so the list
/// does not explode into a wall of code blocks.
fn permission_action_markdown_single_line(i18n: &I18n, action: &PermissionAction) -> String {
    match action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => {
            let base = i18n.text_args(
                "overlay-permission-action-tool",
                &agena_tui::fl_args!("tool" => tool_name.clone()),
            );
            match qualifier.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(command) => format!("{base} · `{command}`"),
                None => base,
            }
        }
        _ => permission_action_label(i18n, action),
    }
}

#[derive(Clone, Copy)]
enum PromptFieldTone {
    Normal,
    Muted,
    Strong,
}

fn append_prompt_primary_action(
    i18n: &I18n,
    lines: &mut Vec<agena_tui::permission_prompt::PermissionPromptLine>,
    action: &PermissionAction,
) {
    match action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => {
            append_prompt_field(
                i18n,
                lines,
                "overlay-permission-field-tool",
                tool_name,
                PromptFieldTone::Strong,
            );
            if let Some(qualifier) = qualifier
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                append_prompt_field(
                    i18n,
                    lines,
                    "overlay-permission-field-target",
                    qualifier,
                    PromptFieldTone::Normal,
                );
            }
        }
        PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => {
            append_prompt_field(
                i18n,
                lines,
                "overlay-permission-field-access",
                permission_rule_path_access_kind_display(i18n, access_kind),
                PromptFieldTone::Strong,
            );
            append_prompt_field(
                i18n,
                lines,
                "overlay-permission-field-path",
                target_path,
                PromptFieldTone::Strong,
            );
            if !workspace_root.trim().is_empty() && workspace_root != target_path {
                append_prompt_field(
                    i18n,
                    lines,
                    "overlay-permission-field-workspace",
                    workspace_root,
                    PromptFieldTone::Muted,
                );
            }
        }
        PermissionAction::NetworkAccess { target, host, port } => {
            let endpoint = if target.trim().is_empty() {
                port.map(|port| format!("{host}:{port}"))
                    .unwrap_or_else(|| host.clone())
            } else {
                target.clone()
            };
            append_prompt_field(
                i18n,
                lines,
                "overlay-permission-field-network",
                endpoint,
                PromptFieldTone::Strong,
            );
            let host_label = port
                .map(|port| format!("{host}:{port}"))
                .unwrap_or_else(|| host.clone());
            if host_label != target.trim() {
                append_prompt_field(
                    i18n,
                    lines,
                    "overlay-permission-field-host",
                    host_label,
                    PromptFieldTone::Muted,
                );
            }
        }
    }
}

fn append_prompt_field(
    i18n: &I18n,
    lines: &mut Vec<agena_tui::permission_prompt::PermissionPromptLine>,
    label_key: &str,
    value: impl AsRef<str>,
    tone: PromptFieldTone,
) {
    use agena_tui::permission_prompt::PermissionPromptLine;

    lines.push(PermissionPromptLine::muted(i18n.text(label_key)));
    let value = format!("  {}", value.as_ref());
    lines.push(match tone {
        PromptFieldTone::Normal => PermissionPromptLine::normal(value),
        PromptFieldTone::Muted => PermissionPromptLine::muted(value),
        PromptFieldTone::Strong => PermissionPromptLine::strong(value),
    });
}

fn permission_request_scope_label(i18n: &I18n, scope: PermissionScope) -> String {
    ui_text::t(
        i18n,
        match scope {
            PermissionScope::Session => "value-session",
            PermissionScope::Workspace => "value-workspace",
            PermissionScope::Global => "value-global",
        },
    )
}

fn permission_trace_step_label(i18n: &I18n, step: &agena_domain::DecisionTraceStep) -> String {
    let source_kind = match step.source_kind {
        agena_domain::PolicySourceKind::StaticPolicy => "static_policy",
        agena_domain::PolicySourceKind::PersistedRule => "persisted_rule",
        agena_domain::PolicySourceKind::PluginAdvice => "plugin_advice",
        agena_domain::PolicySourceKind::ManagedPolicy => "managed_policy",
    };
    let mut facts = vec![source_kind.to_string()];
    if let Some(source) = step.source.as_deref() {
        facts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-source").as_str(),
            source,
        ));
    }
    if let Some(scope) = step.scope {
        facts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-scope").as_str(),
            permission_request_scope_label(i18n, scope).as_str(),
        ));
    }
    if let Some(operator) = step.operator.as_deref() {
        facts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-operator").as_str(),
            operator,
        ));
    }
    format!("- {} — {}", join_inline_segments(facts), step.summary)
}
