use crate::{
    I18n, PermissionConfig, PermissionMode, PermissionStudioAction, PermissionStudioItem,
    PermissionStudioModeTarget as Target, PermissionStudioOverlay, PermissionStudioSource,
    PermissionStudioValueSource as Source, ToolPermissionRules, path_access_modes_summary,
    permission_mode_label, permission_studio_mode_target_value,
};

/// The rows describe the effective policy; mutations still start from the
/// selected layer. Domain merging preserves whole-network/whole-tool-rule
/// replacement rather than inventing a separate UI inheritance model.
pub(crate) struct PermissionStudioProjection {
    pub(crate) selected: PermissionConfig,
    pub(crate) effective: PermissionConfig,
    read_only: bool,
}

fn canonical_permission(permission: &PermissionConfig) -> PermissionConfig {
    let mut permission = permission.clone();
    permission.tools = permission
        .tools
        .map(|tools| tools.with_canonical_tool_names());
    if let Some(ToolPermissionRules::Ordered(entries)) = permission
        .tools
        .as_mut()
        .and_then(|tools| tools.rules.get_mut("*"))
    {
        let mode = entries
            .iter()
            .filter(|(pattern, _)| is_read_only_class(pattern))
            .next_back()
            .map(|(_, mode)| *mode);
        if let Some(mode) = mode {
            // The runtime applies the last matching class entry. Preserve that
            // decision when an old spelling is edited through the canonical row.
            entries.retain(|pattern, _| !is_read_only_class(pattern));
            entries.insert("read-only".to_string(), mode);
        }
    }
    permission
}

impl PermissionStudioProjection {
    pub(crate) fn new(dialog: &PermissionStudioOverlay) -> Self {
        let selected = canonical_permission(&dialog.permission);
        let read_only = matches!(
            dialog.source,
            PermissionStudioSource::EffectiveSession { .. }
        );
        let mut effective = canonical_permission(&dialog.inherited_permission);
        if read_only {
            effective = selected.clone();
        } else {
            effective.merge_from(selected.clone());
        }
        Self {
            selected,
            effective,
            read_only,
        }
    }

    pub(crate) fn network(&self) -> agena_domain::NetworkPermissionConfig {
        self.effective
            .network
            .as_ref()
            .unwrap_or_else(|| super::permission_studio_default_network())
            .resolved()
    }

    pub(crate) fn origin(&self, target: &Target) -> Source {
        if self.read_only {
            Source::Effective
        } else if target_has_override(&self.selected, target)
            || (matches!(target, Target::ToolCommandPattern { tool_name, pattern } if tool_name == "*" && pattern == "read-only")
                && self
                    .selected
                    .tools
                    .as_ref()
                    .is_some_and(|tools| tools.rules.contains_key("*")))
        {
            Source::Current
        } else {
            Source::Inherited
        }
    }

    pub(crate) fn mode_item(
        &self,
        i18n: &I18n,
        label: String,
        target: Target,
    ) -> PermissionStudioItem {
        let value = match &target {
            Target::PathWorkspace | Target::PathExternal | Target::PathRule { .. } => {
                let path = self.effective.path.as_ref();
                let defaults = PermissionConfig::global_default();
                let default_path = defaults.path.as_ref();
                let modes = match &target {
                    Target::PathWorkspace => path
                        .and_then(|path| path.workspace.as_ref())
                        .or_else(|| default_path.and_then(|path| path.workspace.as_ref())),
                    Target::PathExternal => path
                        .and_then(|path| path.external.as_ref())
                        .or_else(|| default_path.and_then(|path| path.external.as_ref())),
                    Target::PathRule { pattern } => path.and_then(|path| path.rules.get(pattern)),
                    _ => None,
                };
                if modes.and_then(|modes| modes.sandbox_mode().ok())
                    == Some(agena_domain::PathPermissionMode::Readwrite)
                {
                    i18n.text("permission-studio-path-readwrite-compact")
                } else {
                    path_access_modes_summary(i18n, modes)
                }
            }
            Target::NetworkEgressDefault
            | Target::NetworkIngressDefault
            | Target::NetworkHostLoopback => {
                let network = self.network();
                let action = match &target {
                    Target::NetworkEgressDefault => {
                        network.egress.as_ref().and_then(|egress| egress.default)
                    }
                    Target::NetworkIngressDefault => {
                        network.ingress.as_ref().and_then(|ingress| ingress.default)
                    }
                    _ => network
                        .ingress
                        .as_ref()
                        .and_then(|ingress| ingress.host_loopback),
                }
                .unwrap_or_default();
                i18n.text(if action == agena_domain::NetworkAction::Allow {
                    "value-allow"
                } else {
                    "value-deny"
                })
            }
            Target::ToolCommandPattern { tool_name, pattern }
                if tool_name == "*" && pattern == "read-only" =>
            {
                let mode = read_only_class_mode(&self.effective).unwrap_or(PermissionMode::Allow);
                if matches!(
                    self.effective
                        .tools
                        .as_ref()
                        .and_then(|tools| tools.rules.get("*")),
                    Some(ToolPermissionRules::Mode(_))
                ) {
                    i18n.text("permission-studio-read-only-invalid")
                } else if mode == PermissionMode::Auto {
                    i18n.text_args("permission-studio-read-only-follows-default", &agena_tui::fl_args!("value" => permission_mode_label(i18n, tool_default(&self.effective))))
                } else {
                    permission_mode_label(i18n, mode)
                }
            }
            Target::ToolDefault => permission_mode_label(i18n, tool_default(&self.effective)),
            _ => permission_studio_mode_target_value(&self.effective, &target)
                .map(|mode| permission_mode_label(i18n, mode))
                .unwrap_or_else(|| i18n.text("value-unset")),
        };
        PermissionStudioItem {
            label,
            value,
            source: Some(self.origin(&target)),
            action: PermissionStudioAction::EditMode(target),
        }
    }
}

pub(crate) fn tool_default(permission: &PermissionConfig) -> PermissionMode {
    permission
        .tools
        .as_ref()
        .and_then(|tools| tools.default)
        .unwrap_or(PermissionMode::Allow)
}

pub(crate) fn read_only_class_mode(permission: &PermissionConfig) -> Option<PermissionMode> {
    match permission.tools.as_ref()?.rules.get("*")? {
        ToolPermissionRules::Ordered(entries) => entries
            .iter()
            .filter(|(pattern, _)| is_read_only_class(pattern))
            .next_back()
            .map(|(_, mode)| *mode),
        ToolPermissionRules::Mode(_) => None,
    }
}

fn is_read_only_class(pattern: &str) -> bool {
    matches!(
        pattern.to_ascii_lowercase().replace('_', "-").as_str(),
        "read-only" | "readonly"
    )
}

fn target_has_override(permission: &PermissionConfig, target: &Target) -> bool {
    match target {
        Target::PathWorkspace => permission
            .path
            .as_ref()
            .is_some_and(|path| path.workspace.is_some()),
        Target::PathExternal => permission
            .path
            .as_ref()
            .is_some_and(|path| path.external.is_some()),
        Target::PathRule { pattern } => permission
            .path
            .as_ref()
            .is_some_and(|path| path.rules.contains_key(pattern)),
        Target::NetworkEgressDefault
        | Target::NetworkIngressDefault
        | Target::NetworkHostLoopback => permission.network.is_some(),
        Target::ToolDefault => permission
            .tools
            .as_ref()
            .is_some_and(|tools| tools.default.is_some()),
        Target::ToolName { key } => permission
            .tools
            .as_ref()
            .is_some_and(|tools| tools.names.contains_key(key)),
        Target::ToolRule { tool_name } => permission
            .tools
            .as_ref()
            .is_some_and(|tools| tools.rules.contains_key(tool_name)),
        Target::ToolCommandPattern { tool_name, pattern } => permission
            .tools
            .as_ref()
            .and_then(|tools| tools.rules.get(tool_name))
            .is_some_and(|rules| match rules {
                ToolPermissionRules::Ordered(entries) => entries.contains_key(pattern),
                ToolPermissionRules::Mode(_) => pattern == "*",
            }),
    }
}

pub(crate) fn permission_studio_local_config(dialog: &PermissionStudioOverlay) -> PermissionConfig {
    canonical_permission(&dialog.permission)
}

pub(crate) fn permission_studio_has_override(
    dialog: &PermissionStudioOverlay,
    target: &Target,
) -> bool {
    target_has_override(&canonical_permission(&dialog.permission), target)
}

/// Editing an inherited command pattern creates a local copy of its complete
/// tool group. `tools.rules` merges at the tool key, not at the pattern key.
pub(crate) fn permission_studio_mode_edit_config(
    dialog: &PermissionStudioOverlay,
    target: &Target,
    reset: bool,
) -> PermissionConfig {
    if target.is_network() {
        return super::permission_studio_custom_network(dialog);
    }
    let mut permission = dialog.permission.clone();
    if matches!(
        target,
        Target::ToolDefault
            | Target::ToolName { .. }
            | Target::ToolRule { .. }
            | Target::ToolCommandPattern { .. }
    ) {
        permission = canonical_permission(&permission);
    }
    if let Target::ToolCommandPattern { tool_name, .. } = target
        && !reset
        && !permission
            .tools
            .as_ref()
            .is_some_and(|tools| tools.rules.contains_key(tool_name))
    {
        let projection = PermissionStudioProjection::new(dialog);
        if let Some(rules) = projection
            .effective
            .tools
            .as_ref()
            .and_then(|tools| tools.rules.get(tool_name))
        {
            permission
                .tools
                .get_or_insert_with(Default::default)
                .rules
                .insert(tool_name.clone(), rules.clone());
        }
    }
    permission
}

pub(crate) fn permission_studio_source_label(i18n: &I18n, source: Source) -> String {
    i18n.text(match source {
        Source::Current => "permission-studio-origin-current",
        Source::Inherited => "permission-studio-origin-inherited",
        Source::Effective => "permission-studio-origin-effective",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dialog() -> PermissionStudioOverlay {
        PermissionStudioOverlay {
            title: String::new(),
            footer: String::new(),
            source: PermissionStudioSource::WorkspaceConfig,
            title_context: "workspace".into(),
            editable: true,
            permission: PermissionConfig::default(),
            inherited_permission: PermissionConfig::global_default(),
            network_rule: None,
            save_request: None,
            page: crate::PermissionStudioPage::Access,
            state: agena_tui_components::SectionedListState::new(
                Vec::new(),
                0,
                0,
                crate::PermissionStudioFocus::Items,
            ),
            editor: None,
        }
    }

    fn class_target() -> Target {
        Target::ToolCommandPattern {
            tool_name: "*".into(),
            pattern: "read-only".into(),
        }
    }

    #[test]
    fn permission_projection_uses_domain_merge_and_tracks_rule_origins() {
        let i18n = I18n::english();
        let mut dialog = dialog();
        dialog.inherited_permission.merge_from(serde_json::from_value(json!({
            "path": {"rules": {"/tmp/inherited": "readonly", "/tmp/local": "readonly"}},
            "tools": {"default": "deny", "names": {"custom.inherited": "ask", "custom.local": "ask"},
                "rules": {"agena.shell.exec": {"git status": "allow", "git push": "deny"}}}
        })).unwrap());
        dialog.permission = serde_json::from_value(json!({
            "path": {"workspace": "readonly", "rules": {"/tmp/local": "deny"}},
            "network": {},
            "tools": {"names": {"custom.local": "allow"}, "rules": {"agena.shell.exec": {"git log": "allow"}}}
        })).unwrap();
        let projection = PermissionStudioProjection::new(&dialog);
        let mut expected = canonical_permission(&dialog.inherited_permission);
        expected.merge_from(canonical_permission(&dialog.permission));
        assert_eq!(projection.effective, expected);
        assert_eq!(projection.origin(&Target::PathWorkspace), Source::Current);
        assert_eq!(projection.origin(&Target::PathExternal), Source::Inherited);
        assert_eq!(
            projection.origin(&Target::PathRule {
                pattern: "/tmp/inherited".into()
            }),
            Source::Inherited
        );
        assert_eq!(
            projection.origin(&Target::ToolName {
                key: "custom.local".into()
            }),
            Source::Current
        );
        assert_eq!(
            projection.origin(&Target::ToolName {
                key: "custom.inherited".into()
            }),
            Source::Inherited
        );
        assert_eq!(tool_default(&projection.effective), PermissionMode::Deny);
        assert_eq!(
            projection.network(),
            agena_domain::NetworkPermissionConfig::offline().resolved()
        );
        assert_eq!(
            projection
                .mode_item(&i18n, String::new(), Target::NetworkEgressDefault)
                .source,
            Some(Source::Current)
        );
        let ToolPermissionRules::Ordered(entries) =
            &projection.effective.tools.as_ref().unwrap().rules["agena.shell.exec"]
        else {
            unreachable!()
        };
        assert_eq!(entries.len(), 1);
        assert!(entries.contains_key("git log"));
        assert!(!entries.contains_key("git push"));

        let sections = crate::permission_studio_sections(&i18n, &dialog);
        let paths = sections
            .iter()
            .find(|section| section.id == crate::PermissionStudioSectionId::PathRules)
            .unwrap();
        assert_eq!(paths.items[0].label, "/tmp/local");
        assert!(
            paths.items.iter().any(
                |item| item.label == "/tmp/inherited" && item.source == Some(Source::Inherited)
            )
        );
        assert!(matches!(
            paths.items.last().unwrap().action,
            crate::PermissionStudioAction::CreateRule
        ));
    }

    #[test]
    fn permission_read_only_class_resolves_fallback_auto_and_alias_order() {
        let i18n = I18n::english();
        let mut dialog = dialog();
        dialog.inherited_permission = PermissionConfig::default();
        dialog.permission = serde_json::from_value(json!({"tools": {"default": "deny"}})).unwrap();
        let projection = PermissionStudioProjection::new(&dialog);
        let row = projection.mode_item(&i18n, String::new(), class_target());
        assert_eq!(row.value, i18n.text("value-allow"));
        assert_eq!(row.source, Some(Source::Inherited));

        // An explicit empty wildcard group replaces its parent group and
        // reaches the runtime's Allow fallback; it is still a local decision.
        dialog.inherited_permission =
            serde_json::from_value(json!({"tools": {"rules": {"*": {"read-only": "deny"}}}}))
                .unwrap();
        dialog.permission.tools.as_mut().unwrap().rules.insert(
            "*".into(),
            ToolPermissionRules::Ordered(indexmap::IndexMap::new()),
        );
        let row = PermissionStudioProjection::new(&dialog).mode_item(
            &i18n,
            String::new(),
            class_target(),
        );
        assert_eq!(row.value, i18n.text("value-allow"));
        assert_eq!(row.source, Some(Source::Current));

        let mut entries = indexmap::IndexMap::new();
        entries.insert("read_only".into(), PermissionMode::Allow);
        entries.insert("read-only".into(), PermissionMode::Ask);
        entries.insert("READONLY".into(), PermissionMode::Auto);
        dialog
            .permission
            .tools
            .as_mut()
            .unwrap()
            .rules
            .insert("*".into(), ToolPermissionRules::Ordered(entries));
        assert_eq!(
            read_only_class_mode(&dialog.permission),
            Some(PermissionMode::Auto)
        );
        let projection = PermissionStudioProjection::new(&dialog);
        let row = projection.mode_item(&i18n, String::new(), class_target());
        assert_eq!(
            row.value,
            i18n.text_args(
                "permission-studio-read-only-follows-default",
                &agena_tui::fl_args!("value" => i18n.text("value-deny"))
            )
        );
        assert_eq!(row.source, Some(Source::Current));
        let mut edited = permission_studio_mode_edit_config(&dialog, &class_target(), false);
        crate::apply_permission_studio_mode_input(&i18n, &mut edited, &class_target(), "ask")
            .unwrap();
        let ToolPermissionRules::Ordered(entries) = &edited.tools.as_ref().unwrap().rules["*"]
        else {
            unreachable!()
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries["read-only"], PermissionMode::Ask);

        dialog
            .permission
            .tools
            .as_mut()
            .unwrap()
            .rules
            .insert("*".into(), ToolPermissionRules::Mode(PermissionMode::Deny));
        assert_eq!(
            PermissionStudioProjection::new(&dialog)
                .mode_item(&i18n, String::new(), class_target())
                .value,
            i18n.text("permission-studio-read-only-invalid")
        );
    }

    #[test]
    fn permission_inherited_pattern_edit_copies_the_whole_group() {
        let i18n = I18n::english();
        let mut dialog = dialog();
        dialog.inherited_permission.merge_from(serde_json::from_value(json!({
            "tools": {"rules": {"agena.shell.exec": {"git status": "allow", "git push": "deny"}}}
        })).unwrap());
        dialog.permission = serde_json::from_value(json!({
            "path": {"external": "readonly"}, "network": {}, "tools": {"names": {"custom.tool": "ask"}}
        })).unwrap();
        dialog.permission.approval_model =
            Some(agena_domain::ApprovalModelSelection::from_model_ref(
                &agena_domain::ModelRef::new("test", "approval"),
            ));
        let original = dialog.permission.clone();
        for pattern in ["git status", "git log"] {
            let target = Target::ToolCommandPattern {
                tool_name: "agena.shell.exec".into(),
                pattern: pattern.into(),
            };
            let mut edited = permission_studio_mode_edit_config(&dialog, &target, false);
            crate::apply_permission_studio_mode_input(&i18n, &mut edited, &target, "ask").unwrap();
            let ToolPermissionRules::Ordered(entries) =
                &edited.tools.as_ref().unwrap().rules["agena.shell.exec"]
            else {
                unreachable!()
            };
            assert_eq!(entries["git push"], PermissionMode::Deny);
            assert_eq!(entries[pattern], PermissionMode::Ask);
            assert_eq!(edited.path, original.path);
            assert_eq!(edited.network, original.network);
            assert_eq!(edited.approval_model, original.approval_model);
            assert_eq!(
                edited.tools.as_ref().unwrap().names,
                original.tools.as_ref().unwrap().names
            );
            let reset = permission_studio_mode_edit_config(&dialog, &target, true);
            assert!(
                !reset
                    .tools
                    .as_ref()
                    .unwrap()
                    .rules
                    .contains_key("agena.shell.exec")
            );
        }
        assert_eq!(dialog.permission, original);
    }

    #[test]
    fn permission_canonical_tool_rows_keep_local_origin_and_edit_one_key() {
        let i18n = I18n::english();
        let mut dialog = dialog();
        let tools = dialog.permission.tools.get_or_insert_with(Default::default);
        tools.names.insert("shell.exec".into(), PermissionMode::Ask);
        tools.rules.insert(
            "shell.exec".into(),
            ToolPermissionRules::Ordered(indexmap::IndexMap::from([(
                "git status".into(),
                PermissionMode::Deny,
            )])),
        );
        let projection = PermissionStudioProjection::new(&dialog);
        let target = Target::ToolCommandPattern {
            tool_name: "agena.shell.exec".into(),
            pattern: "git status".into(),
        };
        assert_eq!(projection.origin(&target), Source::Current);
        assert!(permission_studio_has_override(&dialog, &target));
        assert_eq!(
            projection.origin(&Target::ToolName {
                key: "agena.shell.exec".into()
            }),
            Source::Current
        );
        let mut edited = permission_studio_mode_edit_config(&dialog, &target, false);
        crate::apply_permission_studio_mode_input(&i18n, &mut edited, &target, "allow").unwrap();
        let tools = edited.tools.unwrap();
        assert!(!tools.rules.contains_key("shell.exec"));
        assert!(!tools.names.contains_key("shell.exec"));
        assert_eq!(tools.names["agena.shell.exec"], PermissionMode::Ask);
        let ToolPermissionRules::Ordered(entries) = &tools.rules["agena.shell.exec"] else {
            unreachable!()
        };
        assert_eq!(entries["git status"], PermissionMode::Allow);
    }

    #[test]
    fn permission_partial_effective_response_has_safe_runtime_defaults() {
        let i18n = I18n::english();
        let mut dialog = dialog();
        dialog.source = PermissionStudioSource::EffectiveSession { session_id: 7 };
        dialog.inherited_permission = PermissionConfig::default();
        dialog.editable = false;
        let projection = PermissionStudioProjection::new(&dialog);
        assert_eq!(
            projection
                .mode_item(&i18n, String::new(), Target::NetworkEgressDefault)
                .value,
            i18n.text("value-allow")
        );
        assert_eq!(
            projection
                .mode_item(&i18n, String::new(), Target::NetworkIngressDefault)
                .value,
            i18n.text("value-deny")
        );
        assert_eq!(
            projection
                .mode_item(&i18n, String::new(), Target::PathWorkspace)
                .value,
            i18n.text("permission-studio-path-readwrite-compact")
        );
        assert_eq!(projection.origin(&Target::ToolDefault), Source::Effective);
        assert!(!crate::permission_studio_sections(&i18n, &dialog).is_empty());
        assert_eq!(
            super::super::permission_studio_network(&dialog).resolved(),
            agena_domain::NetworkPermissionConfig::allow_all().resolved()
        );
    }
}
