//! Typed permission projection. Canonical name normalization remains domain-owned.

use super::*;

impl PermissionConfigResource {
    /// Project a selected configuration for a client write. Runtime-only plugin
    /// grants are deliberately excluded, as they are by the domain serializer.
    /// The `From<&PermissionConfig>` projection remains the effective-policy
    /// view used in server responses, where those grants belong.
    /// Configuration-only approval options cannot be silently dropped from a
    /// session write. Keep the previous serializer-to-wire validation failure.
    pub fn try_from_configuration(
        value: &agena_domain::PermissionConfig,
    ) -> Result<Self, serde_json::Error> {
        if value
            .approval_model
            .as_ref()
            .is_some_and(|model| model.parallel_tool_calls.is_some())
        {
            return Err(<serde_json::Error as serde::de::Error>::custom(
                "unknown field `parallel_tool_calls` in session permission approval_model",
            ));
        }
        let mut resource = Self::from(value);
        if let (Some(tools), Some(resource_tools)) = (&value.tools, &mut resource.tools) {
            resource_tools.names = tools
                .names
                .iter()
                .map(|(name, mode)| (name.clone(), permission_mode_resource_from_domain(*mode)))
                .collect();
        }
        Ok(resource)
    }
}

impl From<&agena_domain::PermissionConfig> for PermissionConfigResource {
    fn from(value: &agena_domain::PermissionConfig) -> Self {
        PermissionConfigResource {
            path: value
                .path
                .as_ref()
                .map(|path| PathPermissionConfigResource {
                    workspace: path
                        .workspace
                        .as_ref()
                        .map(path_access_modes_resource_from_domain),
                    external: path
                        .external
                        .as_ref()
                        .map(path_access_modes_resource_from_domain),
                    rules: path
                        .rules
                        .iter()
                        .map(|(pattern, modes)| {
                            (
                                pattern.clone(),
                                path_access_modes_resource_from_domain(modes),
                            )
                        })
                        .collect(),
                }),
            network: value
                .network
                .as_ref()
                .map(|network| NetworkPermissionConfigResource {
                    internet: network.internet.map(permission_mode_resource_from_domain),
                    private: network.private.map(permission_mode_resource_from_domain),
                    loopback: network.loopback.map(permission_mode_resource_from_domain),
                    rules: network
                        .rules
                        .iter()
                        .map(|(pattern, mode)| {
                            (pattern.clone(), permission_mode_resource_from_domain(*mode))
                        })
                        .collect(),
                }),
            tools: value
                .tools
                .as_ref()
                .map(|tools| ToolPermissionConfigResource {
                    default: tools.default.map(permission_mode_resource_from_domain),
                    names: tools
                        .names
                        .iter()
                        .chain(tools.plugin.iter())
                        .map(|(name, mode)| {
                            (name.clone(), permission_mode_resource_from_domain(*mode))
                        })
                        .collect(),
                    rules: tools
                        .rules
                        .iter()
                        .map(|(name, rules)| {
                            let rules = match rules {
                                agena_domain::ToolPermissionRules::Mode(mode) => {
                                    ToolPermissionRulesResource::Mode(
                                        permission_mode_resource_from_domain(*mode),
                                    )
                                }
                                agena_domain::ToolPermissionRules::Ordered(rules) => {
                                    ToolPermissionRulesResource::Ordered(
                                        rules
                                            .iter()
                                            .map(|(pattern, mode)| {
                                                (
                                                    pattern.clone(),
                                                    permission_mode_resource_from_domain(*mode),
                                                )
                                            })
                                            .collect(),
                                    )
                                }
                            };
                            (name.clone(), rules)
                        })
                        .collect(),
                }),
            approval_model: value.approval_model.as_ref().map(|model| {
                crate::resource::ApprovalModelSelectionResource {
                    provider: model.provider.clone(),
                    adapter: model.adapter.clone(),
                    model: model.model.clone(),
                    thinking_mode: model.thinking_mode.clone(),
                    speed_mode: model.speed_mode.clone(),
                    verbosity: model.verbosity.clone(),
                }
            }),
        }
    }
}
fn path_access_modes_resource_from_domain(
    value: &agena_domain::PathAccessModes,
) -> crate::resource::PathAccessModesResource {
    crate::resource::PathAccessModesResource {
        read: value.read.map(permission_mode_resource_from_domain),
        write: value.write.map(permission_mode_resource_from_domain),
    }
}

const fn permission_mode_resource_from_domain(
    value: agena_domain::PermissionMode,
) -> crate::resource::PermissionMode {
    match value {
        agena_domain::PermissionMode::Allow => crate::resource::PermissionMode::Allow,
        agena_domain::PermissionMode::Auto => crate::resource::PermissionMode::Auto,
        agena_domain::PermissionMode::Ask => crate::resource::PermissionMode::Ask,
        agena_domain::PermissionMode::Deny => crate::resource::PermissionMode::Deny,
    }
}

impl From<PermissionConfigResource> for agena_domain::PermissionConfig {
    fn from(value: PermissionConfigResource) -> Self {
        Self {
            path: value.path.map(|path| agena_domain::PathPermissionConfig {
                workspace: path.workspace.map(path_access_modes_to_domain),
                external: path.external.map(path_access_modes_to_domain),
                rules: path
                    .rules
                    .into_iter()
                    .map(|(pattern, modes)| (pattern, path_access_modes_to_domain(modes)))
                    .collect(),
            }),
            network: value
                .network
                .map(|network| agena_domain::NetworkPermissionConfig {
                    internet: network.internet.map(permission_mode_to_domain),
                    private: network.private.map(permission_mode_to_domain),
                    loopback: network.loopback.map(permission_mode_to_domain),
                    rules: network
                        .rules
                        .into_iter()
                        .map(|(pattern, mode)| (pattern, permission_mode_to_domain(mode)))
                        .collect(),
                }),
            tools: value.tools.map(|tools| {
                agena_domain::ToolPermissionConfig {
                    default: tools.default.map(permission_mode_to_domain),
                    names: tools
                        .names
                        .into_iter()
                        .map(|(name, mode)| (name, permission_mode_to_domain(mode)))
                        .collect(),
                    plugin: Default::default(),
                    rules: tools
                        .rules
                        .into_iter()
                        .map(|(name, rules)| {
                            (
                                name,
                                match rules {
                                    ToolPermissionRulesResource::Mode(mode) => {
                                        agena_domain::ToolPermissionRules::Mode(
                                            permission_mode_to_domain(mode),
                                        )
                                    }
                                    ToolPermissionRulesResource::Ordered(rules) => {
                                        agena_domain::ToolPermissionRules::Ordered(
                                            rules
                                                .into_iter()
                                                .map(|(pattern, mode)| {
                                                    (pattern, permission_mode_to_domain(mode))
                                                })
                                                .collect(),
                                        )
                                    }
                                },
                            )
                        })
                        .collect(),
                }
                .with_canonical_tool_names()
            }),
            approval_model: value.approval_model.map(|model| {
                agena_domain::ApprovalModelSelection {
                    provider: model.provider,
                    adapter: model.adapter,
                    model: model.model,
                    thinking_mode: model.thinking_mode,
                    speed_mode: model.speed_mode,
                    verbosity: model.verbosity,
                    // This runtime option has never been part of the permission wire resource.
                    parallel_tool_calls: None,
                }
            }),
        }
    }
}

fn path_access_modes_to_domain(value: PathAccessModesResource) -> agena_domain::PathAccessModes {
    agena_domain::PathAccessModes {
        read: value.read.map(permission_mode_to_domain),
        write: value.write.map(permission_mode_to_domain),
    }
}

const fn permission_mode_to_domain(value: PermissionMode) -> agena_domain::PermissionMode {
    match value {
        PermissionMode::Allow => agena_domain::PermissionMode::Allow,
        PermissionMode::Auto => agena_domain::PermissionMode::Auto,
        PermissionMode::Ask => agena_domain::PermissionMode::Ask,
        PermissionMode::Deny => agena_domain::PermissionMode::Deny,
    }
}

impl From<agena_domain::PermissionReplyKind> for PermissionReplyKind {
    fn from(value: agena_domain::PermissionReplyKind) -> Self {
        match value {
            agena_domain::PermissionReplyKind::AllowOnce => Self::AllowOnce,
            agena_domain::PermissionReplyKind::AllowAlways => Self::AllowAlways,
            agena_domain::PermissionReplyKind::DenyOnce => Self::DenyOnce,
            agena_domain::PermissionReplyKind::DenyAlways => Self::DenyAlways,
            agena_domain::PermissionReplyKind::AutoApprove => Self::AutoApprove,
        }
    }
}

impl From<agena_domain::PermissionScope> for PermissionScope {
    fn from(value: agena_domain::PermissionScope) -> Self {
        match value {
            agena_domain::PermissionScope::Session => Self::Session,
            agena_domain::PermissionScope::Workspace => Self::Workspace,
            agena_domain::PermissionScope::Global => Self::Global,
        }
    }
}

impl From<agena_domain::UserInputReply> for UserInputReply {
    fn from(value: agena_domain::UserInputReply) -> Self {
        Self {
            request_id: value.request_id,
            kind: match value.kind {
                agena_domain::UserInputReplyKind::Submit => UserInputReplyKind::Submit,
                agena_domain::UserInputReplyKind::Cancel => UserInputReplyKind::Cancel,
                agena_domain::UserInputReplyKind::Timeout => UserInputReplyKind::Timeout,
            },
            answers: value.answers,
            reason: value.reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
