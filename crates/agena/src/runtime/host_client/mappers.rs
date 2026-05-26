use super::*;

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

pub(super) fn parse_subagent_type(value: &str) -> Result<TaskSubagentType, PluginError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "explore" => Ok(TaskSubagentType::Explore),
        "implement" => Ok(TaskSubagentType::Implement),
        "verify" => Ok(TaskSubagentType::Verify),
        other => Err(PluginError::invalid_params(format!(
            "unknown subagent_type '{other}'"
        ))),
    }
}

pub(super) fn render_tool_descriptor(
    tool: crate::plugin::registry::RegisteredTool,
) -> ToolDescriptor {
    let description = tool.description_text().trim().to_string();
    let summary = tool.summary_text().map(ToString::to_string);
    let help = tool.help_text().map(ToString::to_string);
    let input_schema = Some(tool.sanitized_input_schema());
    let description_mode = tool.decl.description_mode;
    let tags = tool.effective_tags();
    ToolDescriptor {
        name: tool.exposed_name,
        description: (!description.is_empty()).then_some(description),
        summary,
        help,
        input_schema,
        description_mode,
        tags,
        plugin_id: (!tool.plugin_id.trim().is_empty()).then_some(tool.plugin_id),
    }
}

pub(super) fn render_monitor_handle(summary: crate::message::MonitorSummary) -> MonitorHandle {
    MonitorHandle {
        id: summary.monitor_id,
        label: (!summary.description.trim().is_empty()).then_some(summary.description),
        command: (!summary.command.trim().is_empty()).then_some(summary.command),
        status: Some(
            match summary.status {
                MonitorStatus::Running => "running",
                MonitorStatus::Exited => "exited",
                MonitorStatus::Failed => "failed",
                MonitorStatus::Stopped => "stopped",
                MonitorStatus::TimedOut => "timed_out",
            }
            .to_string(),
        ),
        persistent: summary.persistent,
        started_at_ms: summary.started_at_ms,
        ended_at_ms: summary.ended_at_ms,
        buffered_lines: summary.buffered_lines,
        last_seq: summary.last_seq,
        dropped_lines: summary.dropped_lines,
        exit_code: summary.exit_code,
    }
}

pub(super) fn render_monitor_event(event: crate::message::MonitorEvent) -> MonitorEvent {
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
                MonitorStream::Stdout => stdout.push(event.line.clone()),
                MonitorStream::Stderr => stderr.push(event.line.clone()),
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
        running: matches!(read.status, MonitorStatus::Running),
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
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect();
        return Ok(AskUserToolInput { questions });
    }

    if req.prompt.trim().is_empty() {
        return Err(PluginError::invalid_params(
            "ask_user prompt must not be empty",
        ));
    }
    if req.options.is_empty() && !req.allow_free_text {
        return Err(PluginError::invalid_params(
            "ask_user requires options or allow_free_text",
        ));
    }
    let options = req
        .options
        .into_iter()
        .map(|label| UserInputOption {
            label,
            description: String::new(),
        })
        .collect();
    Ok(AskUserToolInput {
        questions: vec![UserInputQuestion {
            id: "reply".to_string(),
            header: String::new(),
            question: req.prompt,
            options,
            multiple: false,
            allow_custom: req.allow_free_text,
        }],
    })
}

pub(super) fn todo_item_from_host(item: HostTodoItem) -> TodoItem {
    TodoItem {
        content: item.content,
        status: match item.status {
            HostTodoStatus::Pending => TodoStatus::Pending,
            HostTodoStatus::InProgress => TodoStatus::InProgress,
            HostTodoStatus::Completed => TodoStatus::Completed,
            HostTodoStatus::Cancelled => TodoStatus::Cancelled,
        },
        priority: match item.priority {
            HostTodoPriority::High => TodoPriority::High,
            HostTodoPriority::Medium => TodoPriority::Medium,
            HostTodoPriority::Low => TodoPriority::Low,
        },
    }
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

pub(super) fn host_goal_from_session_goal(goal: crate::session::SessionGoal) -> HostGoal {
    HostGoal {
        id: goal.id,
        objective: goal.objective,
        status: match goal.status {
            crate::session::GoalStatus::Active => HostGoalStatus::Active,
            crate::session::GoalStatus::Paused => HostGoalStatus::Paused,
            crate::session::GoalStatus::Completed => HostGoalStatus::Completed,
        },
        completed_at_ms: goal.completed_at.map(|value| value.timestamp_millis()),
    }
}

pub(super) fn session_goal_status_from_host(status: HostGoalStatus) -> crate::session::GoalStatus {
    match status {
        HostGoalStatus::Active => crate::session::GoalStatus::Active,
        HostGoalStatus::Paused => crate::session::GoalStatus::Paused,
        HostGoalStatus::Completed => crate::session::GoalStatus::Completed,
    }
}

pub(super) fn map_create_goal_error(err: crate::AppError) -> PluginError {
    match err {
        crate::AppError::Internal(message)
            if message.contains("goal objective must not be empty")
                || message.contains("already has an active goal") =>
        {
            PluginError::invalid_params(message)
        }
        other => PluginError::new(other.to_string()),
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
        state: status.state.as_str().to_string(),
        pid: status.pid,
        restart_count: status.restart_count,
        last_exit_code: status.last_exit_code,
        last_restart_at_ms: status.last_restart_at_ms,
        last_error: status.last_error,
    }
}

pub(super) mod active_invocations {
    //! Reentrancy guard for plugin → host → plugin invocations. We track
    //! the *task-local* set of plugin ids currently being invoked so that a
    //! plugin cannot recurse into itself via the host callback.

    use std::cell::RefCell;
    use std::collections::HashSet;

    thread_local! {
        static ACTIVE: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    }

    pub fn contains(id: &str) -> bool {
        ACTIVE.with(|set| set.borrow().contains(id))
    }

    pub fn current_plugin() -> Option<String> {
        ACTIVE.with(|set| set.borrow().iter().next().cloned())
    }

    pub struct Guard(String);

    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.with(|set| {
                set.borrow_mut().remove(&self.0);
            });
        }
    }

    pub fn enter(id: String) -> Guard {
        ACTIVE.with(|set| {
            set.borrow_mut().insert(id.clone());
        });
        Guard(id)
    }
}
