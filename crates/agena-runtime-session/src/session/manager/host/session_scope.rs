//! Session-scoped plugin-to-plugin tool invocation.
//!
//! A hook (`session.start`, `chat.*`, `run.pre`, …) belongs to a session but
//! has no transcript tool call. Such a callback may still delegate work to
//! another plugin's tool, so long as the delegation:
//!
//! * is authorized under the owning session's policy and workspace rules;
//! * never claims a transcript tool identity, so it cannot publish a result
//!   part, an attachment receipt, or a launch provenance it does not own;
//! * never opens a user-interaction channel (a hook has no tool slot to reply
//!   through), so `ask`/`auto` policy becomes an explicit unavailable result
//!   instead of a silent approval or a wedged callback.

use super::*;
use agena_domain::{PermissionDecision, PermissionResolutionSource};
use agena_plugin_host::host::ImportedToolDecision;
use agena_plugin_host::sdk;
use serde_json::Value;

mod skill_shell;

enum ScopedToolOrigin {
    Host,
    McpApp,
    ClaudeMod { nested: bool },
}

/// Allocate the call id for one session-scoped delegation.
///
/// A hook has no transcript tool call, but tools still need a *distinct*
/// non-colliding id: `bash` and `monitor` derive a managed process id from
/// `(session, call)`, so one shared sentinel would make two concurrent
/// delegates in the same session fight over the same process slot. Ids are
/// negative and strictly decreasing, so they can never collide with a real
/// (nonnegative) `ToolId`.
fn next_session_scoped_call() -> i64 {
    static NEXT: std::sync::atomic::AtomicI64 =
        std::sync::atomic::AtomicI64::new(agena_plugin_host::sdk::host_api::SESSION_SCOPED_CALL_ID);
    NEXT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed)
}

impl SessionManager {
    pub async fn imported_hook_profile(&self, session: i64) -> Result<String, AppError> {
        let session = session_id(session)?;
        let config = self
            .kernel()
            .reader
            .configuration(session)
            .await
            .map_err(kernel_error)?;
        let state = self.state_for_session(session).await?;
        Ok(state
            .config
            .effective_profile_name(config.profile.as_deref(), config.relation_kind)
            .to_owned())
    }

    /// MCP callbacks are part of an existing hook. Run the MCP producer under
    /// the session policy without redispatching tool hooks around the callback.
    pub async fn call_imported_plugin_mcp(
        &self,
        session: i64,
        input: Value,
    ) -> Result<ToolInvocationExecution, AppError> {
        self.invoke_imported_hook_tool(session, "agena.mcp.tools.call", input)
            .await
    }

    pub async fn read_imported_plugin_mcp(
        &self,
        session: i64,
        input: Value,
    ) -> Result<ToolInvocationExecution, AppError> {
        self.invoke_imported_hook_tool(session, "agena.mcp.resources.read", input)
            .await
    }

    /// Check the corresponding MCP tool policy without invoking it. Cordis
    /// commands have no transcript slot for an interactive approval.
    pub async fn authorize_deepseek_mcp_manager(
        &self,
        session: i64,
        plugin: &sdk::PluginKey,
        tool: &str,
    ) -> Result<ToolExecutor, AppError> {
        let session_key = session_id(session)?;
        let state = self.state_for_session(session_key).await?;
        let context = self
            .execution_context_with_state(session_key, &state)
            .await?;
        let invocation = ToolInvocation::new(tool, Default::default());
        let executor = state
            .executor
            .for_session_tool_permission(&context, &invocation);
        if context.global
            || context.owner.is_some()
            || context.channel_authority.is_some()
            || !executor.plugin_is_enabled(plugin)
        {
            return Err(AppError::Config(
                "The MCP manager requires an enabled DeepSeek plugin in a project session".into(),
            ));
        }
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(&invocation, Some(session))
            .await
            .map_err(tool_error)?;
        self.authorize_session_scoped_checks(session, &executor, checks)
            .await?;
        Ok(executor)
    }

    /// Run one tool under the hook session's policy without firing the hook
    /// again through tool.before, tool.after, or model request interception.
    pub(in crate::session::manager) async fn invoke_imported_hook_tool(
        &self,
        session: i64,
        name: &str,
        input: Value,
    ) -> Result<ToolInvocationExecution, AppError> {
        let _admission = self.reserve_execution()?;
        let session_key = session_id(session)?;
        let state = self.state_for_session(session_key).await?;
        let context = self.execution_context_value(session_key).await?;
        let cancel = sdk::current_cancellation_token()
            .as_ref()
            .unwrap_or(&self.shutdown)
            .child_token();
        let _cancel_on_drop = cancel.clone().drop_guard();
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(cancel.clone()));
        let executor = self.with_session_scoped_path_authorizer(executor, session, &cancel);
        let invocation = ToolInvocation::new(
            name,
            agena_domain::StructuredObject::try_from(input)
                .map_err(|error| AppError::Config(error.to_string()))?,
        );
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(&invocation, Some(session))
            .await
            .map_err(tool_error)?;
        self.authorize_session_scoped_checks(session, &executor, checks)
            .await?;
        let executor = self
            .sandbox_executor_for_session(&executor, session)
            .await
            .map_err(tool_error)?
            .without_tool_after_hooks();
        Box::pin(executor.execute_invocation(
            &invocation,
            crate::tool::ToolRuntimeContext {
                session_id: Some(session),
                call_id: Some(next_session_scoped_call()),
                ..Default::default()
            },
        ))
        .await
        .map_err(tool_error)
    }

    /// Execute `invocation` on behalf of a session-scoped plugin callback.
    ///
    /// The plugin host callback dispatcher has already validated the caller's
    /// callback authority. This path grants no transcript authority: the
    /// returned execution is delivered to the calling plugin only.
    pub async fn execute_session_scoped_tool(
        &self,
        session: i64,
        invocation: ToolInvocation,
    ) -> Result<ToolInvocationExecution, AppError> {
        self.execute_scoped_tool(session, invocation, ScopedToolOrigin::Host)
            .await
    }

    pub async fn execute_session_scoped_mod_tool(
        &self,
        session: i64,
        invocation: ToolInvocation,
        nested: bool,
    ) -> Result<ToolInvocationExecution, AppError> {
        self.execute_scoped_tool(session, invocation, ScopedToolOrigin::ClaudeMod { nested })
            .await
    }

    pub(super) async fn execute_session_scoped_app_tool(
        &self,
        session: i64,
        invocation: ToolInvocation,
    ) -> Result<ToolInvocationExecution, AppError> {
        self.execute_scoped_tool(session, invocation, ScopedToolOrigin::McpApp)
            .await
    }

    async fn execute_scoped_tool(
        &self,
        session: i64,
        invocation: ToolInvocation,
        origin: ScopedToolOrigin,
    ) -> Result<ToolInvocationExecution, AppError> {
        let catalog_wait = matches!(origin, ScopedToolOrigin::McpApp)
            && sdk::host_api::mcp_app_resource_catalog_wait();
        let app_task = matches!(origin, ScopedToolOrigin::McpApp)
            .then(sdk::host_api::mcp_app_server_task).flatten();
        let _admission = (!catalog_wait).then(|| self.reserve_execution()).transpose()?;
        let call = next_session_scoped_call();
        let session_key = session_id(session)?;
        let state = self.state_for_session(session_key).await?;
        let context = self.execution_context_value(session_key).await?;
        let cancel = self.shutdown.child_token();
        let _cancel_app_on_drop = matches!(origin, ScopedToolOrigin::McpApp)
            .then(|| cancel.clone().drop_guard());
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(cancel.clone()));
        let executor = self.with_session_scoped_path_authorizer(executor, session, &cancel);

        let checks = executor
            .collect_permission_checks_for_invocation_in_session(&invocation, Some(session))
            .await
            .map_err(tool_error)?;
        self.authorize_session_scoped_checks(session, &executor, checks)
            .await?;

        let prepared = executor
            .prepare_invocation(&invocation, session, call)
            .await
            .map_err(tool_error)?;
        require_live_tool_for_hook_approval(&prepared).map_err(tool_error)?;
        if matches!(origin, ScopedToolOrigin::McpApp) && prepared.invocation != invocation {
            return Err(AppError::Config("The MCP App invocation changed.".into()));
        }
        let (invocation, shell) = executor
            .prepare_shell_invocation(&prepared.invocation, session, call)
            .await
            .map_err(tool_error)?;
        // Before-hooks may rewrite the command or widen declared effects, so
        // authorize the final shape exactly as an interactive call would.
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(&invocation, Some(session))
            .await
            .map_err(tool_error)?;
        self.authorize_session_scoped_checks(session, &executor, checks)
            .await?;
        if matches!(origin, ScopedToolOrigin::ClaudeMod { .. }) {
            let verdict = executor
                .run_imported_tool_check(&invocation, session, call, PermissionDecision::Allow)
                .await;
            let refusal = match verdict {
                Some(ImportedToolDecision::Ask { reason }) => Some(
                    reason.unwrap_or_else(|| {
                        "Claude mod requested confirmation for this tool".into()
                    }) + ". This tool request has no approval interaction.",
                ),
                Some(ImportedToolDecision::Deny { reason }) => {
                    Some(reason.unwrap_or_else(|| "Claude mod refused this tool".into()))
                }
                _ => None,
            };
            if let Some(reason) = refusal {
                return Err(AppError::CapabilityUnavailable(Box::new(
                    agena_domain::CapabilityUnavailableResult {
                        capability: "tool_execution".into(),
                        tool_name: Some(invocation.name.clone()),
                        reason,
                        source: agena_domain::CapabilitySourceKind::RuntimeConfiguration,
                        retryable: false,
                    },
                )));
            }
        }
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let executor = self
            .sandbox_executor_for_session(&executor, session)
            .await
            .map_err(tool_error)?;

        let _permit = if catalog_wait {
            None
        } else if matches!(origin, ScopedToolOrigin::ClaudeMod { nested: true }) {
            // A parent with no transcript tool cannot use the live permit
            // handoff. Do not wait behind a slot that the parent may hold.
            Some(state.tools.clone().try_acquire_owned().map_err(|_| {
                AppError::CapabilityUnavailable(Box::new(
                    agena_domain::CapabilityUnavailableResult {
                        capability: "tool_execution".into(),
                        tool_name: Some(invocation.name.clone()),
                        reason: "No execution slot is available for this dependent tool call"
                            .into(),
                        source: agena_domain::CapabilitySourceKind::RuntimeConfiguration,
                        retryable: true,
                    },
                ))
            })?)
        } else {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                _ = async { app_task.as_ref().expect("App task").cancellation().cancelled().await }, if app_task.is_some() => return Err(AppError::Cancelled),
                permit = state.tools.clone().acquire_owned() => Some(permit.map_err(|_| AppError::Cancelled)?),
            }
        };
        let context = crate::tool::ToolRuntimeContext {
            session_id: Some(session),
            // A unique negative id: no transcript tool call exists,
            // but managed child processes still need a stable owner.
            call_id: Some(call),
            prepared_shell_command: shell,
            launch_provenance: None,
            output: None,
            tool: None,
        };
        let mut execution = match origin {
            ScopedToolOrigin::Host => executor.execute_invocation(&invocation, context).await,
            ScopedToolOrigin::McpApp => {
                executor.without_tool_after_hooks().execute_invocation(&invocation, context).await
            }
            ScopedToolOrigin::ClaudeMod { .. } => {
                executor.run_authorized_tool(&invocation, context).await
            }
        }
        .map_err(tool_error)?;
        if let Some(title) = prepared.title_override {
            execution.view.set_title(title);
        }
        execution.view.metadata.extend(prepared.metadata);
        execution
            .view
            .additional_context
            .splice(..0, prepared.additional_context);
        Ok(execution)
    }

    fn with_session_scoped_path_authorizer(
        &self,
        executor: ToolExecutor,
        session: i64,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> ToolExecutor {
        // Recheck concrete paths as tools open supporting files or descend a
        // directory. A session callback has no interactive approval slot.
        let authorizer = executor.clone();
        let manager = self.clone();
        let runtime = tokio::runtime::Handle::current();
        let cancel = cancel.clone();
        executor.with_path_effect_authorizer(Arc::new(move |access, path| {
            if cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let check = authorizer.path_permission_check(access, &path.to_string_lossy());
            runtime
                .block_on(manager.authorize_session_scoped_checks(
                    session,
                    &authorizer,
                    vec![check],
                ))
                .map_err(|error| ToolError::invalid_input(error.to_string()))
        }))
    }

    /// Authorize one session-scoped delegation.
    ///
    /// This mirrors the interactive rule resolution but substitutes an
    /// explicit unavailable result for the approval interaction, because a
    /// hook has no tool slot to answer a prompt.
    pub(crate) async fn authorize_session_scoped_checks(
        &self,
        session: i64,
        executor: &ToolExecutor,
        checks: Vec<agena_tool::ToolPermissionCheck>,
    ) -> Result<(), AppError> {
        if checks.is_empty() {
            return Ok(());
        }
        let workspace = self
            .kernel()
            .reader
            .metadata(session_id(session)?)
            .await
            .map_err(kernel_error)?
            .workspace_id;
        for check in checks {
            if self.shutdown.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let action_key = super::super::execution::permission_key(&check.action)
                .map_err(|error| AppError::Config(error.to_string()))?;
            let rules = self
                .kernel_runtime
                .permissions
                .resolve(&action_key, Some(session), workspace)
                .await
                .map_err(|error| AppError::Config(error.to_string()))?;
            let rules = rules
                .into_iter()
                .map(|rule| agena_permission::RuleEntry {
                    id: rule.id,
                    revision_ms: rule.updated_at_ms.or(rule.created_at_ms),
                    scope: rule.scope,
                    source: rule.source,
                    reason: rule.reason,
                    operator: rule.operator,
                    mode: rule.mode,
                })
                .collect::<Vec<_>>();
            let resolution = agena_permission::apply_rules(&check.decision, &rules);
            match resolution.decision.clone() {
                PermissionDecision::Allow => {}
                PermissionDecision::Deny { reason } => {
                    let (source, scope, operator, rule_id, revision, authority) =
                        match &resolution.source {
                            PermissionResolutionSource::PersistedRule {
                                source,
                                scope,
                                operator,
                                rule_id,
                                revision_ms,
                                ..
                            } => (
                                Some(source.clone()),
                                Some(*scope),
                                operator.clone(),
                                *rule_id,
                                *revision_ms,
                                agena_domain::PermissionAuthorityKind::PersistedRule,
                            ),
                            _ => (
                                Some("static_policy".into()),
                                None,
                                None,
                                None,
                                None,
                                agena_domain::PermissionAuthorityKind::StaticPolicy,
                            ),
                        };
                    return Err(AppError::PolicyDenied(Box::new(
                        agena_domain::PolicyDeniedResult {
                            action: check.action.clone(),
                            related_actions: Vec::new(),
                            denied_actions: vec![check.action],
                            reason,
                            explanation: resolution.explanation,
                            source,
                            scope,
                            operator,
                            authority,
                            rule_id,
                            rule_revision_ms: revision,
                            trace: resolution.trace,
                        },
                    )));
                }
                PermissionDecision::Ask { reason } | PermissionDecision::Auto { reason } => {
                    return Err(AppError::CapabilityUnavailable(Box::new(
                        agena_domain::CapabilityUnavailableResult {
                            capability: "plugin_delegated_approval".into(),
                            tool_name: None,
                            reason: format!(
                                "{reason} A plugin callback outside a tool invocation has no \
                                 approval channel for this action. An explicit applicable \
                                 permission rule is required, or the work must run through a \
                                 tool call in the session."
                            ),
                            source: agena_domain::CapabilitySourceKind::RuntimeConfiguration,
                            retryable: false,
                        },
                    )));
                }
            }
        }
        let _ = executor;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
