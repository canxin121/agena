//! In-process plugin that wraps agena's built-in tools.
//!
//! The plugin manifest is the canonical declaration source for built-in tool
//! metadata. Built-in payload wire structs remain the executor-facing input and
//! output shapes, but the builtin catalog should project from the declarations
//! in this module instead of maintaining a second hand-written list elsewhere.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{
    AskUserOption as HostAskUserOption, AskUserQuestion as HostAskUserQuestion, AskUserRequest,
    BuiltinToolRequest, HostClient, MonitorHandle, MonitorReadRequest, MonitorReadResponse,
    MonitorStartRequest, MonitorStopRequest, SpawnSubtaskRequest, ToolDescriptor,
};
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginEntryDecl, PluginManifest,
    Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
};

use crate::entry::monitor::{MonitorRead, MonitorStart, MonitorStopOutcome};
use crate::entry::result::BuiltinExecution;
use crate::entry::{
    BuiltinExecutionContext, ToolExecutionView, ToolExecutor, ask_user, monitor_tool, orchestrator,
    tool_search,
};
use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, BashToolInput, BuiltinToolInput, BuiltinToolOutput,
    GlobToolInput, GrepToolInput, MonitorStatus, MonitorStream, MonitorSummary, MonitorToolInput,
    PowerShellToolInput, TaskToolInput, ToolSearchToolInput,
};

thread_local! {
    static BUILTIN_CTX: RefCell<Option<ToolExecutor>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BuiltinContextKey {
    session_id: i64,
    call_id: i64,
    tool_name: String,
}

static BUILTIN_CTX_BY_CALL: LazyLock<Mutex<HashMap<BuiltinContextKey, ToolExecutor>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn with_executor<R>(
    executor: &ToolExecutor,
    session_id: i64,
    call_id: i64,
    tool_name: String,
    f: impl FnOnce() -> R,
) -> R {
    let key = BuiltinContextKey {
        session_id,
        call_id,
        tool_name,
    };
    if let Ok(mut contexts) = BUILTIN_CTX_BY_CALL.lock() {
        contexts.insert(key.clone(), executor.clone());
    }
    let out = BUILTIN_CTX.with(|cell| {
        let prev = cell.replace(Some(executor.clone()));
        let out = f();
        *cell.borrow_mut() = prev;
        out
    });
    if let Ok(mut contexts) = BUILTIN_CTX_BY_CALL.lock() {
        contexts.remove(&key);
    }
    out
}

fn current_executor(
    session_id: i64,
    call_id: i64,
    tool_name: &str,
) -> Result<ToolExecutor, PluginError> {
    if let Some(executor) = BUILTIN_CTX.with(|cell| cell.borrow().clone()) {
        return Ok(executor);
    }
    let key = BuiltinContextKey {
        session_id,
        call_id,
        tool_name: tool_name.to_string(),
    };
    BUILTIN_CTX_BY_CALL
        .lock()
        .ok()
        .and_then(|contexts| contexts.get(&key).cloned())
        .ok_or_else(|| PluginError::new("built-in plugin invoked without executor context"))
}

#[cfg(test)]
pub(crate) fn current_executor_for_test(
    session_id: i64,
    call_id: i64,
    tool_name: &str,
) -> Option<ToolExecutor> {
    current_executor_lookup(session_id, call_id, tool_name)
}

pub(crate) fn current_executor_lookup(
    session_id: i64,
    call_id: i64,
    tool_name: &str,
) -> Option<ToolExecutor> {
    if let Some(executor) = BUILTIN_CTX.with(|cell| cell.borrow().clone()) {
        return Some(executor);
    }
    let key = BuiltinContextKey {
        session_id,
        call_id,
        tool_name: tool_name.to_string(),
    };
    BUILTIN_CTX_BY_CALL
        .lock()
        .ok()
        .and_then(|contexts| contexts.get(&key).cloned())
}

/// Static plugin id used for every built-in tool. Keep stable: hot-reload
/// reuses transports keyed by id.
pub(crate) const BUILTIN_PLUGIN_ID: &str = "agena.builtin";

/// One-stop in-process plugin that exposes every built-in tool. We use a
/// single plugin to keep the manifest small and avoid duplicating registration
/// boilerplate.
pub(crate) struct BuiltinPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

impl BuiltinPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("built-in plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("built-in plugin invoked before init"))
    }
    fn host_ask_user_questions(input: &AskUserToolInput) -> Vec<HostAskUserQuestion> {
        input
            .questions
            .iter()
            .map(|question| HostAskUserQuestion {
                id: question.id.clone(),
                header: question.header.clone(),
                question: question.question.clone(),
                options: question
                    .options
                    .iter()
                    .map(|option| HostAskUserOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect()
    }

    fn monitor_status_from_str(
        status: Option<&str>,
        running: bool,
        exit_code: Option<i32>,
    ) -> Result<MonitorStatus, PluginError> {
        match status.map(str::trim).filter(|value| !value.is_empty()) {
            Some("running") => Ok(MonitorStatus::Running),
            Some("exited") => Ok(MonitorStatus::Exited),
            Some("failed") => Ok(MonitorStatus::Failed),
            Some("stopped") => Ok(MonitorStatus::Stopped),
            Some("timed_out") | Some("timed-out") | Some("timeout") => Ok(MonitorStatus::TimedOut),
            Some(other) => Err(PluginError::invalid_params(format!(
                "unknown monitor status '{other}'"
            ))),
            None if running => Ok(MonitorStatus::Running),
            None if exit_code.is_some() => Ok(MonitorStatus::Exited),
            None => Ok(MonitorStatus::Exited),
        }
    }

    fn monitor_stream_from_str(stream: &str) -> Result<MonitorStream, PluginError> {
        match stream.trim() {
            "stdout" | "out" => Ok(MonitorStream::Stdout),
            "stderr" | "err" => Ok(MonitorStream::Stderr),
            other => Err(PluginError::invalid_params(format!(
                "unknown monitor stream '{other}'"
            ))),
        }
    }

    fn monitor_summary_from_handle(
        handle: MonitorHandle,
        fallback_command: String,
    ) -> Result<MonitorSummary, PluginError> {
        let status = Self::monitor_status_from_str(
            handle.status.as_deref(),
            matches!(handle.status.as_deref(), Some("running")),
            handle.exit_code,
        )?;
        Ok(MonitorSummary {
            monitor_id: handle.id,
            command: handle.command.unwrap_or(fallback_command),
            description: handle.label.unwrap_or_default(),
            status,
            persistent: handle.persistent,
            started_at_ms: handle.started_at_ms,
            ended_at_ms: handle.ended_at_ms,
            buffered_lines: handle.buffered_lines,
            last_seq: handle.last_seq,
            dropped_lines: handle.dropped_lines,
            exit_code: handle.exit_code,
        })
    }

    fn monitor_read_from_response(
        response: MonitorReadResponse,
        fallback_monitor_id: String,
    ) -> Result<MonitorRead, PluginError> {
        let status = Self::monitor_status_from_str(
            response.status.as_deref(),
            response.running,
            response.exit_code,
        )?;
        let monitor_id = response.monitor_id.unwrap_or(fallback_monitor_id);
        let events = response
            .events
            .into_iter()
            .map(|event| {
                Ok(crate::message::MonitorEvent {
                    seq: event.seq,
                    stream: Self::monitor_stream_from_str(event.stream.as_str())?,
                    ts_ms: event.ts_ms,
                    line: event.line,
                })
            })
            .collect::<Result<Vec<_>, PluginError>>()?;
        Ok(MonitorRead {
            monitor_id,
            status,
            events,
            last_seq: response.last_seq,
            has_more: response.has_more,
            dropped_lines: response.dropped_lines,
            exit_code: response.exit_code,
        })
    }

    fn searchable_tool_from_descriptor(descriptor: ToolDescriptor) -> tool_search::SearchableTool {
        let behavior_label = descriptor.behavior.unwrap_or_else(|| {
            if descriptor.read_only {
                "read_only".to_string()
            } else {
                "mutating".to_string()
            }
        });
        tool_search::SearchableTool {
            name: descriptor.name,
            description: descriptor.description.unwrap_or_default(),
            search_terms: descriptor.search_terms,
            behavior_label,
            read_only: descriptor.read_only,
            deferred: descriptor.deferred,
        }
    }

    async fn invoke_ask_user(&self, input: &AskUserToolInput) -> SdkResult<ToolInvokeOutput> {
        ask_user::validate(input).map_err(|err| PluginError::invalid_params(err.to_string()))?;
        let host = self.host()?;
        let response = host
            .ask_user(AskUserRequest {
                questions: Self::host_ask_user_questions(input),
                prompt: String::new(),
                options: Vec::new(),
                allow_free_text: false,
            })
            .await?;
        if response.cancelled {
            let reason = if response.reply.trim().is_empty() {
                "user declined to answer requested questions".to_string()
            } else {
                response.reply
            };
            return Err(PluginError::new(reason));
        }

        let mut answers = response.answers;
        if answers.is_empty()
            && let Some(question) = input.questions.first()
            && !response.reply.trim().is_empty()
        {
            answers.insert(question.id.clone(), vec![response.reply]);
        }

        let execution = ask_user::execution_from_answers(input, answers);
        Ok(builtin_to_invoke_output(execution))
    }

    async fn invoke_task(&self, input: &TaskToolInput) -> SdkResult<ToolInvokeOutput> {
        let host = self.host()?;
        let response = host
            .spawn_subtask(SpawnSubtaskRequest {
                subagent_type: input.subagent_type.to_string(),
                description: input.description.clone(),
                prompt: input.prompt.clone(),
                task_id: input.task_id.clone(),
                command: input.command.clone(),
                model: None,
            })
            .await?;

        let session_id = response.metadata.get("session_id").cloned();
        let model_provider_id = response.metadata.get("model_provider_id").cloned();
        let model_id = response.metadata.get("model_id").cloned();
        let output_text = if response.final_text.trim().is_empty() {
            format!(
                "Created/resumed subtask session {} for profile '{}' in workspace {}.",
                session_id.as_deref().unwrap_or("unknown"),
                input.subagent_type,
                "<unknown>"
            )
        } else {
            response.final_text
        };
        let mut view = ToolExecutionView::simple(
            format!("Task {} ({})", input.description, input.subagent_type),
            output_text,
        );
        view.metadata
            .insert("description".to_string(), input.description.clone());
        view.metadata
            .insert("subagent_type".to_string(), input.subagent_type.to_string());
        view.metadata.insert(
            "profile_guidance".to_string(),
            input.subagent_type.guidance().to_string(),
        );
        if let Some(session_id_value) = session_id.clone() {
            view.metadata
                .insert("session_id".to_string(), session_id_value);
        }
        if let Some(command) = input.command.clone() {
            view.metadata.insert("command".to_string(), command);
        }
        if let Some(model_provider_id) = model_provider_id.clone() {
            view.metadata
                .insert("model_provider_id".to_string(), model_provider_id);
        }
        if let Some(model_id) = model_id.clone() {
            view.metadata.insert("model_id".to_string(), model_id);
        }
        for (key, value) in response.metadata {
            view.metadata.entry(key).or_insert(value);
        }

        let output = BuiltinToolOutput::Task {
            session_id,
            model_provider_id,
            model_id,
        };
        Ok(builtin_to_invoke_output(BuiltinExecution::new(
            output, view,
        )))
    }

    async fn invoke_tool_search(&self, input: &ToolSearchToolInput) -> SdkResult<ToolInvokeOutput> {
        let host = self.host()?;
        let catalog = host
            .list_tools()
            .await?
            .into_iter()
            .map(Self::searchable_tool_from_descriptor)
            .collect::<Vec<_>>();
        let execution = tool_search::execute_with_tools(&catalog, input)
            .map_err(|err| PluginError::invalid_params(err.to_string()))?;
        Ok(builtin_to_invoke_output(execution))
    }

    async fn invoke_host_builtin<T: serde::Serialize>(
        &self,
        tool_name: &str,
        input: &T,
    ) -> SdkResult<ToolInvokeOutput> {
        let host = self.host()?;
        let input = serde_json::to_value(input)
            .map_err(|err| PluginError::invalid_params(err.to_string()))?;
        host.execute_builtin_tool(BuiltinToolRequest {
            tool_name: tool_name.to_string(),
            input,
        })
        .await
    }

    async fn invoke_monitor(&self, input: &MonitorToolInput) -> SdkResult<ToolInvokeOutput> {
        let host = self.host()?;
        let execution = match input {
            MonitorToolInput::Start {
                command,
                description,
                workdir,
                timeout_ms,
                persistent,
                include_pattern,
                max_buffered_lines,
                capture_stderr,
            } => {
                let handle = host
                    .monitor_start(MonitorStartRequest {
                        command: vec![command.clone()],
                        cwd: workdir.clone(),
                        env: BTreeMap::new(),
                        label: (!description.trim().is_empty()).then_some(description.clone()),
                        timeout_ms: *timeout_ms,
                        persistent: *persistent,
                        include_pattern: include_pattern.clone(),
                        max_buffered_lines: *max_buffered_lines,
                        capture_stderr: *capture_stderr,
                    })
                    .await?;
                let summary = Self::monitor_summary_from_handle(handle, command.clone())?;
                monitor_tool::render_start(MonitorStart { summary })
            }
            MonitorToolInput::List {} => {
                let monitors = host
                    .monitor_list()
                    .await?
                    .into_iter()
                    .map(|handle| Self::monitor_summary_from_handle(handle, String::new()))
                    .collect::<Result<Vec<_>, _>>()?;
                monitor_tool::render_list(monitors)
            }
            MonitorToolInput::Read {
                monitor_id,
                since_seq,
                limit,
                wait_ms,
            } => {
                let read = host
                    .monitor_read(MonitorReadRequest {
                        id: monitor_id.clone(),
                        follow: *wait_ms > 0,
                        since_seq: *since_seq,
                        limit: *limit,
                        wait_ms: *wait_ms,
                    })
                    .await?;
                let read = Self::monitor_read_from_response(read, monitor_id.clone())?;
                monitor_tool::render_read(read)
            }
            MonitorToolInput::Stop { monitor_id } => {
                let summary = host
                    .monitor_stop(MonitorStopRequest {
                        id: monitor_id.clone(),
                        force: false,
                    })
                    .await?;
                let summary = Self::monitor_summary_from_handle(summary, monitor_id.clone())?;
                monitor_tool::render_stop(MonitorStopOutcome { summary })
            }
        };
        Ok(builtin_to_invoke_output(execution))
    }
}

pub(crate) fn entry_decls() -> Vec<PluginEntryDecl> {
    // All built-in entries have been migrated to dedicated first-party plugins
    // (`agena.fs`, `agena.shell`, `agena.web`, `agena.workflow`, `agena.lsp`,
    // `agena.cron`). The `agena.builtin` plugin keeps the in-process backend
    // implementations alive (used by `host.execute_builtin_tool`) but no
    // longer exposes any model-visible entries directly.
    Vec::new()
}

#[async_trait]
impl Plugin for BuiltinPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-builtins", env!("CARGO_PKG_VERSION"))
            .description("Agena built-in tools delivered as in-process plugin.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .entries(entry_decls())
            .build()
    }

    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("built-in plugin host lock poisoned"))? = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
        permission_paths_for(tool, input)
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let tool_name = input.tool_name.clone();
        let session_id = input.session_id;
        let call_id = input.call_id;
        let builtin = parse_builtin(&tool_name, input.input)
            .map_err(|err| PluginError::new(format!("parse {}: {err}", tool_name)))?;

        match &builtin {
            BuiltinToolInput::AskUser(payload) => return self.invoke_ask_user(payload).await,
            BuiltinToolInput::Task(payload) => return self.invoke_task(payload).await,
            BuiltinToolInput::ToolSearch(payload) => return self.invoke_tool_search(payload).await,
            BuiltinToolInput::Monitor(payload) => return self.invoke_monitor(payload).await,
            BuiltinToolInput::EnterPlanMode(payload) => {
                return self.invoke_host_builtin("enter_plan_mode", payload).await;
            }
            BuiltinToolInput::ExitPlanMode(payload) => {
                return self.invoke_host_builtin("exit_plan_mode", payload).await;
            }
            BuiltinToolInput::SkillRun(payload) => {
                return self.invoke_host_builtin("skill_run", payload).await;
            }
            BuiltinToolInput::EnterWorktree(payload) => {
                return self.invoke_host_builtin("enter_worktree", payload).await;
            }
            BuiltinToolInput::ExitWorktree(payload) => {
                return self.invoke_host_builtin("exit_worktree", payload).await;
            }
            BuiltinToolInput::CronCreate(payload) => {
                return self.invoke_host_builtin("cron_create", payload).await;
            }
            BuiltinToolInput::CronList(payload) => {
                return self.invoke_host_builtin("cron_list", payload).await;
            }
            BuiltinToolInput::CronDelete(payload) => {
                return self.invoke_host_builtin("cron_delete", payload).await;
            }
            BuiltinToolInput::ScheduleWakeup(payload) => {
                return self.invoke_host_builtin("schedule_wakeup", payload).await;
            }
            BuiltinToolInput::LspDefinition(payload) => {
                return self.invoke_host_builtin("lsp_definition", payload).await;
            }
            BuiltinToolInput::LspReferences(payload) => {
                return self.invoke_host_builtin("lsp_references", payload).await;
            }
            BuiltinToolInput::LspHover(payload) => {
                return self.invoke_host_builtin("lsp_hover", payload).await;
            }
            BuiltinToolInput::LspDiagnostics(payload) => {
                return self.invoke_host_builtin("lsp_diagnostics", payload).await;
            }
            _ => {}
        }

        let executor = current_executor(session_id, call_id, &tool_name)?;
        let context = BuiltinExecutionContext {
            session_id: if session_id < 0 {
                None
            } else {
                Some(session_id)
            },
            call_id: if call_id < 0 { None } else { Some(call_id) },
        };
        let execution = orchestrator::execute_builtin(&executor, &builtin, context)
            .map_err(|err| PluginError::new(format!("{}: {err}", tool_name)))?;
        Ok(builtin_to_invoke_output(execution))
    }
}

pub(crate) fn permission_paths_for(
    tool: &str,
    input: &serde_json::Value,
) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
    match tool {
        "apply_patch" => {
            let payload: ApplyPatchToolInput = serde_json::from_value(input.clone())?;
            let paths = crate::entry::apply_patch::planned_paths(&payload.patch)
                .map_err(|err| PluginError::new(err.to_string()))?;
            Ok(paths
                .into_iter()
                .map(crate::plugin::sdk::PathRequest::write)
                .collect())
        }
        "bash" => {
            let payload: BashToolInput = serde_json::from_value(input.clone())?;
            Ok(default_workspace_read(payload.workdir.is_none()))
        }
        "powershell" => {
            let payload: PowerShellToolInput = serde_json::from_value(input.clone())?;
            Ok(default_workspace_read(payload.workdir.is_none()))
        }
        "glob" => {
            let payload: GlobToolInput = serde_json::from_value(input.clone())?;
            Ok(default_workspace_read(payload.path.is_none()))
        }
        "grep" => {
            let payload: GrepToolInput = serde_json::from_value(input.clone())?;
            Ok(default_workspace_read(payload.path.is_none()))
        }
        "monitor" => {
            let payload: MonitorToolInput = serde_json::from_value(input.clone())?;
            let needs_workspace = matches!(payload, MonitorToolInput::Start { workdir: None, .. });
            Ok(default_workspace_read(needs_workspace))
        }
        _ => Ok(Vec::new()),
    }
}

fn default_workspace_read(needs_workspace: bool) -> Vec<crate::plugin::sdk::PathRequest> {
    needs_workspace
        .then(|| crate::plugin::sdk::PathRequest::read(""))
        .into_iter()
        .collect()
}

pub(crate) fn parse_builtin(
    tool: &str,
    input: JsonValue,
) -> Result<BuiltinToolInput, serde_json::Error> {
    Ok(match tool {
        "bash" => BuiltinToolInput::Bash(serde_json::from_value(input)?),
        "read" => BuiltinToolInput::Read(serde_json::from_value(input)?),
        "view_file" => BuiltinToolInput::ViewFile(serde_json::from_value(input)?),
        "apply_patch" => BuiltinToolInput::ApplyPatch(serde_json::from_value(input)?),
        "glob" => BuiltinToolInput::Glob(serde_json::from_value(input)?),
        "grep" => BuiltinToolInput::Grep(serde_json::from_value(input)?),
        "task" => BuiltinToolInput::Task(serde_json::from_value(input)?),
        "tool_search" => BuiltinToolInput::ToolSearch(serde_json::from_value(input)?),
        "todo_write" => BuiltinToolInput::TodoWrite(serde_json::from_value(input)?),
        "ask_user" => BuiltinToolInput::AskUser(serde_json::from_value(input)?),
        "monitor" => BuiltinToolInput::Monitor(serde_json::from_value(input)?),
        "web_fetch" => BuiltinToolInput::WebFetch(serde_json::from_value(input)?),
        "web_search" => BuiltinToolInput::WebSearch(serde_json::from_value(input)?),
        "enter_plan_mode" => BuiltinToolInput::EnterPlanMode(serde_json::from_value(input)?),
        "exit_plan_mode" => BuiltinToolInput::ExitPlanMode(serde_json::from_value(input)?),
        "skill_run" => BuiltinToolInput::SkillRun(serde_json::from_value(input)?),
        "enter_worktree" => BuiltinToolInput::EnterWorktree(serde_json::from_value(input)?),
        "exit_worktree" => BuiltinToolInput::ExitWorktree(serde_json::from_value(input)?),
        "cron_create" => BuiltinToolInput::CronCreate(serde_json::from_value(input)?),
        "cron_list" => BuiltinToolInput::CronList(serde_json::from_value(input)?),
        "cron_delete" => BuiltinToolInput::CronDelete(serde_json::from_value(input)?),
        "schedule_wakeup" => BuiltinToolInput::ScheduleWakeup(serde_json::from_value(input)?),
        "lsp_definition" => BuiltinToolInput::LspDefinition(serde_json::from_value(input)?),
        "lsp_references" => BuiltinToolInput::LspReferences(serde_json::from_value(input)?),
        "lsp_hover" => BuiltinToolInput::LspHover(serde_json::from_value(input)?),
        "lsp_diagnostics" => BuiltinToolInput::LspDiagnostics(serde_json::from_value(input)?),
        "notebook_edit" => BuiltinToolInput::NotebookEdit(serde_json::from_value(input)?),
        "powershell" => BuiltinToolInput::PowerShell(serde_json::from_value(input)?),
        other => {
            return Err(serde::de::Error::custom(format!(
                "unknown built-in tool `{other}`"
            )));
        }
    })
}

pub(crate) fn builtin_to_invoke_output(execution: BuiltinExecution) -> ToolInvokeOutput {
    let envelope = BuiltinResponseEnvelope {
        output: execution.output,
        apply_patch: execution.apply_patch,
    };
    let payload = serde_json::to_value(&envelope).ok();
    ToolInvokeOutput {
        title: execution.view.title,
        output_text: execution.view.output_text,
        payload,
        metadata: execution.view.metadata.into_iter().collect(),
        attachments: execution.view.attachments,
    }
}

/// Wire envelope used to round-trip a [`BuiltinExecution`] through the
/// plugin host's JSON payload. Lets the executor reconstruct the
/// `apply_patch` side-channel that the plugin wire types do not preserve.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct BuiltinResponseEnvelope {
    pub output: BuiltinToolOutput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_patch: Option<crate::entry::apply_patch::ApplyPatchExecution>,
}

/// Decode the payload emitted by [`builtin_to_invoke_output`] back into a
/// [`BuiltinResponseEnvelope`].
pub(crate) fn payload_to_builtin_envelope(
    payload: Option<&JsonValue>,
) -> Result<BuiltinResponseEnvelope, serde_json::Error> {
    match payload {
        Some(value) => serde_json::from_value(value.clone()),
        None => Err(serde::de::Error::custom(
            "built-in plugin response missing payload",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::message::{SkillRunToolInput, TaskSubagentType, UserInputOption, UserInputQuestion};
    use crate::plugin::sdk::host_api::{
        AskUserRequest, AskUserResponse, BuiltinToolRequest, EventSubscription, LogLevel,
        MonitorEvent as HostMonitorEvent, MonitorHandle, MonitorReadRequest, MonitorReadResponse,
        MonitorStartRequest, MonitorStopRequest, SpawnSubtaskRequest, SpawnSubtaskResponse,
        ToolDescriptor,
    };
    use crate::plugin::sdk::{
        EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision, Plugin, ToolInvokeInput,
    };

    use super::*;

    struct TestHost;

    #[async_trait::async_trait]
    impl HostClient for TestHost {
        async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

        async fn publish_event(&self, _env: EventEnvelope) -> SdkResult<()> {
            Ok(())
        }

        async fn subscribe_events(&self, _filter: EventFilter) -> SdkResult<EventSubscription> {
            Ok(EventSubscription { id: "sub".into() })
        }

        async fn ask_permission(&self, _req: PermissionAskInput) -> SdkResult<PermissionDecision> {
            Ok(PermissionDecision::Prompt)
        }

        async fn read_config(&self, _path: Option<String>) -> SdkResult<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }

        async fn invoke_tool(
            &self,
            tool: String,
            _input: serde_json::Value,
        ) -> SdkResult<ToolInvokeOutput> {
            Err(PluginError::new(format!(
                "unexpected invoke_tool for {tool}"
            )))
        }

        async fn ask_user(&self, req: AskUserRequest) -> SdkResult<AskUserResponse> {
            let question_id = req
                .questions
                .first()
                .map(|question| question.id.clone())
                .unwrap_or_else(|| "reply".to_string());
            Ok(AskUserResponse {
                reply: String::new(),
                cancelled: false,
                answers: BTreeMap::from([(question_id, vec!["blue".to_string()])]),
            })
        }

        async fn spawn_subtask(&self, req: SpawnSubtaskRequest) -> SdkResult<SpawnSubtaskResponse> {
            Ok(SpawnSubtaskResponse {
                final_text: format!("spawned {}", req.description),
                metadata: BTreeMap::from([
                    ("session_id".to_string(), "child-1".to_string()),
                    ("model_provider_id".to_string(), "provider".to_string()),
                    ("model_id".to_string(), "model".to_string()),
                ]),
            })
        }

        async fn list_tools(&self) -> SdkResult<Vec<ToolDescriptor>> {
            Ok(vec![ToolDescriptor {
                name: "bash".to_string(),
                description: Some("Execute shell commands".to_string()),
                search_terms: vec!["shell".to_string()],
                behavior: Some("mutating".to_string()),
                deferred: true,
                read_only: false,
                plugin_id: None,
            }])
        }

        async fn execute_builtin_tool(
            &self,
            req: BuiltinToolRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            assert_eq!(req.tool_name, "skill_run");
            let name = req
                .input
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            Ok(builtin_to_invoke_output(BuiltinExecution::new(
                BuiltinToolOutput::SkillRun {
                    name,
                    body_chars: 42,
                    allowed_tools: vec!["read".to_string()],
                },
                ToolExecutionView::simple("Skill demo", "loaded skill"),
            )))
        }

        async fn monitor_start(&self, req: MonitorStartRequest) -> SdkResult<MonitorHandle> {
            Ok(MonitorHandle {
                id: "mon-1".to_string(),
                label: req.label,
                command: Some(req.command.join(" ")),
                status: Some("running".to_string()),
                persistent: req.persistent,
                started_at_ms: 123,
                ended_at_ms: None,
                buffered_lines: 0,
                last_seq: 0,
                dropped_lines: 0,
                exit_code: None,
            })
        }

        async fn monitor_list(&self) -> SdkResult<Vec<MonitorHandle>> {
            Ok(Vec::new())
        }

        async fn monitor_read(&self, req: MonitorReadRequest) -> SdkResult<MonitorReadResponse> {
            Ok(MonitorReadResponse {
                monitor_id: Some(req.id),
                events: vec![HostMonitorEvent {
                    seq: 1,
                    stream: "stdout".to_string(),
                    ts_ms: 456,
                    line: "ready".to_string(),
                }],
                monitors: Vec::new(),
                stdout: "ready".to_string(),
                stderr: String::new(),
                running: false,
                status: Some("exited".to_string()),
                last_seq: 1,
                has_more: false,
                dropped_lines: 0,
                exit_code: Some(0),
            })
        }

        async fn monitor_stop(&self, req: MonitorStopRequest) -> SdkResult<MonitorHandle> {
            Ok(MonitorHandle {
                id: req.id,
                label: None,
                command: None,
                status: Some("stopped".to_string()),
                persistent: false,
                started_at_ms: 123,
                ended_at_ms: Some(456),
                buffered_lines: 0,
                last_seq: 0,
                dropped_lines: 0,
                exit_code: None,
            })
        }
    }

    async fn initialized_plugin() -> BuiltinPlugin {
        let plugin = BuiltinPlugin::new();
        plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: PathBuf::from("/tmp"),
                    plugin_id: BUILTIN_PLUGIN_ID.to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    options: serde_json::Value::Null,
                    protocol_version: crate::plugin::sdk::rpc::PROTOCOL_VERSION,
                },
                Arc::new(TestHost),
            )
            .await
            .expect("builtin plugin init");
        plugin
    }

    fn invoke_input<T: serde::Serialize>(tool_name: &str, input: T) -> ToolInvokeInput {
        ToolInvokeInput {
            tool_name: tool_name.to_string(),
            session_id: 1,
            call_id: 2,
            workspace_root: "/tmp".to_string(),
            input: serde_json::to_value(input).expect("serialize tool input"),
        }
    }

    #[tokio::test]
    async fn ask_user_invokes_host_without_executor_context() {
        let plugin = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_input(
                "ask_user",
                AskUserToolInput {
                    questions: vec![UserInputQuestion {
                        id: "color".to_string(),
                        header: "Color".to_string(),
                        question: "Which color?".to_string(),
                        options: vec![UserInputOption {
                            label: "blue".to_string(),
                            description: String::new(),
                        }],
                        multiple: false,
                        allow_custom: false,
                    }],
                },
            ))
            .await
            .expect("ask_user host invoke");
        let envelope = payload_to_builtin_envelope(output.payload.as_ref()).unwrap();
        match envelope.output {
            BuiltinToolOutput::AskUser { answers } => {
                assert_eq!(answers["color"], vec!["blue".to_string()]);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn task_invokes_host_without_executor_context() {
        let plugin = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_input(
                "task",
                TaskToolInput {
                    description: "inspect".to_string(),
                    prompt: "look around".to_string(),
                    subagent_type: TaskSubagentType::Explore,
                    task_id: None,
                    command: None,
                },
            ))
            .await
            .expect("task host invoke");
        let envelope = payload_to_builtin_envelope(output.payload.as_ref()).unwrap();
        match envelope.output {
            BuiltinToolOutput::Task {
                session_id,
                model_provider_id,
                model_id,
            } => {
                assert_eq!(session_id.as_deref(), Some("child-1"));
                assert_eq!(model_provider_id.as_deref(), Some("provider"));
                assert_eq!(model_id.as_deref(), Some("model"));
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_search_invokes_host_catalog_without_executor_context() {
        let plugin = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_input(
                "tool_search",
                ToolSearchToolInput {
                    query: "shell".to_string(),
                    load: Vec::new(),
                    limit: None,
                },
            ))
            .await
            .expect("tool_search host invoke");
        let envelope = payload_to_builtin_envelope(output.payload.as_ref()).unwrap();
        match envelope.output {
            BuiltinToolOutput::ToolSearch { results, .. } => {
                assert_eq!(results, vec!["bash".to_string()]);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn substrate_builtin_invokes_host_without_executor_context() {
        let plugin = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_input(
                "skill_run",
                SkillRunToolInput {
                    name: "demo".to_string(),
                    args: None,
                },
            ))
            .await
            .expect("substrate host invoke");
        let envelope = payload_to_builtin_envelope(output.payload.as_ref()).unwrap();
        match envelope.output {
            BuiltinToolOutput::SkillRun {
                name,
                body_chars,
                allowed_tools,
            } => {
                assert_eq!(name, "demo");
                assert_eq!(body_chars, 42);
                assert_eq!(allowed_tools, vec!["read".to_string()]);
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[tokio::test]
    async fn monitor_invokes_host_without_executor_context() {
        let plugin = initialized_plugin().await;
        let output = plugin
            .tool_invoke(invoke_input(
                "monitor",
                MonitorToolInput::Start {
                    command: "cargo check".to_string(),
                    description: "check".to_string(),
                    workdir: None,
                    timeout_ms: Some(1000),
                    persistent: false,
                    include_pattern: None,
                    max_buffered_lines: None,
                    capture_stderr: true,
                },
            ))
            .await
            .expect("monitor host invoke");
        let envelope = payload_to_builtin_envelope(output.payload.as_ref()).unwrap();
        match envelope.output {
            BuiltinToolOutput::Monitor {
                action,
                monitor_id,
                status,
                ..
            } => {
                assert_eq!(action, "start");
                assert_eq!(monitor_id.as_deref(), Some("mon-1"));
                assert_eq!(status, Some(MonitorStatus::Running));
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[test]
    fn builtin_entry_decls_have_unique_names() {
        let names = entry_decls()
            .into_iter()
            .map(|decl| decl.name)
            .collect::<Vec<_>>();
        let unique = names.iter().cloned().collect::<BTreeSet<_>>();
        assert_eq!(names.len(), unique.len());
    }

    #[test]
    fn builtin_manifest_names_match_parser_samples() {
        for decl in entry_decls() {
            let sample = builtin_sample_input(decl.name.as_str());
            let parsed = parse_builtin(decl.name.as_str(), sample)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", decl.name));
            assert_eq!(parsed.tool_name(), decl.name);
        }
    }

    #[test]
    fn request_user_input_alias_is_not_a_builtin_entry() {
        let err = parse_builtin("request_user_input", serde_json::json!({"questions": []}))
            .expect_err("legacy alias should not parse");
        assert!(err.to_string().contains("unknown built-in tool"));
    }

    fn builtin_sample_input(name: &str) -> serde_json::Value {
        match name {
            "bash" => serde_json::json!({"command": "echo hi", "description": "demo"}),
            "read" => serde_json::json!({"file_path": "notes.txt"}),
            "view_file" => serde_json::json!({"path": "notes.txt"}),
            "apply_patch" => serde_json::json!({"patch": "*** Begin Patch\n*** End Patch"}),
            "glob" => serde_json::json!({"pattern": "*.rs"}),
            "grep" => serde_json::json!({"pattern": "needle"}),
            "task" => serde_json::json!({
                "description": "inspect",
                "prompt": "look around",
                "subagent_type": "explore"
            }),
            "tool_search" => serde_json::json!({"query": "read"}),
            "todo_write" => serde_json::json!({"items": []}),
            "ask_user" => serde_json::json!({"questions": []}),
            "monitor" => serde_json::json!({"action": "list"}),
            "web_fetch" => serde_json::json!({"url": "https://example.com"}),
            "web_search" => serde_json::json!({"query": "agena"}),
            "enter_plan_mode" => serde_json::json!({}),
            "exit_plan_mode" => serde_json::json!({}),
            "skill_run" => serde_json::json!({"name": "demo"}),
            "enter_worktree" => serde_json::json!({}),
            "exit_worktree" => serde_json::json!({"action": "keep", "discard_changes": false}),
            "cron_create" => serde_json::json!({"expression": "0 0 1 1 * *", "prompt": "demo"}),
            "cron_list" => serde_json::json!({}),
            "cron_delete" => serde_json::json!({"id": "job-1"}),
            "schedule_wakeup" => serde_json::json!({"delay_seconds": 60, "prompt": "demo"}),
            "lsp_definition" => {
                serde_json::json!({"file_path": "src/main.rs", "line": 1, "character": 1})
            }
            "lsp_references" => {
                serde_json::json!({"file_path": "src/main.rs", "line": 1, "character": 1})
            }
            "lsp_hover" => {
                serde_json::json!({"file_path": "src/main.rs", "line": 1, "character": 1})
            }
            "lsp_diagnostics" => serde_json::json!({"file_path": "src/main.rs"}),
            "notebook_edit" => serde_json::json!({
                "notebook_path": "demo.ipynb",
                "edit_mode": "replace",
                "new_source": "print(1)"
            }),
            "powershell" => serde_json::json!({"command": "Write-Host hi", "description": "demo"}),
            other => panic!("missing sample input for builtin {other}"),
        }
    }
}
