use super::*;
use agena_domain::{
    DecisionTraceStep, PermissionAuthorityKind, PolicyDeniedResult, PolicySourceKind,
};
use agena_plugin_host::host::ImportedToolDecision;
use agena_tool::ToolPermissionCheck;

pub(super) enum ImportedPermission {
    Ask(String),
    Deny(String),
}

impl ImportedPermission {
    pub(super) fn apply(
        self,
        action: &PermissionAction,
        resolution: &PermissionResolution,
    ) -> Result<Option<String>, ToolError> {
        if matches!(resolution.decision, PermissionDecision::Deny { .. }) {
            return Ok(None);
        }
        match self {
            Self::Ask(reason) => Ok(Some(reason)),
            Self::Deny(reason) => {
                let source = Some("plugin_hook".to_owned());
                let mut trace = resolution.trace.clone();
                trace.push(DecisionTraceStep {
                    source_kind: PolicySourceKind::PluginAdvice,
                    summary: reason.clone(),
                    source: source.clone(),
                    scope: None,
                    operator: None,
                });
                Err(ToolError::PolicyDenied(Box::new(PolicyDeniedResult {
                    action: action.clone(),
                    related_actions: Vec::new(),
                    denied_actions: vec![action.clone()],
                    explanation: reason.clone(),
                    reason,
                    source,
                    scope: None,
                    operator: None,
                    authority: PermissionAuthorityKind::PluginPolicy,
                    rule_id: None,
                    rule_revision_ms: None,
                    trace,
                })))
            }
        }
    }
}

impl SessionManager {
    // This callback can run inside tool.call's continuation. Keep its VM and
    // permission preflight future off the enclosing tool runner's stack.
    pub(super) fn imported_permission_decision<'a>(
        &'a self,
        invocation: PermissionInvocation<'a>,
        checks: &'a [ToolPermissionCheck],
        executor: &'a ToolExecutor,
        control: &'a RunControl,
        call_id: i64,
        workspace: Option<i64>,
    ) -> futures_util::future::BoxFuture<'a, Result<Option<ImportedPermission>, ToolError>> {
        self.check_imported_permission(invocation, checks, executor, control, call_id, workspace)
            .boxed()
    }

    async fn check_imported_permission(
        &self,
        invocation: PermissionInvocation<'_>,
        checks: &[ToolPermissionCheck],
        executor: &ToolExecutor,
        control: &RunControl,
        call_id: i64,
        workspace: Option<i64>,
    ) -> Result<Option<ImportedPermission>, ToolError> {
        let mut baseline = PermissionDecision::Allow;
        for check in checks {
            if control.cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let key = permission_key(&check.action)
                .map_err(|error| ToolError::invalid_input(error.to_string()))?;
            let resolution = self
                .resolve_tool_permission(&key, &check.decision, control.session, workspace)
                .await?;
            match resolution.decision {
                deny @ PermissionDecision::Deny { .. } => {
                    baseline = deny;
                    break;
                }
                ask @ PermissionDecision::Ask { .. } => baseline = ask,
                auto @ PermissionDecision::Auto { .. }
                    if matches!(baseline, PermissionDecision::Allow) =>
                {
                    baseline = auto;
                }
                _ => {}
            }
        }
        let denied = matches!(baseline, PermissionDecision::Deny { .. });
        let classic_ask = invocation.hook_ask.map(str::to_owned);
        if !denied && let Some(reason) = &classic_ask {
            baseline = PermissionDecision::Ask {
                reason: reason.clone(),
            };
        }
        let decision = if invocation.run_mod_check && executor.has_imported_tool_check_hooks() {
            tokio::select! {
                biased;
                _ = control.cancel.cancelled() => return Err(ToolError::Cancelled),
                decision = executor.run_imported_tool_check(invocation.tool, control.session.get(), call_id, baseline) => decision,
            }
        } else {
            None
        };
        if denied {
            return Ok(None);
        }
        let reason = |value: Option<String>, fallback: &str| {
            let value = value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(fallback);
            value[..value.floor_char_boundary(value.len().min(8192))].to_owned()
        };
        Ok(match decision {
            Some(ImportedToolDecision::Ask { reason: text }) => Some(ImportedPermission::Ask(
                reason(text, "Claude mod requested confirmation for this tool"),
            )),
            Some(ImportedToolDecision::Deny { reason: text }) => Some(ImportedPermission::Deny(
                reason(text, "Claude mod refused this tool"),
            )),
            Some(ImportedToolDecision::Allow) | None => classic_ask.map(|text| {
                ImportedPermission::Ask(reason(
                    Some(text),
                    "PreToolUse hook requested confirmation for this tool",
                ))
            }),
        })
    }
}
