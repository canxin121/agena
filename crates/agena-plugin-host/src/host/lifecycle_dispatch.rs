//! Runtime-facing dispatch of typed lifecycle events. Permission decisions are
//! guarded; terminal observations never turn a completed action into an error.

use super::{HookRunRecord, HookRunStatus, PluginHost};
use crate::event_pipeline::{PluginGuardReport, PluginObserveReport};
use crate::sdk::{
    CompactionAfterInput, CompactionBeforeInput, ModelRequestAfterInput, ModelRequestBeforeInput,
    ModelRequestErrorInput, PermissionRequestInput, PluginError, PluginErrorKind,
    SubagentStartInput, SubagentStopInput,
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
    ) -> Result<(), PluginError> {
        let session = input.session_id;
        let report = self.lifecycle.compaction_before.dispatch(input).await;
        self.guard_result("compaction.before", session, report)
    }

    pub async fn broadcast_compaction_after(&self, input: CompactionAfterInput) {
        let session = input.session_id;
        let report = self.lifecycle.compaction_after.dispatch(input).await;
        self.observe_result("compaction.after", session, report);
    }

    pub async fn dispatch_subagent_start(
        &self,
        input: SubagentStartInput,
    ) -> Result<(), PluginError> {
        let session = input.session_id;
        let report = self.lifecycle.subagent_start.dispatch(input).await;
        self.guard_result("subagent.start", session, report)
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
