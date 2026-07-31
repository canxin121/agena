//! Structured ephemeral terminal notifications.

use std::time::{Duration, Instant};

pub const DEFAULT_NOTICE_DURATION: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeSeverity {
    Success,
    Warning,
    Error,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoticeScope {
    #[default]
    Global,
    Composer,
    Field,
    Form,
    Session,
    ToolCall,
    Provider,
    Plugin,
    Settings,
    BackgroundTask,
    Startup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeAction {
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone)]
pub struct UiNotice {
    pub summary: String,
    pub detail: Option<String>,
    pub severity: NoticeSeverity,
    pub scope: NoticeScope,
    pub action: Option<NoticeAction>,
    pub failure_id: Option<agena_failure::FailureId>,
    pub expires_at: Instant,
}

impl UiNotice {
    pub fn message(severity: NoticeSeverity, summary: impl Into<String>) -> Self {
        Self::with_lifetime(severity, summary, DEFAULT_NOTICE_DURATION)
    }

    pub fn with_lifetime(
        severity: NoticeSeverity,
        summary: impl Into<String>,
        lifetime: Duration,
    ) -> Self {
        Self {
            summary: summary.into(),
            detail: None,
            severity,
            scope: NoticeScope::Global,
            action: None,
            failure_id: None,
            expires_at: Instant::now() + lifetime,
        }
    }

    pub fn from_failure(failure: &agena_failure::Failure) -> Self {
        Self::from_problem(&agena_failure::UserProblem::from(failure))
    }

    pub fn from_problem(problem: &agena_failure::UserProblem) -> Self {
        let mut notice = Self::message(NoticeSeverity::Error, problem.user.fallback.clone());
        notice.failure_id = problem.is_unexpected().then_some(problem.id);
        notice.action = recovery_action(problem.recovery);
        notice
    }

    pub fn display_summary(&self) -> String {
        match self.failure_id {
            Some(id) => format!("{}  Reference: {id}", self.summary),
            None => self.summary.clone(),
        }
    }

    pub fn is_expired_at(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

fn recovery_action(recovery: agena_failure::RecoveryDirective) -> Option<NoticeAction> {
    use agena_failure::RecoveryDirective;
    let (label, command) = match recovery {
        RecoveryDirective::Refresh => ("Refresh", "session.refresh"),
        RecoveryDirective::Reauthenticate => ("Sign in", "provider.authenticate"),
        RecoveryDirective::OpenSettings => ("Open settings", "settings.open"),
        RecoveryDirective::RequestPermission => ("Permissions", "permissions.open"),
        RecoveryDirective::Retry => ("Retry", "operation.retry"),
        RecoveryDirective::ChooseAlternative => ("Choose another", "alternative.choose"),
        RecoveryDirective::RestartPlugin => ("Restart plugin", "plugin.restart"),
        RecoveryDirective::RestartRuntime => ("Restart runtime", "runtime.restart"),
        RecoveryDirective::None | RecoveryDirective::AskUser => return None,
    };
    Some(NoticeAction {
        label: label.to_owned(),
        command: command.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{NoticeSeverity, UiNotice};
    use std::time::Duration;

    #[test]
    fn notice_expiration_is_a_presentation_policy() {
        let notice = UiNotice::with_lifetime(NoticeSeverity::Info, "saved", Duration::ZERO);
        assert_eq!(notice.summary, "saved");
        assert!(notice.is_expired_at(notice.expires_at));
    }

    #[test]
    fn unexpected_failure_shows_reference_but_not_machine_code() {
        let failure = agena_failure::Failure::new(
            agena_failure::FailureCode::new("internal.database"),
            agena_failure::FailureCategory::Internal,
            agena_failure::FailureResponsibility::System,
            agena_failure::RetryDirective::Unknown,
            agena_failure::RecoveryDirective::Retry,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::new("internal", "Something went wrong."),
        );
        let notice = UiNotice::from_failure(&failure);
        let display = notice.display_summary();
        assert!(display.contains("Reference:"));
        assert!(!display.contains("internal.database"));
    }
}
