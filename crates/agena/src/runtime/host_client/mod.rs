//! Concrete `HostClient` impl backed by the live `AgenaRuntime`. Plugins
//! that run as subprocess (stdio) or remote (HTTP) call back into this via
//! JSON-RPC; the `HostHandle` in `agena-plugin-host` routes those calls
//! through this client.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::message::{
    AskUserToolInput, EnterPlanModeToolInput, EnterWorktreeToolInput, ExitPlanModeToolInput,
    ExitWorktreeToolInput, MonitorStatus, MonitorStream, StructuredObject, TaskSubagentType,
    TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput, ToolInvocation, UserInputOption,
    UserInputQuestion,
};
use crate::plugin::sdk::host_api::{
    AskUserRequest, AskUserResponse, EventSubscription, HostAgentDefaultModelConfig,
    HostAgentDescriptor, HostAgentGetRequest, HostAgentGetResponse, HostAgentListResponse,
    HostAgentRegisterRequest, HostAgentRemoveRequest, HostAgentRemoveResponse,
    HostAgentRestoreRequest, HostAgentRestoreResponse, HostAgentSwitchRequest,
    HostAgentSwitchResponse, HostCallbackContext, HostClearGoalRequest, HostClearGoalResponse,
    HostClient, HostConfigReloadResponse, HostCreateGoalRequest, HostCreateGoalResponse,
    HostEnterPlanModeRequest, HostEnterWorktreeRequest, HostExitPlanModeRequest,
    HostExitWorktreeRequest, HostGetGoalRequest, HostGetGoalResponse, HostGetSessionRequest,
    HostGetSessionResponse, HostGoal, HostGoalStatus, HostLspDiagnostic,
    HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse, HostLspListServersResponse,
    HostLspServer, HostMcpAddServerRequest, HostMcpHttpMode, HostMcpListServersResponse,
    HostMcpRemoveServerRequest, HostMcpRemoveServerResponse, HostMcpServerSpec,
    HostNetworkPermissionCheckRequest, HostPathPermissionCheckRequest, HostPermissionCheckResponse,
    HostPlanEntry, HostPlanGetRequest, HostPlanGetResponse, HostPlanListResponse, HostPluginStatus,
    HostPluginStatusGetRequest, HostPluginStatusGetResponse, HostPluginStatusListResponse,
    HostRenameSessionRequest, HostRenameSessionResponse, HostSchedulerCreateRequest,
    HostSchedulerCreateResponse, HostSchedulerDeleteRequest, HostSchedulerDeleteResponse,
    HostSchedulerJob, HostSchedulerListResponse, HostSecretDeleteRequest, HostSecretGetRequest,
    HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest, HostSession,
    HostStorageDeleteRequest, HostStorageEntry, HostStorageGetRequest, HostStorageGetResponse,
    HostStorageListRequest, HostStorageListResponse, HostStorageSetRequest, HostTodoItem,
    HostTodoPriority, HostTodoStatus, HostTodoWriteRequest, HostUpdateGoalRequest,
    HostUpdateGoalResponse, HostWorktreeEntry, HostWorktreeListResponse, LogLevel, MonitorEvent,
    MonitorHandle, MonitorReadRequest, MonitorReadResponse, MonitorStartRequest,
    MonitorStopRequest, NoopHostClient, SpawnSubtaskRequest, SpawnSubtaskResponse, ToolDescriptor,
    current_host_callback_context,
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

mod mappers;

use mappers::*;

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

    fn callback_or_requested_session_id(
        &self,
        requested: Option<i64>,
        action: &str,
    ) -> Result<i64, PluginError> {
        match requested {
            Some(session_id) => Ok(session_id),
            None => self.callback_context()?.session_id.ok_or_else(|| {
                host_unavailable(format!(
                    "host callback context is missing session_id for {action}"
                ))
            }),
        }
    }
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
        _: PluginEventFilter,
    ) -> Result<EventSubscription, PluginError> {
        // Translate the SDK filter to agena's filter and confirm; the actual
        // event push back to the plugin already happens via the snapshot's
        // `event_bridge`. Returning a deterministic id so plugins can ack.
        let id = format!("sub-{}", uuid::Uuid::new_v4().simple());
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
        crate::config::get_json_path(&value, path.as_deref())
            .map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    async fn reload_config(&self) -> Result<HostConfigReloadResponse, PluginError> {
        let report = self
            .runtime
            .reload()
            .await
            .map_err(|e| PluginError::new(e.to_string()))?;
        Ok(HostConfigReloadResponse {
            previous_generation: report.previous_generation,
            generation: report.generation,
            loaded_at: report.loaded_at.to_rfc3339(),
        })
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
            .detailed_tools()
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

    async fn get_session(
        &self,
        req: HostGetSessionRequest,
    ) -> Result<HostGetSessionResponse, PluginError> {
        let session_id = self.callback_or_requested_session_id(req.session_id, "get_session")?;
        let session = self
            .session_manager()?
            .get_session(session_id)
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(HostGetSessionResponse {
            session: host_session_from_session(&session),
        })
    }

    async fn rename_session(
        &self,
        req: HostRenameSessionRequest,
    ) -> Result<HostRenameSessionResponse, PluginError> {
        let session_id = self.callback_or_requested_session_id(req.session_id, "rename_session")?;
        let title = req.title.trim();
        if title.is_empty() {
            return Err(PluginError::invalid_params(
                "session title must not be empty",
            ));
        }
        let session = self
            .session_manager()?
            .rename_session(session_id, title.to_string())
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(HostRenameSessionResponse {
            session: host_session_from_session(&session),
        })
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
                default: crate::agents::AgentDefaultModelConfig {
                    provider: req.agent.default.provider,
                    adapter: req.agent.default.adapter,
                    model: req.agent.default.model,
                },
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

    async fn agent_get(
        &self,
        req: HostAgentGetRequest,
    ) -> Result<HostAgentGetResponse, PluginError> {
        if req.name.trim().is_empty() {
            return Err(PluginError::invalid_params("agent.name must not be empty"));
        }
        Ok(HostAgentGetResponse {
            agent: self.agents().get(req.name.trim()).map(agent_to_descriptor),
        })
    }

    async fn agent_switch(
        &self,
        req: HostAgentSwitchRequest,
    ) -> Result<HostAgentSwitchResponse, PluginError> {
        let session_id = match req.session_id {
            Some(id) => id,
            None => self.callback_context()?.session_id.ok_or_else(|| {
                host_unavailable("host callback context is missing session_id for agent.switch")
            })?,
        };
        let outcome = self
            .session_manager()?
            .switch_session_agent(session_id, req.agent, req.push_previous)
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(HostAgentSwitchResponse {
            session_id: outcome.session_id,
            previous_agent: outcome.previous_agent,
            current_agent: outcome.current_agent,
            stack_depth: outcome.stack_depth,
        })
    }

    async fn agent_restore(
        &self,
        req: HostAgentRestoreRequest,
    ) -> Result<HostAgentRestoreResponse, PluginError> {
        let session_id = match req.session_id {
            Some(id) => id,
            None => self.callback_context()?.session_id.ok_or_else(|| {
                host_unavailable("host callback context is missing session_id for agent.restore")
            })?,
        };
        let outcome = self
            .session_manager()?
            .restore_session_agent(session_id)
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(HostAgentRestoreResponse {
            session_id: outcome.session_id,
            restored: outcome.restored,
            previous_agent: outcome.previous_agent,
            current_agent: outcome.current_agent,
            stack_depth: outcome.stack_depth,
        })
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
                mode,
                bearer,
                headers,
            } => {
                let url = url::Url::parse(&url)
                    .map_err(|e| PluginError::invalid_params(format!("invalid mcp url: {e}")))?;
                let auth = bearer.map(agena_mcp_client::HttpAuth::Bearer);
                let mode = match mode {
                    HostMcpHttpMode::Sse => agena_mcp_client::HttpTransportMode::Sse,
                    HostMcpHttpMode::StreamableHttp => {
                        agena_mcp_client::HttpTransportMode::StreamableHttp
                    }
                };
                agena_mcp_client::ServerSpec::Http {
                    url,
                    mode,
                    headers: headers.into_iter().collect(),
                    auth,
                }
            }
            HostMcpServerSpec::Ws {
                url,
                bearer,
                headers,
            } => {
                let url = url::Url::parse(&url)
                    .map_err(|e| PluginError::invalid_params(format!("invalid mcp url: {e}")))?;
                let auth = bearer.map(agena_mcp_client::HttpAuth::Bearer);
                agena_mcp_client::ServerSpec::Ws {
                    url,
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
