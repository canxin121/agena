impl ToolExecutor {
    pub(crate) fn invocation_definition(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<RegisteredTool> {
        if let Some(function) = invocation
            .tool_api_call
            .as_ref()
            .map(|call| call.function)
            .filter(|function| *function != agena_domain::ToolApiFunction::Call)
        {
            if invocation.name != function.function_name() || invocation.plugin_name.is_some() {
                return None;
            }
            return self
                .registered_tools_with_definition_overrides()
                .into_iter()
                .filter_map(crate::tool::ToolApiBinding::from_registered_tool)
                .find(|binding| binding.function() == function)
                .and_then(|binding| binding.handler().cloned());
        }
        self.plugin_invocation_definition(&PluginInvocation::from_tool_invocation(invocation))
    }

    pub fn validate_advertised_tool_identity(
        &self,
        invocation: &ToolInvocation,
        advertised_identity: Option<&str>,
    ) -> Result<(), ToolError> {
        let Some(advertised_identity) = advertised_identity
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(());
        };
        let current = self
            .invocation_definition(invocation)
            .map(|definition| definition.definition_identity());
        if current.as_deref() == Some(advertised_identity) {
            return Ok(());
        }
        Err(ToolError::StaleToolCall {
            tool: invocation_name(invocation).to_string(),
        })
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

    pub(crate) fn invocation_plugin_name_for(&self, invocation: &ToolInvocation) -> String {
        self.plugin_invocation_plugin_name_for(&PluginInvocation::from_tool_invocation(invocation))
    }

    pub(crate) fn plugin_invocation_plugin_name_for(
        &self,
        invocation: &PluginInvocation,
    ) -> String {
        if let Some(entry) = self.plugin_invocation_definition(invocation) {
            return entry.plugin_full_name();
        }

        self.plugin_resolution_for_plugin_invocation(invocation)
            .map(|entry| entry.plugin_full_name())
            .unwrap_or_else(|| "custom".to_string())
    }

    pub(crate) fn invocation_streaming_mode(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<SdkToolStreamingMode> {
        self.invocation_definition(invocation)
            .map(|entry| entry.definition.runtime.streaming)
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
        let resolution = self.plugin_resolution_for_invocation(invocation);
        let mut tool_name_aliases = vec![tool_name.as_str()];
        if let Some(resolution) = resolution.as_ref()
            && resolution.tool_name() != tool_name
            && self.plugin_tool_name_is_unambiguous(resolution.tool_name())
        {
            tool_name_aliases.push(resolution.tool_name());
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

    pub(crate) fn plugin_resolution_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<agena_plugin_host::registry::RegisteredTool> {
        if invocation
            .tool_api_call
            .as_ref()
            .is_some_and(|call| call.function != agena_domain::ToolApiFunction::Call)
        {
            return self.invocation_definition(invocation);
        }
        self.plugin_resolution_for_plugin_invocation(&PluginInvocation::from_tool_invocation(
            invocation,
        ))
    }

    pub(crate) fn plugin_resolution_for_plugin_invocation(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<agena_plugin_host::registry::RegisteredTool> {
        self.plugins
            .lookup_tool_for_name_scope(invocation.tool_name.as_str(), self.plugin_scope.as_ref())
            .or_else(|| {
                unique_registered_tool_match(
                    self.registered_tools_with_definition_overrides(),
                    invocation.tool_name.as_str(),
                )
            })
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
            ToolPayloadInput::Monitor(crate::part::MonitorToolInput::Start {
                ws: Some(ws),
                ..
            }) => {
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
        let (command, effects, network, workdir) = match input {
            crate::part::ShellToolInput::Run { command, .. } => (
                command.command.as_str(),
                command.filesystem_effects(),
                command.network.as_slice(),
                command.workdir.as_deref(),
            ),
            crate::part::ShellToolInput::Write { input } => (
                input.chars.as_str(),
                agena_domain::FilesystemEffects {
                    read: input.reads.clone(),
                    write: input.writes.clone(),
                },
                input.network.as_slice(),
                None,
            ),
            crate::part::ShellToolInput::List {}
            | crate::part::ShellToolInput::Logs { .. }
            | crate::part::ShellToolInput::Stop { .. }
            | crate::part::ShellToolInput::Resize { .. }
            | crate::part::ShellToolInput::Signal { .. } => return Ok(()),
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
