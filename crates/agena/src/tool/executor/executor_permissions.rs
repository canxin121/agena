impl ToolExecutor {
    pub fn is_concurrency_safe_invocation(&self, invocation: &ToolInvocation) -> bool {
        let invocation = PluginInvocation::from_tool_invocation(invocation);
        let Some(entry) = self.plugin_invocation_definition(&invocation) else {
            return false;
        };
        entry.definition.runtime.concurrency_safe
            && !entry.has_tag(crate::plugin::sdk::ToolTag::Interactive)
            && is_concurrency_safe_tool_invocation(&entry, &invocation)
    }

    pub fn available_tools_for_messages(&self, messages: &[Message]) -> Vec<RegisteredTool> {
        let _ = messages;
        self.available_tools()
    }

    pub(crate) fn invocation_definition(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<RegisteredTool> {
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
        self.catalogued_tools()
            .into_iter()
            .find(|entry| registered_tool_matches_name(entry, invocation.tool_name.as_str()))
            .or_else(|| {
                let canonical = canonical_tool_name(invocation.tool_name.as_str());
                self.catalogued_tools()
                    .into_iter()
                    .find(|entry| registered_tool_matches_name(entry, canonical))
            })
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
        self.plugin_invocation_streaming_mode(&PluginInvocation::from_tool_invocation(invocation))
    }

    pub(crate) fn plugin_invocation_streaming_mode(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<SdkToolStreamingMode> {
        self.plugin_resolution_for_plugin_invocation(invocation)
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
        let tags = invocation_effective_tags(&definition, invocation);
        if !self.tool_catalog().are_tags_enabled(&tags) {
            return Err(ToolError::PermissionDenied(format!(
                "tool '{tool_name}' disabled for current model profile"
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
            self.agent
                .authorize_tool_names(&tool_name_aliases, command.as_deref(), &tags),
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
    ) -> Option<crate::plugin::registry::RegisteredTool> {
        self.plugin_resolution_for_plugin_invocation(&PluginInvocation::from_tool_invocation(
            invocation,
        ))
    }

    pub(crate) fn plugin_resolution_for_plugin_invocation(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<crate::plugin::registry::RegisteredTool> {
        self.plugins
            .lookup_tool(invocation.tool_name.as_str())
            .or_else(|| {
                self.plugins
                    .lookup_tool(canonical_tool_name(invocation.tool_name.as_str()))
            })
            .or_else(|| {
                self.plugins
                    .registered_tools()
                    .into_iter()
                    .find(|tool| registered_tool_matches_name(tool, invocation.tool_name.as_str()))
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
        registered_tool: &crate::plugin::registry::RegisteredTool,
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
            Err(err)
                if err.code == crate::plugin::sdk::PluginErrorCode::NotImplemented
                    || err.message.contains("method not found")
                    || err.message.contains("not implemented") =>
            {
                return Ok(());
            }
            Err(err) if err.code == crate::plugin::sdk::PluginErrorCode::InvalidParams => {
                return Err(ToolError::InvalidInput(err.message));
            }
            Err(err) => return Err(ToolError::Plugin(err.message)),
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
        registered_tool: &crate::plugin::registry::RegisteredTool,
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
            Err(err)
                if err.code == crate::plugin::sdk::PluginErrorCode::NotImplemented
                    || err.message.contains("method not found")
                    || err.message.contains("not implemented") =>
            {
                return Ok(());
            }
            Err(err) if err.code == crate::plugin::sdk::PluginErrorCode::InvalidParams => {
                return Err(ToolError::InvalidInput(err.message));
            }
            Err(err) => return Err(ToolError::Plugin(err.message)),
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
                validate_shell_filesystem_effects(tool_name, command, effects.as_slice())?;
            }
            let workdir = input
                .get("workdir")
                .or_else(|| input.pointer("/args/workdir"))
                .and_then(serde_json::Value::as_str);
            let base = self.shell_effect_base_path(workdir);
            self.push_filesystem_effect_checks(checks, effects.as_slice(), base.as_path());
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

    pub(crate) fn requested_path_permission_check(
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
        effects: &[FilesystemEffect],
        base_path: &Path,
    ) {
        for effect in effects {
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
use super::{
    AccessKind, FilesystemEffect, Message, Path, PermissionDecision, PluginInvocation,
    PluginToolPermissionNetworksInput, PluginToolPermissionPathsInput, RegisteredTool,
    SdkInputNetworkSpec, SdkInputPathSpec, SdkNetworkAccessSpec, SdkPathAccessSpec, SdkPathKind,
    SdkToolStreamingMode, ToolError, ToolExecutor, ToolInvocation, ToolPermissionCheck,
    canonical_tool_name, extract_input_network_requests, extract_input_path_requests,
    filesystem_effects_from_input, invocation_effective_tags, invocation_name,
    is_concurrency_safe_tool_invocation, registered_tool_matches_name,
    sdk_path_kind_to_access_kind, shell_command_from_invocation, validate_shell_filesystem_effects,
};
