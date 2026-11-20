use crate::{JsonPathError, format_json_path, parse_json_path};
use serde_json::Value;

/// Whether a configuration document declares the value at this settings path.
///
/// Optional schema fields use null as unset. Client preferences under `ui`
/// (apart from the typed `locale` field) use recursive JSON overlays instead:
/// an explicit null clears the inherited value. A scalar or array preference
/// also replaces its whole subtree, including paths absent from that subtree.
/// Walking the original document preserves that distinction from a missing key.
pub fn config_setting_has_override(
    document: &Value,
    path: Option<&str>,
) -> Result<bool, JsonPathError> {
    Ok(config_setting_override_path(document, path)?.is_some())
}

/// The actual local setting to remove when restoring defaults for a path.
/// Non-atomic leaves retain the requested spelling. Replacing group paths use
/// parsed segments so dotted preference and tool keys remain a single key.
/// Per-tool command rule groups also replace built-in rules in global settings.
pub fn config_setting_override_path(
    document: &Value,
    path: Option<&str>,
) -> Result<Option<String>, JsonPathError> {
    config_setting_override_path_with_parent(document, path, false)
}

/// Restore boundary for a layer with an actual parent. Model selections replace
/// their parent as a whole, including omitted child fields. Global model fields
/// instead fill omissions from program defaults and use the ordinary helper.
pub fn config_setting_inheritance_override_path(
    document: &Value,
    path: Option<&str>,
) -> Result<Option<String>, JsonPathError> {
    config_setting_override_path_with_parent(document, path, true)
}

/// Whether a direct leaf edit would replace a local ancestor rather than edit
/// an existing object. Resetting an atomic object group and editing one of its
/// children are separate operations; only the former uses the ancestor path.
pub fn config_setting_requires_group_reset(
    document: &Value,
    path: Option<&str>,
    override_path: Option<&str>,
) -> Result<bool, JsonPathError> {
    let Some(override_path) = override_path else {
        return Ok(false);
    };
    let segments = parse_json_path(path.unwrap_or_default())?;
    let boundary = parse_json_path(override_path)?;
    if boundary.len() >= segments.len() {
        return Ok(false);
    }
    Ok(!crate::get_json_path(document, Some(override_path))?.is_object())
}

fn config_setting_override_path_with_parent(
    document: &Value,
    path: Option<&str>,
    has_parent: bool,
) -> Result<Option<String>, JsonPathError> {
    let segments = parse_json_path(path.unwrap_or_default())?;
    let atomic_depth = if segments.len() > 4
        && segments[0] == "permission"
        && segments[1] == "tools"
        && segments[2] == "rules"
    {
        Some(4)
    } else if has_parent
        && segments.len() > 2
        && matches!(
            (segments[0].as_str(), segments[1].as_str()),
            ("providers", "default_selection")
                | ("runtime", "auxiliary_model")
                | ("permission", "approval_model")
        )
    {
        Some(2)
    } else {
        None
    };
    if let Some(depth) = atomic_depth {
        let group = segments[..depth].iter().try_fold(document, |value, key| {
            value.as_object().and_then(|object| object.get(key))
        });
        if group.is_some_and(|value| !value.is_null()) {
            return Ok(Some(format_json_path(&segments[..depth])));
        }
    }
    let opaque_preference = segments.first().is_some_and(|value| value == "ui")
        && segments.get(1).is_some_and(|value| value != "locale");
    let mut cursor = document;
    for (depth, segment) in segments.iter().enumerate() {
        cursor = match cursor {
            Value::Object(object) => match object.get(segment) {
                Some(value) => value,
                None => return Ok(None),
            },
            // Opaque arrays replace the parent array, rather than inheriting
            // its missing indexes. The same holds for a null/scalar ancestor.
            Value::Array(_) if opaque_preference && depth >= 2 => {
                return Ok(Some(format_json_path(&segments[..depth])));
            }
            Value::Array(items) => match segment
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get(index))
            {
                Some(value) => value,
                None => return Ok(None),
            },
            _ => {
                return Ok(
                    (opaque_preference && depth >= 2).then(|| format_json_path(&segments[..depth]))
                );
            }
        };
    }
    Ok((!cursor.is_null() || opaque_preference).then(|| path.unwrap_or_default().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn optional_nulls_and_missing_keys_do_not_hide_real_falsy_values() {
        let document = json!({
            "sandbox": null,
            "session": {"enabled": false, "limit": 0, "label": ""},
            "ui": {"locale": null},
        });
        for path in ["sandbox", "ui.locale", "session.missing", "missing"] {
            assert!(!config_setting_has_override(&document, Some(path)).unwrap());
        }
        for path in ["session.enabled", "session.limit", "session.label"] {
            assert!(config_setting_has_override(&document, Some(path)).unwrap());
        }
    }

    #[test]
    fn opaque_preference_null_is_a_local_clear_including_quoted_keys() {
        let document =
            json!({"ui":{"tui":{"theme":null,"tool_visibility":{"agena.fs.read":null}}}});
        for path in ["ui.tui.theme", "ui.tui.tool_visibility.\"agena.fs.read\""] {
            assert!(config_setting_has_override(&document, Some(path)).unwrap());
        }
        assert!(!config_setting_has_override(&document, Some("ui.tui.width")).unwrap());
        assert!(!config_setting_has_override(&json!({"ui":null}), Some("ui.tui.theme")).unwrap());
    }

    #[test]
    fn replacing_an_opaque_subtree_does_not_inherit_missing_descendants() {
        for value in [Value::Null, json!(false), json!("default"), json!([])] {
            let document = json!({"ui":{"tui":value}});
            assert!(config_setting_has_override(&document, Some("ui.tui.theme")).unwrap());
        }
        let document = json!({"ui":{"tui":{"colors":["red"]}}});
        assert!(config_setting_has_override(&document, Some("ui.tui.colors.1")).unwrap());
        assert!(
            !config_setting_has_override(&json!({"ui":{"tui":{}}}), Some("ui.tui.theme")).unwrap()
        );
    }

    #[test]
    fn model_children_restore_the_atomic_parent_only_when_a_parent_layer_exists() {
        let document = json!({
            "providers":{"default_selection":{"provider":"local","model":"main"}},
            "runtime":{"auxiliary_model":{"provider":"local","model":"small"}},
            "permission":{"approval_model":{"provider":"local","model":"check"}}
        });
        for group in [
            "providers.default_selection",
            "runtime.auxiliary_model",
            "permission.approval_model",
        ] {
            let missing = format!("{group}.thinking_mode");
            assert_eq!(
                config_setting_override_path(&document, Some(&missing)).unwrap(),
                None
            );
            let boundary =
                config_setting_inheritance_override_path(&document, Some(&missing)).unwrap();
            assert_eq!(boundary.as_deref(), Some(group));
            assert!(
                !config_setting_requires_group_reset(
                    &document,
                    Some(&missing),
                    boundary.as_deref()
                )
                .unwrap()
            );
            let present = format!("{group}.model");
            assert_eq!(
                config_setting_override_path(&document, Some(&present))
                    .unwrap()
                    .as_deref(),
                Some(present.as_str())
            );
            assert_eq!(
                config_setting_inheritance_override_path(&document, Some(&present))
                    .unwrap()
                    .as_deref(),
                Some(group)
            );
        }
        assert_eq!(
            config_setting_inheritance_override_path(
                &json!({"providers":{"default_selection":null}}),
                Some("providers.default_selection.model"),
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn tool_rule_groups_restore_builtin_rules_but_allow_direct_child_edits() {
        let document =
            json!({"permission":{"tools":{"rules":{"agena.shell.exec":{"routine":"allow"}}}}});
        let path = "permission.tools.rules.\"agena.shell.exec\".dangerous";
        let group = "permission.tools.rules.\"agena.shell.exec\"";
        for boundary in [
            config_setting_override_path(&document, Some(path)).unwrap(),
            config_setting_inheritance_override_path(&document, Some(path)).unwrap(),
        ] {
            assert_eq!(boundary.as_deref(), Some(group));
            assert!(
                !config_setting_requires_group_reset(&document, Some(path), boundary.as_deref())
                    .unwrap()
            );
        }
        assert_eq!(
            config_setting_inheritance_override_path(
                &json!({"permission":{"tools":{"names":{}}}}),
                Some("permission.tools.names.\"agena.shell.exec\""),
            )
            .unwrap(),
            None
        );
        for value in [Value::Null, json!(false), json!([])] {
            let opaque = json!({"ui":{"tui":value}});
            let path = "ui.tui.theme";
            let boundary = config_setting_override_path(&opaque, Some(path)).unwrap();
            assert!(
                config_setting_requires_group_reset(&opaque, Some(path), boundary.as_deref())
                    .unwrap()
            );
            assert!(
                !config_setting_requires_group_reset(&opaque, Some("ui.tui"), Some("ui.tui"))
                    .unwrap()
            );
        }
    }

    #[test]
    fn invalid_paths_are_errors_and_an_unavailable_root_has_no_override() {
        assert!(config_setting_has_override(&json!({}), Some("ui.\"unterminated")).is_err());
        assert!(!config_setting_has_override(&Value::Null, None).unwrap());
        assert!(config_setting_has_override(&json!({}), None).unwrap());
    }

    #[test]
    fn restore_paths_identify_the_actual_clear_boundary_without_splitting_quoted_keys() {
        let document = json!({"ui":{"tui":{"theme":null,"tool.visibility":null}}});
        let leaf = "ui.'tui'.theme";
        assert_eq!(
            config_setting_override_path(&document, Some(leaf)).unwrap(),
            Some(leaf.to_owned())
        );
        assert_eq!(
            config_setting_override_path(&document, Some("ui.tui.\"tool.visibility\".read"))
                .unwrap(),
            Some("ui.tui.\"tool.visibility\"".to_owned())
        );
        assert_eq!(
            config_setting_override_path(&document, Some("ui.tui.absent")).unwrap(),
            None
        );
    }
}
