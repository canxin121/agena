pub(super) fn host_unavailable(message: impl Into<String>) -> PluginError {
    PluginError {
        code: crate::plugin::sdk::PluginErrorCode::HostUnavailable,
        message: message.into(),
        hook: None,
        plugin: None,
        data: None,
    }
}

pub(super) fn tool_execution_to_invoke_output(
    execution: crate::tool::ToolInvocationExecution,
) -> ToolInvokeOutput {
    ToolInvokeOutput {
        title: execution.view.title,
        output_text: execution.view.output_text,
        payload: execution.output.to_json_payload(),
        metadata: execution.view.metadata.into_iter().collect(),
        attachments: execution.view.attachments,
    }
}

pub(super) fn map_storage_error(err: PluginStorageError) -> PluginError {
    use crate::plugin::sdk::PluginErrorCode;
    match err {
        PluginStorageError::MissingPluginId
        | PluginStorageError::MissingSessionId
        | PluginStorageError::MissingWorkspaceRoot
        | PluginStorageError::EmptyNamespace
        | PluginStorageError::EmptyKey
        | PluginStorageError::Data(_) => PluginError::invalid_params(err.to_string()),
        PluginStorageError::SecretUnavailable(_) => PluginError {
            code: PluginErrorCode::HostUnavailable,
            message: err.to_string(),
            hook: None,
            plugin: None,
            data: None,
        },
        PluginStorageError::Io(_) | PluginStorageError::Secret(_) => {
            PluginError::new(err.to_string())
        }
    }
}

pub(super) fn host_permission_check_response_from_resolution(
    resolution: crate::permission::PermissionResolution,
) -> HostPermissionCheckResponse {
    let (decision, reason) = plugin_permission_decision_and_reason(resolution.decision);
    HostPermissionCheckResponse {
        decision,
        reason,
        explanation: resolution.explanation,
    }
}

pub(super) fn host_permission_check_response_from_decision(
    decision: crate::permission::PermissionDecision,
) -> HostPermissionCheckResponse {
    let (decision, reason) = plugin_permission_decision_and_reason(decision);
    let explanation = reason
        .clone()
        .unwrap_or_else(|| "permission allowed by current policy".to_string());
    HostPermissionCheckResponse {
        decision,
        reason,
        explanation,
    }
}

pub(super) fn plugin_permission_decision_and_reason(
    decision: crate::permission::PermissionDecision,
) -> (PluginPermissionDecision, Option<String>) {
    match decision {
        crate::permission::PermissionDecision::Allow => (PluginPermissionDecision::Allow, None),
        crate::permission::PermissionDecision::Ask { reason } => {
            (PluginPermissionDecision::Prompt, Some(reason))
        }
        crate::permission::PermissionDecision::Deny { reason } => {
            (PluginPermissionDecision::Deny, Some(reason))
        }
    }
}

pub(super) fn render_tool_descriptor(
    tool: crate::plugin::registry::RegisteredTool,
) -> ToolDescriptor {
    let brief_summary = tool.summary_text().map(ToString::to_string);
    let mut help_parts = Vec::new();
    if let Some(before_help) = tool.before_help_text() {
        help_parts.push(before_help.to_string());
    }
    if let Some(help) = tool.help_text() {
        help_parts.push(help.to_string());
    }
    if let Some(after_help) = tool.after_help_text() {
        help_parts.push(after_help.to_string());
    }
    let help = (!help_parts.is_empty()).then(|| help_parts.join("\n\n"));
    let summary = match tool.definition.preferred_description_mode() {
        Some(crate::plugin::ToolDescriptionMode::Detailed) => {
            let mut parts = brief_summary.into_iter().collect::<Vec<_>>();
            if let Some(help) = help.as_deref()
                && !parts.iter().any(|part| part.trim() == help.trim())
            {
                parts.push(help.to_owned());
            }
            (!parts.is_empty()).then(|| parts.join("\n\n"))
        }
        Some(crate::plugin::ToolDescriptionMode::Brief) | None => brief_summary,
    };
    let input_schema = Some(tool.input_schema());
    ToolDescriptor {
        name: crate::tool::catalog_target_name(tool.canonical_name().as_str()),
        summary,
        help,
        examples: tool.definition.model.examples.clone(),
        input_schema,
    }
}

pub(super) fn render_monitor_handle(summary: crate::message::ProcessSummary) -> MonitorHandle {
    MonitorHandle {
        id: summary.process_id,
        label: (!summary.description.trim().is_empty()).then_some(summary.description),
        command: (!summary.command.trim().is_empty()).then_some(summary.command),
        status: Some(
            match summary.status {
                ProcessStatus::Running => "running",
                ProcessStatus::Exited => "exited",
                ProcessStatus::Failed => "failed",
                ProcessStatus::Stopped => "stopped",
                ProcessStatus::TimedOut => "timed_out",
            }
            .to_string(),
        ),
        persistent: summary.background,
        started_at_ms: summary.started_at_ms,
        ended_at_ms: summary.ended_at_ms,
        buffered_lines: summary.buffered_lines,
        last_seq: summary.last_seq,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
    }
}

pub(super) fn render_monitor_event(event: crate::message::ProcessEvent) -> MonitorEvent {
    MonitorEvent {
        seq: event.seq,
        stream: event.stream.to_string(),
        ts_ms: event.ts_ms,
        line: event.line,
    }
}

pub(super) fn render_monitor_read(read: crate::tool::MonitorRead) -> MonitorReadResponse {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let events = read
        .events
        .into_iter()
        .map(|event| {
            match event.stream {
                ProcessStream::Stdout => stdout.push(event.line.clone()),
                ProcessStream::Stderr => stderr.push(event.line.clone()),
            }
            render_monitor_event(event)
        })
        .collect::<Vec<_>>();
    MonitorReadResponse {
        monitor_id: Some(read.monitor_id),
        events,
        monitors: Vec::new(),
        stdout: stdout.join("\n"),
        stderr: stderr.join("\n"),
        running: matches!(read.status, ProcessStatus::Running),
        status: Some(read.status.to_string()),
        last_seq: read.last_seq,
        has_more: read.has_more,
        dropped_lines: read.dropped_lines,
        exit_code: read.exit_code,
    }
}

pub(super) fn map_monitor_error(err: MonitorError) -> PluginError {
    match err {
        MonitorError::NotFound(_) | MonitorError::Invalid(_) | MonitorError::InvalidPattern(_) => {
            PluginError::invalid_params(err.to_string())
        }
        other => PluginError::new(other.to_string()),
    }
}

pub(super) fn join_monitor_command(command: &[String]) -> Result<String, PluginError> {
    if command.is_empty() {
        return Err(PluginError::invalid_params(
            "monitor_start requires at least one command token",
        ));
    }
    Ok(command.join(" "))
}

pub(super) fn ask_user_tool_input(req: AskUserRequest) -> Result<AskUserToolInput, PluginError> {
    if !req.questions.is_empty() {
        let questions = req
            .questions
            .into_iter()
            .map(|question| UserInputQuestion {
                id: question.id,
                header: question.header,
                question: question.question,
                options: question
                    .options
                    .into_iter()
                    .map(|option| UserInputOption {
                        label: option.label,
                        description: option.description,
                        preview_markdown: option.preview_markdown,
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect();
        let input = AskUserToolInput {
            title: req.title,
            body_markdown: req.body_markdown,
            kind: req.kind,
            submit_label: req.submit_label,
            cancel_label: req.cancel_label,
            auto_resolution_ms: req.auto_resolution_ms,
            questions,
        };
        return AskUserToolInput::parse_input(
            serde_json::to_value(input)
                .map_err(|err| PluginError::invalid_params(err.to_string()))?,
        );
    }

    let options = req
        .options
        .into_iter()
        .map(|label| UserInputOption {
            label,
            description: String::new(),
            preview_markdown: String::new(),
        })
        .collect();
    let input = AskUserToolInput {
        title: req.title,
        body_markdown: req.body_markdown,
        kind: req.kind,
        submit_label: req.submit_label,
        cancel_label: req.cancel_label,
        auto_resolution_ms: req.auto_resolution_ms,
        questions: vec![UserInputQuestion {
            id: "reply".to_string(),
            header: String::new(),
            question: req.prompt,
            options,
            multiple: false,
            allow_custom: req.allow_free_text,
        }],
    };
    AskUserToolInput::parse_input(
        serde_json::to_value(input).map_err(|err| PluginError::invalid_params(err.to_string()))?,
    )
}

pub(super) fn host_session_from_session(session: &crate::session::Session) -> HostSession {
    HostSession {
        id: session.id,
        parent_id: session.parent_id,
        root_id: session.root_id,
        workspace_id: session.workspace_id,
        title: session.title.clone(),
        is_subagent: session.is_subagent,
    }
}

pub(super) fn workflow_tool_output(
    executor: &crate::tool::ToolExecutor,
    tool_name: &str,
    input: serde_json::Value,
    session_id: Option<i64>,
    call_id: Option<i64>,
    session_context: Option<&crate::session::SessionExecutionContext>,
) -> Result<ToolInvokeOutput, PluginError> {
    executor
        .execute_tool_payload_for_host(tool_name, input, session_id, call_id, session_context)
        .map_err(|err| PluginError::new(err.to_string()))
}

pub(super) fn agent_scope_from_str(scope: &str) -> crate::agents::AgentScope {
    match scope {
        "project" => crate::agents::AgentScope::Project,
        "user" => crate::agents::AgentScope::User,
        _ => crate::agents::AgentScope::Default,
    }
}

pub(super) fn agent_to_descriptor(profile: crate::agents::AgentProfile) -> HostAgentDescriptor {
    HostAgentDescriptor {
        name: profile.name,
        description: profile.frontmatter.description,
        permission: sdk_agent_permission_from_core(profile.frontmatter.permission),
        defaults: HostAgentSelectionConfig {
            provider: profile.frontmatter.defaults.provider,
            adapter: profile.frontmatter.defaults.adapter,
            model: profile.frontmatter.defaults.model,
            thinking_mode: profile.frontmatter.defaults.thinking_mode,
            speed_mode: profile.frontmatter.defaults.speed_mode,
            verbosity: profile.frontmatter.defaults.verbosity,
            parallel_tool_calls: profile.frontmatter.defaults.parallel_tool_calls,
        },
        allowed_tools: profile.frontmatter.tools.allow,
        prompt: profile.prompt,
        scope: match profile.scope {
            crate::agents::AgentScope::Project => "project",
            crate::agents::AgentScope::User => "user",
            crate::agents::AgentScope::Default => "default",
        }
        .to_string(),
    }
}

pub(super) fn core_agent_permission_from_sdk(
    permission: crate::plugin::sdk::host_api::AgentPermissionConfig,
) -> crate::agent::AgentPermissionConfig {
    crate::agent::AgentPermissionConfig {
        path: permission.path.map(core_path_permission_from_sdk),
        network: permission.network.map(core_network_permission_from_sdk),
        tools: permission.tools.map(core_tool_permission_from_sdk),
    }
}

pub(super) fn sdk_agent_permission_from_core(
    permission: crate::agent::AgentPermissionConfig,
) -> crate::plugin::sdk::host_api::AgentPermissionConfig {
    crate::plugin::sdk::host_api::AgentPermissionConfig {
        path: permission.path.map(sdk_path_permission_from_core),
        network: permission.network.map(sdk_network_permission_from_core),
        tools: permission.tools.map(sdk_tool_permission_from_core),
    }
}

pub(super) fn core_path_permission_from_sdk(
    path: crate::plugin::sdk::host_api::AgentPathPermissionConfig,
) -> crate::agent::PathPermissionConfig {
    crate::agent::PathPermissionConfig {
        workspace: path.workspace.map(core_path_access_modes_from_sdk),
        external: path.external.map(core_path_access_modes_from_sdk),
        rules: path
            .rules
            .into_iter()
            .map(|(pattern, rule)| (pattern, core_path_access_rule_from_sdk(rule)))
            .collect(),
    }
}

pub(super) fn sdk_path_permission_from_core(
    path: crate::agent::PathPermissionConfig,
) -> crate::plugin::sdk::host_api::AgentPathPermissionConfig {
    crate::plugin::sdk::host_api::AgentPathPermissionConfig {
        workspace: path.workspace.map(sdk_path_access_modes_from_core),
        external: path.external.map(sdk_path_access_modes_from_core),
        rules: path
            .rules
            .into_iter()
            .map(|(pattern, rule)| (pattern, sdk_path_access_rule_from_core(rule)))
            .collect(),
    }
}

pub(super) fn core_path_access_modes_from_sdk(
    modes: crate::plugin::sdk::host_api::AgentPathAccessModes,
) -> crate::agent::PathAccessModes {
    crate::agent::PathAccessModes {
        read: modes.read.map(core_permission_mode_from_sdk),
        write: modes.write.map(core_permission_mode_from_sdk),
    }
}

pub(super) fn sdk_path_access_modes_from_core(
    modes: crate::agent::PathAccessModes,
) -> crate::plugin::sdk::host_api::AgentPathAccessModes {
    crate::plugin::sdk::host_api::AgentPathAccessModes {
        read: modes.read.map(sdk_permission_mode_from_core),
        write: modes.write.map(sdk_permission_mode_from_core),
    }
}

pub(super) fn core_path_access_rule_from_sdk(
    rule: crate::plugin::sdk::host_api::AgentPathAccessRule,
) -> crate::agent::PathAccessRuleConfig {
    match rule {
        crate::plugin::sdk::host_api::AgentPathAccessRule::Modes(modes) => {
            crate::agent::PathAccessRuleConfig::Modes(core_path_access_modes_from_sdk(modes))
        }
        crate::plugin::sdk::host_api::AgentPathAccessRule::Shorthand(value) => {
            crate::agent::PathAccessRuleConfig::Shorthand(value)
        }
    }
}

pub(super) fn sdk_path_access_rule_from_core(
    rule: crate::agent::PathAccessRuleConfig,
) -> crate::plugin::sdk::host_api::AgentPathAccessRule {
    match rule {
        crate::agent::PathAccessRuleConfig::Modes(modes) => {
            crate::plugin::sdk::host_api::AgentPathAccessRule::Modes(
                sdk_path_access_modes_from_core(modes),
            )
        }
        crate::agent::PathAccessRuleConfig::Shorthand(value) => {
            crate::plugin::sdk::host_api::AgentPathAccessRule::Shorthand(value)
        }
    }
}

pub(super) fn core_network_permission_from_sdk(
    network: crate::plugin::sdk::host_api::AgentNetworkPermissionConfig,
) -> crate::agent::NetworkPermissionConfig {
    crate::agent::NetworkPermissionConfig {
        internet: network.internet.map(core_permission_mode_from_sdk),
        private: network.private.map(core_permission_mode_from_sdk),
        loopback: network.loopback.map(core_permission_mode_from_sdk),
        rules: network
            .rules
            .into_iter()
            .map(|(pattern, mode)| (pattern, core_permission_mode_from_sdk(mode)))
            .collect(),
    }
}

pub(super) fn sdk_network_permission_from_core(
    network: crate::agent::NetworkPermissionConfig,
) -> crate::plugin::sdk::host_api::AgentNetworkPermissionConfig {
    crate::plugin::sdk::host_api::AgentNetworkPermissionConfig {
        internet: network.internet.map(sdk_permission_mode_from_core),
        private: network.private.map(sdk_permission_mode_from_core),
        loopback: network.loopback.map(sdk_permission_mode_from_core),
        rules: network
            .rules
            .into_iter()
            .map(|(pattern, mode)| (pattern, sdk_permission_mode_from_core(mode)))
            .collect(),
    }
}

pub(super) fn core_tool_permission_from_sdk(
    tools: crate::plugin::sdk::host_api::AgentToolPermissionConfig,
) -> crate::agent::ToolPermissionConfig {
    crate::agent::ToolPermissionConfig {
        default: tools.default.map(core_permission_mode_from_sdk),
        tags: tools
            .tags
            .into_iter()
            .map(|(tag, mode)| (tag, core_permission_mode_from_sdk(mode)))
            .collect(),
        names: tools
            .names
            .into_iter()
            .map(|(tool, mode)| (tool, core_permission_mode_from_sdk(mode)))
            .collect(),
        plugin: tools
            .plugin
            .into_iter()
            .map(|(tool, mode)| (tool, core_permission_mode_from_sdk(mode)))
            .collect(),
        rules: tools
            .rules
            .into_iter()
            .map(|(tool, rules)| (tool, core_tool_permission_rules_from_sdk(rules)))
            .collect(),
    }
}

pub(super) fn sdk_tool_permission_from_core(
    tools: crate::agent::ToolPermissionConfig,
) -> crate::plugin::sdk::host_api::AgentToolPermissionConfig {
    crate::plugin::sdk::host_api::AgentToolPermissionConfig {
        default: tools.default.map(sdk_permission_mode_from_core),
        tags: tools
            .tags
            .into_iter()
            .map(|(tag, mode)| (tag, sdk_permission_mode_from_core(mode)))
            .collect(),
        names: tools
            .names
            .into_iter()
            .map(|(tool, mode)| (tool, sdk_permission_mode_from_core(mode)))
            .collect(),
        plugin: tools
            .plugin
            .into_iter()
            .map(|(tool, mode)| (tool, sdk_permission_mode_from_core(mode)))
            .collect(),
        rules: tools
            .rules
            .into_iter()
            .map(|(tool, rules)| (tool, sdk_tool_permission_rules_from_core(rules)))
            .collect(),
    }
}

pub(super) fn core_tool_permission_rules_from_sdk(
    rules: crate::plugin::sdk::host_api::AgentToolPermissionRules,
) -> crate::agent::ToolPermissionRules {
    match rules {
        crate::plugin::sdk::host_api::AgentToolPermissionRules::Mode(mode) => {
            crate::agent::ToolPermissionRules::Mode(core_permission_mode_from_sdk(mode))
        }
        crate::plugin::sdk::host_api::AgentToolPermissionRules::Ordered(entries) => {
            crate::agent::ToolPermissionRules::Ordered(
                entries
                    .into_iter()
                    .map(|(pattern, mode)| (pattern, core_permission_mode_from_sdk(mode)))
                    .collect(),
            )
        }
    }
}

pub(super) fn sdk_tool_permission_rules_from_core(
    rules: crate::agent::ToolPermissionRules,
) -> crate::plugin::sdk::host_api::AgentToolPermissionRules {
    match rules {
        crate::agent::ToolPermissionRules::Mode(mode) => {
            crate::plugin::sdk::host_api::AgentToolPermissionRules::Mode(
                sdk_permission_mode_from_core(mode),
            )
        }
        crate::agent::ToolPermissionRules::Ordered(entries) => {
            crate::plugin::sdk::host_api::AgentToolPermissionRules::Ordered(
                entries
                    .into_iter()
                    .map(|(pattern, mode)| (pattern, sdk_permission_mode_from_core(mode)))
                    .collect(),
            )
        }
    }
}

pub(super) fn core_permission_mode_from_sdk(
    mode: crate::plugin::sdk::host_api::AgentPermissionMode,
) -> crate::permission::PermissionMode {
    match mode {
        crate::plugin::sdk::host_api::AgentPermissionMode::Allow => {
            crate::permission::PermissionMode::Allow
        }
        crate::plugin::sdk::host_api::AgentPermissionMode::Ask => {
            crate::permission::PermissionMode::Ask
        }
        crate::plugin::sdk::host_api::AgentPermissionMode::Deny => {
            crate::permission::PermissionMode::Deny
        }
    }
}

pub(super) fn sdk_permission_mode_from_core(
    mode: crate::permission::PermissionMode,
) -> crate::plugin::sdk::host_api::AgentPermissionMode {
    match mode {
        crate::permission::PermissionMode::Allow => {
            crate::plugin::sdk::host_api::AgentPermissionMode::Allow
        }
        crate::permission::PermissionMode::Ask => {
            crate::plugin::sdk::host_api::AgentPermissionMode::Ask
        }
        crate::permission::PermissionMode::Deny => {
            crate::plugin::sdk::host_api::AgentPermissionMode::Deny
        }
    }
}

pub(super) fn scheduler_job_to_sdk(job: agena_scheduler::ScheduledJob) -> HostSchedulerJob {
    let (kind, cron_expression, fire_at_ms) = match &job.kind {
        agena_scheduler::JobKind::Cron { expression, .. } => {
            ("cron".to_string(), Some(expression.clone()), None)
        }
        agena_scheduler::JobKind::Once { at } => {
            ("once".to_string(), None, Some(at.timestamp_millis()))
        }
    };
    HostSchedulerJob {
        id: job.id.to_string(),
        kind,
        prompt: job.prompt.clone(),
        cron_expression,
        fire_at_ms,
        owner_session_id: job.owner_session_id,
        next_fire_at_ms: job.next_fire_at.map(|t| t.timestamp_millis()),
        last_fired_at_ms: job.last_fired_at.map(|t| t.timestamp_millis()),
    }
}

pub(super) fn lsp_severity_string(
    severity: Option<agena_lsp::lsp_types::DiagnosticSeverity>,
) -> String {
    match severity {
        Some(agena_lsp::lsp_types::DiagnosticSeverity::ERROR) => "error".to_string(),
        Some(agena_lsp::lsp_types::DiagnosticSeverity::WARNING) => "warning".to_string(),
        Some(agena_lsp::lsp_types::DiagnosticSeverity::INFORMATION) => "information".to_string(),
        Some(agena_lsp::lsp_types::DiagnosticSeverity::HINT) => "hint".to_string(),
        Some(_) => "unknown".to_string(),
        None => "unknown".to_string(),
    }
}

pub(super) fn host_status_to_sdk(
    status: agena_plugin_host::status::PluginStatus,
) -> HostPluginStatus {
    HostPluginStatus {
        plugin_id: status.plugin_id,
        kind: status.kind.to_string(),
        state: status.state.to_string(),
        pid: status.pid,
        restart_count: status.restart_count,
        last_exit_code: status.last_exit_code,
        last_restart_at_ms: status.last_restart_at_ms,
        last_error: status.last_error,
    }
}

pub(super) mod active_invocations {
    //! Reentrancy guard for plugin → host → plugin invocations.
    //!
    //! A guard must follow the logical host invocation, not the executor
    //! thread. Plugin callbacks can await, migrate between Tokio workers, and
    //! enter nested plugin-host runtimes. A `thread_local!` guard therefore
    //! both leaked when it was dropped on a different worker and made
    //! unrelated calls on one worker look recursive. The session/call pair is
    //! stable throughout a host callback chain, so it is the correct scope.

    use std::collections::{HashMap, HashSet};
    use std::sync::{LazyLock, Mutex};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    struct InvocationScope {
        session_id: i64,
        call_id: i64,
    }

    static ACTIVE: LazyLock<Mutex<HashMap<InvocationScope, HashSet<String>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    pub struct Guard {
        scope: InvocationScope,
        plugin_id: String,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            let Ok(mut active) = ACTIVE.lock() else {
                return;
            };
            let Some(plugins) = active.get_mut(&self.scope) else {
                return;
            };
            plugins.remove(self.plugin_id.as_str());
            if plugins.is_empty() {
                active.remove(&self.scope);
            }
        }
    }

    /// Atomically enter a target plugin for one logical host invocation.
    /// Returns `None` only when that exact invocation already has the target
    /// plugin on its call chain.
    pub fn try_enter(session_id: i64, call_id: i64, plugin_id: String) -> Option<Guard> {
        let scope = InvocationScope {
            session_id,
            call_id,
        };
        let mut active = ACTIVE.lock().ok()?;
        let plugins = active.entry(scope).or_default();
        if !plugins.insert(plugin_id.clone()) {
            return None;
        }
        Some(Guard { scope, plugin_id })
    }

    #[cfg(test)]
    fn is_active(session_id: i64, call_id: i64, plugin_id: &str) -> bool {
        let scope = InvocationScope {
            session_id,
            call_id,
        };
        ACTIVE
            .lock()
            .ok()
            .and_then(|active| active.get(&scope).cloned())
            .is_some_and(|plugins| plugins.contains(plugin_id))
    }

    #[cfg(test)]
    mod tests {
        use super::{is_active, try_enter};

        #[test]
        fn reentrancy_is_scoped_to_one_session_call_chain() {
            let first = try_enter(9_001, 101, "example.target".to_string())
                .expect("enter first invocation");
            assert!(is_active(9_001, 101, "example.target"));

            assert!(try_enter(9_001, 101, "example.target".to_string()).is_none());
            let other_call = try_enter(9_001, 102, "example.target".to_string())
                .expect("same target is valid for a distinct call");
            let other_session = try_enter(9_002, 101, "example.target".to_string())
                .expect("same target is valid for a distinct session");

            drop(other_call);
            drop(other_session);
            drop(first);
            assert!(!is_active(9_001, 101, "example.target"));
            assert!(try_enter(9_001, 101, "example.target".to_string()).is_some());
        }

        #[test]
        fn guard_cleanup_is_not_bound_to_the_entry_thread() {
            let guard =
                try_enter(9_003, 103, "example.target".to_string()).expect("enter invocation");
            assert!(is_active(9_003, 103, "example.target"));

            std::thread::spawn(move || drop(guard))
                .join()
                .expect("drop guard from another thread");

            assert!(!is_active(9_003, 103, "example.target"));
            assert!(try_enter(9_003, 103, "example.target".to_string()).is_some());
        }
    }
}
use super::{
    AskUserRequest, AskUserToolInput, HostAgentDescriptor, HostAgentSelectionConfig,
    HostPermissionCheckResponse, HostPluginStatus, HostSchedulerJob, HostSession, MonitorError,
    MonitorEvent, MonitorHandle, MonitorReadResponse, PluginError, PluginPermissionDecision,
    PluginStorageError, ProcessStatus, ProcessStream, ToolDescriptor, ToolInvokeOutput,
    UserInputOption, UserInputQuestion,
};
