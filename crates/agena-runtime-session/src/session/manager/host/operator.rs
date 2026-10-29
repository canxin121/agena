//! Session-free operator execution (including the MCP server). This grants
//! neither a fabricated session nor blanket filesystem/network access.

use super::*;
use agena_domain::{PermissionDecision, PermissionResolutionSource};

/// Revocation is synchronous, including on errors, panic unwinding, and a
/// dropped request future. A detached callback cannot keep this scope alive.
struct OperatorInvocationScope {
    calls: Arc<StdMutex<OperatorInvocationMap>>,
    call: i64,
    authority: Arc<OperatorInvocationAuthority>,
}

impl Drop for OperatorInvocationScope {
    fn drop(&mut self) {
        self.authority.cancel.cancel();
        self.calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.call);
    }
}

impl SessionManager {
    pub async fn execute_unscoped_tool(
        &self,
        invocation: ToolInvocation,
        call_id: i64,
    ) -> Result<crate::SessionToolExecutionOutcome, AppError> {
        let _admission = self.reserve_execution()?;
        let state = self.state.load_full();
        // Resolve the identity from the same frozen executor as the policy.
        // The manager's cached workspace can belong to an earlier executor,
        // and rereading current state after an await can observe a reload.
        let workspace_path = tokio::fs::canonicalize(state.executor.workspace_root())
            .await
            .map_err(|error| AppError::internal_error(&error))?;
        let workspace = self
            .kernel_runtime
            .workspaces
            .ensure_id(&workspace_path.to_string_lossy())
            .await
            .map_err(|error| AppError::internal_error(&error))?;
        let scope = self
            .begin_operator_invocation(call_id, state.executor.clone(), workspace)
            .map_err(tool_error)?;
        let _permit = tokio::select! {
            biased;
            _ = scope.authority.cancel.cancelled() => return Err(AppError::Cancelled),
            permit = state.tools.clone().acquire_owned() => permit.map_err(|_| AppError::Cancelled)?,
        };
        let execution = self
            .execute_operator_invocation(call_id, &invocation)
            .await
            .map_err(tool_error)?;
        Ok(crate::SessionToolExecutionOutcome::Completed(
            execution.summary(),
        ))
    }

    fn begin_operator_invocation(
        &self,
        call: i64,
        executor: ToolExecutor,
        workspace: i64,
    ) -> Result<OperatorInvocationScope, ToolError> {
        if self.shutdown.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        if call < 0 {
            return Err(ToolError::invalid_input(
                "operator call_id must be nonnegative",
            ));
        }
        let cancel = self.shutdown.child_token();
        let authority = Arc::new(OperatorInvocationAuthority {
            executor: executor.with_cancellation_token(Some(cancel.clone())),
            workspace,
            cancel,
        });
        let mut calls = self
            .operator_calls
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match calls.entry(call) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(authority.clone());
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                return Err(ToolError::invalid_input(
                    "operator call_id is already active",
                ));
            }
        }
        Ok(OperatorInvocationScope {
            calls: self.operator_calls.clone(),
            call,
            authority,
        })
    }

    pub(super) fn operator_invocation(
        &self,
        call: i64,
    ) -> Result<Arc<OperatorInvocationAuthority>, ToolError> {
        let authority = self
            .operator_calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&call)
            .cloned()
            .ok_or_else(|| {
                ToolError::CapabilityUnavailable(Box::new(
                    agena_domain::CapabilityUnavailableResult {
                        capability: "operator_invocation".into(),
                        tool_name: None,
                        reason: "the host callback has no active operator invocation authority"
                            .into(),
                        source: agena_domain::CapabilitySourceKind::RuntimeConfiguration,
                        retryable: false,
                    },
                ))
            })?;
        if authority.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        Ok(authority)
    }

    /// The caller has passed plugin-host callback-token validation. Reuse the
    /// exact originating operator principal, never a session's policy or a
    /// fresh principal from a later runtime generation.
    pub(super) async fn execute_operator_invocation(
        &self,
        call: i64,
        invocation: &ToolInvocation,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let authority = self.operator_invocation(call)?;
        let manager = self.clone();
        let authorizer = authority.clone();
        let runtime = tokio::runtime::Handle::current();
        let executor = authority
            .executor
            .clone()
            .with_read_path_authorizer(Arc::new(move |path| {
                let check = authorizer
                    .executor
                    .path_permission_check(agena_domain::AccessKind::Read, &path.to_string_lossy());
                runtime.block_on(manager.authorize_operator_checks(&authorizer, vec![check]))
            }));

        // Authorize before hooks, then authorize their final command/path
        // effects again. Plugins owning their I/O additionally check each
        // concrete target through require_host_* while this scope is live.
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(invocation, None)
            .await?;
        self.authorize_operator_checks(&authority, checks).await?;
        let prepared = executor
            .prepare_invocation(invocation, OPERATOR_SESSION, call)
            .await?;
        let (invocation, shell) = executor
            .prepare_shell_invocation(&prepared.invocation, OPERATOR_SESSION, call)
            .await?;
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(&invocation, None)
            .await?;
        self.authorize_operator_checks(&authority, checks).await?;
        let mut execution = executor
            .execute_invocation(
                &invocation,
                crate::tool::ToolRuntimeContext {
                    session_id: None,
                    call_id: Some(call),
                    prepared_shell_command: shell,
                    launch_provenance: None,
                    output: None,
                    tool: None,
                },
            )
            .await?;
        if let Some(title) = prepared.title_override {
            execution.view.set_title(title);
        }
        execution.view.metadata.extend(prepared.metadata);
        Ok(execution)
    }

    pub(super) async fn authorize_operator_checks(
        &self,
        authority: &OperatorInvocationAuthority,
        checks: Vec<agena_tool::ToolPermissionCheck>,
    ) -> Result<(), ToolError> {
        for check in checks {
            if authority.cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let action_key = super::super::execution::permission_key(&check.action)
                .map_err(|error| ToolError::invalid_input(error.to_string()))?;
            let rules = self
                .kernel_runtime
                .permissions
                .resolve(&action_key, None, Some(authority.workspace))
                .await
                .map_err(|error| ToolError::invalid_input(error.to_string()))?
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
            if authority.cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            // No grant cache: a newly effective deny applies to the next
            // target, even if this call previously accessed the same path.
            let resolution = agena_permission::apply_rules(&check.decision, &rules);
            match resolution.decision.clone() {
                PermissionDecision::Allow => {}
                PermissionDecision::Deny { reason } => {
                    let (source, scope, operator, rule_id, revision, kind) =
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
                    return Err(ToolError::PolicyDenied(Box::new(
                        agena_domain::PolicyDeniedResult {
                            action: check.action.clone(),
                            related_actions: Vec::new(),
                            denied_actions: vec![check.action],
                            reason,
                            explanation: resolution.explanation,
                            source,
                            scope,
                            operator,
                            authority: kind,
                            rule_id,
                            rule_revision_ms: revision,
                            trace: resolution.trace,
                        },
                    )));
                }
                PermissionDecision::Ask { reason } | PermissionDecision::Auto { reason } => {
                    return Err(ToolError::CapabilityUnavailable(Box::new(
                        agena_domain::CapabilityUnavailableResult {
                            capability: "operator_permission_approval".into(),
                            tool_name: None,
                            reason: format!(
                                "{reason} This operator call has no session approval channel; an explicit applicable permission rule or a session invocation is required."
                            ),
                            source: agena_domain::CapabilitySourceKind::RuntimeConfiguration,
                            retryable: false,
                        },
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
