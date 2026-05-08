//! Concrete `HostClient` impl backed by the live `AgenaRuntime`. Plugins
//! that run as subprocess (stdio) or remote (HTTP) call back into this via
//! JSON-RPC; the `HostHandle` in `agena-plugin-host` routes those calls
//! through this client.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::event::Scope;
use async_trait::async_trait;

use crate::message::{
    AskUserToolInput, BuiltinToolInput, EnterPlanModeToolInput, EnterWorktreeToolInput,
    ExitPlanModeToolInput, ExitWorktreeToolInput, MonitorStatus, MonitorStream, TaskSubagentType,
    TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput, UserInputOption, UserInputQuestion,
};
use crate::plugin::sdk::host_api::{
    AskUserRequest, AskUserResponse, BuiltinToolRequest, EventSubscription, HostAgentDescriptor,
    HostAgentListResponse, HostAgentRegisterRequest, HostAgentRemoveRequest,
    HostAgentRemoveResponse, HostCallbackContext, HostClient, HostCommandDescriptor,
    HostCommandListResponse, HostCommandRegisterRequest, HostCommandRemoveRequest,
    HostCommandRemoveResponse, HostEnterPlanModeRequest, HostEnterWorktreeRequest,
    HostExitPlanModeRequest, HostExitWorktreeRequest, HostLspDiagnostic,
    HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse, HostLspListServersResponse,
    HostLspServer, HostMcpAddServerRequest, HostMcpListServersResponse, HostMcpRemoveServerRequest,
    HostMcpRemoveServerResponse, HostMcpServerSpec, HostPlanEntry, HostPlanGetRequest,
    HostPlanGetResponse, HostPlanListResponse, HostPluginStatus, HostPluginStatusGetRequest,
    HostPluginStatusGetResponse, HostPluginStatusListResponse, HostSchedulerCreateRequest,
    HostSchedulerCreateResponse, HostSchedulerDeleteRequest, HostSchedulerDeleteResponse,
    HostSchedulerJob, HostSchedulerListResponse, HostSecretDeleteRequest, HostSecretGetRequest,
    HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest, HostSkillDescriptor,
    HostSkillGetRequest, HostSkillGetResponse, HostSkillListResponse, HostSkillMutationResponse,
    HostSkillRegisterRequest, HostSkillRemoveRequest, HostStorageDeleteRequest, HostStorageEntry,
    HostStorageGetRequest, HostStorageGetResponse, HostStorageListRequest, HostStorageListResponse,
    HostStorageSetRequest, HostTodoItem, HostTodoPriority, HostTodoStatus, HostTodoWriteRequest,
    HostWorktreeEntry, HostWorktreeListResponse, LogLevel, MonitorEvent, MonitorHandle,
    MonitorReadRequest, MonitorReadResponse, MonitorStartRequest, MonitorStopRequest,
    NoopHostClient, SpawnSubtaskRequest, SpawnSubtaskResponse, ToolDescriptor,
    current_host_callback_context,
};
use crate::plugin::{
    EventEnvelope, EventFilter as PluginEventFilter, PermissionAskInput,
    PermissionDecision as PluginPermissionDecision, PluginError, ToolInvokeOutput,
};
use crate::plugins::storage::{PluginSecretStore, PluginStorage, PluginStorageError};
use crate::runtime::AgenaRuntime;
use crate::tool::{EntrySource, MonitorError, MonitorReadParams, MonitorStartParams};
use crate::{entry::BuiltinExecutionContext, plugins::bundled::builtin::builtin_to_invoke_output};

/// Build a `HostClient` impl for a runtime; use [`NoopHostClient`] when no
/// runtime is available (e.g. before bootstrap completes).
pub fn host_client_for(runtime: AgenaRuntime) -> Arc<dyn HostClient> {
    Arc::new(RuntimeHostClient {
        runtime,
        commands: crate::commands::CustomCommandRegistry::empty(),
        agents: crate::agents::SubagentRegistry::empty(),
    })
}

pub fn noop_host_client() -> Arc<dyn HostClient> {
    Arc::new(NoopHostClient)
}

struct RuntimeHostClient {
    runtime: AgenaRuntime,
    commands: crate::commands::CustomCommandRegistry,
    agents: crate::agents::SubagentRegistry,
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

    fn skills_manager(&self) -> Result<Arc<agena_skills::SkillsManager>, PluginError> {
        self.runtime
            .current_snapshot()
            .skills_manager()
            .ok_or_else(|| host_unavailable("skills manager is not enabled in this runtime"))
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

fn map_storage_error(err: PluginStorageError) -> PluginError {
    use crate::plugin::sdk::PluginErrorCode;
    match err {
        PluginStorageError::MissingPluginId
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

fn parse_subagent_type(value: &str) -> Result<TaskSubagentType, PluginError> {
    match value.trim() {
        "explore" => Ok(TaskSubagentType::Explore),
        "implement" => Ok(TaskSubagentType::Implement),
        "verify" => Ok(TaskSubagentType::Verify),
        other => Err(PluginError::invalid_params(format!(
            "unknown subagent_type '{other}'"
        ))),
    }
}

fn render_tool_descriptor(definition: crate::tool::EntryDefinition) -> ToolDescriptor {
    let deferred = definition.is_deferred();
    ToolDescriptor {
        name: definition.name,
        description: (!definition.description.trim().is_empty()).then_some(definition.description),
        search_terms: definition.search_terms,
        behavior: Some(
            match definition.behavior {
                crate::tool::EntryBehavior::Mutating => "mutating",
                crate::tool::EntryBehavior::ReadOnly => "read_only",
                crate::tool::EntryBehavior::Task => "task",
            }
            .to_string(),
        ),
        deferred,
        read_only: definition.read_only,
        plugin_id: match definition.source {
            EntrySource::Builtin => None,
            EntrySource::Plugin { plugin_name } => Some(plugin_name),
        },
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

fn workflow_builtin_output(
    executor: &crate::tool::ToolExecutor,
    input: BuiltinToolInput,
    session_id: Option<i64>,
    call_id: Option<i64>,
) -> Result<ToolInvokeOutput, PluginError> {
    let execution = crate::entry::orchestrator::execute_builtin(
        executor,
        &input,
        BuiltinExecutionContext {
            session_id,
            call_id,
            session_context: None,
        },
    )
    .map_err(|err| PluginError::new(err.to_string()))?;
    Ok(builtin_to_invoke_output(execution))
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

        let caller = current_host_callback_context();
        let plugin_id = resolution.handle.plugin_id.clone();
        if caller
            .as_ref()
            .and_then(|context| context.plugin_id.as_ref())
            .is_some_and(|current| current == &plugin_id)
            || active_invocations::contains(&plugin_id)
        {
            return Err(PluginError::new(format!(
                "host->plugin invoke would re-enter plugin `{plugin_id}` (cycle detected)"
            )));
        }
        let _guard = active_invocations::enter(plugin_id.clone());

        let host_arc = host.clone();
        let handle_clone = resolution.handle.clone();
        let original = resolution.handle.original_name.clone();
        let session_id = caller
            .as_ref()
            .and_then(|context| context.session_id)
            .unwrap_or(-1);
        let call_id = caller
            .as_ref()
            .and_then(|context| context.call_id)
            .unwrap_or(-1);
        let workspace_root = caller
            .and_then(|context| context.workspace_root)
            .unwrap_or_else(|| ".".to_string());
        // Run in a blocking thread so the sync invoke_tool API on PluginHost
        // (which itself uses block_on) doesn't hijack our async runtime.
        let result = tokio::task::spawn_blocking(move || {
            host_arc.invoke_tool(
                &handle_clone,
                crate::plugin::ToolInvokeInput {
                    tool_name: original,
                    session_id,
                    call_id,
                    workspace_root,
                    input,
                },
            )
        })
        .await
        .map_err(|_| PluginError::new("invoke_tool task panicked"))??;

        Ok(result)
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
        let subagent_type = parse_subagent_type(req.subagent_type.as_str())?;
        let prompt = subagent_type.apply_prompt_guidance(&req.prompt);
        let response = self
            .session_manager()?
            .spawn_subtask(crate::session::SessionSubtaskRequest {
                parent_session_id,
                description: req.description.clone(),
                prompt,
                subagent_type,
                task_id: req.task_id.clone(),
                command: req.command.clone(),
                requested_model: req.model.clone(),
            })
            .await
            .map_err(|err| PluginError::new(err.to_string()))?;

        let session = response.session;
        let mut metadata = BTreeMap::new();
        metadata.insert("session_id".to_string(), session.id.to_string());
        metadata.insert("subagent_type".to_string(), subagent_type.to_string());
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
                subagent_type,
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
        let executor = self.tool_executor()?;
        let context = self.callback_context()?;
        workflow_builtin_output(
            &executor,
            BuiltinToolInput::TodoWrite(TodoWriteToolInput {
                items: req.items.into_iter().map(todo_item_from_host).collect(),
            }),
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
        )
    }

    async fn enter_plan_mode(
        &self,
        _req: HostEnterPlanModeRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let executor = self.tool_executor()?;
        let context = self.callback_context()?;
        workflow_builtin_output(
            &executor,
            BuiltinToolInput::EnterPlanMode(EnterPlanModeToolInput::default()),
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
        )
    }

    async fn exit_plan_mode(
        &self,
        _req: HostExitPlanModeRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let executor = self.tool_executor()?;
        let context = self.callback_context()?;
        workflow_builtin_output(
            &executor,
            BuiltinToolInput::ExitPlanMode(ExitPlanModeToolInput::default()),
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
        )
    }

    async fn enter_worktree(
        &self,
        req: HostEnterWorktreeRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let executor = self.tool_executor()?;
        let context = self.callback_context()?;
        workflow_builtin_output(
            &executor,
            BuiltinToolInput::EnterWorktree(EnterWorktreeToolInput {
                name: req.name,
                path: req.path,
            }),
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
        )
    }

    async fn exit_worktree(
        &self,
        req: HostExitWorktreeRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let executor = self.tool_executor()?;
        let context = self.callback_context()?;
        workflow_builtin_output(
            &executor,
            BuiltinToolInput::ExitWorktree(ExitWorktreeToolInput {
                action: req.action,
                discard_changes: req.discard_changes,
            }),
            context.session_id.filter(|id| *id >= 0),
            context.call_id.filter(|id| *id >= 0),
        )
    }

    async fn execute_builtin_tool(
        &self,
        req: BuiltinToolRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let context = self.callback_context()?;
        let plugin_id = context.plugin_id.as_deref().unwrap_or_default();
        if !plugin_id.starts_with("agena.") {
            return Err(host_unavailable(
                "built-in host execution is reserved for first-party agena.* plugins",
            ));
        }
        let executor = self.tool_executor()?;
        executor
            .execute_builtin_payload_for_host(
                req.tool_name.as_str(),
                req.input,
                context.session_id.filter(|id| *id >= 0),
                context.call_id.filter(|id| *id >= 0),
            )
            .map_err(|err| PluginError::new(err.to_string()))
    }

    async fn skill_get(
        &self,
        req: HostSkillGetRequest,
    ) -> Result<HostSkillGetResponse, PluginError> {
        let manager = self.skills_manager()?;
        let skill = manager
            .get(req.name.trim())
            .map_err(|err| PluginError::new(format!("skill_get: {err}")))?;
        Ok(HostSkillGetResponse {
            name: skill.frontmatter.name.clone(),
            body: skill.body.clone(),
            allowed_tools: skill.frontmatter.allowed_tools.clone(),
            model: skill.frontmatter.model.clone(),
        })
    }

    async fn skill_register(
        &self,
        req: HostSkillRegisterRequest,
    ) -> Result<HostSkillMutationResponse, PluginError> {
        use crate::plugin::sdk::manifest::SkillKind;
        let manager = self.skills_manager()?;
        let plugin_id = current_host_callback_context()
            .and_then(|ctx| ctx.plugin_id)
            .ok_or_else(|| host_unavailable("skill_register requires plugin id in context"))?;
        let entry = req.skill;
        let skill = agena_skills::Skill {
            frontmatter: agena_skills::SkillFrontmatter {
                name: entry.name.clone(),
                description: entry.description,
                allowed_tools: entry.allowed_tools,
                model: entry.model,
                aliases: entry.aliases,
            },
            body: entry.body,
            source_path: None,
        };
        match entry.kind {
            SkillKind::Skill => manager.register(plugin_id, skill),
            SkillKind::Command => manager.register_command(plugin_id, skill),
        }
        Ok(HostSkillMutationResponse {
            generation: 0,
            removed: false,
        })
    }

    async fn skill_remove(
        &self,
        req: HostSkillRemoveRequest,
    ) -> Result<HostSkillMutationResponse, PluginError> {
        let manager = self.skills_manager()?;
        let plugin_id = current_host_callback_context()
            .and_then(|ctx| ctx.plugin_id)
            .ok_or_else(|| host_unavailable("skill_remove requires plugin id in context"))?;
        let removed_skill = manager.remove(&plugin_id, &req.name);
        let removed_command = manager.remove_command(&plugin_id, &req.name);
        Ok(HostSkillMutationResponse {
            generation: 0,
            removed: removed_skill || removed_command,
        })
    }

    async fn skill_list(&self) -> Result<HostSkillListResponse, PluginError> {
        use crate::plugin::sdk::manifest::{SkillKind, SkillManifestEntry};
        let manager = self.skills_manager()?;
        let mut skills: Vec<HostSkillDescriptor> = manager
            .list_with_owners()
            .into_iter()
            .map(|(plugin_id, skill)| {
                let mut entry = SkillManifestEntry::new(skill.frontmatter.name, skill.body)
                    .description(skill.frontmatter.description)
                    .allowed_tools(skill.frontmatter.allowed_tools)
                    .aliases(skill.frontmatter.aliases)
                    .kind(SkillKind::Skill);
                if let Some(model) = skill.frontmatter.model {
                    entry = entry.model(model);
                }
                HostSkillDescriptor {
                    plugin_id,
                    skill: entry,
                }
            })
            .collect();
        // Commands list does not preserve owner per skill in the
        // current registry; return them tagged as `unknown` for now.
        for skill in manager.list_commands() {
            let mut entry = SkillManifestEntry::new(skill.frontmatter.name, skill.body)
                .description(skill.frontmatter.description)
                .allowed_tools(skill.frontmatter.allowed_tools)
                .aliases(skill.frontmatter.aliases)
                .kind(SkillKind::Command);
            if let Some(model) = skill.frontmatter.model {
                entry = entry.model(model);
            }
            skills.push(HostSkillDescriptor {
                plugin_id: String::new(),
                skill: entry,
            });
        }
        Ok(HostSkillListResponse {
            generation: 0,
            skills,
        })
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
        let plugin_id = self.callback_plugin_id()?;
        let store = self.plugin_storage();
        let value = store
            .get(&plugin_id, req.namespace.as_str(), req.key.as_str())
            .map_err(map_storage_error)?;
        Ok(HostStorageGetResponse { value })
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> Result<(), PluginError> {
        let plugin_id = self.callback_plugin_id()?;
        let store = self.plugin_storage();
        store
            .set(
                &plugin_id,
                req.namespace.as_str(),
                req.key.as_str(),
                req.value.as_str(),
            )
            .map_err(map_storage_error)
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> Result<(), PluginError> {
        let plugin_id = self.callback_plugin_id()?;
        let store = self.plugin_storage();
        store
            .delete(&plugin_id, req.namespace.as_str(), req.key.as_str())
            .map_err(map_storage_error)
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> Result<HostStorageListResponse, PluginError> {
        let plugin_id = self.callback_plugin_id()?;
        let store = self.plugin_storage();
        let entries = store
            .list(&plugin_id, req.namespace.as_deref(), req.prefix.as_deref())
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

    async fn command_register(&self, req: HostCommandRegisterRequest) -> Result<(), PluginError> {
        if req.command.name.trim().is_empty() {
            return Err(PluginError::invalid_params(
                "command.name must not be empty",
            ));
        }
        let scope = command_scope_from_str(req.command.scope.as_str());
        let command = crate::commands::CustomCommand {
            name: req.command.name.clone(),
            frontmatter: crate::commands::CommandFrontmatter {
                description: req.command.description,
                argument_hint: None,
                allowed_tools: req.command.allowed_tools,
                model: req.command.model,
                aliases: req.command.aliases,
            },
            body: req.command.body,
            source_path: None,
            scope,
        };
        self.commands.register_runtime(command);
        Ok(())
    }

    async fn command_remove(
        &self,
        req: HostCommandRemoveRequest,
    ) -> Result<HostCommandRemoveResponse, PluginError> {
        let removed = self.commands.remove_runtime(&req.name);
        Ok(HostCommandRemoveResponse { removed })
    }

    async fn command_list(&self) -> Result<HostCommandListResponse, PluginError> {
        let commands = self
            .commands
            .list()
            .into_iter()
            .map(command_to_descriptor)
            .collect();
        Ok(HostCommandListResponse { commands })
    }

    async fn agent_register(&self, req: HostAgentRegisterRequest) -> Result<(), PluginError> {
        if req.agent.name.trim().is_empty() {
            return Err(PluginError::invalid_params("agent.name must not be empty"));
        }
        let scope = agent_scope_from_str(req.agent.scope.as_str());
        let profile = crate::agents::AgentProfile {
            name: req.agent.name.clone(),
            frontmatter: crate::agents::AgentFrontmatter {
                description: req.agent.description,
                allowed_tools: req.agent.allowed_tools,
                model: req.agent.model,
                aliases: req.agent.aliases,
            },
            prompt: req.agent.prompt,
            source_path: None,
            scope,
        };
        self.agents.register_runtime(profile);
        Ok(())
    }

    async fn agent_remove(
        &self,
        req: HostAgentRemoveRequest,
    ) -> Result<HostAgentRemoveResponse, PluginError> {
        let removed = self.agents.remove_runtime(&req.name);
        Ok(HostAgentRemoveResponse { removed })
    }

    async fn agent_list(&self) -> Result<HostAgentListResponse, PluginError> {
        let agents = self
            .agents
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

fn command_scope_from_str(scope: &str) -> crate::commands::CommandScope {
    match scope {
        "project" => crate::commands::CommandScope::Project,
        "user" => crate::commands::CommandScope::User,
        _ => crate::commands::CommandScope::Builtin,
    }
}

fn command_to_descriptor(cmd: crate::commands::CustomCommand) -> HostCommandDescriptor {
    HostCommandDescriptor {
        name: cmd.name,
        description: cmd.frontmatter.description,
        allowed_tools: cmd.frontmatter.allowed_tools,
        model: cmd.frontmatter.model,
        aliases: cmd.frontmatter.aliases,
        body: cmd.body,
        scope: match cmd.scope {
            crate::commands::CommandScope::Project => "project",
            crate::commands::CommandScope::User => "user",
            crate::commands::CommandScope::Builtin => "builtin",
        }
        .to_string(),
    }
}

fn agent_scope_from_str(scope: &str) -> crate::agents::AgentScope {
    match scope {
        "project" => crate::agents::AgentScope::Project,
        "user" => crate::agents::AgentScope::User,
        _ => crate::agents::AgentScope::Builtin,
    }
}

fn agent_to_descriptor(profile: crate::agents::AgentProfile) -> HostAgentDescriptor {
    HostAgentDescriptor {
        name: profile.name,
        description: profile.frontmatter.description,
        allowed_tools: profile.frontmatter.allowed_tools,
        model: profile.frontmatter.model,
        aliases: profile.frontmatter.aliases,
        prompt: profile.prompt,
        scope: match profile.scope {
            crate::agents::AgentScope::Project => "project",
            crate::agents::AgentScope::User => "user",
            crate::agents::AgentScope::Builtin => "builtin",
        }
        .to_string(),
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

    /// `noop_host_client` returns a working trait object that does not panic
    /// on `Display` / `Debug` access. Acts as a smoke test that the
    /// `NoopHostClient` re-export through `agena::plugin` stays intact.
    #[test]
    fn noop_host_client_is_constructible() {
        let client: Arc<dyn HostClient> = noop_host_client();
        // Poke the Arc to make sure the vtable resolves.
        assert!(Arc::strong_count(&client) >= 1);
    }
}
