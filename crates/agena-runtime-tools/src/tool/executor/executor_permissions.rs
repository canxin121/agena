impl ToolExecutor {
    pub(crate) fn invocation_definition(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<RegisteredTool> {
        self.plugin_invocation_definition(&PluginInvocation::from_tool_invocation(invocation))
    }

    pub(crate) fn plugin_invocation_definition(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<RegisteredTool> {
        // Resolve identity against the complete registry, not the capability-
        // filtered catalog. The caller must be able to distinguish a tool that
        // does not exist (`ToolUnavailable`) from a registered tool excluded by
        // the current execution context (`CapabilityUnavailable`).
        unique_registered_tool_match(
            self.registered_tools_with_definition_overrides(),
            invocation.tool_name.as_str(),
        )
    }

    pub(crate) fn invocation_streaming_mode(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<SdkToolStreamingMode> {
        self.invocation_definition(invocation)
            .map(|entry| entry.definition().runtime.streaming)
    }

    /// Declared plugin streams need the streaming entry point even when the
    /// model returns them in a batch with buffered tools.
    pub fn invocation_requires_streaming(&self, invocation: &ToolInvocation) -> bool {
        matches!(
            self.invocation_streaming_mode(invocation),
            Some(SdkToolStreamingMode::Streaming)
        )
    }

    pub(crate) fn authorize_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<(String, PermissionDecision), ToolError> {
        let tool_name = invocation_name(invocation);
        let definition = self
            .invocation_definition(invocation)
            .ok_or_else(|| self.unknown_tool_error(tool_name.as_str()))?;
        if !self.tool_is_within_capability(&definition) {
            return Err(ToolError::CapabilityUnavailable(Box::new(
                agena_domain::CapabilityUnavailableResult {
                    capability: "tool_execution".to_string(),
                    tool_name: Some(tool_name.clone()),
                    reason: format!(
                        "tool '{tool_name}' is outside the current session tool capability set"
                    ),
                    source: agena_domain::CapabilitySourceKind::AgentProfile,
                    retryable: false,
                },
            )));
        }
        if !self.builtin_tool_set().is_tool_enabled(&definition) {
            return Err(ToolError::CapabilityUnavailable(Box::new(
                agena_domain::CapabilityUnavailableResult {
                    capability: "model_tool_profile".to_string(),
                    tool_name: Some(tool_name.clone()),
                    reason: format!("tool '{tool_name}' is disabled for the current model profile"),
                    source: agena_domain::CapabilitySourceKind::ModelProfile,
                    retryable: false,
                },
            )));
        }
        let command = shell_command_from_invocation(invocation);
        let canonical_name = definition.canonical_name();
        let mut tool_name_aliases = vec![tool_name.as_str(), canonical_name.as_str()];
        if definition.tool_name() != tool_name
            && self.plugin_tool_name_is_unambiguous(definition.tool_name())
        {
            tool_name_aliases.push(definition.tool_name());
        }
        Ok((
            tool_name.clone(),
            self.principal.authorize_tool_names(
                &tool_name_aliases,
                command.as_deref(),
                &definition.effective_tags(),
            ),
        ))
    }

    pub(crate) fn plugin_tool_name_is_unambiguous(&self, plugin_tool_name: &str) -> bool {
        self.registered_tools_with_definition_overrides()
            .into_iter()
            .filter(|tool| tool.tool_name() == plugin_tool_name)
            .take(2)
            .count()
            == 1
    }

    /// Record the path and network effects of the host's *own* builtin tools.
    ///
    /// This is not the deleted declaration surface coming back: nothing here is
    /// read from a plugin manifest. `ToolPayloadInput` is the host's own typed
    /// payload for the tools the executor itself dispatches (see
    /// `ToolPayloadInput::from_executor_backed_invocation`), so resolving its
    /// path arguments is the host reading its own arguments — which is exactly
    /// the ownership boundary the inversion kept. A tool with a real plugin
    /// handler has no payload and therefore contributes nothing here.
    pub(crate) fn collect_builtin_effect_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        resolution: &agena_plugin_host::registry::RegisteredTool,
        invocation: &ToolInvocation,
    ) -> Result<(), ToolError> {
        if resolution.namespace() == "agena"
            && resolution.plugin_name() == "fs"
            && resolution.tool_name() == "read_media"
        {
            let input = crate::part::ReadMediaInput::parse_input(serde_json::Value::from(
                invocation.input.clone(),
            ))
            .map_err(|error| self.plugin_error_or_cancelled(error))?;
            self.push_resolved_path_check(checks, AccessKind::Read, &input.path);
            return Ok(());
        }
        let Some(payload) =
            ToolPayloadInput::from_executor_backed_invocation(resolution, invocation)
        else {
            return Ok(());
        };
        match payload.map_err(|error| ToolError::invalid_input_error(&error))? {
            ToolPayloadInput::Read(input) => {
                self.push_resolved_path_check(checks, AccessKind::Read, input.file_path.as_str());
            }
            ToolPayloadInput::Glob(input) => {
                self.push_optional_read_path_check(checks, input.path.as_deref());
            }
            ToolPayloadInput::Grep(input) => {
                self.push_optional_read_path_check(checks, input.path.as_deref());
            }
            ToolPayloadInput::ApplyPatch(input) => {
                for path in crate::tool::apply_patch::planned_paths(input.patch.as_str())? {
                    self.push_resolved_path_check(checks, AccessKind::Write, path.as_str());
                }
            }
            ToolPayloadInput::LspDefinition(input) => {
                self.push_resolved_path_check(
                    checks,
                    AccessKind::Read,
                    input.position.file_path.as_str(),
                );
            }
            ToolPayloadInput::LspReferences(input) => {
                self.push_resolved_path_check(
                    checks,
                    AccessKind::Read,
                    input.position.file_path.as_str(),
                );
            }
            ToolPayloadInput::LspHover(input) => {
                self.push_resolved_path_check(
                    checks,
                    AccessKind::Read,
                    input.position.file_path.as_str(),
                );
            }
            ToolPayloadInput::LspDiagnostics(input) => {
                self.push_resolved_path_check(checks, AccessKind::Read, input.file_path.as_str());
            }
            ToolPayloadInput::Shell(input) => {
                self.collect_shell_effect_checks(checks, &input)?;
            }
            ToolPayloadInput::Monitor(crate::part::MonitorToolInput::Start { ws, .. }) => {
                self.push_network_check(checks, ws.url.as_str())?;
            }
            _ => {}
        }
        Ok(())
    }

    fn push_optional_read_path_check(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        path: Option<&str>,
    ) {
        if let Some(path) = path {
            self.push_resolved_path_check(checks, AccessKind::Read, path);
        }
    }

    fn collect_shell_effect_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        input: &crate::part::ShellToolInput,
    ) -> Result<(), ToolError> {
        let (command, effects, network, workdir) =
            if let Some((_, command)) = input.launch_command() {
                (
                    command.command.as_str(),
                    command.filesystem_effects(),
                    command.network.as_slice(),
                    command.workdir.as_deref(),
                )
            } else {
                match input {
                    crate::part::ShellToolInput::Write { input } => (
                        input.chars.as_str(),
                        agena_domain::FilesystemEffects {
                            read: input.reads.clone(),
                            write: input.writes.clone(),
                        },
                        input.network.as_slice(),
                        None,
                    ),
                    _ => return Ok(()),
                }
            };
        if !command.is_empty() {
            crate::tool::shell_tools::validate_declared_filesystem_effects(
                "shell", command, &effects,
            )?;
            self.push_declared_network_checks(checks, command, network)?;
        }
        let base = self.shell_effect_base_path(workdir);
        self.push_filesystem_effect_checks(checks, &effects, base.as_path());
        Ok(())
    }

    /// Declared outbound targets for a shell command. The declaration is still
    /// required of the model — a command that provably uses the network must
    /// name its targets — and each named target is then checked.
    fn push_declared_network_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        command: &str,
        targets: &[String],
    ) -> Result<(), ToolError> {
        if targets.is_empty()
            && let Some(reason) = agena_tool::shell_analysis::network_command_reason(command)
        {
            return Err(ToolError::invalid_input(format!(
                "shell network must declare at least one target because the command appears to use the network: {reason}"
            )));
        }
        for target in targets {
            self.push_network_check(checks, target.as_str())?;
        }
        Ok(())
    }
}
use super::{
    AccessKind, PermissionDecision, RegisteredTool, SdkToolStreamingMode, ToolError, ToolExecutor,
    ToolInvocation, ToolPayloadInput, ToolPermissionCheck, invocation_name,
    shell_command_from_invocation, unique_registered_tool_match,
};
use agena_domain::PluginInvocation;

#[cfg(test)]
mod snapshot_contract_tests {
    use super::*;
    use crate::authorization::ExecutionPrincipal;
    use crate::permission::{PermissionPolicy, ToolPermissionPolicy};
    use agena_plugin_host::sdk::{PluginKey, ToolContract, ToolDefinition, ToolTag};
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn frozen_definition_drives_resolution_after_live_registry_replacement() {
        let plugin: PluginKey = "test.snapshot".parse().unwrap();
        let original = RegisteredTool::new(
            plugin.clone(),
            ToolDefinition {
                name: "run".into(),
                contract: ToolContract {
                    input_schema: json!({"type":"object","required":["original"]}),
                    ..Default::default()
                },
                docs: Default::default(),
                runtime: Default::default(),
                tags: vec![ToolTag::ReadOnly],
            },
        )
        .unwrap();
        let host = agena_plugin_host::PluginHost::new_empty();
        let handle = host.host_handle();
        let owner = handle.begin_plugin_instance(plugin);
        let registry = handle.scoped_tool_registry();
        let _registration = registry
            .register(
                &owner,
                None,
                original.tool_key().clone(),
                original.clone(),
                "original contract",
            )
            .unwrap();
        let mut executor = ToolExecutor::new(
            std::env::temp_dir(),
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            host.clone(),
            None,
            None,
        );
        executor.definition_catalog = Some(Arc::new(vec![original.clone()]));

        let mut replacement = original.definition().clone();
        replacement.contract.input_schema = json!({"type":"object","required":["replacement"]});
        replacement.tags = vec![ToolTag::Mutate];
        let replacement = original.with_definition(replacement).unwrap();
        let (_replacement_registration, _) = registry
            .replace_owned(
                &owner,
                None,
                original.tool_key().clone(),
                replacement.clone(),
                "tool",
                "replacement contract",
            )
            .unwrap();
        assert_eq!(
            host.lookup_tool(&original.canonical_name()),
            Some(replacement)
        );

        let invocation = ToolInvocation::new(original.canonical_name(), Default::default());
        let advertised = executor.invocation_definition(&invocation).unwrap();
        let execution = executor
            .plugin_invocation_definition(&PluginInvocation::from_tool_invocation(&invocation))
            .unwrap();
        assert_eq!(advertised, original);
        assert_eq!(execution, advertised);
        assert_eq!(execution.effective_tags(), vec![ToolTag::ReadOnly]);
        assert!(execution.validate_input(&json!({"original":true})).is_ok());
        assert!(
            execution
                .validate_input(&json!({"replacement":true}))
                .is_err()
        );
    }
}

#[cfg(test)]
mod session_role_tests {
    use super::*;
    use crate::authorization::ExecutionPrincipal;
    use crate::permission::{PermissionPolicy, ToolPermissionPolicy};
    use agena_domain::{PermissionConfig, SessionRelationKind};
    use agena_plugin_host::sdk::{ToolDefinition, ToolTag};
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::sync::Arc;

    const PARENT_TOOLS: &[(&str, &str)] = &[
        ("agena.interaction", "ask"),
        ("agena.interaction", "notify"),
        ("agena.plan", "set"),
        ("agena.plan", "review"),
        ("agena.plan", "phase"),
        ("agena.plan", "clear"),
        ("agena.agent", "spawn"),
        ("agena.agent", "followup"),
        ("agena.agent", "cancel"),
    ];

    fn tool(plugin: &str, name: &str, tags: Vec<ToolTag>) -> RegisteredTool {
        RegisteredTool::new(
            plugin.parse().unwrap(),
            ToolDefinition {
                name: name.into(),
                contract: Default::default(),
                docs: Default::default(),
                runtime: Default::default(),
                tags,
            },
        )
        .unwrap()
    }

    fn executor(tools: Vec<RegisteredTool>) -> ToolExecutor {
        let mut executor = ToolExecutor::new(
            std::env::temp_dir(),
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        );
        executor.definition_catalog = Some(Arc::new(tools));
        executor
    }

    fn catalog() -> Vec<RegisteredTool> {
        // Role restrictions follow tool identity, even without interactive tags.
        let mut tools = PARENT_TOOLS
            .iter()
            .map(|(plugin, name)| tool(plugin, name, vec![ToolTag::Mutate]))
            .collect::<Vec<_>>();
        tools.extend([
            tool("agena.plan", "get", vec![ToolTag::ReadOnly]),
            tool("agena.plan", "edit", vec![ToolTag::Mutate]),
            tool("agena.agent", "message", vec![ToolTag::Agent]),
            tool("agena.agent", "wait", vec![ToolTag::Agent]),
            tool(
                "agena.shell",
                "open",
                vec![ToolTag::Shell, ToolTag::Interactive, ToolTag::Mutate],
            ),
            tool(
                "agena.web",
                "browser_open",
                vec![ToolTag::Network, ToolTag::Interactive],
            ),
            tool("agena.fs", "read", vec![ToolTag::ReadOnly]),
        ]);
        tools
    }

    fn available_names(executor: &ToolExecutor) -> BTreeSet<String> {
        executor
            .available_execution_tools()
            .into_iter()
            .map(|tool| tool.canonical_name())
            .collect()
    }

    #[test]
    fn subagent_catalog_and_invocation_share_role_restrictions() {
        let mut executor = executor(catalog());
        executor.session_relation_kind = SessionRelationKind::Subagent;
        assert_eq!(
            available_names(&executor),
            [
                "agena.plan.get",
                "agena.plan.edit",
                "agena.agent.message",
                "agena.agent.wait",
                "agena.shell.open",
                "agena.web.browser_open",
                "agena.fs.read",
            ]
            .map(str::to_owned)
            .into_iter()
            .collect()
        );

        for (plugin, name) in PARENT_TOOLS {
            let canonical = format!("{plugin}.{name}");
            let compact = format!("{}.{name}", plugin.strip_prefix("agena.").unwrap());
            for alias in [&canonical, &compact] {
                let result =
                    executor.authorize_invocation(&ToolInvocation::new(alias, Default::default()));
                assert!(
                    matches!(result, Err(ToolError::CapabilityUnavailable(_))),
                    "{alias} must be rejected by the role capability policy"
                );
            }
            // Underscore spellings are not execution-tool aliases.
            assert!(matches!(
                executor.authorize_invocation(&ToolInvocation::new(
                    canonical.replace('.', "_"),
                    Default::default(),
                )),
                Err(ToolError::ToolUnavailable(_))
            ));
        }
        for name in available_names(&executor) {
            assert!(
                executor
                    .authorize_invocation(&ToolInvocation::new(&name, Default::default()))
                    .is_ok(),
                "{name} must remain usable by subagents"
            );
        }
    }

    #[test]
    fn user_facing_relations_keep_parent_workflow_tools() {
        let tools = catalog();
        let expected: BTreeSet<_> = tools.iter().map(RegisteredTool::canonical_name).collect();
        for relation in [
            SessionRelationKind::Root,
            SessionRelationKind::Child,
            SessionRelationKind::Fork,
            SessionRelationKind::Rewind,
        ] {
            let mut executor = executor(tools.clone());
            executor.session_relation_kind = relation;
            assert_eq!(available_names(&executor), expected, "{relation:?}");
            for name in available_names(&executor) {
                assert!(
                    executor
                        .authorize_invocation(&ToolInvocation::new(&name, Default::default()))
                        .is_ok(),
                    "{relation:?} must retain {name}"
                );
            }
        }
    }

    #[derive(Default)]
    struct SessionContext {
        relation: SessionRelationKind,
        permission: PermissionConfig,
        denied: BTreeSet<String>,
    }

    impl crate::ToolSessionContext for SessionContext {
        fn relation_kind(&self) -> SessionRelationKind {
            self.relation
        }

        fn effective_workspace_root(&self) -> Option<&Path> {
            None
        }

        fn effective_permission(&self) -> &PermissionConfig {
            &self.permission
        }

        fn permission_ceiling(&self) -> &PermissionConfig {
            &self.permission
        }

        fn capability_denied_tool_names(&self) -> &BTreeSet<String> {
            &self.denied
        }

        fn selected_model(&self) -> Option<&str> {
            None
        }
    }

    #[tokio::test]
    async fn scoped_executor_derives_subagent_role_without_persisted_denials() {
        let executor = executor(Vec::new());
        let handle = executor.plugin_manager().host_handle();
        let registry = handle.scoped_tool_registry();
        let mut owners = Vec::new();
        let mut registrations = Vec::new();
        for tool in [
            tool("agena.interaction", "ask", vec![ToolTag::Interactive]),
            tool("agena.agent", "message", vec![ToolTag::Agent]),
            tool(
                "agena.shell",
                "open",
                vec![ToolTag::Shell, ToolTag::Interactive, ToolTag::Mutate],
            ),
        ] {
            let owner = handle.begin_plugin_instance(tool.plugin_key().clone());
            registrations.push(
                registry
                    .register(
                        &owner,
                        None,
                        tool.tool_key().clone(),
                        tool,
                        "session role test",
                    )
                    .unwrap(),
            );
            owners.push(owner);
        }

        let child = executor
            .for_session_context_async(&SessionContext {
                relation: SessionRelationKind::Subagent,
                ..Default::default()
            })
            .await;
        assert_eq!(child.session_relation_kind(), SessionRelationKind::Subagent);
        assert_eq!(executor.session_relation_kind(), SessionRelationKind::Root);
        assert_eq!(
            available_names(&child),
            ["agena.agent.message", "agena.shell.open"]
                .map(str::to_owned)
                .into_iter()
                .collect()
        );
        assert!(matches!(
            child.authorize_invocation(&ToolInvocation::new("interaction.ask", Default::default())),
            Err(ToolError::CapabilityUnavailable(_))
        ));
        assert!(
            child
                .authorize_invocation(&ToolInvocation::new("shell.open", Default::default()))
                .is_ok()
        );

        let restricted = executor
            .for_session_context_async(&SessionContext {
                relation: SessionRelationKind::Subagent,
                denied: BTreeSet::from(["agent.message".into()]),
                ..Default::default()
            })
            .await;
        assert_eq!(
            available_names(&restricted),
            BTreeSet::from(["agena.shell.open".into()])
        );
        assert!(matches!(
            restricted
                .authorize_invocation(&ToolInvocation::new("agent.message", Default::default())),
            Err(ToolError::CapabilityUnavailable(_))
        ));

        let fork = executor
            .for_session_context_async(&SessionContext {
                relation: SessionRelationKind::Fork,
                ..Default::default()
            })
            .await;
        assert!(available_names(&fork).contains("agena.interaction.ask"));
        assert!(
            fork.authorize_invocation(&ToolInvocation::new("interaction.ask", Default::default()))
                .is_ok()
        );
        drop(registrations);
        drop(owners);
    }
}
