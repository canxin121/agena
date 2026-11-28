//! Native process policies use the same live authority and approval paths as
//! filesystem tools. Resource declarations never carry an authorized policy.
use super::*;
use agena_plugin_host::sdk::host_api::{
    HostPreparedProcessSandbox, HostProcessExecution, HostProcessResources,
};

impl SessionManager {
    pub async fn prepare_host_process_sandbox(
        &self,
        session: i64,
        call: i64,
        resources: HostProcessResources,
    ) -> Result<HostPreparedProcessSandbox, AppError> {
        self.prepare_host_sandbox(session, call, resources, false)
            .await
    }

    pub async fn prepare_host_worker_sandbox(
        &self,
        session: i64,
        call: i64,
        resources: HostProcessResources,
    ) -> Result<HostPreparedProcessSandbox, AppError> {
        self.prepare_host_sandbox(session, call, resources, true)
            .await
    }

    async fn prepare_host_sandbox(
        &self,
        session: i64,
        call: i64,
        resources: HostProcessResources,
        worker: bool,
    ) -> Result<HostPreparedProcessSandbox, AppError> {
        if session == OPERATOR_SESSION {
            let authority = self.operator_invocation(call).map_err(tool_error)?;
            require_process_sandbox(&authority.executor, &resources)?;
            let resources = if worker {
                authority
                    .executor
                    .resolve_worker_sandbox_resources(resources)
            } else {
                authority
                    .executor
                    .resolve_process_sandbox_resources(resources)
            }
            .map_err(tool_error)?;
            let checks = authority
                .executor
                .process_sandbox_permission_checks_async(&resources)
                .await
                .map_err(tool_error)?;
            self.authorize_operator_checks(&authority, checks)
                .await
                .map_err(tool_error)?;
            let records = self
                .kernel_runtime
                .permissions
                .resolve_snapshot(None, Some(authority.workspace))
                .await
                .map_err(|error| AppError::Internal(error.to_string()))?;
            let executor = authority
                .executor
                .clone()
                .with_sandbox_persisted_rules(records)
                .map_err(tool_error)?;
            let cwd = resources
                .cwd
                .clone()
                .ok_or_else(|| AppError::Config("process cwd is missing".into()))?;
            let (policy, _) = if worker {
                executor.prepare_worker_sandbox_policy(resources).await
            } else {
                executor.prepare_process_sandbox_policy(resources).await
            }
            .map_err(tool_error)?;
            let permission_snapshot = Some(
                serde_json::to_value(executor.sandbox_permission_snapshot())
                    .map_err(|error| AppError::Internal(error.to_string()))?,
            );
            if authority.cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            return Ok(HostPreparedProcessSandbox {
                scope: format!("operator:{}:call:{call}", authority.workspace),
                workspace_root: executor.workspace_root().to_string_lossy().into_owned(),
                cwd,
                policy,
                additional_working_directories: Vec::new(),
                permission_snapshot,
                call_bound: true,
                execution: None,
            });
        }

        let live = if call >= 0 {
            Some(self.live_tool(session, call)?)
        } else {
            None
        };
        let key = session_id(session)?;
        let state = match live.as_ref() {
            Some((_, control)) => control.state.clone(),
            None => self.state_for_session(key).await?,
        };
        let admission = HOST_POLICY_READS
            .acquire()
            .await
            .expect("host policy admission is never closed");
        let mut context = self.execution_context_value(key).await?;
        if !worker && call < 0 && agena_plugin_host::host::is_worktree_removal(session) {
            let configuration = self.kernel().reader.configuration(key).await.map_err(kernel_error)?;
            if let Some(worktree) = &configuration.worktree {
                context.effective_workspace_root = Some(worktree.source.clone());
                context.working_directory = None;
            }
        }
        let cleanup = !worker
            && call == agena_plugin_host::sdk::host_api::SESSION_SCOPED_CALL_ID
            && agena_plugin_host::host::is_shutdown_session_end(session);
        let cancel = live
            .as_ref()
            .map(|(_, control)| control.cancel.clone())
            .unwrap_or_else(|| {
                if cleanup {
                    agena_plugin_host::sdk::current_cancellation_token().unwrap_or_default()
                } else {
                    self.shutdown.child_token()
                }
            });
        let executor = state
            .executor
            .for_session_resource_context(&context)
            .with_cancellation_token(Some(cancel.clone()));
        drop(admission);
        require_process_sandbox(&executor, &resources)?;
        let resources = if worker {
            executor.resolve_worker_sandbox_resources(resources)
        } else {
            executor.resolve_process_sandbox_resources(resources)
        }
        .map_err(tool_error)?;
        let checks = executor
            .process_sandbox_permission_checks_async(&resources)
            .await
            .map_err(tool_error)?;
        if !checks.is_empty() {
            if cleanup {
                return Err(AppError::Cancelled);
            }
            if let Some((tool, control)) = live.as_ref() {
                self.authorize_checks(tool, checks, &executor, control, &state)
                    .await
                    .map_err(tool_error)?;
            } else {
                self.authorize_session_scoped_checks(session, &executor, checks)
                    .await?;
            }
        }
        let executor = {
            let _admission = HOST_POLICY_READS
                .acquire()
                .await
                .expect("host policy admission is never closed");
            self.sandbox_executor_for_session(&executor, session)
                .await
                .map_err(tool_error)?
        };
        let cwd = resources
            .cwd
            .clone()
            .ok_or_else(|| AppError::Config("process cwd is missing".into()))?;
        let (policy, call_bound) = if worker {
            executor.prepare_worker_sandbox_policy(resources).await
        } else {
            executor.prepare_process_sandbox_policy(resources).await
        }
        .map_err(tool_error)?;
        let snapshot = executor.sandbox_permission_snapshot();
        let additional_working_directories = snapshot.authorized_additional_working_directories();
        let permission_snapshot = Some(
            serde_json::to_value(snapshot)
                .map_err(|error| AppError::Internal(error.to_string()))?,
        );
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        Ok(HostPreparedProcessSandbox {
            scope: if call_bound {
                format!("session:{session}:call:{call}")
            } else {
                format!("session:{session}")
            },
            workspace_root: executor.workspace_root().to_string_lossy().into_owned(),
            cwd,
            policy,
            additional_working_directories,
            permission_snapshot,
            call_bound,
            execution: live.as_ref().map(|(_, control)| HostProcessExecution {
                id: control.execution.to_string(),
                model: control.model.model_id.to_string(),
            }),
        })
    }
}

fn require_process_sandbox(
    executor: &ToolExecutor,
    resources: &HostProcessResources,
) -> Result<(), AppError> {
    if resources.require_sandbox && !executor.sandbox_enabled() {
        return Err(AppError::Config(
            "this plugin process requires Agena process sandboxing".into(),
        ));
    }
    Ok(())
}
