//! Concrete `HostClient` impl backed by the live `AgenaRuntime`. Plugins
//! that run as subprocess (stdio) or remote (HTTP) call back into this via
//! JSON-RPC; the `HostHandle` in `agena-plugin-host` routes those calls
//! through this client.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::event::Scope;
use async_trait::async_trait;

use crate::message::{
    AskUserToolInput, MonitorStatus, MonitorStream, TaskSubagentType, UserInputOption,
    UserInputQuestion,
};
use crate::plugin::sdk::host_api::{
    AskUserRequest, AskUserResponse, BuiltinToolRequest, EventSubscription, HostCallbackContext,
    HostClient, HostSkillGetRequest, HostSkillGetResponse, LogLevel, MonitorEvent, MonitorHandle,
    MonitorReadRequest, MonitorReadResponse, MonitorStartRequest, MonitorStopRequest,
    NoopHostClient, SpawnSubtaskRequest, SpawnSubtaskResponse, ToolDescriptor,
    current_host_callback_context,
};
use crate::plugin::{
    EventEnvelope, EventFilter as PluginEventFilter, PermissionAskInput,
    PermissionDecision as PluginPermissionDecision, PluginError, ToolInvokeOutput,
};
use crate::runtime::AgenaRuntime;
use crate::tool::{EntrySource, MonitorError, MonitorReadParams, MonitorStartParams};

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
        let executor = self.tool_executor()?;
        let subagent_type = parse_subagent_type(req.subagent_type.as_str())?;
        let prompt = subagent_type.apply_prompt_guidance(&req.prompt);
        let session = executor
            .subtask_manager()
            .create_or_resume(crate::tool::SubtaskSessionRequest {
                requested_task_id: req.task_id.clone(),
                description: req.description.clone(),
                prompt,
                subagent_type,
                command: req.command.clone(),
            })
            .map_err(|err| PluginError::new(err.to_string()))?;

        let mut metadata = BTreeMap::new();
        metadata.insert("session_id".to_string(), session.session_id.clone());
        metadata.insert(
            "subagent_type".to_string(),
            session.subagent_type.to_string(),
        );
        if let Some(model) = req.model {
            metadata.insert("requested_model".to_string(), model);
        }
        if let Some(model_provider_id) = session.model_provider_id.clone() {
            metadata.insert("model_provider_id".to_string(), model_provider_id);
        }
        if let Some(model_id) = session.model_id.clone() {
            metadata.insert("model_id".to_string(), model_id);
        }
        if let Some(command) = session.command.clone() {
            metadata.insert("command".to_string(), command);
        }
        metadata.insert("description".to_string(), session.description.clone());

        Ok(SpawnSubtaskResponse {
            final_text: format!(
                "Created/resumed subtask session {} for profile '{}' in workspace {}.",
                session.session_id,
                session.subagent_type,
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

    async fn execute_builtin_tool(
        &self,
        req: BuiltinToolRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let context = self.callback_context()?;
        if context.plugin_id.as_deref() != Some(crate::tool::builtins_plugin_id()) {
            return Err(host_unavailable(
                "built-in host execution is only available to agena.builtin",
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
        let executor = self.tool_executor()?;
        let manager = executor
            .skills_manager()
            .ok_or_else(|| host_unavailable("skills manager is not enabled in this runtime"))?;
        let skill = manager
            .get(req.name.trim())
            .map_err(|err| PluginError::new(format!("skill_get: {err}")))?;
        Ok(HostSkillGetResponse {
            name: skill.frontmatter.name.clone(),
            body: skill.body.clone(),
            allowed_tools: skill.frontmatter.allowed_tools.clone(),
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
