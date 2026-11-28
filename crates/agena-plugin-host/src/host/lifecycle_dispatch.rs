//! Runtime-facing dispatch of typed lifecycle events. Permission decisions are
//! guarded; terminal observations never turn a completed action into an error.

use std::time::Duration;

use super::{HookRunRecord, HookRunStatus, PluginHost};
use crate::event_pipeline::{PluginGuardReport, PluginObserveReport, PluginTransformBailOutcome};
use crate::sdk::{
    CompactionAfterInput, CompactionBeforeInput, HookSubscription, ModelRequestAfterInput,
    ModelRequestBeforeInput, ModelRequestErrorInput, PermissionRequestInput, PluginError,
    PluginErrorKind, SubagentStartInput, SubagentStartPatch, SubagentStopInput, rpc::method,
};

impl PluginHost {
    fn guard_result(
        &self,
        hook: &'static str,
        session_id: i64,
        report: PluginGuardReport,
    ) -> Result<(), PluginError> {
        for failure in report.failures {
            self.push_hook_runs(vec![HookRunRecord::new(
                hook,
                failure.owner.to_string(),
                Some(session_id),
                HookRunStatus::Failed,
                "lifecycle guard failed",
                Some(failure.message),
            )]);
        }
        if let Some(denial) = report.denial {
            self.push_hook_runs(vec![HookRunRecord::new(
                hook,
                denial.owner.to_string(),
                Some(session_id),
                HookRunStatus::Applied,
                "lifecycle guard denied operation",
                Some(denial.reason.clone()),
            )]);
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                format!("{hook}: {}", denial.reason),
            ));
        }
        Ok(())
    }

    fn observe_result(&self, hook: &'static str, session_id: i64, report: PluginObserveReport) {
        for failure in report.failures {
            self.push_hook_runs(vec![HookRunRecord::new(
                hook,
                failure.owner.to_string(),
                Some(session_id),
                HookRunStatus::Failed,
                "lifecycle observer failed",
                Some(failure.message),
            )]);
        }
    }

    pub async fn dispatch_model_request_before(
        &self,
        input: ModelRequestBeforeInput,
    ) -> Result<(), PluginError> {
        let session = input.session_id;
        let report = self.lifecycle.model_before.dispatch(input).await;
        self.guard_result("model.request.before", session, report)
    }

    pub async fn broadcast_model_request_after(&self, input: ModelRequestAfterInput) {
        let session = input.session_id;
        let report = self.lifecycle.model_after.dispatch(input).await;
        self.observe_result("model.request.after", session, report);
    }

    pub async fn dispatch_model_request_error(&self, input: ModelRequestErrorInput) -> bool {
        let session = input.session_id;
        let report = self.lifecycle.model_error.dispatch(input).await;
        for failure in report.failures {
            self.push_hook_runs(vec![HookRunRecord::new(
                "model.request.error",
                failure.owner.to_string(),
                Some(session),
                HookRunStatus::Failed,
                "model error recovery failed",
                Some(failure.message),
            )]);
        }
        report.value.unwrap_or(false)
    }

    pub async fn dispatch_compaction_before(
        &self,
        input: CompactionBeforeInput,
    ) -> Result<String, PluginError> {
        let session = Some(input.session_id);
        let report = self
            .lifecycle
            .compaction_before
            .dispatch((input, String::new()))
            .await
            .map_err(|error| {
                self.push_hook_runs(vec![HookRunRecord::new(
                    "compaction.before",
                    error.failure.owner.to_string(),
                    session,
                    HookRunStatus::Failed,
                    "compaction hook failed",
                    Some(error.failure.message.clone()),
                )]);
                PluginError::from_kind(
                    PluginErrorKind::PolicyDenied,
                    format!("compaction.before: {}", error.failure.message),
                )
            })?;
        match report.outcome {
            PluginTransformBailOutcome::Continue((_, instructions)) => Ok(instructions),
            PluginTransformBailOutcome::Bail((owner, reason)) => {
                self.push_hook_runs(vec![HookRunRecord::new(
                    "compaction.before",
                    owner.to_string(),
                    session,
                    HookRunStatus::Applied,
                    "compaction hook denied the operation",
                    Some(reason.clone()),
                )]);
                Err(PluginError::from_kind(
                    PluginErrorKind::PolicyDenied,
                    format!("compaction.before: {reason}"),
                ))
            }
        }
    }

    pub async fn broadcast_compaction_after(&self, input: CompactionAfterInput) -> Option<String> {
        let session = input.session_id;
        let report = self
            .lifecycle
            .compaction_after
            .dispatch(input.clone())
            .await;
        self.observe_result("compaction.after", session, report);
        input.compact_summary.as_ref()?;
        let params = serde_json::json!(input);
        let mut stop_reason = None;
        for plugin in self.imported_hook_observers(HookSubscription::COMPACTION_AFTER) {
            match self
                .call_session_hook(
                    plugin,
                    method::HOOK_COMPACTION_AFTER,
                    params.clone(),
                    self.timeouts.chat_or(Duration::from_secs(10)),
                    Some(session),
                )
                .await
            {
                Ok(value) => {
                    if let Some(reason) = value.as_str() {
                        self.push_hook_runs(vec![HookRunRecord::new(
                            "compaction.after",
                            plugin.key().to_string(),
                            Some(session),
                            HookRunStatus::Applied,
                            "PostCompact stopped continuation",
                            Some(reason.to_owned()),
                        )]);
                        stop_reason.get_or_insert_with(|| reason.to_owned());
                    }
                }
                Err(error) => {
                    self.push_hook_runs(vec![crate::dispatcher::transport_failure_record(
                        "compaction.after",
                        &plugin.key().to_string(),
                        Some(session),
                        &error,
                    )])
                }
            }
        }
        stop_reason
    }

    pub async fn dispatch_subagent_start(
        &self,
        input: SubagentStartInput,
    ) -> Result<Vec<String>, PluginError> {
        let session = input.session_id;
        let report = self.lifecycle.subagent_start.dispatch(input.clone()).await;
        self.guard_result("subagent.start", session, report)?;
        let params = serde_json::json!(input);
        let mut context = Vec::new();
        for plugin in self.plugins().iter().filter(|plugin| {
            plugin.agent_plugin().is_some() && plugin.subscribes(HookSubscription::SUBAGENT_START)
        }) {
            match self
                .call_session_hook(
                    plugin,
                    method::HOOK_SUBAGENT_START,
                    params.clone(),
                    self.timeouts.chat_or(Duration::from_secs(10)),
                    Some(input.child_session_id),
                )
                .await
            {
                Ok(raw) => match serde_json::from_value::<Option<SubagentStartPatch>>(raw) {
                    Ok(Some(patch)) => {
                        if let Some(reason) = patch.deny_reason {
                            self.push_hook_runs(vec![HookRunRecord::new(
                                "subagent.start",
                                plugin.key().to_string(),
                                Some(session),
                                HookRunStatus::Applied,
                                "SubagentStart hook denied launch",
                                Some(reason.clone()),
                            )]);
                            return Err(PluginError::from_kind(
                                PluginErrorKind::PolicyDenied,
                                reason,
                            ));
                        }
                        context.extend(patch.additional_context);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(plugin = %plugin.key(), %error, "invalid SubagentStart result")
                    }
                },
                Err(error) => {
                    self.push_hook_runs(vec![crate::dispatcher::transport_failure_record(
                        "subagent.start",
                        &plugin.key().to_string(),
                        Some(session),
                        &error,
                    )])
                }
            }
        }
        Ok(context)
    }

    pub async fn broadcast_subagent_stop(&self, input: SubagentStopInput) {
        let session = input.session_id;
        let report = self.lifecycle.subagent_stop.dispatch(input).await;
        self.observe_result("subagent.stop", session, report);
    }

    pub async fn dispatch_permission_request(
        &self,
        input: PermissionRequestInput,
    ) -> Result<(), PluginError> {
        let session = input.session_id;
        let report = self.lifecycle.permission_request.dispatch(input).await;
        self.guard_result("permission.request", session, report)
    }
}
