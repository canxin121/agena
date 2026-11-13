//! Typed permission projection. Canonical name normalization remains domain-owned.

#[cfg(test)]
mod tests {
    use crate::resource::{PermissionConfigResource, PermissionMode};
    use serde_json::json;

    #[test]
    fn typed_permission_projection_preserves_wire_values_and_canonicalizes_aliases() {
        let wire = json!({
            "path": {"workspace":{"read":"allow","write":"ask"},"rules":{"/tmp/**":{"write":"deny"}}},
            "network":{"internet":"auto","private":"deny","loopback":"allow","rules":{"example.com":"ask"}},
            "tools":{"default":"ask","names":{"shell.exec":"allow","agena.shell.exec":"deny"},
                "rules":{"shell.exec":{"routine":"allow","dangerous":"deny"},"fixture.tool":"auto"}},
            "approval_model":{"provider":"fixture","adapter":"adapter","model":"model","thinking_mode":"high","speed_mode":"fast","verbosity":"low"}
        });
        let resource: PermissionConfigResource = serde_json::from_value(wire.clone()).unwrap();
        let expected: agena_domain::PermissionConfig = serde_json::from_value(wire).unwrap();
        let domain: agena_domain::PermissionConfig = resource.into();
        assert_eq!(domain, expected);
        assert_eq!(
            domain.tools.as_ref().unwrap().names["agena.shell.exec"],
            agena_domain::PermissionMode::Deny
        );
        let resource = PermissionConfigResource::from(&domain);
        assert_eq!(
            serde_json::to_value(resource).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    #[test]
    fn configuration_writes_exclude_runtime_plugin_grants_and_keep_selected_names() {
        let mut domain: agena_domain::PermissionConfig = serde_json::from_value(json!({
            "tools": {"default": "ask", "names": {"fixture.tool": "deny"},
                "rules": {"fixture.tool": {"routine": "ask"}}},
            "path": {"workspace": {"read": "allow"}},
            "network": {"internet": "deny"},
            "approval_model": {"provider": "fixture", "model": "approval"}
        }))
        .unwrap();
        let tools = domain.tools.as_mut().unwrap();
        tools
            .plugin
            .insert("fixture.tool".into(), agena_domain::PermissionMode::Allow);
        tools
            .plugin
            .insert("runtime.only".into(), agena_domain::PermissionMode::Allow);

        let configuration = PermissionConfigResource::try_from_configuration(&domain).unwrap();
        assert_eq!(
            serde_json::to_value(&configuration).unwrap(),
            serde_json::to_value(&domain).unwrap(),
            "configuration writes preserve the previous serialized contract",
        );
        let names = &configuration.tools.as_ref().unwrap().names;
        assert_eq!(names["fixture.tool"], PermissionMode::Deny);
        assert!(!names.contains_key("runtime.only"));

        let effective = PermissionConfigResource::from(&domain);
        let names = &effective.tools.as_ref().unwrap().names;
        assert_eq!(names["fixture.tool"], PermissionMode::Allow);
        assert_eq!(names["runtime.only"], PermissionMode::Allow);
    }

    #[test]
    fn configuration_write_rejects_unsupported_approval_options_like_the_previous_wire_decode() {
        for parallel in [None, Some(false), Some(true)] {
            let mut domain: agena_domain::PermissionConfig = serde_json::from_value(json!({
                "approval_model": {"provider": "fixture", "model": "approval"}
            }))
            .unwrap();
            domain.approval_model.as_mut().unwrap().parallel_tool_calls = parallel;
            // Reproduce the old transport boundary only in this regression fixture.
            let previous = serde_json::from_value::<PermissionConfigResource>(
                serde_json::to_value(&domain).unwrap(),
            );
            let projected = PermissionConfigResource::try_from_configuration(&domain);
            if parallel.is_none() {
                assert_eq!(
                    serde_json::to_value(projected.unwrap()).unwrap(),
                    serde_json::to_value(previous.unwrap()).unwrap(),
                );
            } else {
                let previous = previous.unwrap_err();
                let projected = projected.unwrap_err();
                assert_eq!(projected.classify(), previous.classify());
                assert!(projected.to_string().contains("parallel_tool_calls"));
            }
            // Effective-policy responses have always omitted this runtime option.
            let effective = serde_json::to_value(PermissionConfigResource::from(&domain)).unwrap();
            assert!(
                effective["approval_model"]
                    .get("parallel_tool_calls")
                    .is_none()
            );
        }
    }

    #[test]
    fn permission_wire_still_rejects_unknown_fields_and_preserves_empty_sections() {
        assert!(
            serde_json::from_value::<PermissionConfigResource>(json!({"unexpected":true})).is_err()
        );
        let resource: PermissionConfigResource =
            serde_json::from_value(json!({"tools":{},"path":{}})).unwrap();
        let domain: agena_domain::PermissionConfig = resource.into();
        assert!(domain.tools.is_some());
        assert!(domain.path.is_some());
        assert!(domain.is_empty());
    }
}
