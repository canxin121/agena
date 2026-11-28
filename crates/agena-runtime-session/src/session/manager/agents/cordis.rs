//! Direct Cordis activation on the same permission and settlement paths as native agents.

use std::path::{Path, PathBuf};

use agena_domain::{AccessKind, FinishReason};
use agena_plugin_host::sdk::{
    PluginError, PluginKey, Result as PluginResult,
    host_api::{
        CordisSubagentDelivery, CordisSubagentResult, CordisSubagentRun, CordisSubagentStart,
        CordisSubagentStarted, CordisSubagentStopReason, CordisSubagentWait,
    },
};
use tokio::sync::watch;

use super::*;
use crate::session::manager::{cordis_workflows, working_directories};

pub(super) struct Launch {
    pub label: String,
    pub cwd: Option<PathBuf>,
    pub max_output_tokens: Option<u32>,
    pub max_active_children: u64,
    pub max_depth: Option<u64>,
    pub attachments: Vec<agena_domain::AttachmentItem>,
    pub identity: CordisAgentConfiguration,
    pub cancellation: CancellationToken,
}

/// A local activation can own tree work before it has a session row and after
/// its final execution phase is saved. Retain that work through rollback or
/// publication of the result to the initiating module.
pub(super) struct TreeWork {
    active: Arc<parking_lot::Mutex<HashMap<SessionId, usize>>>,
    root: SessionId,
}

impl Drop for TreeWork {
    fn drop(&mut self) {
        let mut active = self.active.lock();
        let count = active.get_mut(&self.root).expect("tree work is registered");
        *count -= 1;
        if *count == 0 {
            active.remove(&self.root);
        }
    }
}

#[derive(Clone)]
pub(in crate::session::manager) struct CordisExecution {
    pub max_output_tokens: Option<u32>,
    finish: Arc<StdMutex<Option<FinishReason>>>,
}

impl CordisExecution {
    pub fn new(max_output_tokens: Option<u32>) -> Self {
        Self {
            max_output_tokens,
            finish: Default::default(),
        }
    }

    pub fn record_finish(&self, finish: FinishReason) {
        *self
            .finish
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(finish);
    }
}

type ResultSender = watch::Sender<Option<PluginResult<CordisSubagentResult>>>;

pub(super) struct Run {
    manager: SessionManager,
    child: SessionId,
    execution: ExecutionId,
    generation: AttemptGeneration,
    result: watch::Receiver<Option<PluginResult<CordisSubagentResult>>>,
}

impl Run {
    pub(super) fn new(
        manager: SessionManager,
        child: SessionId,
        execution: ExecutionId,
        generation: AttemptGeneration,
    ) -> (Self, ResultSender) {
        let (send, result) = watch::channel(None);
        (
            Self {
                manager,
                child,
                execution,
                generation,
                result,
            },
            send,
        )
    }
}

#[async_trait::async_trait]
impl CordisSubagentRun for Run {
    async fn result(&self) -> PluginResult<CordisSubagentResult> {
        let mut result = self.result.clone();
        result
            .wait_for(Option::is_some)
            .await
            .map_err(|_| PluginError::internal("Cordis child settlement stopped"))?
            .clone()
            .expect("settlement supplies a result")
    }

    async fn dispose(&self) -> PluginResult<()> {
        if self.result.borrow().is_none() {
            self.manager
                .cancel_execution_attempt_with_outcome(
                    self.child.get(),
                    self.execution,
                    self.generation,
                )
                .await
                .map_err(|error| result_error(&error))?;
        }
        let mut result = self.result.clone();
        result
            .wait_for(Option::is_some)
            .await
            .map_err(|_| PluginError::internal("Cordis child settlement stopped"))?;
        Ok(())
    }
}

pub(super) fn result_error(error: &AppError) -> PluginError {
    PluginError::internal_error(error)
}

impl SessionManager {
    pub(super) fn register_cordis_tree_work(&self, root: SessionId) -> TreeWork {
        *self.cordis_tree_work.lock().entry(root).or_default() += 1;
        TreeWork {
            active: self.cordis_tree_work.clone(),
            root,
        }
    }

    pub async fn start_cordis_subagent(
        &self,
        session: i64,
        call: Option<i64>,
        plugin: PluginKey,
        mut start: CordisSubagentStart,
        cancel: CancellationToken,
    ) -> Result<CordisSubagentStarted, AppError> {
        let cancellation = cancel.child_token();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        if start.label.trim().is_empty() || start.label.len() > 1024 {
            return Err(AppError::Config(
                "Cordis child labels must contain 1–1024 bytes".into(),
            ));
        }
        if start.max_active_children == 0
            || start.max_output_tokens == Some(0)
            || (!start.request.name.is_empty()
                && !SessionAgentConfiguration::valid_cordis_name(&start.request.name))
            || start
                .reserved_id
                .as_deref()
                .is_some_and(|id| !SessionAgentConfiguration::valid_cordis_reserved_id(id))
        {
            return Err(AppError::Config(
                "Invalid Cordis child name, childId, concurrency limit or model output budget"
                    .into(),
            ));
        }
        let auto_name = start.request.name.is_empty();
        if auto_name {
            // A name is required for native tool validation. Its final
            // generated value is reserved under the child creation lock.
            start.request.name = "agent".into();
        }
        let empty_prompt_with_media =
            start.request.prompt.trim().is_empty() && !start.attachments.is_empty();
        if empty_prompt_with_media {
            start.request.prompt = "[attachment]".into();
        }
        let (request, cwd) = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(AppError::Cancelled),
            result = self.authorize_cordis_subagent(session, call, &plugin, &start, &cancellation) => result?,
        };
        start.request = request;
        if auto_name && start.request.name == "agent" {
            start.request.name.clear();
        }
        if empty_prompt_with_media && start.request.prompt == "[attachment]" {
            start.request.prompt.clear();
        }
        let mut launch = AgentLaunch::new(session, call, start.request);
        launch.cordis = Some(Launch {
            label: start.label.clone(),
            cwd,
            max_output_tokens: start.max_output_tokens,
            max_active_children: start.max_active_children,
            max_depth: start.max_depth,
            attachments: start.attachments,
            identity: CordisAgentConfiguration {
                provider: start.provider,
                label: Some(start.label),
                result_to_caller: matches!(start.delivery, CordisSubagentDelivery::Caller),
                max_active_children: Some(start.max_active_children),
                reserved_id: start.reserved_id,
            },
            cancellation,
        });
        self.launch_agent(launch)
            .await?
            .cordis
            .ok_or_else(|| AppError::Internal("Cordis child activation was not recorded".into()))
    }

    async fn authorize_cordis_subagent(
        &self,
        session: i64,
        call: Option<i64>,
        plugin: &PluginKey,
        start: &CordisSubagentStart,
        cancel: &CancellationToken,
    ) -> Result<(AgentSpawnRequest, Option<PathBuf>), AppError> {
        let (executor, context) = self.host_callback_executor(Some(session), call).await?;
        let Some(context) = context.filter(|context| {
            !context.global
                && context.owner.is_none()
                && context.channel_authority.is_none()
                && executor.plugin_is_enabled(plugin)
        }) else {
            return Err(AppError::Config(
                "Cordis delegation requires an enabled plugin in a project session".into(),
            ));
        };
        let parent = session_id(session)?;
        let lineage = self.cordis_lineage(parent).await?;
        lineage.admit_child(start.max_depth)?;
        let mut executor = executor.with_cancellation_token(Some(cancel.clone()));
        if context.relation_kind.is_subagent() {
            let config = read_with_busy_retry(parent, "cordis_nested_parent", || {
                self.kernel().reader.configuration(parent)
            })
            .await?;
            if !config
                .agent
                .as_ref()
                .is_some_and(|agent| agent.cordis.is_some())
            {
                return Err(AppError::Config(
                    "Nested Cordis delegation requires a Cordis child".into(),
                ));
            }
            executor = executor.for_cordis_agent_permission("spawn");
        }
        let invocation = cordis_workflows::agent_invocation(&start.request)?;
        let prepared = executor
            .prepare_invocation(&invocation, session, call.unwrap_or(-1))
            .await
            .map_err(tool_error)?;
        let target = start
            .cwd
            .as_deref()
            .map(|cwd| executor.workspace_root().join(Path::new(cwd)))
            .or(context.working_directory);
        let (cwd, path_check) = if let Some(target) = target {
            let root =
                working_directories::canonical_directory(executor.permission_workspace_root())
                    .await?;
            let path = working_directories::canonical_directory(&target).await?;
            let relative = path.strip_prefix(&root).map_err(|_| {
                AppError::Config(
                    "Cordis child cwd must be inside the parent's project workspace".into(),
                )
            })?;
            let cwd = (!relative.as_os_str().is_empty()).then(|| relative.to_path_buf());
            let check = executor.path_permission_check(AccessKind::Read, &path.to_string_lossy());
            (cwd, Some(check))
        } else {
            (None, None)
        };
        self.authorize_cordis_agent_tool(session, call, &executor, &prepared, path_check)
            .await?;
        let request = serde_json::from_value(serde_json::Value::from(prepared.invocation.input))?;
        Ok((request, cwd))
    }

    pub async fn wait_for_cordis_subagent(
        &self,
        session: i64,
        call: Option<i64>,
        wait: CordisSubagentWait,
    ) -> Result<(), AppError> {
        let control = call
            .map(|call| self.live_tool(session, call).map(|(_, control)| control))
            .transpose()?;
        self.with_agent_wait_permit(session, call, control.as_deref(), async move {
            wait.await;
            Ok(())
        })
        .await
    }

    pub(super) async fn next_cordis_agent_name(
        &self,
        parent: SessionId,
    ) -> Result<String, AppError> {
        for ordinal in 1..=u32::MAX {
            let name = format!("agent-{ordinal}");
            if read_with_busy_retry(parent, "cordis_child_name", || {
                self.kernel().reader.find_agent(parent, &name)
            })
            .await?
            .is_none()
            {
                return Ok(name);
            }
        }
        Err(AppError::Config(
            "No Cordis child names are available".into(),
        ))
    }

    pub(super) async fn capture_cordis_subagent(
        &self,
        control: &RunControl,
        status: AgentStatus,
    ) -> PluginResult<CordisSubagentResult> {
        let finish = control
            .budget
            .as_ref()
            .and_then(|budget| budget.cordis.as_ref())
            .and_then(|execution| {
                *execution
                    .finish
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
            });
        let stop_reason = match (status, finish) {
            (AgentStatus::Cancelled | AgentStatus::Interrupted, _) => {
                CordisSubagentStopReason::Aborted
            }
            (AgentStatus::Completed, Some(FinishReason::MaxTokens)) => {
                CordisSubagentStopReason::MaxTokens
            }
            (AgentStatus::Completed, Some(FinishReason::ContentFilter)) => {
                CordisSubagentStopReason::Refusal
            }
            (AgentStatus::Completed, _) => CordisSubagentStopReason::Completed,
            _ => CordisSubagentStopReason::Error,
        };
        let text = self
            .final_execution_text(
                control.session,
                control.execution,
                Some(control.generation),
                64 * 1024,
            )
            .await
            .map_err(|error| result_error(&error))?;
        Ok(CordisSubagentResult { stop_reason, text })
    }
}
