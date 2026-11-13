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
        let _admission = self.reserve_execution()?;
        let call = next_session_scoped_call();
        let session_key = session_id(session)?;
        let state = self.state_for_session(session_key).await?;
        let context = self.execution_context_value(session_key).await?;
        let cancel = self.shutdown.child_token();
        let executor = state
            .executor
            .for_session_context_async(&context)
            .await
            .with_cancellation_token(Some(cancel.clone()));
        // Tools that read supporting files during execution ask this
        // authorizer instead of the interactive approval slot.
        let authorizer = executor.clone();
        let manager = self.clone();
        let runtime = tokio::runtime::Handle::current();
        let cancel_for_reads = cancel.clone();
        let executor = executor.with_read_path_authorizer(Arc::new(move |path| {
            if cancel_for_reads.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let check = authorizer
                .path_permission_check(agena_domain::AccessKind::Read, &path.to_string_lossy());
            runtime
                .block_on(manager.authorize_session_scoped_checks(
                    session,
                    &authorizer,
                    vec![check],
                ))
                .map_err(|error| ToolError::invalid_input(error.to_string()))
        }));

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
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }

        let _permit = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AppError::Cancelled),
            permit = state.tools.clone().acquire_owned() => permit.map_err(|_| AppError::Cancelled)?,
        };
        let mut execution = executor
            .execute_invocation(
                &invocation,
                crate::tool::ToolRuntimeContext {
                    session_id: Some(session),
                    // A unique negative id: no transcript tool call exists,
                    // but managed child processes still need a stable owner.
                    call_id: Some(call),
                    prepared_shell_command: shell,
                    launch_provenance: None,
                    output: None,
                    tool: None,
                },
            )
            .await
            .map_err(tool_error)?;
        if let Some(title) = prepared.title_override {
            execution.view.set_title(title);
        }
        execution.view.metadata.extend(prepared.metadata);
        Ok(execution)
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
