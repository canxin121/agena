//! Concrete `HostClient` impl backed by the live `AgenaRuntime`. Plugins
//! that run as subprocess (stdio) or remote (HTTP) call back into this via
//! JSON-RPC; the `HostHandle` in `agena-plugin-host` routes those calls
//! through this client.

use std::{future::Future, sync::Arc};

use agena_plugin_sdk::PluginKey;
use async_trait::async_trait;

use crate::message::{AskUserToolInput, EnterSnapshotToolInput, ExitSnapshotToolInput};
use crate::plugins::storage::{
    PluginSecretStore, PluginStorage, PluginStorageError, StorageLocator,
};
use crate::runtime::AgenaRuntime;
use crate::tool::{MonitorError, MonitorReadParams, MonitorStartParams};
use agena_domain::ToolInvocation;
use agena_domain::{StructuredObject, UserInputOption, UserInputQuestion};
use agena_plugin_host::sdk::host_api::{
    AskUserRequest, AskUserResponse, CancelSubtaskRequest, EventSubscription, HostCallbackContext,
    HostClient, HostConfigReloadResponse, HostContextStatusRequest, HostContextStatusResponse,
    HostEnterSnapshotRequest, HostExitSnapshotRequest, HostGetSessionRequest,
    HostGetSessionResponse, HostImageExecuteRequest, HostImageExecuteResponse, HostImageOperation,
    HostLspDiagnostic, HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse,
    HostLspListServersResponse, HostLspServer, HostMcpAddServerRequest, HostMcpListServersResponse,
    HostMcpRemoveServerRequest, HostMcpRemoveServerResponse, HostMcpServerSpec, HostPluginStatus,
    HostPluginStatusGetRequest, HostPluginStatusGetResponse, HostPluginStatusListResponse,
    HostRenameSessionRequest, HostRenameSessionResponse, HostSchedulerCreateRequest,
    HostSchedulerCreateResponse, HostSchedulerDeleteRequest, HostSchedulerDeleteResponse,
    HostSchedulerJob, HostSchedulerListResponse, HostSecretDeleteRequest, HostSecretGetRequest,
    HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest, HostSession,
    HostSetSessionModelRequest, HostSetSessionModelResponse, HostSnapshotListResponse,
    HostSnapshotSummary, HostStorageDeleteRequest, HostStorageGetRequest, HostStorageGetResponse,
    HostStorageListRequest, HostStorageListResponse, HostStorageRecord, HostStorageSetRequest,
    LogLevel, MessageSubtaskRequest, MonitorEvent, MonitorHandle, MonitorReadRequest,
    MonitorReadResponse, MonitorStartRequest, MonitorStopRequest, ReadSubtaskOutputRequest,
    ReadSubtaskOutputResponse, RunSubtaskRequest, RunSubtaskResponse, RunSubtaskStatus,
    RunSubtaskUsage, SubtaskControlResponse, SubtaskOutputChunk, ToolDescriptor,
    current_host_callback_context,
};
use agena_plugin_host::{
    EventEnvelope, EventFilter as PluginEventFilter, PluginError, ToolInvokeOutput,
};

mod image;
mod mappers;

use image::*;
use mappers::*;

pub(crate) fn host_client_for(runtime: AgenaRuntime) -> Arc<dyn HostClient> {
    Arc::new(RuntimeHostClient { runtime })
}

pub fn install_plugin_host_event_publisher(
    host_handle: Arc<agena_plugin_host::host::HostHandle>,
    runtime: AgenaRuntime,
) {
    let listener = Arc::new(
        move |event: agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent| {
            let runtime = runtime.clone();
            if let Ok(_handle) = tokio::runtime::Handle::try_current() {
                agena_runtime::spawn_detached(async move {
                    publish_tool_registry_changed_event(runtime, event).await;
                });
            } else {
                tracing::debug!(
                    target: "agena_plugin_host::events",
                    "skipping tool-registry event publish: no tokio runtime available"
                );
            }
        },
    );
    host_handle.set_tool_registry_event_listener(Some(listener));
}

async fn publish_tool_registry_changed_event(
    runtime: AgenaRuntime,
    event: agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent,
) {
    let snapshot = runtime.current_snapshot();
    let Some(manager) = snapshot.session_manager() else {
        tracing::debug!(
            target: "agena_plugin_host::events",
            generation = event.generation,
            tool = %event.tool_key,
            "skipping tool-registry event publish: no session manager"
        );
        return;
    };
    if let Err(err) = manager
        .event_publisher()
        .publish(
            crate::event::PublishContext::default(),
            crate::event::EventKind::PluginToolRegistryChanged(event),
        )
        .await
    {
        tracing::warn!(
            target: "agena_plugin_host::events",
            "failed to publish tool-registry change event: {err}"
        );
    }
}

fn plugin_error(error: impl ToString) -> PluginError {
    PluginError::internal(error.to_string())
}

fn plugin_error_from_app(error: crate::AppError) -> PluginError {
    match error {
        crate::AppError::PolicyDenied(denial) => {
            let mut error = PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                denial.reason.clone(),
            );
            error.diagnostic.data = Some(serde_json::json!({ "denial": denial }));
            error
        }
        crate::AppError::UserDeclined(decline) => {
            let reason = decline
                .reason
                .clone()
                .unwrap_or_else(|| "the user declined the permission request".to_string());
            let mut error = PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::UserDeclined,
                reason,
            );
            error.diagnostic.data = Some(serde_json::json!({ "decline": decline }));
            error
        }
        crate::AppError::CapabilityUnavailable(unavailable) => {
            let mut error = PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::CapabilityUnavailable,
                unavailable.reason.clone(),
            );
            error.diagnostic.data = Some(serde_json::json!({ "unavailable": unavailable }));
            error
        }
        crate::AppError::ToolUnavailable(unavailable) => {
            let mut error = PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::ToolUnavailable,
                unavailable.reason.clone(),
            );
            error.diagnostic.data = Some(serde_json::json!({ "unavailable": unavailable }));
            error
        }
        crate::AppError::Tool(error) => error.into_plugin_error(),
        other => PluginError::internal(other.to_string()),
    }
}

struct RuntimeHostClient {
    runtime: AgenaRuntime,
}

impl RuntimeHostClient {
    fn snapshot(&self) -> Arc<crate::runtime::RuntimeSnapshot> {
        self.runtime.current_snapshot()
    }

    fn session_manager(&self) -> Result<Arc<crate::session::SessionManager>, PluginError> {
        self.snapshot()
            .session_manager()
            .ok_or_else(|| host_unavailable("session manager is not enabled in this runtime"))
    }

    fn optional_session_manager(&self) -> Option<Arc<crate::session::SessionManager>> {
        self.snapshot().session_manager()
    }

    fn tool_executor(&self) -> Result<crate::tool::ToolExecutor, PluginError> {
        Ok(self.session_manager()?.tool_executor())
    }

    async fn use_session_manager<T, E, F>(
        &self,
        use_manager: impl FnOnce(Arc<crate::session::SessionManager>) -> F,
    ) -> Result<T, PluginError>
    where
        E: ToString,
        F: Future<Output = Result<T, E>>,
    {
        use_manager(self.session_manager()?)
            .await
            .map_err(plugin_error)
    }

    fn snapshot_feature<T>(
        &self,
        feature: impl FnOnce(&crate::runtime::RuntimeSnapshot) -> Option<T>,
        unavailable: &'static str,
    ) -> Result<T, PluginError> {
        let snapshot = self.snapshot();
        feature(snapshot.as_ref()).ok_or_else(|| host_unavailable(unavailable))
    }

    fn executor_feature<T>(
        &self,
        feature: impl FnOnce(&crate::tool::ToolExecutor) -> Option<T>,
        unavailable: &'static str,
    ) -> Result<(crate::tool::ToolExecutor, T), PluginError> {
        let executor = self.tool_executor()?;
        let feature = feature(&executor).ok_or_else(|| host_unavailable(unavailable))?;
        Ok((executor, feature))
    }

    async fn callback_session_context(
        &self,
    ) -> Result<Option<crate::session::model::SessionExecutionContext>, PluginError> {
        let Some(session_id) =
            current_host_callback_context().and_then(|context| context.session_id)
        else {
            return Ok(None);
        };
        if session_id < 0 {
            return Ok(None);
        }
        let Some(manager) = self.optional_session_manager() else {
            return Ok(None);
        };
        let session = manager
            .get_session(session_id)
            .await
            .map_err(plugin_error)?;
        Ok(Some(session.runtime().execution.clone()))
    }

    async fn callback_scoped_tool_executor(
        &self,
    ) -> Result<
        (
            crate::tool::ToolExecutor,
            Option<crate::session::model::SessionExecutionContext>,
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

    fn callback_plugin_key(&self) -> Result<PluginKey, PluginError> {
        let plugin_id = self
            .callback_context()?
            .plugin_id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| host_unavailable("host callback context is missing plugin_id"))?;
        plugin_id
            .parse()
            .map_err(|err| host_unavailable(format!("invalid host callback plugin_id: {err}")))
    }

    fn storage_locator(
        &self,
        scope: agena_plugin_host::sdk::host_api::HostStorageScope,
        visibility: agena_plugin_host::sdk::host_api::HostStorageVisibility,
    ) -> Result<StorageLocator, PluginError> {
        let plugin_id = self.callback_plugin_key()?;
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
        self.snapshot().plugin_storage()
    }

    fn plugin_secret_store(&self) -> Arc<dyn PluginSecretStore> {
        self.snapshot().plugin_secret_store()
    }

    fn plugin_manager(&self) -> Arc<agena_plugin_host::PluginHost> {
        self.snapshot().plugin_manager()
    }

    fn use_plugin_storage<T>(
        &self,
        scope: agena_plugin_host::sdk::host_api::HostStorageScope,
        visibility: agena_plugin_host::sdk::host_api::HostStorageVisibility,
        use_store: impl FnOnce(&dyn PluginStorage, &StorageLocator) -> Result<T, PluginStorageError>,
    ) -> Result<T, PluginError> {
        let locator = self.storage_locator(scope, visibility)?;
        let store = self.plugin_storage();
        use_store(store.as_ref(), &locator).map_err(map_storage_error)
    }

    fn use_plugin_secret_store<T>(
        &self,
        use_store: impl FnOnce(&dyn PluginSecretStore, &PluginKey) -> Result<T, PluginStorageError>,
    ) -> Result<T, PluginError> {
        let plugin_id = self.callback_plugin_key()?;
        let store = self.plugin_secret_store();
        use_store(store.as_ref(), &plugin_id).map_err(map_storage_error)
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
        let _ = self
            .runtime
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
            .unwrap_or_else(|| "<unknown>".into());
        let plugin_id = plugin_id.parse().unwrap_or_else(|_| {
            agena_plugin_host::PluginKey::new("unknown", "unknown").expect("static plugin key")
        });
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
            .map_err(|e| PluginError::internal(format!("event publish failed: {e}")))?;
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

    async fn read_config(&self, path: Option<String>) -> Result<serde_json::Value, PluginError> {
        let snapshot = self.runtime.current_snapshot();
        let value = snapshot
            .config_value()
            .map_err(|e| PluginError::invalid_params(e.to_string()))?;
        agena_domain::get_json_path(&value, path.as_deref())
            .map_err(|e| PluginError::invalid_params(e.to_string()))
    }

    async fn reload_config(&self) -> Result<HostConfigReloadResponse, PluginError> {
        let report = self
            .runtime
            .reload()
            .await
            .map_err(|e| PluginError::internal(e.to_string()))?;
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
        let host = self.plugin_manager();
        let resolution = if let Some(resolution) = host.lookup_tool(&tool) {
            resolution
        } else {
            let mut candidates = host
                .registered_tools()
                .into_iter()
                .filter(|candidate| {
                    crate::tool::registered_tool_matches_name(candidate, tool.as_str())
                })
                .collect::<Vec<_>>();
            candidates.sort_by_key(|candidate| candidate.canonical_name());
            match candidates.as_slice() {
                [] => return Err(PluginError::internal(format!("tool `{tool}` not found"))),
                [resolution] => resolution.clone(),
                _ => {
                    let names = candidates
                        .iter()
                        .map(|candidate| format!("`{}`", candidate.canonical_name()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(PluginError::invalid_params(format!(
                        "tool `{tool}` is ambiguous; use one of {names}"
                    )));
                }
            }
        };

        let caller = self.callback_context()?;
        let plugin_id = resolution.plugin_full_name();
        if caller
            .plugin_id
            .as_ref()
            .is_some_and(|current| current == &plugin_id)
        {
            return Err(PluginError::internal(format!(
                "host->plugin invoke would re-enter plugin `{plugin_id}` (cycle detected)"
            )));
        }

        let session_id = caller
            .session_id
            .ok_or_else(|| host_unavailable("host/tool.invoke requires session_id"))?;
        let call_id = caller.call_id.unwrap_or(-1);
        let structured = StructuredObject::try_from(input)
            .map_err(|err| PluginError::invalid_params(format!("invoke_tool input: {err}")))?;
        let invocation = ToolInvocation::new(resolution.canonical_name(), structured);
        let _guard = agena_runtime::try_enter_invocation(session_id, call_id, plugin_id.clone())
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "host->plugin invoke would re-enter plugin `{plugin_id}` (cycle detected)"
                ))
            })?;
        let execution = self
            .session_manager()?
            .execute_host_invoked_tool(session_id, call_id, invocation)
            .await
            .map_err(plugin_error_from_app)?;

        Ok(tool_execution_to_invoke_output(execution))
    }

    async fn ask_user(&self, req: AskUserRequest) -> Result<AskUserResponse, PluginError> {
        let (session_id, call_id) = self.callback_session_and_call()?;
        let input = ask_user_tool_input(req)?;
        self.session_manager()?
            .request_host_user_input(session_id, call_id, input)
            .await
            .map_err(plugin_error)
    }

    async fn run_subtask(&self, req: RunSubtaskRequest) -> Result<RunSubtaskResponse, PluginError> {
        let parent_session_id = match req.parent_session_id {
            Some(parent_session_id) => parent_session_id,
            None => self.callback_session_and_call()?.0,
        };
        let selection = req.selection.unwrap_or_default();
        let response = self
            .use_session_manager(|manager| async move {
                manager
                    .run_subtask(crate::session::SessionSubtaskRequest {
                        parent_session_id,
                        description: req.description,
                        prompt: req.prompt,
                        access: match req.access {
                            agena_plugin_host::sdk::host_api::RunSubtaskAccess::Inherit => {
                                agena_domain::ExecutionAccess::Inherit
                            }
                            agena_plugin_host::sdk::host_api::RunSubtaskAccess::ReadOnly => {
                                agena_domain::ExecutionAccess::ReadOnly
                            }
                        },
                        task_id: req.task_id,
                        requested_model_selection: agena_domain::ModelSelectionConfig {
                            provider: selection.provider,
                            adapter: selection.adapter,
                            model: selection.model,
                            thinking_mode: selection.thinking_mode,
                            speed_mode: selection.speed_mode,
                            verbosity: selection.verbosity,
                            parallel_tool_calls: selection.parallel_tool_calls,
                        },
                        timeout_ms: req.timeout_ms,
                        max_tokens: req.max_tokens,
                        max_cost_microusd: req.max_cost_microusd,
                    })
                    .await
            })
            .await?;
        let problem = response.failure.as_ref().map(Into::into);
        let model_feedback = response
            .failure
            .as_ref()
            .and_then(|failure| failure.model.clone());
        Ok(RunSubtaskResponse {
            task_id: response.task_id,
            session_id: response.session.id,
            parent_session_id: response.parent_session_id,
            status: match response.status {
                agena_domain::SubtaskStatus::Created => RunSubtaskStatus::Created,
                agena_domain::SubtaskStatus::Running => RunSubtaskStatus::Running,
                agena_domain::SubtaskStatus::Completed => RunSubtaskStatus::Completed,
                agena_domain::SubtaskStatus::Failed => RunSubtaskStatus::Failed,
                agena_domain::SubtaskStatus::Cancelled => RunSubtaskStatus::Cancelled,
                agena_domain::SubtaskStatus::TimedOut => RunSubtaskStatus::TimedOut,
                agena_domain::SubtaskStatus::Interrupted => RunSubtaskStatus::Interrupted,
            },
            resumed: response.resumed,
            final_text: response.final_text,
            problem,
            model_feedback,
            model_provider_id: response.model_provider_id,
            model_adapter_id: response.model_adapter_id,
            model_id: response.model_id,
            budget_exceeded: response.budget_exceeded,
            usage: RunSubtaskUsage {
                input_tokens: response.usage.input_tokens,
                output_tokens: response.usage.output_tokens,
                reasoning_tokens: response.usage.reasoning_tokens,
                cache_write_tokens: response.usage.cache_write_tokens,
                cache_read_tokens: response.usage.cache_read_tokens,
                total_cost: response.usage.total_cost,
            },
        })
    }

    async fn cancel_subtask(
        &self,
        req: CancelSubtaskRequest,
    ) -> Result<SubtaskControlResponse, PluginError> {
        let parent_session_id = req
            .parent_session_id
            .or_else(|| current_host_callback_context().and_then(|context| context.session_id))
            .ok_or_else(|| PluginError::invalid_params("parent session is required"))?;
        let task_id = req.task_id.trim().to_string();
        let session_id = self
            .session_manager()?
            .cancel_subtask(parent_session_id, task_id.as_str())
            .await
            .map_err(plugin_error)?;
        Ok(SubtaskControlResponse {
            task_id,
            session_id,
            accepted: true,
        })
    }

    async fn message_subtask(
        &self,
        req: MessageSubtaskRequest,
    ) -> Result<SubtaskControlResponse, PluginError> {
        let parent_session_id = req
            .parent_session_id
            .or_else(|| current_host_callback_context().and_then(|context| context.session_id))
            .ok_or_else(|| PluginError::invalid_params("parent session is required"))?;
        let task_id = req.task_id.trim().to_string();
        let session_id = self
            .session_manager()?
            .message_subtask(parent_session_id, task_id.as_str(), req.message)
            .await
            .map_err(plugin_error)?;
        Ok(SubtaskControlResponse {
            task_id,
            session_id,
            accepted: true,
        })
    }

    async fn read_subtask_output(
        &self,
        req: ReadSubtaskOutputRequest,
    ) -> Result<ReadSubtaskOutputResponse, PluginError> {
        let parent_session_id = req
            .parent_session_id
            .or_else(|| current_host_callback_context().and_then(|context| context.session_id))
            .ok_or_else(|| PluginError::invalid_params("parent session is required"))?;
        let task_id = req.task_id.trim().to_string();
        let output = self
            .session_manager()?
            .read_subtask_output(
                parent_session_id,
                task_id.as_str(),
                req.after_cursor,
                req.limit,
            )
            .await
            .map_err(plugin_error)?;
        Ok(ReadSubtaskOutputResponse {
            task_id,
            session_id: output.session_id,
            chunks: output
                .chunks
                .into_iter()
                .map(|chunk| SubtaskOutputChunk {
                    cursor: chunk.cursor,
                    role: chunk.role.to_string(),
                    text: chunk.text,
                    created_at_ms: chunk.created_at_ms,
                })
                .collect(),
            next_cursor: output.next_cursor,
            has_more: output.has_more,
        })
    }

    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, PluginError> {
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let tools = executor.detailed_execution_tools();
        let names = crate::tool::execution_tool_names(&tools);
        Ok(tools
            .into_iter()
            .zip(names)
            .map(|(tool, name)| {
                let mut descriptor = render_tool_descriptor(tool.into_registered());
                descriptor.name = name;
                descriptor
            })
            .collect())
    }

    async fn get_context_status(
        &self,
        req: HostContextStatusRequest,
    ) -> Result<HostContextStatusResponse, PluginError> {
        let session_id = req
            .session_id
            .or_else(|| current_host_callback_context().and_then(|context| context.session_id))
            .ok_or_else(|| PluginError::invalid_params("session id is required"))?;
        let manager = self.session_manager()?;
        let session = manager
            .get_session(session_id)
            .await
            .map_err(plugin_error)?;
        let usage = manager.session_usage(&session).map_err(plugin_error)?;
        let prompt_window = &session.runtime().prompt_window;
        let compaction = prompt_window.compaction.as_ref();
        Ok(HostContextStatusResponse {
            session_id,
            current_tokens: usage.current_tokens,
            measured_prompt_tokens: usage.measured_prompt_tokens,
            projected_tokens: usage.projected_tokens,
            limit_tokens: usage.limit_tokens,
            remaining_tokens: usage
                .limit_tokens
                .map(|limit| limit.saturating_sub(usage.current_tokens)),
            reserved_tokens: u64::from(usage.reserved_tokens.unwrap_or_default()),
            model_context_window_tokens: usage.model_context_window_tokens,
            model_max_input_tokens: usage.model_max_input_tokens,
            model_max_output_tokens: usage.model_max_output_tokens,
            model_provider_id: session.runtime().execution.selection.provider.clone(),
            model_adapter_id: session.runtime().execution.selection.adapter.clone(),
            model_id: session.runtime().execution.selection.model.clone(),
            thinking_mode: session.runtime().execution.selection.thinking_mode.clone(),
            speed_mode: session.runtime().execution.selection.speed_mode.clone(),
            verbosity: session.runtime().execution.selection.verbosity.clone(),
            prompt_window_generation: prompt_window.generation,
            compacted: compaction.is_some(),
            last_compaction_before_tokens: compaction.map(|value| value.before_tokens),
            last_compaction_after_tokens: compaction.map(|value| value.after_tokens),
            auto_compaction_disabled: prompt_window.auto_compaction_disabled,
            consecutive_compaction_failures: prompt_window.consecutive_compaction_failures,
        })
    }

    async fn get_session(
        &self,
        req: HostGetSessionRequest,
    ) -> Result<HostGetSessionResponse, PluginError> {
        let session_id = self.callback_or_requested_session_id(req.session_id, "get_session")?;
        let session = self
            .use_session_manager(|manager| async move { manager.get_session(session_id).await })
            .await?;
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
        let title = title.to_string();
        let session = self
            .use_session_manager(|manager| async move {
                manager.rename_session(session_id, title).await
            })
            .await?;
        Ok(HostRenameSessionResponse {
            session: host_session_from_session(&session),
        })
    }

    async fn set_session_model(
        &self,
        req: HostSetSessionModelRequest,
    ) -> Result<HostSetSessionModelResponse, PluginError> {
        use std::str::FromStr;

        let session_id =
            self.callback_or_requested_session_id(req.session_id, "set_session_model")?;
        let model = agena_domain::ModelRef::from_str(req.model.trim()).map_err(|error| {
            PluginError::invalid_params(format!(
                "skill model must be a fully-qualified provider/model reference: {error}"
            ))
        })?;
        let provider_id = model.provider_id.to_string();
        let adapter_id = model.adapter_id.as_ref().map(ToString::to_string);
        let model_id = model.model_id.to_string();
        let session = self
            .use_session_manager(|manager| async move {
                manager.set_session_model_override(session_id, model).await
            })
            .await?;
        Ok(HostSetSessionModelResponse {
            session: host_session_from_session(&session),
            provider_id,
            adapter_id,
            model_id,
        })
    }

    async fn image_execute(
        &self,
        req: HostImageExecuteRequest,
    ) -> Result<HostImageExecuteResponse, PluginError> {
        let (callback_session_id, call_id) = self.callback_session_and_call()?;
        if req
            .session_id
            .is_some_and(|session_id| session_id != callback_session_id)
        {
            return Err(PluginError::invalid_params(
                "direct image requests cannot target a session other than the active tool callback session",
            ));
        }
        let manager = self.session_manager()?;
        let session = manager
            .get_session(callback_session_id)
            .await
            .map_err(plugin_error)?;
        let snapshot = self.snapshot();
        let model = match session
            .runtime()
            .effective_model_ref()
            .map_err(plugin_error)?
        {
            Some(model) => model,
            None => snapshot
                .resolve_default_model()
                .map_err(plugin_error)?
                .ok_or_else(|| {
                    host_unavailable(
                        "the active session has no selected model and no default provider route",
                    )
                })?,
        };
        let provider_registry = snapshot.provider_registry();
        let capabilities = provider_registry
            .image_capabilities(&model)
            .map_err(plugin_error)?
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "selected provider/model route `{model}` does not enable a direct image generation API"
                ))
            })?;
        let operation = match req.operation {
            HostImageOperation::Generate => agena_provider::ProviderImageOperation::Generate,
            HostImageOperation::Edit => agena_provider::ProviderImageOperation::Edit,
        };
        if !capabilities.supports(operation) {
            return Err(PluginError::internal(format!(
                "selected provider/model route `{model}` does not support the requested image operation"
            )));
        }
        let prompt = req.prompt.trim();
        if prompt.is_empty() {
            return Err(PluginError::invalid_params(
                "image prompt must not be empty",
            ));
        }
        match req.operation {
            HostImageOperation::Generate if !req.inputs.is_empty() => {
                return Err(PluginError::invalid_params(
                    "image generate does not accept edit inputs",
                ));
            }
            HostImageOperation::Edit if req.inputs.is_empty() => {
                return Err(PluginError::invalid_params(
                    "image edit requires at least one input image",
                ));
            }
            _ => {}
        }
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let inputs =
            prepare_provider_image_inputs(&executor, req.inputs.as_slice(), &capabilities).await?;
        let provider_request = agena_provider::ProviderImageRequest {
            operation,
            prompt: prompt.to_owned(),
            inputs,
            options: agena_provider::ProviderHostedImageGenerationConfig {
                background: req.background,
                size: req.size,
                quality: req.quality,
                moderation: req.moderation,
                // Provider-specific escape hatches stay route-config-owned;
                // the model-visible tool cannot inject arbitrary request keys.
                provider_options: None,
            },
        };
        let response = provider_registry
            .execute_image(&model, provider_request)
            .await
            .map_err(plugin_error)?;
        let workspace_root = self
            .callback_context()?
            .workspace_root
            .map(std::path::PathBuf::from)
            .ok_or_else(|| {
                host_unavailable("host callback context is missing workspace_root for image output")
            })?;
        let attachments = persist_provider_image_artifacts(
            workspace_root.as_path(),
            callback_session_id,
            call_id,
            response.artifacts.as_slice(),
        )
        .await?;
        Ok(HostImageExecuteResponse {
            operation: req.operation,
            provider_id: model.provider_id.to_string(),
            adapter_id: model.adapter_id.as_ref().map(ToString::to_string),
            model_id: model.model_id.to_string(),
            revised_prompt: response.revised_prompt,
            attachments,
        })
    }

    async fn enter_snapshot(
        &self,
        req: HostEnterSnapshotRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let (session_id, _) = self.callback_session_and_call()?;
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let execution = executor
            .enter_snapshot_internal(
                &EnterSnapshotToolInput {
                    name: req.name,
                    path: req.path,
                },
                session_id,
            )
            .map_err(crate::tool::ToolError::into_plugin_error)?;
        Ok(crate::tool::router::tool_execution_to_invoke_output(
            execution,
        ))
    }

    async fn exit_snapshot(
        &self,
        req: HostExitSnapshotRequest,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let (session_id, _) = self.callback_session_and_call()?;
        let (executor, _) = self.callback_scoped_tool_executor().await?;
        let execution = executor
            .exit_snapshot_internal(
                &ExitSnapshotToolInput {
                    action: req.action,
                    discard_changes: req.discard_changes,
                },
                session_id,
            )
            .map_err(crate::tool::ToolError::into_plugin_error)?;
        Ok(crate::tool::router::tool_execution_to_invoke_output(
            execution,
        ))
    }

    async fn monitor_start(&self, req: MonitorStartRequest) -> Result<MonitorHandle, PluginError> {
        let (executor, registry) = self.executor_feature(
            |executor| executor.monitor_registry().cloned(),
            "background process registry is not enabled in this runtime",
        )?;
        let cwd = req
            .cwd
            .as_deref()
            .map(|path| executor.resolve_target_path(path))
            .unwrap_or_else(|| executor.workspace_root().to_path_buf());
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
                monitored: true,
                include_pattern: req.include_pattern,
                success_pattern: None,
                failure_pattern: None,
                quiet_period_ms: None,
                max_buffered_lines: req.max_buffered_lines,
                capture_stderr: req.capture_stderr,
                env,
            })
            .map_err(map_monitor_error)?
            .summary;
        Ok(render_monitor_handle(summary))
    }

    async fn monitor_list(&self) -> Result<Vec<MonitorHandle>, PluginError> {
        let (_, registry) = self.executor_feature(
            |executor| executor.monitor_registry().cloned(),
            "background process registry is not enabled in this runtime",
        )?;
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
        let (_, registry) = self.executor_feature(
            |executor| executor.monitor_registry().cloned(),
            "background process registry is not enabled in this runtime",
        )?;
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
        let (_, registry) = self.executor_feature(
            |executor| executor.monitor_registry().cloned(),
            "background process registry is not enabled in this runtime",
        )?;
        let stop = registry.stop(req.id.as_str()).map_err(map_monitor_error)?;
        Ok(render_monitor_handle(stop.summary))
    }

    async fn storage_get(
        &self,
        req: HostStorageGetRequest,
    ) -> Result<HostStorageGetResponse, PluginError> {
        let value = self.use_plugin_storage(req.scope, req.visibility, |store, locator| {
            store.get(locator, req.namespace.as_str(), req.key.as_str())
        })?;
        Ok(HostStorageGetResponse { value })
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> Result<(), PluginError> {
        self.use_plugin_storage(req.scope, req.visibility, |store, locator| {
            store.set(
                locator,
                req.namespace.as_str(),
                req.key.as_str(),
                req.value.as_str(),
            )
        })
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> Result<(), PluginError> {
        self.use_plugin_storage(req.scope, req.visibility, |store, locator| {
            store.delete(locator, req.namespace.as_str(), req.key.as_str())
        })
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> Result<HostStorageListResponse, PluginError> {
        let records = self
            .use_plugin_storage(req.scope, req.visibility, |store, locator| {
                store.list(locator, req.namespace.as_deref(), req.prefix.as_deref())
            })?
            .into_iter()
            .map(|entry| HostStorageRecord {
                namespace: entry.namespace,
                key: entry.key,
            })
            .collect();
        Ok(HostStorageListResponse { records })
    }

    async fn secret_get(
        &self,
        req: HostSecretGetRequest,
    ) -> Result<HostSecretGetResponse, PluginError> {
        let value = self
            .use_plugin_secret_store(|store, plugin_id| store.get(plugin_id, req.name.as_str()))?;
        Ok(HostSecretGetResponse { value })
    }

    async fn secret_set(&self, req: HostSecretSetRequest) -> Result<(), PluginError> {
        self.use_plugin_secret_store(|store, plugin_id| {
            store.set(plugin_id, req.name.as_str(), req.value.as_str())
        })
    }

    async fn secret_delete(&self, req: HostSecretDeleteRequest) -> Result<(), PluginError> {
        self.use_plugin_secret_store(|store, plugin_id| store.delete(plugin_id, req.name.as_str()))
    }

    async fn secret_list(&self) -> Result<HostSecretListResponse, PluginError> {
        let names = self.use_plugin_secret_store(|store, plugin_id| store.list(plugin_id))?;
        Ok(HostSecretListResponse { names })
    }

    async fn plugin_status_list(&self) -> Result<HostPluginStatusListResponse, PluginError> {
        let host = self.plugin_manager();
        let statuses = host
            .plugin_statuses()
            .into_iter()
            .map(host_status_to_sdk)
            .collect();
        Ok(HostPluginStatusListResponse { statuses })
    }

    async fn plugin_status_get(
        &self,
        req: HostPluginStatusGetRequest,
    ) -> Result<HostPluginStatusGetResponse, PluginError> {
        let host = self.plugin_manager();
        Ok(HostPluginStatusGetResponse {
            status: host
                .plugin_status_by_key(&req.plugin_id)
                .map(host_status_to_sdk),
        })
    }

    async fn lsp_list_servers(&self) -> Result<HostLspListServersResponse, PluginError> {
        let (_, registry) = self.executor_feature(
            |executor| executor.lsp_registry().cloned(),
            "lsp registry is not enabled in this runtime",
        )?;
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
        let (_, registry) = self.executor_feature(
            |executor| executor.lsp_registry().cloned(),
            "lsp registry is not enabled in this runtime",
        )?;
        let pairs = registry.collect_diagnostics().await;
        let mut diagnostics_out = Vec::new();
        for (uri, diagnostics) in pairs {
            if let Some(filter) = req.uri.as_ref()
                && filter != &uri
            {
                continue;
            }
            for diagnostic in diagnostics {
                diagnostics_out.push(HostLspDiagnostic {
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
        Ok(HostLspListDiagnosticsResponse {
            diagnostics: diagnostics_out,
        })
    }

    async fn snapshot_list(&self) -> Result<HostSnapshotListResponse, PluginError> {
        let (_, registry) = self.executor_feature(
            |executor| executor.snapshot_registry().cloned(),
            "snapshot registry is not enabled in this runtime",
        )?;
        let snapshots: Vec<HostSnapshotSummary> = agena_runtime::list_active_snapshots(&registry)
            .into_iter()
            .map(|w| HostSnapshotSummary {
                session_id: w.session_id,
                path: w.path.display().to_string(),
                branch: w.branch,
                created_here: w.created_here,
            })
            .collect();
        Ok(HostSnapshotListResponse { snapshots })
    }

    async fn scheduler_list(&self) -> Result<HostSchedulerListResponse, PluginError> {
        let (_, scheduler) = self.executor_feature(
            |executor| executor.scheduler().cloned(),
            "scheduler is not enabled in this runtime",
        )?;
        let jobs = scheduler.list().await;
        let entries = jobs.into_iter().map(scheduler_job_to_sdk).collect();
        Ok(HostSchedulerListResponse { jobs: entries })
    }

    async fn scheduler_create(
        &self,
        req: HostSchedulerCreateRequest,
    ) -> Result<HostSchedulerCreateResponse, PluginError> {
        let (_, scheduler) = self.executor_feature(
            |executor| executor.scheduler().cloned(),
            "scheduler is not enabled in this runtime",
        )?;
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
                    job.set_owner(session);
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
                    job.set_owner(session);
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
        let (_, scheduler) = self.executor_feature(
            |executor| executor.scheduler().cloned(),
            "scheduler is not enabled in this runtime",
        )?;
        let id = uuid::Uuid::parse_str(&req.id)
            .map_err(|err| PluginError::invalid_params(format!("invalid scheduler id: {err}")))?;
        let removed = scheduler.remove(id).await;
        Ok(HostSchedulerDeleteResponse { removed })
    }

    async fn mcp_list_servers(&self) -> Result<HostMcpListServersResponse, PluginError> {
        let manager = self.snapshot_feature(
            |snapshot| snapshot.mcp_manager(),
            "mcp manager is not enabled in this runtime",
        )?;
        let servers = manager.server_names().await;
        Ok(HostMcpListServersResponse { servers })
    }

    async fn mcp_add_server(&self, req: HostMcpAddServerRequest) -> Result<(), PluginError> {
        let manager = self.snapshot_feature(
            |snapshot| snapshot.mcp_manager(),
            "mcp manager is not enabled in this runtime",
        )?;
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
                tool_policy: Default::default(),
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
                    headers: headers.into_iter().collect(),
                    auth,
                    tool_policy: Default::default(),
                }
            }
        };
        manager
            .add_server(&req.name, spec)
            .await
            .map_err(|e| PluginError::internal(format!("mcp.add_server: {e}")))
    }

    async fn mcp_remove_server(
        &self,
        req: HostMcpRemoveServerRequest,
    ) -> Result<HostMcpRemoveServerResponse, PluginError> {
        let manager = self.snapshot_feature(
            |snapshot| snapshot.mcp_manager(),
            "mcp manager is not enabled in this runtime",
        )?;
        match manager.remove_server(&req.name).await {
            Ok(()) => Ok(HostMcpRemoveServerResponse { removed: true }),
            Err(_) => Ok(HostMcpRemoveServerResponse { removed: false }),
        }
    }
}
