impl ToolExecutor {
    pub fn is_concurrency_safe_invocation(&self, invocation: &ToolInvocation) -> bool {
        let Some(entry) = self.invocation_definition(invocation) else {
            return false;
        };
        entry.definition.runtime.concurrency_safe
            && !entry.definition.permissions.interactive
            && is_concurrency_safe_tool_invocation(
                &entry,
                &PluginInvocation::from_tool_invocation(invocation),
            )
    }

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

        self.plugins
            .lookup_tool(invocation.tool_name.as_str())
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
                        "tool '{tool_name}' is outside the current session execution-access profile"
                    ),
                    source: agena_domain::CapabilitySourceKind::ExecutionAccess,
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
                &definition.definition.permissions,
            ),
        ))
    }

    pub(crate) fn plugin_tool_name_is_unambiguous(&self, plugin_tool_name: &str) -> bool {
        self.plugins
            .registered_tools()
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
            .lookup_tool(invocation.tool_name.as_str())
            .or_else(|| {
                self.plugins
                    .lookup_tool(canonical_tool_name(invocation.tool_name.as_str()))
            })
            .or_else(|| {
                unique_registered_tool_match(
                    self.plugins.registered_tools(),
                    invocation.tool_name.as_str(),
                )
            })
    }

    pub(crate) fn collect_declared_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        input: &serde_json::Value,
        specs: &[SdkInputPathSpec],
        static_specs: &[SdkPathAccessSpec],
    ) -> Result<(), ToolError> {
        for spec in static_specs {
            self.push_requested_path_checks(checks, spec.path.as_str(), spec.kind);
        }
        for path_request in extract_input_path_requests(input, specs)? {
            self.push_requested_path_checks(checks, &path_request.path, path_request.kind);
        }
        Ok(())
    }

    pub(crate) fn collect_dynamic_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        registered_tool: &agena_plugin_host::registry::RegisteredTool,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let result = self.plugins.dispatch_tool_permission_paths_cancellable(
            registered_tool,
            PluginToolPermissionPathsInput {
                tool_name: registered_tool.tool_name().to_string(),
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: input.clone(),
            },
            self.cancellation_token.clone(),
        );

        let path_requests = match result {
            Ok(path_requests) => path_requests,
            Err(_)
                if self
                    .cancellation_token()
                    .is_some_and(tokio_util::sync::CancellationToken::is_cancelled) =>
            {
                return Err(ToolError::Cancelled);
            }
            Err(err) if err.kind == agena_plugin_host::sdk::PluginErrorKind::NotImplemented => {
                return Ok(());
            }
            Err(err) => return Err(ToolError::from_plugin_error(err)),
        };

        for path_request in path_requests {
            self.push_requested_path_checks(checks, &path_request.path, path_request.kind);
        }
        Ok(())
    }

    pub(crate) fn collect_declared_network_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        input: &serde_json::Value,
        input_specs: &[SdkInputNetworkSpec],
        static_specs: &[SdkNetworkAccessSpec],
    ) -> Result<(), ToolError> {
        for spec in static_specs {
            self.push_network_check(checks, spec.target.as_str())?;
        }
        for request in extract_input_network_requests(input, input_specs)? {
            self.push_network_check(checks, request.target.as_str())?;
        }
        Ok(())
    }

    pub(crate) fn collect_dynamic_network_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        registered_tool: &agena_plugin_host::registry::RegisteredTool,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let result = self.plugins.dispatch_tool_permission_networks_cancellable(
            registered_tool,
            PluginToolPermissionNetworksInput {
                tool_name: registered_tool.tool_name().to_string(),
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: input.clone(),
            },
            self.cancellation_token.clone(),
        );

        let network_requests = match result {
            Ok(network_requests) => network_requests,
            Err(_)
                if self
                    .cancellation_token()
                    .is_some_and(tokio_util::sync::CancellationToken::is_cancelled) =>
            {
                return Err(ToolError::Cancelled);
            }
            Err(err) if err.kind == agena_plugin_host::sdk::PluginErrorKind::NotImplemented => {
                return Ok(());
            }
            Err(err) => return Err(ToolError::from_plugin_error(err)),
        };

        for request in network_requests {
            self.push_network_check(checks, request.target.as_str())?;
        }
        Ok(())
    }

    pub(crate) fn collect_declared_filesystem_effect_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        if let Some(effects) = filesystem_effects_from_input(input)? {
            let command = input
                .pointer("/args/command")
                .or_else(|| {
                    input.get("command").filter(|value| {
                        !matches!(
                            value.as_str(),
                            Some(
                                "bash"
                                    | "powershell"
                                    | "exec"
                                    | "monitor"
                                    | "monitor_start"
                                    | "monitor_list"
                                    | "monitor_read"
                                    | "monitor_stop"
                            )
                        )
                    })
                })
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if !command.is_empty() {
                validate_shell_filesystem_effects(tool_name, command, &effects)?;
            }
            let workdir = input
                .get("workdir")
                .or_else(|| input.pointer("/args/workdir"))
                .and_then(serde_json::Value::as_str);
            let base = self.shell_effect_base_path(workdir);
            self.push_filesystem_effect_checks(checks, &effects, base.as_path());
        }
        Ok(())
    }

    pub(crate) fn push_requested_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        path: &str,
        kind: SdkPathKind,
    ) {
        let target = self.resolve_target_path(path);
        self.push_path_checks(checks, sdk_path_kind_to_access_kind(kind), &target);
    }

    pub fn requested_path_permission_check(
        &self,
        path: &str,
        kind: SdkPathKind,
    ) -> ToolPermissionCheck {
        let mut checks = Vec::with_capacity(1);
        self.push_requested_path_checks(&mut checks, path, kind);
        checks.remove(0)
    }

    pub(crate) fn push_filesystem_effect_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        effects: &FilesystemEffects,
        base_path: &Path,
    ) {
        for effect in effects.to_effects() {
            let target = self.resolve_filesystem_effect_path(effect.path.as_str(), base_path);
            if effect.access.includes_read() {
                self.push_path_checks(checks, AccessKind::Read, &target);
            }
            if effect.access.includes_write() {
                self.push_path_checks(checks, AccessKind::Write, &target);
            }
        }
    }
}
use agena_domain::PluginInvocation;

use super::{
    AccessKind, Path, PermissionDecision, PluginToolPermissionNetworksInput,
    PluginToolPermissionPathsInput, RegisteredTool, SdkInputNetworkSpec, SdkInputPathSpec,
    SdkNetworkAccessSpec, SdkPathAccessSpec, SdkPathKind, SdkToolStreamingMode, ToolError,
    ToolExecutor, ToolInvocation, ToolPermissionCheck, canonical_tool_name,
    extract_input_network_requests, extract_input_path_requests, filesystem_effects_from_input,
    invocation_name, is_concurrency_safe_tool_invocation, sdk_path_kind_to_access_kind,
    shell_command_from_invocation, unique_registered_tool_match, validate_shell_filesystem_effects,
};
use agena_domain::FilesystemEffects;
