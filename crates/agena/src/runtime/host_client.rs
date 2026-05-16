//! Concrete `HostClient` impl backed by the live `AgenaRuntime`. Plugins
//! that run as subprocess (stdio) or remote (HTTP) call back into this via
//! JSON-RPC; the `HostHandle` in `agena-plugin-host` routes those calls
//! through this client.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::event::Scope;
use async_trait::async_trait;

use crate::message::{
    AskUserToolInput, EnterPlanModeToolInput, EnterWorktreeToolInput, ExitPlanModeToolInput,
    ExitWorktreeToolInput, MonitorStatus, MonitorStream, StructuredObject, TaskSubagentType,
    TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput, ToolInvocation, UserInputOption,
    UserInputQuestion,
};
use crate::plugin::sdk::host_api::{
    AskUserRequest, AskUserResponse, EventSubscription, HostAgentDescriptor, HostAgentListResponse,
    HostAgentRegisterRequest, HostAgentRemoveRequest, HostAgentRemoveResponse, HostCallbackContext,
    HostClearGoalRequest, HostClearGoalResponse, HostClient, HostCreateGoalRequest,
    HostCreateGoalResponse, HostEnterPlanModeRequest, HostEnterWorktreeRequest,
    HostExitPlanModeRequest, HostExitWorktreeRequest, HostGetGoalRequest, HostGetGoalResponse,
    HostGoal, HostGoalStatus, HostLspDiagnostic, HostLspListDiagnosticsRequest,
    HostLspListDiagnosticsResponse, HostLspListServersResponse, HostLspServer,
    HostMcpAddServerRequest, HostMcpListServersResponse, HostMcpRemoveServerRequest,
    HostMcpRemoveServerResponse, HostMcpServerSpec, HostNetworkPermissionCheckRequest,
    HostPathPermissionCheckRequest, HostPermissionCheckResponse, HostPlanEntry, HostPlanGetRequest,
    HostPlanGetResponse, HostPlanListResponse, HostPluginStatus, HostPluginStatusGetRequest,
    HostPluginStatusGetResponse, HostPluginStatusListResponse, HostSchedulerCreateRequest,
    HostSchedulerCreateResponse, HostSchedulerDeleteRequest, HostSchedulerDeleteResponse,
    HostSchedulerJob, HostSchedulerListResponse, HostSecretDeleteRequest, HostSecretGetRequest,
    HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest, HostStorageDeleteRequest,
    HostStorageEntry, HostStorageGetRequest, HostStorageGetResponse, HostStorageListRequest,
    HostStorageListResponse, HostStorageSetRequest, HostTodoItem, HostTodoPriority, HostTodoStatus,
    HostTodoWriteRequest, HostUpdateGoalRequest, HostUpdateGoalResponse, HostWorktreeEntry,
    HostWorktreeListResponse, LogLevel, MonitorEvent, MonitorHandle, MonitorReadRequest,
    MonitorReadResponse, MonitorStartRequest, MonitorStopRequest, NoopHostClient,
    SpawnSubtaskRequest, SpawnSubtaskResponse, ToolDescriptor, current_host_callback_context,
};
use crate::plugin::{
    EventEnvelope, EventFilter as PluginEventFilter, PermissionAskInput,
    PermissionDecision as PluginPermissionDecision, PluginError, ToolInvokeOutput,
};
use crate::plugins::storage::{
    PluginSecretStore, PluginStorage, PluginStorageError, StorageLocator,
};
use crate::runtime::AgenaRuntime;
use crate::tool::{MonitorError, MonitorReadParams, MonitorStartParams};

/// Build a `HostClient` impl for a runtime; use [`NoopHostClient`] when no
/// runtime is available (e.g. before bootstrap completes).
pub fn host_client_for(runtime: AgenaRuntime) -> Arc<dyn HostClient> {
    Arc::new(RuntimeHostClient { runtime })
}

pub fn noop_host_client() -> Arc<dyn HostClient> {
    Arc::new(NoopHostClient)
}

struct RuntimeHostClient {
    runtime: AgenaRuntime,
}

impl RuntimeHostClient {
    fn session_manager(&self) -> Result<Arc<crate::session::SessionManager>, PluginError> {
        self.runtime
            .current_snapshot()
            .session_manager()
            .ok_or_else(|| host_unavailable("session manager is not enabled in this runtime"))
    }

    fn tool_executor(&self) -> Result<crate::tool::ToolExecutor, PluginError> {
        Ok(self.session_manager()?.tool_executor())
    }

    async fn callback_session_context(
        &self,
    ) -> Result<Option<crate::session::SessionExecutionContext>, PluginError> {
        let Some(session_id) =
            current_host_callback_context().and_then(|context| context.session_id)
        else {
            return Ok(None);
        };
        let Some(manager) = self.runtime.current_snapshot().session_manager() else {
            return Ok(None);
        };
        let session = manager
            .get_session(session_id)
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(Some(session.runtime.execution))
    }

    async fn callback_scoped_tool_executor(
        &self,
    ) -> Result<
        (
            crate::tool::ToolExecutor,
            Option<crate::session::SessionExecutionContext>,
        ),
        PluginError,
    > {
        let executor = self.tool_executor()?;
        let session_context = self.callback_session_context().await?;
        let executor = session_context
            .as_ref()
            .map(|context| executor.for_session_context(context))
            .unwrap_or(executor);
        Ok((executor, session_context))
    }

    fn callback_context(&self) -> Result<HostCallbackContext, PluginError> {
        current_host_callback_context()
            .ok_or_else(|| host_unavailable("host callback context is not available"))
    }

    fn callback_session_and_call(&self) -> Result<(i64, i64), PluginError> {
        let context = self.callback_context()?;
        let session_id = context
            .session_id
            .ok_or_else(|| host_unavailable("host callback context is missing session_id"))?;
        let call_id = context
            .call_id
            .ok_or_else(|| host_unavailable("host callback context is missing call_id"))?;
        Ok((session_id, call_id))
    }

    fn callback_plugin_id(&self) -> Result<String, PluginError> {
        self.callback_context()?
            .plugin_id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| host_unavailable("host callback context is missing plugin_id"))
    }

    fn storage_locator(
        &self,
        scope: crate::plugin::sdk::host_api::HostStorageScope,
        visibility: crate::plugin::sdk::host_api::HostStorageVisibility,
    ) -> Result<StorageLocator, PluginError> {
        let plugin_id = self.callback_plugin_id()?;
        let context = self.callback_context()?;
        StorageLocator::new(
            scope,
            visibility,
            plugin_id,
            context.session_id,
            context.workspace_root,
        )
        .map_err(map_storage_error)
    }

    fn plugin_storage(&self) -> Arc<dyn PluginStorage> {
        self.runtime
            .current_snapshot()
            .config_resolution()
            .config
            .plugin_storage()
    }

    fn plugin_secret_store(&self) -> Arc<dyn PluginSecretStore> {
        self.runtime
            .current_snapshot()
            .config_resolution()
            .config
            .plugin_secret_store()
    }

    fn agents(&self) -> crate::agents::SubagentRegistry {
        self.runtime.current_snapshot().agents()
    }

    async fn resolve_permission_check(
        &self,
        check: crate::tool::ToolPermissionCheck,
    ) -> Result<HostPermissionCheckResponse, PluginError> {
        let session_id = current_host_callback_context()
            .and_then(|context| context.session_id)
            .filter(|session_id| *session_id >= 0);
        let Some(manager) = self.runtime.current_snapshot().session_manager() else {
            return Ok(host_permission_check_response_from_decision(check.decision));
        };
        let resolution = manager
            .resolve_tool_permission_check(session_id, &check)
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(host_permission_check_response_from_resolution(resolution))
    }
}

fn host_unavailable(message: impl Into<String>) -> PluginError {
    PluginError {
        code: crate::plugin::sdk::PluginErrorCode::HostUnavailable,
        message: message.into(),
        hook: None,
        plugin: None,
        data: None,
    }
}

fn tool_execution_to_invoke_output(
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

fn map_storage_error(err: PluginStorageError) -> PluginError {
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

fn host_permission_check_response_from_resolution(
    resolution: crate::permission::PermissionResolution,
) -> HostPermissionCheckResponse {
    let (decision, reason) = plugin_permission_decision_and_reason(resolution.decision);
    HostPermissionCheckResponse {
        decision,
        reason,
        explanation: resolution.explanation,
    }
}

fn host_permission_check_response_from_decision(
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

fn plugin_permission_decision_and_reason(
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

fn parse_subagent_type(value: &str) -> Result<TaskSubagentType, PluginError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "explore" => Ok(TaskSubagentType::Explore),
        "implement" => Ok(TaskSubagentType::Implement),
        "verify" => Ok(TaskSubagentType::Verify),
        other => Err(PluginError::invalid_params(format!(
            "unknown subagent_type '{other}'"
        ))),
    }
}

fn render_tool_descriptor(tool: crate::plugin::registry::PluginEntry) -> ToolDescriptor {
    let deferred = tool.is_deferred();
    let description = tool.description_text().trim().to_string();
    let tags = tool.effective_tags();
    ToolDescriptor {
        name: tool.exposed_name,
        description: (!description.is_empty()).then_some(description),
        tags,
        deferred,
        plugin_id: (!tool.plugin_name.trim().is_empty()).then_some(tool.plugin_name),
    }
}

fn render_monitor_handle(summary: crate::message::MonitorSummary) -> MonitorHandle {
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

fn render_monitor_event(event: crate::message::MonitorEvent) -> MonitorEvent {
    MonitorEvent {
        seq: event.seq,
        stream: event.stream.to_string(),
        ts_ms: event.ts_ms,
        line: event.line,
    }
}

fn render_monitor_read(read: crate::tool::MonitorRead) -> MonitorReadResponse {
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

fn map_monitor_error(err: MonitorError) -> PluginError {
    match err {
        MonitorError::NotFound(_) | MonitorError::Invalid(_) | MonitorError::InvalidPattern(_) => {
            PluginError::invalid_params(err.to_string())
        }
        other => PluginError::new(other.to_string()),
    }
}

fn join_monitor_command(command: &[String]) -> Result<String, PluginError> {
    if command.is_empty() {
        return Err(PluginError::invalid_params(
            "monitor_start requires at least one command token",
        ));
    }
    Ok(command.join(" "))
}

fn ask_user_tool_input(req: AskUserRequest) -> Result<AskUserToolInput, PluginError> {
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

fn todo_item_from_host(item: HostTodoItem) -> TodoItem {
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

fn host_goal_from_session_goal(goal: crate::session::SessionGoal) -> HostGoal {
    HostGoal {
        id: goal.id,
        objective: goal.objective,
        status: match goal.status {
            crate::session::GoalStatus::Active => HostGoalStatus::Active,
            crate::session::GoalStatus::Paused => HostGoalStatus::Paused,
            crate::session::GoalStatus::BudgetLimited => HostGoalStatus::BudgetLimited,
            crate::session::GoalStatus::Completed => HostGoalStatus::Completed,
        },
        token_budget: goal.token_budget,
        tokens_used: goal.tokens_used,
        time_used_seconds: goal.time_used_seconds,
        completed_at_ms: goal.completed_at.map(|value| value.timestamp_millis()),
    }
}

fn session_goal_status_from_host(status: HostGoalStatus) -> crate::session::GoalStatus {
    match status {
        HostGoalStatus::Active => crate::session::GoalStatus::Active,
        HostGoalStatus::Paused => crate::session::GoalStatus::Paused,
        HostGoalStatus::BudgetLimited => crate::session::GoalStatus::BudgetLimited,
        HostGoalStatus::Completed => crate::session::GoalStatus::Completed,
    }
}

fn map_create_goal_error(err: crate::AppError) -> PluginError {
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

fn workflow_tool_output(
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

#[async_trait]
impl HostClient for RuntimeHostClient {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        match level {
            LogLevel::Trace => {
                tracing::trace!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Debug => {
                tracing::debug!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Info => {
                tracing::info!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Warn => {
                tracing::warn!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Error => {
                tracing::error!(target: "plugin", ?fields, "{message}");
            }
        }
        let plugin_id = current_host_callback_context()
            .and_then(|context| context.plugin_id)
            .unwrap_or_else(|| "<unknown>".into());
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .append_plugin_log(
                plugin_id,
                format!("{level:?}").to_lowercase(),
                "plugin",
                message,
                fields,
            );
    }

    async fn publish_event(&self, env: EventEnvelope) -> Result<(), PluginError> {
        let snapshot = self.runtime.current_snapshot();
        let Some(manager) = snapshot.session_manager() else {
            tracing::debug!(
                target: "plugin",
                "publish_event ignored: no session manager"
            );
            return Ok(());
        };
        let publisher = manager.event_publisher();
        let plugin_id = current_host_callback_context()
            .and_then(|context| context.plugin_id)
            .or_else(active_invocations::current_plugin)
            .unwrap_or_else(|| "<unknown>".into());
        let kind = crate::event::EventKind::PluginEvent(crate::event::PluginEventPayload {
            plugin_id,
            kind_label: env.kind,
            payload: env.payload,
        });
        let ctx = match env.session_id {
            Some(id) => crate::event::PublishContext::for_session(id),
            None => crate::event::PublishContext::default(),
        };
        publisher
            .publish(ctx, kind)
            .await
            .map_err(|e| PluginError::new(format!("event publish failed: {e}")))?;
        Ok(())
    }

    async fn subscribe_events(
        &self,
        filter: PluginEventFilter,
    ) -> Result<EventSubscription, PluginError> {
        // Translate the SDK filter to agena's filter and confirm; the actual
        // event push back to the plugin already happens via the snapshot's
        // `event_bridge`. Returning a deterministic id so plugins can ack.
        let id = format!("sub-{}", uuid::Uuid::new_v4().simple());
        let _ = filter; // currently unused beyond existence
        let _bus = self
            .runtime
            .current_snapshot()
            .session_manager()
            .map(|mgr| {
                let _ = mgr
                    .event_bus()
                    .subscribe(crate::event::EventFilter::new(Scope::Global));
            });
        Ok(EventSubscription { id })
    }

    async fn ask_permission(
        &self,
        _req: PermissionAskInput,
    ) -> Result<PluginPermissionDecision, PluginError> {
        // The host doesn't surface a unified "ask user" affordance here.
        // For now, default to Prompt (i.e. "host has no opinion, fall back").
        Ok(PluginPermissionDecision::Prompt)
    }

    async fn check_path_permission(
        &self,
        req: HostPathPermissionCheckRequest,
    ) -> Result<HostPermissionCheckResponse, PluginError> {
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        self.resolve_permission_check(
            executor.requested_path_permission_check(req.path.as_str(), req.kind),
        )
        .await
    }

    async fn check_network_permission(
        &self,
        req: HostNetworkPermissionCheckRequest,
    ) -> Result<HostPermissionCheckResponse, PluginError> {
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let check = executor
            .network_permission_check(req.target.as_str())
            .map_err(|err| PluginError::invalid_params(err.to_string()))?;
        self.resolve_permission_check(check).await
    }

    async fn read_config(&self, path: Option<String>) -> Result<serde_json::Value, PluginError> {
        let snapshot = self.runtime.current_snapshot();
        let value = serde_json::to_value(snapshot.config_resolution())
            .map_err(|e| PluginError::invalid_params(e.to_string()))?;
        if let Some(path) = path {
            // Dot-notation path: `runtime.session_cache.max_sessions`
            let mut cursor = &value;
            for segment in path.split('.') {
                if segment.is_empty() {
                    continue;
                }
                cursor = match cursor.get(segment) {
                    Some(v) => v,
                    None => return Ok(serde_json::Value::Null),
                };
            }
            Ok(cursor.clone())
        } else {
            Ok(value)
        }
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let host = self.runtime.current_snapshot().plugin_manager();
        let resolution = host
            .lookup_entry(&tool)
            .ok_or_else(|| PluginError::new(format!("entry `{tool}` not found")))?;

        let caller = self.callback_context()?;
        let plugin_id = resolution.plugin_name.clone();
        if caller
            .plugin_id
            .as_ref()
            .is_some_and(|current| current == &plugin_id)
            || active_invocations::contains(&plugin_id)
        {
            return Err(PluginError::new(format!(
                "host->plugin invoke would re-enter plugin `{plugin_id}` (cycle detected)"
            )));
        }
        let _guard = active_invocations::enter(plugin_id.clone());

        let session_id = caller
            .session_id
            .ok_or_else(|| host_unavailable("host/tool.invoke requires session_id"))?;
        let call_id = caller.call_id.unwrap_or(-1);
        let structured = StructuredObject::try_from(input)
            .map_err(|err| PluginError::invalid_params(format!("invoke_tool input: {err}")))?;
        let invocation = ToolInvocation::new(tool, structured);
        let execution = self
            .session_manager()?
            .execute_host_invoked_tool(session_id, call_id, invocation)
            .await
            .map_err(|err| PluginError::new(format!("host/tool.invoke failed: {err}")))?;

        Ok(tool_execution_to_invoke_output(execution))
    }

    async fn ask_user(&self, req: AskUserRequest) -> Result<AskUserResponse, PluginError> {
        let (session_id, call_id) = self.callback_session_and_call()?;
        let input = ask_user_tool_input(req)?;
        self.session_manager()?
            .request_host_user_input(session_id, call_id, input)
            .await
            .map_err(|err| PluginError::new(err.to_string()))
    }

    async fn spawn_subtask(
        &self,
        req: SpawnSubtaskRequest,
    ) -> Result<SpawnSubtaskResponse, PluginError> {
        let (parent_session_id, _) = self.callback_session_and_call()?;
        let executor = self.tool_executor()?;
        let requested_profile = req.subagent_type.trim();
        if requested_profile.is_empty() {
            return Err(PluginError::invalid_params(
                "subagent_type must not be empty",
            ));
        }
        let subagent_type =
            parse_subagent_type(requested_profile).unwrap_or(TaskSubagentType::Explore);
        let response = self
            .session_manager()?
            .spawn_subtask(crate::session::SessionSubtaskRequest {
                parent_session_id,
                description: req.description.clone(),
                prompt: req.prompt.clone(),
                subagent_type,
                profile_name: Some(requested_profile.to_string()),
                task_id: req.task_id.clone(),
                command: req.command.clone(),
                requested_model: req.model.clone(),
            })
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;

        let session = response.session;
        let mut metadata = BTreeMap::new();
        metadata.insert("session_id".to_string(), session.id.to_string());
        metadata.insert(
            "subagent_type".to_string(),
            response
                .profile_name
                .clone()
                .unwrap_or_else(|| requested_profile.to_string()),
        );
        if let Some(model) = req.model {
            metadata.insert("requested_model".to_string(), model);
        }
        if let Some(model_provider_id) = response.model_provider_id.clone() {
            metadata.insert("model_provider_id".to_string(), model_provider_id);
        }
        if let Some(model_id) = response.model_id.clone() {
            metadata.insert("model_id".to_string(), model_id);
        }
        if let Some(command) = req.command.clone() {
            metadata.insert("command".to_string(), command);
        }
        metadata.insert("description".to_string(), req.description.clone());

        Ok(SpawnSubtaskResponse {
            final_text: format!(
                "Created/resumed subtask session {} for profile '{}' in workspace {}.",
                session.id,
                response
                    .profile_name
                    .as_deref()
                    .unwrap_or(requested_profile),
                executor.display_path(executor.workspace_root())
            ),
            metadata,
        })
    }

    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, PluginError> {
        let executor = self.tool_executor()?;
        Ok(executor
            .searchable_tools()
            .into_iter()
            .map(render_tool_descriptor)
            .collect())
    }

    async fn todo_write(&self, req: HostTodoWriteRequest) -> Result<ToolInvokeOutput, PluginError> {
        let context = self.callback_context()?;
        let (executor, session_context) = self.callback_scoped_tool_executor().await?;
        workflow_tool_output(
            &executor,
            "todo_write",
            serde_json::to_value(TodoWriteToolInput {
                items: req.items.into_iter().map(todo_item_from_host).collect(),
            })
            .map_err(|err| PluginError::new(err.to_string()))?,
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
            session_context.as_ref(),
        )
    }

    async fn get_goal(&self, _req: HostGetGoalRequest) -> Result<HostGetGoalResponse, PluginError> {
        let session_id = self.callback_context()?.session_id.ok_or_else(|| {
            host_unavailable("host callback context is missing session_id for get_goal")
        })?;
        let goal = self
            .session_manager()?
            .get_goal(session_id)
            .await
            .map_err(|err| PluginError::new(err.to_string()))?
            .map(host_goal_from_session_goal);
        Ok(HostGetGoalResponse { goal })
    }

    async fn create_goal(
        &self,
        req: HostCreateGoalRequest,
    ) -> Result<HostCreateGoalResponse, PluginError> {
        let session_id = self.callback_context()?.session_id.ok_or_else(|| {
            host_unavailable("host callback context is missing session_id for create_goal")
        })?;
        if req.objective.trim().is_empty() {
            return Err(PluginError::invalid_params(
                "goal objective must not be empty",
            ));
        }

        let manager = self.session_manager()?;
        if manager
            .get_goal(session_id)
            .await
            .map_err(|err| PluginError::new(err.to_string()))?
            .is_some()
        {
            return Err(PluginError::invalid_params(format!(
                "session {session_id} already has an active goal"
            )));
        }

        let goal = manager
            .create_goal(crate::session::SessionGoalCreateRequest {
                session_id,
                objective: req.objective,
                token_budget: req.token_budget,
            })
            .await
            .map_err(map_create_goal_error)?;
        Ok(HostCreateGoalResponse {
            goal: host_goal_from_session_goal(goal),
        })
    }

    async fn update_goal(
        &self,
        req: HostUpdateGoalRequest,
    ) -> Result<HostUpdateGoalResponse, PluginError> {
        let session_id = self.callback_context()?.session_id.ok_or_else(|| {
            host_unavailable("host callback context is missing session_id for update_goal")
        })?;
        let goal = self
            .session_manager()?
            .update_goal(crate::session::SessionGoalUpdateRequest {
                session_id,
                objective: req.objective,
                status: req.status.map(session_goal_status_from_host),
                token_budget: req.token_budget,
                expected_goal_id: None,
            })
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(HostUpdateGoalResponse {
            goal: host_goal_from_session_goal(goal),
        })
    }

    async fn clear_goal(
        &self,
        _req: HostClearGoalRequest,
    ) -> Result<HostClearGoalResponse, PluginError> {
        let session_id = self.callback_context()?.session_id.ok_or_else(|| {
            host_unavailable("host callback context is missing session_id for clear_goal")
        })?;
        let cleared = self
            .session_manager()?
            .clear_goal(session_id)
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(HostClearGoalResponse { cleared })
    }

    async fn enter_plan_mode(
        &self,
        _req: HostEnterPlanModeRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let context = self.callback_context()?;
        let (executor, session_context) = self.callback_scoped_tool_executor().await?;
        workflow_tool_output(
            &executor,
            "enter_plan_mode",
            serde_json::to_value(EnterPlanModeToolInput::default())
                .map_err(|err| PluginError::new(err.to_string()))?,
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
            session_context.as_ref(),
        )
    }

    async fn exit_plan_mode(
        &self,
        _req: HostExitPlanModeRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let context = self.callback_context()?;
        let (executor, session_context) = self.callback_scoped_tool_executor().await?;
        workflow_tool_output(
            &executor,
            "exit_plan_mode",
            serde_json::to_value(ExitPlanModeToolInput::default())
                .map_err(|err| PluginError::new(err.to_string()))?,
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
            session_context.as_ref(),
        )
    }

    async fn enter_worktree(
        &self,
        req: HostEnterWorktreeRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let context = self.callback_context()?;
        let (executor, session_context) = self.callback_scoped_tool_executor().await?;
        workflow_tool_output(
            &executor,
            "enter_worktree",
            serde_json::to_value(EnterWorktreeToolInput {
                name: req.name,
                path: req.path,
            })
            .map_err(|err| PluginError::new(err.to_string()))?,
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
            session_context.as_ref(),
        )
    }

    async fn exit_worktree(
        &self,
        req: HostExitWorktreeRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let context = self.callback_context()?;
        let (executor, session_context) = self.callback_scoped_tool_executor().await?;
        workflow_tool_output(
            &executor,
            "exit_worktree",
            serde_json::to_value(ExitWorktreeToolInput {
                action: req.action,
                discard_changes: req.discard_changes,
            })
            .map_err(|err| PluginError::new(err.to_string()))?,
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
            session_context.as_ref(),
        )
    }

    async fn monitor_start(&self, req: MonitorStartRequest) -> Result<MonitorHandle, PluginError> {
        let executor = self.tool_executor()?;
        let registry = executor
            .monitor_registry()
            .ok_or_else(|| host_unavailable("monitor registry is not enabled in this runtime"))?;
        let cwd = req
            .cwd
            .as_deref()
            .map(|path| executor.resolve_target_path(path))
            .unwrap_or_else(|| executor.workspace_root().to_path_buf());
        executor
            .ensure_read_permission(&cwd)
            .map_err(|err| PluginError::new(err.to_string()))?;
        let command = join_monitor_command(&req.command)?;
        let env = if req.env.is_empty() {
            std::env::vars().collect()
        } else {
            req.env.into_iter().collect()
        };
        let summary = registry
            .start(MonitorStartParams {
                description: req.label.unwrap_or_else(|| command.clone()),
                command,
                workdir: cwd,
                timeout_ms: req.timeout_ms,
                persistent: req.persistent,
                include_pattern: req.include_pattern,
                max_buffered_lines: req.max_buffered_lines,
                capture_stderr: req.capture_stderr,
                env,
            })
            .map_err(map_monitor_error)?
            .summary;
        Ok(render_monitor_handle(summary))
    }

    async fn monitor_list(&self) -> Result<Vec<MonitorHandle>, PluginError> {
        let executor = self.tool_executor()?;
        let registry = executor
            .monitor_registry()
            .ok_or_else(|| host_unavailable("monitor registry is not enabled in this runtime"))?;
        Ok(registry
            .list()
            .into_iter()
            .map(render_monitor_handle)
            .collect())
    }

    async fn monitor_read(
        &self,
        req: MonitorReadRequest,
    ) -> Result<MonitorReadResponse, PluginError> {
        let executor = self.tool_executor()?;
        let registry = executor
            .monitor_registry()
            .ok_or_else(|| host_unavailable("monitor registry is not enabled in this runtime"))?;
        let read = registry
            .read(MonitorReadParams {
                monitor_id: req.id,
                since_seq: req.since_seq,
                limit: req.limit,
                wait_ms: if req.follow && req.wait_ms == 0 {
                    30_000
                } else {
                    req.wait_ms
                },
            })
            .map_err(map_monitor_error)?;
        Ok(render_monitor_read(read))
    }

    async fn monitor_stop(&self, req: MonitorStopRequest) -> Result<MonitorHandle, PluginError> {
        let executor = self.tool_executor()?;
        let registry = executor
            .monitor_registry()
            .ok_or_else(|| host_unavailable("monitor registry is not enabled in this runtime"))?;
        let stop = registry.stop(req.id.as_str()).map_err(map_monitor_error)?;
        Ok(render_monitor_handle(stop.summary))
    }

    async fn storage_get(
        &self,
        req: HostStorageGetRequest,
    ) -> Result<HostStorageGetResponse, PluginError> {
        let locator = self.storage_locator(req.scope, req.visibility)?;
        let store = self.plugin_storage();
        let value = store
            .get(&locator, req.namespace.as_str(), req.key.as_str())
            .map_err(map_storage_error)?;
        Ok(HostStorageGetResponse { value })
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> Result<(), PluginError> {
        let locator = self.storage_locator(req.scope, req.visibility)?;
        let store = self.plugin_storage();
        store
            .set(
                &locator,
                req.namespace.as_str(),
                req.key.as_str(),
                req.value.as_str(),
            )
            .map_err(map_storage_error)
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> Result<(), PluginError> {
        let locator = self.storage_locator(req.scope, req.visibility)?;
        let store = self.plugin_storage();
        store
            .delete(&locator, req.namespace.as_str(), req.key.as_str())
            .map_err(map_storage_error)
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> Result<HostStorageListResponse, PluginError> {
        let locator = self.storage_locator(req.scope, req.visibility)?;
        let store = self.plugin_storage();
        let entries = store
            .list(&locator, req.namespace.as_deref(), req.prefix.as_deref())
            .map_err(map_storage_error)?
            .into_iter()
            .map(|entry| HostStorageEntry {
                namespace: entry.namespace,
                key: entry.key,
            })
            .collect();
        Ok(HostStorageListResponse { entries })
    }

    async fn secret_get(
        &self,
        req: HostSecretGetRequest,
    ) -> Result<HostSecretGetResponse, PluginError> {
        let plugin_id = self.callback_plugin_id()?;
        let store = self.plugin_secret_store();
        let value = store
            .get(&plugin_id, req.name.as_str())
            .map_err(map_storage_error)?;
        Ok(HostSecretGetResponse { value })
    }

    async fn secret_set(&self, req: HostSecretSetRequest) -> Result<(), PluginError> {
        let plugin_id = self.callback_plugin_id()?;
        let store = self.plugin_secret_store();
        store
            .set(&plugin_id, req.name.as_str(), req.value.as_str())
            .map_err(map_storage_error)
    }

    async fn secret_delete(&self, req: HostSecretDeleteRequest) -> Result<(), PluginError> {
        let plugin_id = self.callback_plugin_id()?;
        let store = self.plugin_secret_store();
        store
            .delete(&plugin_id, req.name.as_str())
            .map_err(map_storage_error)
    }

    async fn secret_list(&self) -> Result<HostSecretListResponse, PluginError> {
        let plugin_id = self.callback_plugin_id()?;
        let store = self.plugin_secret_store();
        let names = store.list(&plugin_id).map_err(map_storage_error)?;
        Ok(HostSecretListResponse { names })
    }

    async fn plugin_status_list(&self) -> Result<HostPluginStatusListResponse, PluginError> {
        let host = self.runtime.current_snapshot().plugin_manager();
        let entries = host
            .plugin_statuses()
            .into_iter()
            .map(host_status_to_sdk)
            .collect();
        Ok(HostPluginStatusListResponse { entries })
    }

    async fn plugin_status_get(
        &self,
        req: HostPluginStatusGetRequest,
    ) -> Result<HostPluginStatusGetResponse, PluginError> {
        let host = self.runtime.current_snapshot().plugin_manager();
        Ok(HostPluginStatusGetResponse {
            status: host.plugin_status(&req.plugin_id).map(host_status_to_sdk),
        })
    }

    async fn lsp_list_servers(&self) -> Result<HostLspListServersResponse, PluginError> {
        let executor = self.tool_executor()?;
        let registry = executor
            .lsp_registry()
            .ok_or_else(|| host_unavailable("lsp registry is not enabled in this runtime"))?;
        let specs = registry.server_specs().await;
        let servers = specs
            .into_iter()
            .map(|spec| HostLspServer {
                name: spec.name,
                command: spec.command,
                args: spec.args,
                file_extensions: spec.file_extensions,
            })
            .collect();
        Ok(HostLspListServersResponse { servers })
    }

    async fn lsp_list_diagnostics(
        &self,
        req: HostLspListDiagnosticsRequest,
    ) -> Result<HostLspListDiagnosticsResponse, PluginError> {
        let executor = self.tool_executor()?;
        let registry = executor
            .lsp_registry()
            .ok_or_else(|| host_unavailable("lsp registry is not enabled in this runtime"))?;
        let pairs = registry.collect_diagnostics().await;
        let mut entries = Vec::new();
        for (uri, diagnostics) in pairs {
            if let Some(filter) = req.uri.as_ref()
                && filter != &uri
            {
                continue;
            }
            for diagnostic in diagnostics {
                entries.push(HostLspDiagnostic {
                    uri: uri.clone(),
                    severity: lsp_severity_string(diagnostic.severity),
                    message: diagnostic.message,
                    start_line: diagnostic.range.start.line,
                    start_character: diagnostic.range.start.character,
                    end_line: diagnostic.range.end.line,
                    end_character: diagnostic.range.end.character,
                    source: diagnostic.source,
                    code: diagnostic.code.map(|code| match code {
                        agena_lsp::lsp_types::NumberOrString::Number(n) => n.to_string(),
                        agena_lsp::lsp_types::NumberOrString::String(s) => s,
                    }),
                });
            }
        }
        Ok(HostLspListDiagnosticsResponse { entries })
    }

    async fn plan_list(&self) -> Result<HostPlanListResponse, PluginError> {
        let executor = self.tool_executor()?;
        let registry = executor
            .plan_registry()
            .ok_or_else(|| host_unavailable("plan registry is not enabled in this runtime"))?;
        let entries: Vec<HostPlanEntry> = registry
            .read()
            .iter()
            .map(|(session_id, plan)| HostPlanEntry {
                session_id: *session_id,
                slug: plan.slug.clone(),
                file_path: plan.file_path.display().to_string(),
                started_at_ms: plan.started_at.timestamp_millis(),
            })
            .collect();
        Ok(HostPlanListResponse { entries })
    }

    async fn plan_get(&self, req: HostPlanGetRequest) -> Result<HostPlanGetResponse, PluginError> {
        let executor = self.tool_executor()?;
        let registry = executor
            .plan_registry()
            .ok_or_else(|| host_unavailable("plan registry is not enabled in this runtime"))?;
        let plan = registry.read().get(&req.session_id).cloned();
        let Some(plan) = plan else {
            return Ok(HostPlanGetResponse::default());
        };
        let body = std::fs::read_to_string(&plan.file_path).ok();
        Ok(HostPlanGetResponse {
            entry: Some(HostPlanEntry {
                session_id: req.session_id,
                slug: plan.slug.clone(),
                file_path: plan.file_path.display().to_string(),
                started_at_ms: plan.started_at.timestamp_millis(),
            }),
            body,
        })
    }

    async fn worktree_list(&self) -> Result<HostWorktreeListResponse, PluginError> {
        let executor = self.tool_executor()?;
        let registry = executor
            .worktree_registry()
            .ok_or_else(|| host_unavailable("worktree registry is not enabled in this runtime"))?;
        let entries: Vec<HostWorktreeEntry> = crate::tool::worktree_list_active(registry)
            .into_iter()
            .map(|w| HostWorktreeEntry {
                session_id: w.session_id,
                path: w.path.display().to_string(),
                branch: w.branch,
                created_here: w.created_here,
            })
            .collect();
        Ok(HostWorktreeListResponse { entries })
    }

    async fn scheduler_list(&self) -> Result<HostSchedulerListResponse, PluginError> {
        let executor = self.tool_executor()?;
        let scheduler = executor
            .scheduler()
            .cloned()
            .ok_or_else(|| host_unavailable("scheduler is not enabled in this runtime"))?;
        let jobs = scheduler.list().await;
        let entries = jobs.into_iter().map(scheduler_job_to_sdk).collect();
        Ok(HostSchedulerListResponse { jobs: entries })
    }

    async fn scheduler_create(
        &self,
        req: HostSchedulerCreateRequest,
    ) -> Result<HostSchedulerCreateResponse, PluginError> {
        let executor = self.tool_executor()?;
        let scheduler = executor
            .scheduler()
            .cloned()
            .ok_or_else(|| host_unavailable("scheduler is not enabled in this runtime"))?;
        let job = match req {
            HostSchedulerCreateRequest::Cron {
                expression,
                prompt,
                max_age_days,
                owner_session_id,
            } => {
                let mut job = agena_scheduler::ScheduledJob::new_cron(
                    expression,
                    prompt,
                    max_age_days.unwrap_or(7),
                )
                .map_err(|err| PluginError::invalid_params(err.to_string()))?;
                if let Some(session) = owner_session_id {
                    job = job.with_owner(session);
                }
                job
            }
            HostSchedulerCreateRequest::Once {
                at_ms,
                prompt,
                owner_session_id,
            } => {
                let at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at_ms)
                    .ok_or_else(|| PluginError::invalid_params("invalid at_ms"))?;
                let mut job = agena_scheduler::ScheduledJob::new_once(at, prompt);
                if let Some(session) = owner_session_id {
                    job = job.with_owner(session);
                }
                job
            }
        };
        let id = job.id;
        scheduler.add(job).await;
        Ok(HostSchedulerCreateResponse { id: id.to_string() })
    }

    async fn scheduler_delete(
        &self,
        req: HostSchedulerDeleteRequest,
    ) -> Result<HostSchedulerDeleteResponse, PluginError> {
        let executor = self.tool_executor()?;
        let scheduler = executor
            .scheduler()
            .cloned()
            .ok_or_else(|| host_unavailable("scheduler is not enabled in this runtime"))?;
        let id = uuid::Uuid::parse_str(&req.id)
            .map_err(|err| PluginError::invalid_params(format!("invalid scheduler id: {err}")))?;
        let removed = scheduler.remove(id).await;
        Ok(HostSchedulerDeleteResponse { removed })
    }

    async fn agent_register(&self, req: HostAgentRegisterRequest) -> Result<(), PluginError> {
        if req.agent.name.trim().is_empty() {
            return Err(PluginError::invalid_params("agent.name must not be empty"));
        }
        let scope = agent_scope_from_str(req.agent.scope.as_str());
        let permission = core_agent_permission_from_sdk(req.agent.permission);
        let mode = core_agent_mode_from_sdk(req.agent.mode.as_str());
        let temperature = req.agent.temperature.map(crate::agent::AgentTemperature);
        let effective_permission =
            permission.effective_with_defaults(&crate::agent::PermissionConfig::default());
        crate::agent::Agent::new(
            req.agent.name.clone(),
            crate::permission::PermissionPolicy::allow_all(),
        )
        .try_with_permission_config(&effective_permission)
        .map_err(|err| {
            PluginError::invalid_params(format!(
                "agent.permission is invalid for '{}': {err}",
                req.agent.name
            ))
        })?;
        let profile = crate::agents::AgentProfile {
            name: req.agent.name.clone(),
            frontmatter: crate::agents::AgentFrontmatter {
                description: req.agent.description,
                mode,
                hidden: req.agent.hidden,
                color: req.agent.color,
                temperature,
                max_output_tokens: req.agent.max_output_tokens,
                steps: req.agent.steps.map(|value| value as usize),
                allowed_tools: req.agent.allowed_tools,
                permission,
                model: req.agent.model,
                aliases: req.agent.aliases,
            },
            prompt: req.agent.prompt,
            source_path: None,
            scope,
        };
        self.agents().register_runtime(profile);
        Ok(())
    }

    async fn agent_remove(
        &self,
        req: HostAgentRemoveRequest,
    ) -> Result<HostAgentRemoveResponse, PluginError> {
        let removed = self.agents().remove_runtime(&req.name);
        Ok(HostAgentRemoveResponse { removed })
    }

    async fn agent_list(&self) -> Result<HostAgentListResponse, PluginError> {
        let agents = self
            .agents()
            .list()
            .into_iter()
            .map(agent_to_descriptor)
            .collect();
        Ok(HostAgentListResponse { agents })
    }

    async fn mcp_list_servers(&self) -> Result<HostMcpListServersResponse, PluginError> {
        let manager = self
            .runtime
            .current_snapshot()
            .mcp_manager()
            .ok_or_else(|| host_unavailable("mcp manager is not enabled in this runtime"))?;
        let servers = manager.server_names().await;
        Ok(HostMcpListServersResponse { servers })
    }

    async fn mcp_add_server(&self, req: HostMcpAddServerRequest) -> Result<(), PluginError> {
        let manager = self
            .runtime
            .current_snapshot()
            .mcp_manager()
            .ok_or_else(|| host_unavailable("mcp manager is not enabled in this runtime"))?;
        let spec = match req.spec {
            HostMcpServerSpec::Stdio {
                command,
                args,
                env,
                cwd,
            } => agena_mcp_client::ServerSpec::Stdio {
                command,
                args,
                env: env.into_iter().collect(),
                cwd: cwd.map(std::path::PathBuf::from),
            },
            HostMcpServerSpec::Http {
                url,
                bearer,
                headers,
            } => {
                let url = url::Url::parse(&url)
                    .map_err(|e| PluginError::invalid_params(format!("invalid mcp url: {e}")))?;
                let auth = bearer.map(agena_mcp_client::HttpAuth::Bearer);
                agena_mcp_client::ServerSpec::Http {
                    url,
                    mode: agena_mcp_client::HttpTransportMode::StreamableHttp,
                    headers: headers.into_iter().collect(),
                    auth,
                }
            }
        };
        manager
            .add_server(&req.name, spec)
            .await
            .map_err(|e| PluginError::new(format!("mcp.add_server: {e}")))
    }

    async fn mcp_remove_server(
        &self,
        req: HostMcpRemoveServerRequest,
    ) -> Result<HostMcpRemoveServerResponse, PluginError> {
        let manager = self
            .runtime
            .current_snapshot()
            .mcp_manager()
            .ok_or_else(|| host_unavailable("mcp manager is not enabled in this runtime"))?;
        match manager.remove_server(&req.name).await {
            Ok(()) => Ok(HostMcpRemoveServerResponse { removed: true }),
            Err(_) => Ok(HostMcpRemoveServerResponse { removed: false }),
        }
    }
}

fn agent_scope_from_str(scope: &str) -> crate::agents::AgentScope {
    match scope {
        "project" => crate::agents::AgentScope::Project,
        "user" => crate::agents::AgentScope::User,
        _ => crate::agents::AgentScope::Default,
    }
}

fn core_agent_mode_from_sdk(mode: &str) -> crate::agent::AgentMode {
    match mode.trim() {
        "subagent" => crate::agent::AgentMode::Subagent,
        "all" => crate::agent::AgentMode::All,
        _ => crate::agent::AgentMode::Primary,
    }
}

fn sdk_agent_mode_from_core(mode: crate::agent::AgentMode) -> String {
    match mode {
        crate::agent::AgentMode::Primary => "primary",
        crate::agent::AgentMode::Subagent => "subagent",
        crate::agent::AgentMode::All => "all",
    }
    .to_string()
}

fn agent_to_descriptor(profile: crate::agents::AgentProfile) -> HostAgentDescriptor {
    HostAgentDescriptor {
        name: profile.name,
        description: profile.frontmatter.description,
        mode: sdk_agent_mode_from_core(profile.frontmatter.mode),
        hidden: profile.frontmatter.hidden,
        color: profile.frontmatter.color,
        temperature: profile.frontmatter.temperature.map(|value| value.0),
        max_output_tokens: profile.frontmatter.max_output_tokens,
        steps: profile.frontmatter.steps.map(|value| value as u32),
        allowed_tools: profile.frontmatter.allowed_tools,
        permission: sdk_agent_permission_from_core(profile.frontmatter.permission),
        model: profile.frontmatter.model,
        aliases: profile.frontmatter.aliases,
        prompt: profile.prompt,
        scope: match profile.scope {
            crate::agents::AgentScope::Project => "project",
            crate::agents::AgentScope::User => "user",
            crate::agents::AgentScope::Default => "default",
        }
        .to_string(),
    }
}

fn core_agent_permission_from_sdk(
    permission: crate::plugin::sdk::host_api::AgentPermissionConfig,
) -> crate::agent::AgentPermissionConfig {
    crate::agent::AgentPermissionConfig {
        inherit: core_permission_inheritance_from_sdk(permission.inherit),
        path: permission.path.map(core_path_permission_from_sdk),
        network: permission.network.map(core_network_permission_from_sdk),
        tools: permission.tools.map(core_tool_permission_from_sdk),
    }
}

fn sdk_agent_permission_from_core(
    permission: crate::agent::AgentPermissionConfig,
) -> crate::plugin::sdk::host_api::AgentPermissionConfig {
    crate::plugin::sdk::host_api::AgentPermissionConfig {
        inherit: sdk_permission_inheritance_from_core(permission.inherit),
        path: permission.path.map(sdk_path_permission_from_core),
        network: permission.network.map(sdk_network_permission_from_core),
        tools: permission.tools.map(sdk_tool_permission_from_core),
    }
}

fn core_permission_inheritance_from_sdk(
    inherit: crate::plugin::sdk::host_api::AgentPermissionInheritance,
) -> crate::agent::PermissionInheritanceConfig {
    match inherit {
        crate::plugin::sdk::host_api::AgentPermissionInheritance::All(value) => {
            crate::agent::PermissionInheritanceConfig::All(value)
        }
        crate::plugin::sdk::host_api::AgentPermissionInheritance::Sections(sections) => {
            crate::agent::PermissionInheritanceConfig::Sections(
                crate::agent::PermissionInheritanceSections {
                    path: sections.path,
                    network: sections.network,
                    tools: sections.tools,
                    plugin_tools: sections.plugin_tools,
                },
            )
        }
    }
}

fn sdk_permission_inheritance_from_core(
    inherit: crate::agent::PermissionInheritanceConfig,
) -> crate::plugin::sdk::host_api::AgentPermissionInheritance {
    match inherit {
        crate::agent::PermissionInheritanceConfig::All(value) => {
            crate::plugin::sdk::host_api::AgentPermissionInheritance::All(value)
        }
        crate::agent::PermissionInheritanceConfig::Sections(sections) => {
            crate::plugin::sdk::host_api::AgentPermissionInheritance::Sections(
                crate::plugin::sdk::host_api::AgentPermissionInheritanceSections {
                    path: sections.path,
                    network: sections.network,
                    tools: sections.tools,
                    plugin_tools: sections.plugin_tools,
                },
            )
        }
    }
}

fn core_path_permission_from_sdk(
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

fn sdk_path_permission_from_core(
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

fn core_path_access_modes_from_sdk(
    modes: crate::plugin::sdk::host_api::AgentPathAccessModes,
) -> crate::agent::PathAccessModes {
    crate::agent::PathAccessModes {
        read: modes.read.map(core_permission_mode_from_sdk),
        write: modes.write.map(core_permission_mode_from_sdk),
    }
}

fn sdk_path_access_modes_from_core(
    modes: crate::agent::PathAccessModes,
) -> crate::plugin::sdk::host_api::AgentPathAccessModes {
    crate::plugin::sdk::host_api::AgentPathAccessModes {
        read: modes.read.map(sdk_permission_mode_from_core),
        write: modes.write.map(sdk_permission_mode_from_core),
    }
}

fn core_path_access_rule_from_sdk(
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

fn sdk_path_access_rule_from_core(
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

fn core_network_permission_from_sdk(
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

fn sdk_network_permission_from_core(
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

fn core_tool_permission_from_sdk(
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

fn sdk_tool_permission_from_core(
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

fn core_tool_permission_rules_from_sdk(
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

fn sdk_tool_permission_rules_from_core(
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

fn core_permission_mode_from_sdk(
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

fn sdk_permission_mode_from_core(
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

fn scheduler_job_to_sdk(job: agena_scheduler::ScheduledJob) -> HostSchedulerJob {
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

fn lsp_severity_string(severity: Option<agena_lsp::lsp_types::DiagnosticSeverity>) -> String {
    match severity {
        Some(agena_lsp::lsp_types::DiagnosticSeverity::ERROR) => "error".to_string(),
        Some(agena_lsp::lsp_types::DiagnosticSeverity::WARNING) => "warning".to_string(),
        Some(agena_lsp::lsp_types::DiagnosticSeverity::INFORMATION) => "information".to_string(),
        Some(agena_lsp::lsp_types::DiagnosticSeverity::HINT) => "hint".to_string(),
        Some(_) => "unknown".to_string(),
        None => "unknown".to_string(),
    }
}

fn host_status_to_sdk(status: agena_plugin_host::status::PluginStatus) -> HostPluginStatus {
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

mod active_invocations {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::config::LoadConfigRequest;
    use crate::plugin::sdk::host_api::with_host_callback_context;
    use crate::session::{GoalStatus, SessionCreateRequest, SessionGoal};
    use chrono::Utc;

    /// `noop_host_client` returns a working trait object that does not panic
    /// on `Display` / `Debug` access. Acts as a smoke test that the
    /// `NoopHostClient` re-export through `agena::plugin` stays intact.
    #[test]
    fn noop_host_client_is_constructible() {
        let client: Arc<dyn HostClient> = noop_host_client();
        // Poke the Arc to make sure the vtable resolves.
        assert!(Arc::strong_count(&client) >= 1);
    }

    #[test]
    fn host_goal_from_session_goal_preserves_paused_status() {
        let now = Utc::now();
        let goal = host_goal_from_session_goal(SessionGoal {
            id: 7,
            session_id: 11,
            objective: "ship the feature".to_string(),
            status: GoalStatus::Paused,
            token_budget: Some(42),
            tokens_used: 9,
            time_used_seconds: 3,
            created_at: now,
            updated_at: now,
            completed_at: None,
        });

        assert_eq!(goal.status, HostGoalStatus::Paused);
    }

    #[tokio::test]
    async fn create_goal_persists_for_session_and_rejects_duplicates() {
        let tempdir = tempfile::tempdir().expect("tempdir should create");
        let config_path = tempdir.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        )
        .expect("config should be written");

        let runtime = AgenaRuntime::builder()
            .with_load_request(LoadConfigRequest {
                config_path: Some(config_path),
                ..LoadConfigRequest::default()
            })
            .with_workspace_root(tempdir.path())
            .with_database_url("sqlite::memory:")
            .build()
            .await
            .expect("runtime should build");
        let manager = runtime
            .session_manager()
            .expect("session manager should be available");
        let session = manager
            .create_session(SessionCreateRequest {
                title: "goal host client".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should be created");
        let client = RuntimeHostClient {
            runtime: runtime.clone(),
        };

        let created = with_host_callback_context(
            HostCallbackContext {
                session_id: Some(session.id),
                ..HostCallbackContext::default()
            },
            async {
                <RuntimeHostClient as HostClient>::create_goal(
                    &client,
                    HostCreateGoalRequest {
                        objective: "ship the feature".to_string(),
                        token_budget: Some(42),
                    },
                )
                .await
            },
        )
        .await
        .expect("create_goal should succeed");
        assert_eq!(created.goal.objective, "ship the feature");
        assert_eq!(created.goal.token_budget, Some(42));
        assert_eq!(created.goal.status, HostGoalStatus::Active);

        let loaded = manager
            .get_goal(session.id)
            .await
            .expect("goal lookup should succeed")
            .expect("goal should persist");
        assert_eq!(loaded.objective, "ship the feature");
        assert_eq!(loaded.token_budget, Some(42));

        let err = with_host_callback_context(
            HostCallbackContext {
                session_id: Some(session.id),
                ..HostCallbackContext::default()
            },
            async {
                <RuntimeHostClient as HostClient>::create_goal(
                    &client,
                    HostCreateGoalRequest {
                        objective: "second goal".to_string(),
                        token_budget: None,
                    },
                )
                .await
            },
        )
        .await
        .expect_err("duplicate goal should be rejected");
        assert_eq!(err.code, crate::plugin::sdk::PluginErrorCode::InvalidParams);
        assert!(
            err.message.contains("already has an active goal"),
            "unexpected error: {err:?}"
        );

        runtime.shutdown();
    }

    #[tokio::test]
    async fn create_goal_sets_objective_updated_runtime_steering() {
        let tempdir = tempfile::tempdir().expect("tempdir should create");
        let config_path = tempdir.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        )
        .expect("config should be written");

        let runtime = AgenaRuntime::builder()
            .with_load_request(LoadConfigRequest {
                config_path: Some(config_path),
                ..LoadConfigRequest::default()
            })
            .with_workspace_root(tempdir.path())
            .with_database_url("sqlite::memory:")
            .build()
            .await
            .expect("runtime should build");
        let manager = runtime
            .session_manager()
            .expect("session manager should be available");
        let session = manager
            .create_session(SessionCreateRequest {
                title: "goal steering".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should be created");
        let client = RuntimeHostClient {
            runtime: runtime.clone(),
        };

        let created = with_host_callback_context(
            HostCallbackContext {
                session_id: Some(session.id),
                ..HostCallbackContext::default()
            },
            async {
                <RuntimeHostClient as HostClient>::create_goal(
                    &client,
                    HostCreateGoalRequest {
                        objective: "queue hidden steering".to_string(),
                        token_budget: Some(7),
                    },
                )
                .await
            },
        )
        .await
        .expect("create_goal should succeed");

        let session = manager
            .get_session(session.id)
            .await
            .expect("session load should succeed");
        let pending = session
            .runtime
            .goal
            .pending_steering()
            .expect("goal runtime should queue steering after create_goal");
        assert_eq!(pending.goal_id, created.goal.id);
        assert_eq!(format!("{:?}", pending.kind), "ObjectiveUpdated");

        runtime.shutdown();
    }

    #[tokio::test]
    async fn update_goal_allows_non_complete_status_transitions() {
        let tempdir = tempfile::tempdir().expect("tempdir should create");
        let config_path = tempdir.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        )
        .expect("config should be written");

        let runtime = AgenaRuntime::builder()
            .with_load_request(LoadConfigRequest {
                config_path: Some(config_path),
                ..LoadConfigRequest::default()
            })
            .with_workspace_root(tempdir.path())
            .with_database_url("sqlite::memory:")
            .build()
            .await
            .expect("runtime should build");
        let manager = runtime
            .session_manager()
            .expect("session manager should be available");
        let session = manager
            .create_session(SessionCreateRequest {
                title: "goal update pause".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should be created");
        manager
            .create_goal(crate::session::SessionGoalCreateRequest {
                session_id: session.id,
                objective: "ship the feature".to_string(),
                token_budget: Some(42),
            })
            .await
            .expect("goal should be created");
        let client = RuntimeHostClient {
            runtime: runtime.clone(),
        };

        let updated = with_host_callback_context(
            HostCallbackContext {
                session_id: Some(session.id),
                ..HostCallbackContext::default()
            },
            async {
                <RuntimeHostClient as HostClient>::update_goal(
                    &client,
                    HostUpdateGoalRequest {
                        objective: None,
                        status: Some(HostGoalStatus::Paused),
                        token_budget: None,
                    },
                )
                .await
            },
        )
        .await
        .expect("update_goal should succeed");

        assert_eq!(updated.goal.status, HostGoalStatus::Paused);
        assert_eq!(updated.goal.objective, "ship the feature");
        assert_eq!(updated.goal.token_budget, Some(42));

        let stored = manager
            .get_goal(session.id)
            .await
            .expect("goal lookup should succeed")
            .expect("goal should persist");
        assert_eq!(stored.status, GoalStatus::Paused);

        runtime.shutdown();
    }

    #[tokio::test]
    async fn update_goal_can_complete_existing_goal() {
        let tempdir = tempfile::tempdir().expect("tempdir should create");
        let config_path = tempdir.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        )
        .expect("config should be written");

        let runtime = AgenaRuntime::builder()
            .with_load_request(LoadConfigRequest {
                config_path: Some(config_path),
                ..LoadConfigRequest::default()
            })
            .with_workspace_root(tempdir.path())
            .with_database_url("sqlite::memory:")
            .build()
            .await
            .expect("runtime should build");
        let manager = runtime
            .session_manager()
            .expect("session manager should be available");
        let session = manager
            .create_session(SessionCreateRequest {
                title: "goal update complete".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should be created");
        manager
            .create_goal(crate::session::SessionGoalCreateRequest {
                session_id: session.id,
                objective: "ship the feature".to_string(),
                token_budget: Some(42),
            })
            .await
            .expect("goal should be created");
        let client = RuntimeHostClient {
            runtime: runtime.clone(),
        };

        let updated = with_host_callback_context(
            HostCallbackContext {
                session_id: Some(session.id),
                ..HostCallbackContext::default()
            },
            async {
                <RuntimeHostClient as HostClient>::update_goal(
                    &client,
                    HostUpdateGoalRequest {
                        objective: None,
                        status: Some(HostGoalStatus::Completed),
                        token_budget: None,
                    },
                )
                .await
            },
        )
        .await
        .expect("update_goal should succeed");

        assert_eq!(updated.goal.status, HostGoalStatus::Completed);
        assert!(updated.goal.completed_at_ms.is_some());

        let stored = manager
            .get_goal(session.id)
            .await
            .expect("goal lookup should succeed")
            .expect("goal should persist");
        assert_eq!(stored.status, GoalStatus::Completed);
        assert!(stored.completed_at.is_some());

        runtime.shutdown();
    }
}
