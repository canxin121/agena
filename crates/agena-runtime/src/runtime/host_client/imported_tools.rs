use agena_domain::{PermissionDecision, StructuredObject, ToolInvocation};
use agena_plugin_host::{
    agent_plugin::{AgentPluginFormat, project_claude_mod_tool_input, project_transcript_tool},
    host::ImportedToolDecision,
    registry::RegisteredTool,
};
use agena_plugin_sdk::{
    PluginError, PluginErrorKind, ToolInvokeOutput,
    host_api::{HookTranscriptFormat, ImportedModToolRequest},
};
use serde_json::{Value, json};

use super::{
    RuntimeHostClient, host_unavailable, plugin_error_from_app, tool_execution_to_invoke_output,
};

impl RuntimeHostClient {
    pub(super) async fn invoke_callback_tool(
        &self,
        tool: &RegisteredTool,
        input: Value,
        allow_own_mod_tools: bool,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let caller = self.callback_context()?;
        let plugin_id = tool.plugin_full_name();
        let cycle = || {
            PluginError::internal(format!(
                "host->plugin invoke would re-enter plugin `{plugin_id}` (cycle detected)"
            ))
        };
        if caller.plugin_id.as_deref() == Some(plugin_id.as_str())
            && !(allow_own_mod_tools
                && tool.claude_mod_tool_name().is_some()
                && caller.tool_name.as_deref() != Some(tool.tool_name()))
        {
            return Err(cycle());
        }
        let session_id = caller
            .session_id
            .ok_or_else(|| host_unavailable("host/tool.invoke requires session_id"))?;
        let input = StructuredObject::try_from(input)
            .map_err(|error| PluginError::invalid_params(format!("invoke_tool input: {error}")))?;
        let invocation = ToolInvocation::new(tool.canonical_name(), input);
        let _delegation = agena_runtime::try_enter_delegation(session_id).ok_or_else(|| {
            PluginError::internal(format!(
                "plugin tool delegation in session {session_id} exceeded its nesting bound (possible cycle through `{plugin_id}`)"
            ))
        })?;
        let manager = self.session_manager()?;
        let execution = match caller.call_id.filter(|call| *call >= 0) {
            Some(call_id) => {
                let _guard =
                    agena_runtime::try_enter_invocation(session_id, call_id, plugin_id.clone())
                        .ok_or_else(cycle)?;
                manager
                    .execute_host_invoked_tool(session_id, call_id, invocation, allow_own_mod_tools)
                    .await
            }
            None if allow_own_mod_tools => {
                let nested = caller.call_id.is_some() && caller.tool_name.is_some();
                manager
                    .execute_session_scoped_mod_tool(session_id, invocation, nested)
                    .await
            }
            None => {
                manager
                    .execute_session_scoped_tool(session_id, invocation)
                    .await
            }
        }
        .map_err(plugin_error_from_app)?;
        Ok(tool_execution_to_invoke_output(execution))
    }

    pub(super) async fn run_imported_mod_tool(
        &self,
        request: ImportedModToolRequest,
    ) -> Result<Value, PluginError> {
        let plugin = self.imported_agent_plugin()?;
        let package = plugin.agent_plugin().expect("imported package");
        let caller = self.callback_context()?;
        let session = caller
            .session_id
            .filter(|id| *id > 0)
            .ok_or_else(|| host_unavailable("Claude tool operations require a project session"))?;
        let manager = self.session_manager()?;
        let (executor, context) = manager
            .host_callback_executor(Some(session), caller.call_id.filter(|call| *call >= 0))
            .await
            .map_err(plugin_error_from_app)?;
        if package.format != AgentPluginFormat::ClaudeCode
            || executor.is_global()
            || context.is_none_or(|context| context.owner.is_some())
            || !executor.plugin_is_enabled(&plugin.key())
        {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "Claude mod tools are unavailable in this session",
            ));
        }
        let tools = executor.detailed_tools_async().await;
        let (name, input, check) = match request {
            ImportedModToolRequest::List => return Ok(list_tools(&tools)),
            ImportedModToolRequest::Check { tool, input } => (tool, input, true),
            ImportedModToolRequest::Call { tool, input } => (tool, input, false),
        };
        let (tool, input) = resolve_tool(&tools, &name, input)?;
        if check {
            let input = StructuredObject::try_from(input).map_err(PluginError::invalid_params)?;
            tool.validate_input(&Value::from(input.clone()))
                .map_err(PluginError::invalid_params)?;
            let invocation = ToolInvocation::new(tool.canonical_name(), input);
            let checks = executor
                .collect_permission_checks_for_invocation_in_session(&invocation, Some(session))
                .await
                .map_err(|error| error.into_plugin_error())?;
            let mut baseline = PermissionDecision::Allow;
            for check in checks {
                match manager
                    .query_host_permission(Some(session), check)
                    .await
                    .map_err(plugin_error_from_app)?
                {
                    denied @ PermissionDecision::Deny { .. } => {
                        baseline = denied;
                        break;
                    }
                    ask @ (PermissionDecision::Ask { .. } | PermissionDecision::Auto { .. }) => {
                        baseline = ask;
                    }
                    PermissionDecision::Allow => {}
                }
            }
            let verdict = executor
                .run_imported_tool_check(&invocation, session, -1, baseline.clone())
                .await;
            let authorized = match (baseline, verdict) {
                (PermissionDecision::Deny { reason }, _) => ImportedToolDecision::Deny {
                    reason: Some(reason),
                },
                (_, Some(verdict @ ImportedToolDecision::Deny { .. })) => verdict,
                (_, Some(verdict @ ImportedToolDecision::Ask { .. })) => verdict,
                (PermissionDecision::Ask { reason } | PermissionDecision::Auto { reason }, _) => {
                    ImportedToolDecision::Ask {
                        reason: Some(reason),
                    }
                }
                (PermissionDecision::Allow, _) => ImportedToolDecision::Allow,
            };
            return Ok(json!(authorized));
        }
        match self.invoke_callback_tool(tool, input, true).await {
            Ok(output) => {
                let text = output.output_text;
                let result = output.payload.unwrap_or_else(|| json!(&text));
                let mut answer = json!({"result": result, "text": text});
                if output
                    .metadata
                    .get("exit_code")
                    .is_some_and(|code| code != "0")
                {
                    answer["isError"] = json!(true);
                }
                Ok(answer)
            }
            Err(error)
                if matches!(
                    error.kind,
                    PluginErrorKind::PolicyDenied
                        | PluginErrorKind::UserDeclined
                        | PluginErrorKind::CapabilityUnavailable
                ) =>
            {
                Ok(json!({"deny":error.failure.user.fallback}))
            }
            Err(error) => Err(error),
        }
    }
}

fn visible_name(tool: &RegisteredTool) -> String {
    let canonical = tool.canonical_name();
    if let Some(name) = tool.claude_mod_tool_name() {
        return name.into();
    }
    let (name, _) = project_transcript_tool(&canonical, Value::Null, HookTranscriptFormat::Claude);
    if name == tool.tool_name()
        || canonical == "agena.shell.spawn"
        || canonical == "agena.notebook.edit_cell"
    {
        canonical
    } else {
        name
    }
}

fn list_tools(tools: &[RegisteredTool]) -> Value {
    let mut names = std::collections::BTreeMap::<String, usize>::new();
    let preferred = tools.iter().map(visible_name).collect::<Vec<_>>();
    for name in &preferred {
        *names.entry(name.clone()).or_default() += 1;
    }
    Value::Array(tools.iter().zip(preferred).map(|(tool, name)| {
        let name = if names[&name] == 1 { name } else { tool.canonical_name() };
        json!({"name":name, "description":tool.help_text().or(tool.summary_text()).unwrap_or_default(), "mcp":false})
    }).collect())
}

fn resolve_tool<'a>(
    tools: &'a [RegisteredTool],
    name: &str,
    mut input: Value,
) -> Result<(&'a RegisteredTool, Value), PluginError> {
    if let Some(tool) = tools.iter().find(|tool| tool.canonical_name() == name) {
        return Ok((tool, input));
    }
    let mut candidates = tools.iter().filter(|tool| visible_name(tool) == name);
    let target = if matches!(name, "Bash" | "PowerShell") {
        let args = input
            .as_object_mut()
            .ok_or_else(|| PluginError::invalid_params("tool input must be an object"))?;
        if let Some(value) = args.remove("dangerouslyDisableSandbox")
            && value != false
        {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "Claude mod tool calls use the session's configured sandbox",
            ));
        }
        if name == "PowerShell" {
            args.insert("shell".into(), json!("powershell"));
        }
        let key = match args.remove("run_in_background") {
            Some(Value::Bool(true)) => "agena.shell.spawn",
            Some(Value::Bool(false)) | None => "agena.shell.exec",
            Some(_) => {
                return Err(PluginError::invalid_params(
                    "run_in_background must be a boolean",
                ));
            }
        };
        tools.iter().find(|tool| tool.canonical_name() == key)
    } else {
        let first = candidates.next();
        if candidates.next().is_some() {
            return Err(PluginError::invalid_params(format!(
                "tool `{name}` is ambiguous; use its full Agena name"
            )));
        }
        first
    };
    let tool = target.ok_or_else(|| {
        PluginError::from_kind(
            PluginErrorKind::ToolUnavailable,
            format!("tool `{name}` is unavailable in this session"),
        )
    })?;
    let input = project_claude_mod_tool_input(tool.tool_key(), input);
    Ok((tool, input))
}
