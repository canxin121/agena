use crate::{ConfigJsonSources, I18n, JsonValue, SettingsFieldSpec, get_json_path};

/// A setting at the layer being edited. The final runtime value is retained
/// only for comparison; a later layer must never become this editor's value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SettingsFieldProjection {
    pub(crate) workspace: bool,
    pub(crate) local: Option<JsonValue>,
    pub(crate) override_path: Option<String>,
    pub(crate) requires_group_reset: bool,
    pub(crate) default_value: Option<JsonValue>,
    pub(crate) inherited: Option<JsonValue>,
    pub(crate) value: Option<JsonValue>,
    pub(crate) effective: Option<JsonValue>,
}

impl SettingsFieldProjection {
    pub(crate) fn new(sources: &ConfigJsonSources, field: &SettingsFieldSpec) -> Self {
        Self::at_path(sources, field.path.as_str(), field.workspace)
    }

    pub(crate) fn at_path(sources: &ConfigJsonSources, path: &str, workspace: bool) -> Self {
        let at = |document: &JsonValue| {
            // A complete projected root can positively report an unset leaf
            // as null. Only an unavailable root is unknown; raw declaration
            // and restore scope are computed separately below.
            document
                .is_object()
                .then(|| get_json_path(document, Some(path)).unwrap_or(JsonValue::Null))
        };
        let raw = if workspace {
            &sources.project_file
        } else {
            &sources.file
        };
        let override_path = if workspace {
            agena_domain::config_setting_inheritance_override_path(raw, Some(path))
        } else {
            agena_domain::config_setting_override_path(raw, Some(path))
        }
        .unwrap_or(None);
        let local = override_path
            .as_ref()
            .map(|_| get_json_path(raw, Some(path)).unwrap_or(JsonValue::Null));
        let requires_group_reset = agena_domain::config_setting_requires_group_reset(
            raw,
            Some(path),
            override_path.as_deref(),
        )
        .unwrap_or(true);
        let default_value = at(&sources.defaults);
        let inherited = workspace
            .then(|| at(&sources.workspace_inherited))
            .flatten();
        let resolved = if workspace {
            &sources.workspace_resolved
        } else {
            &sources.global_resolved
        };
        let value = if resolved.is_object() {
            // The server resolves schema defaults and replacement boundaries.
            // A partial model group must not JSON-merge with its parent, and
            // a missing resolved leaf is not the final runtime value.
            at(resolved)
        } else {
            // Older servers have no layer projection. Retain their raw/parent
            // representation without substituting a later effective layer.
            local.clone().or_else(|| {
                if workspace {
                    inherited.clone()
                } else {
                    default_value.clone()
                }
            })
        };
        Self {
            workspace,
            value,
            override_path,
            requires_group_reset,
            local,
            default_value,
            inherited,
            effective: at(&sources.effective),
        }
    }

    pub(crate) fn fallback_value(&self) -> Option<&JsonValue> {
        if self.workspace {
            self.inherited.as_ref()
        } else {
            self.default_value.as_ref()
        }
    }

    pub(crate) fn source_label(&self, i18n: &I18n) -> String {
        i18n.text(if self.local.is_some() {
            "permission-studio-origin-current"
        } else if self.workspace {
            return settings_inherited_source_label(i18n, false);
        } else {
            "permission-studio-origin-default"
        })
    }

    pub(crate) fn control(&self, i18n: &I18n) -> agena_tui_settings::SettingsStudioControl {
        if self.workspace && self.local.is_none() {
            agena_tui_settings::SettingsStudioControl::Inherited(self.source_label(i18n))
        } else if self.workspace {
            agena_tui_settings::SettingsStudioControl::Source(self.source_label(i18n))
        } else {
            agena_tui_settings::SettingsStudioControl::RestoreDefault {
                enabled: self.local.is_some(),
            }
        }
    }

    pub(crate) fn fallback_label(&self, i18n: &I18n) -> String {
        settings_layer_fallback_label(i18n, self.workspace)
    }

    pub(crate) fn row_value(&self, i18n: &I18n) -> String {
        if self.local.is_none() && self.value.as_ref().is_some_and(JsonValue::is_null) {
            return i18n.text("settings-source-unset");
        }
        settings_layer_value_label(i18n, self.value.as_ref())
    }
}

pub(crate) fn settings_inherited_source_label(i18n: &I18n, session: bool) -> String {
    i18n.text_args("settings-layer-inherited-from", &agena_tui::fl_args!("source" => i18n.text(if session { "settings-layer-parent-config" } else { "settings-layer-parent-global" })))
}

pub(crate) fn settings_inherited_locked_message(i18n: &I18n, session: bool) -> String {
    i18n.text_args("settings-layer-inherited-locked", &agena_tui::fl_args!("source" => i18n.text(if session { "settings-layer-parent-config" } else { "settings-layer-parent-global" })))
}

pub(crate) fn settings_field_value_editable(
    sources: &ConfigJsonSources,
    field: &SettingsFieldSpec,
) -> bool {
    !field.workspace
        || field.explicit_local
        || SettingsFieldProjection::new(sources, field).local.is_some()
}

pub(crate) fn settings_layer_fallback_label(i18n: &I18n, inherited: bool) -> String {
    i18n.text(if inherited {
        "settings-layer-parent-value"
    } else {
        "settings-layer-default-value"
    })
}

pub(crate) fn settings_layer_reset_label(i18n: &I18n, inherited: bool) -> String {
    i18n.text(if inherited {
        "settings-layer-inherit"
    } else {
        "settings-layer-default"
    })
}

pub(crate) fn settings_layer_source_description(i18n: &I18n, inherited: bool) -> String {
    i18n.text(if inherited {
        "settings-layer-source-description"
    } else {
        "settings-layer-default-description"
    })
}

pub(crate) fn settings_layer_value_label(i18n: &I18n, value: Option<&JsonValue>) -> String {
    match value {
        None => i18n.text("settings-source-unset"),
        Some(JsonValue::Null) => "null".to_owned(),
        Some(JsonValue::String(value)) if value.is_empty() => "\"\"".to_owned(),
        Some(JsonValue::String(value)) => value.clone(),
        Some(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sources(global: JsonValue, workspace: JsonValue) -> ConfigJsonSources {
        ConfigJsonSources {
            config_path: "/test/global.json".into(),
            config_found: true,
            project_config_path: "/test/workspace.json".into(),
            project_config_found: true,
            applied_layers: vec!["global".into(), "workspace".into()],
            defaults: json!({"enabled":true,"limit":0,"name":""}),
            global_resolved: JsonValue::Null,
            workspace_resolved: JsonValue::Null,
            global_inherited: JsonValue::Null,
            workspace_inherited: json!({"enabled":false,"limit":0,"name":""}),
            file: global,
            project_file: workspace,
            effective: json!({"enabled":true,"limit":99,"name":"later layer"}),
        }
    }

    #[test]
    fn settings_layer_projection_never_substitutes_a_later_layer_value() {
        let state = sources(json!({"enabled":false}), json!({"enabled":true}));
        let global = SettingsFieldProjection::at_path(&state, "enabled", false);
        assert_eq!(global.local, Some(json!(false)));
        assert_eq!(global.inherited, None);
        assert_eq!(global.default_value, Some(json!(true)));
        assert_eq!(global.value, Some(json!(false)));
        assert_eq!(global.effective, Some(json!(true)));
        let workspace = SettingsFieldProjection::at_path(&state, "enabled", true);
        assert_eq!(workspace.local, Some(json!(true)));
        assert_eq!(workspace.inherited, Some(json!(false)));
        assert_eq!(workspace.value, Some(json!(true)));
        assert_eq!(global.source_label(&I18n::english()), "local");
    }

    #[test]
    fn settings_layer_projection_preserves_false_zero_empty_and_explicit_equal_values() {
        let state = sources(json!({"enabled":true,"limit":0,"name":""}), json!({}));
        for (path, expected) in [
            ("enabled", json!(true)),
            ("limit", json!(0)),
            ("name", json!("")),
        ] {
            let global = SettingsFieldProjection::at_path(&state, path, false);
            assert_eq!(global.local, Some(expected.clone()));
            assert_eq!(global.value, Some(expected));
            assert_eq!(global.source_label(&I18n::english()), "local");
        }
        let workspace = SettingsFieldProjection::at_path(&state, "enabled", true);
        assert_eq!(workspace.local, None);
        assert_eq!(workspace.value, Some(json!(false)));
        assert_eq!(
            workspace.source_label(&I18n::english()),
            settings_inherited_source_label(&I18n::english(), false)
        );
        assert_eq!(
            settings_layer_value_label(&I18n::english(), Some(&json!(""))),
            "\"\""
        );
    }

    #[test]
    fn settings_layer_distinguishes_typed_null_from_an_opaque_preference_clear() {
        let mut state = sources(
            json!({"sandbox":null,"ui":{"locale":null,"tui":{"theme":null}}}),
            json!({}),
        );
        state.defaults = json!({"sandbox":true,"ui":{"locale":"en-US","tui":{"theme":"dark"}}});
        for (path, expected) in [("sandbox", json!(true)), ("ui.locale", json!("en-US"))] {
            let projection = SettingsFieldProjection::at_path(&state, path, false);
            assert_eq!(projection.local, None);
            assert_eq!(projection.value, Some(expected));
        }
        let clear = SettingsFieldProjection::at_path(&state, "ui.tui.theme", false);
        assert_eq!(clear.local, Some(JsonValue::Null));
        assert_eq!(clear.value, Some(JsonValue::Null));
        assert_eq!(clear.inherited, None);
        assert_eq!(clear.default_value, Some(json!("dark")));
        assert_eq!(clear.row_value(&I18n::english()), "null");
    }

    #[test]
    fn settings_layer_opaque_ancestor_clear_does_not_show_parent_descendant_as_current() {
        let mut state = sources(json!({"ui":{"tui":null}}), json!({}));
        state.defaults = json!({"ui":{"tui":{"theme":"dark"}}});
        let cleared = SettingsFieldProjection::at_path(&state, "ui.tui.theme", false);
        assert_eq!(cleared.local, Some(JsonValue::Null));
        assert_eq!(cleared.override_path.as_deref(), Some("ui.tui"));
        assert!(cleared.requires_group_reset);
        assert_eq!(cleared.value, Some(JsonValue::Null));
        state.workspace_inherited = json!({"ui":{"tui":{"theme":null}}});
        let inherited_clear = SettingsFieldProjection::at_path(&state, "ui.tui.theme", true);
        assert_eq!(inherited_clear.local, None);
        assert_eq!(inherited_clear.inherited, Some(JsonValue::Null));
    }

    #[test]
    fn settings_layer_missing_parent_does_not_fall_back_to_effective() {
        let mut state = sources(json!({}), json!({}));
        state.defaults = JsonValue::Null;
        let global = SettingsFieldProjection::at_path(&state, "limit", false);
        assert_eq!(global.value, None);
        assert_eq!(global.effective, Some(json!(99)));
    }
    #[test]
    fn settings_layer_projection_uses_schema_resolved_values_and_keeps_raw_sparse() {
        let mut state = sources(
            json!({"session":{"compaction":{"auto":false}}}),
            json!({"session":{"compaction":{"target_tokens":0}}}),
        );
        state.defaults = json!({"session":{"compaction":{"auto":true,"target_tokens":1200}}});
        state.global_resolved =
            json!({"session":{"compaction":{"auto":false,"target_tokens":1200}}});
        state.workspace_inherited = state.global_resolved.clone();
        state.workspace_resolved =
            json!({"session":{"compaction":{"auto":false,"target_tokens":0}}});
        state.effective = json!({"session":{"compaction":{"auto":true,"target_tokens":9999}}});

        let global = SettingsFieldProjection::at_path(&state, "session.compaction", false);
        assert_eq!(global.local, Some(json!({"auto":false})));
        assert_eq!(
            global.value,
            Some(json!({"auto":false,"target_tokens":1200}))
        );
        assert_eq!(global.inherited, None);
        assert_eq!(
            global.default_value,
            Some(json!({"auto":true,"target_tokens":1200}))
        );
        let workspace = SettingsFieldProjection::at_path(&state, "session.compaction", true);
        assert_eq!(workspace.local, Some(json!({"target_tokens":0})));
        assert_eq!(
            workspace.value,
            Some(json!({"auto":false,"target_tokens":0}))
        );
        assert_eq!(workspace.inherited, global.value);
        let filled =
            SettingsFieldProjection::at_path(&state, "session.compaction.target_tokens", false);
        assert_eq!(filled.local, None);
        assert_eq!(filled.value, Some(json!(1200)));
        assert!(matches!(
            filled.control(&I18n::english()),
            agena_tui_settings::SettingsStudioControl::RestoreDefault { enabled: false }
        ));
    }

    #[test]
    fn settings_layer_projection_respects_replaced_model_groups_and_missing_resolved_leaves() {
        let parent = json!({"providers":{"default_selection":{"provider":"parent","model":"base","parallel_tool_calls":false}}});
        let partial = json!({"providers":{"default_selection":{"verbosity":"low"}}});
        let mut state = sources(parent.clone(), partial.clone());
        state.workspace_inherited = parent;
        state.workspace_resolved = partial;
        state.effective =
            json!({"providers":{"default_selection":{"provider":"runtime","model":"later"}}});
        let group = SettingsFieldProjection::at_path(&state, "providers.default_selection", true);
        assert_eq!(group.value, Some(json!({"verbosity":"low"})));
        let model =
            SettingsFieldProjection::at_path(&state, "providers.default_selection.model", true);
        assert_eq!(model.value, Some(JsonValue::Null));
        assert_eq!(model.local, Some(JsonValue::Null));
        assert_eq!(
            model.override_path.as_deref(),
            Some("providers.default_selection")
        );
        assert_eq!(model.source_label(&I18n::english()), "local");
        assert!(!model.requires_group_reset);
        assert_eq!(model.inherited, Some(json!("base")));
        assert_eq!(model.effective, Some(json!("later")));
    }

    #[test]
    fn settings_layer_projection_does_not_treat_legacy_parent_as_a_known_new_default() {
        let mut state = sources(json!({}), json!({}));
        state.defaults = JsonValue::Null;
        state.global_inherited = json!({"enabled":false});
        state.effective = json!({"enabled":true});
        let global = SettingsFieldProjection::at_path(&state, "enabled", false);
        assert_eq!(global.default_value, None);
        assert_eq!(global.inherited, None);
        assert_eq!(global.value, None);
        assert_eq!(global.effective, Some(json!(true)));
        state.defaults = serde_json::json!({});
        state.global_resolved = serde_json::json!({});
        let known_unset = SettingsFieldProjection::at_path(&state, "enabled", false);
        assert_eq!(known_unset.default_value, Some(JsonValue::Null));
        assert_eq!(known_unset.value, Some(JsonValue::Null));
        assert_eq!(
            known_unset.row_value(&I18n::english()),
            I18n::english().text("settings-source-unset")
        );
    }
    #[test]
    fn settings_layer_reset_follows_atomic_tool_groups_but_not_missing_global_model_modes() {
        let raw = json!({
            "providers":{"default_selection":{"provider":"local","model":"chosen"}},
            "permission":{"tools":{"rules":{"shell.*":{}}}}
        });
        let mut state = sources(raw.clone(), raw.clone());
        state.defaults = json!({"permission":{"tools":{"rules":{"shell.*":{"dangerous":"deny"}}}}});
        state.global_resolved = raw.clone();
        state.workspace_resolved = raw;
        let mode = SettingsFieldProjection::at_path(
            &state,
            "providers.default_selection.verbosity",
            false,
        );
        assert_eq!(mode.local, None);
        assert_eq!(mode.override_path, None);
        assert!(matches!(
            mode.control(&I18n::english()),
            agena_tui_settings::SettingsStudioControl::RestoreDefault { enabled: false }
        ));
        for workspace in [false, true] {
            let rule = SettingsFieldProjection::at_path(
                &state,
                r#"permission.tools.rules."shell.*".dangerous"#,
                workspace,
            );
            assert_eq!(rule.local, Some(JsonValue::Null));
            assert_eq!(rule.value, Some(JsonValue::Null));
            assert_eq!(
                rule.override_path.as_deref(),
                Some(r#"permission.tools.rules."shell.*""#)
            );
        }
    }
}
