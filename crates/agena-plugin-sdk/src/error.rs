use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, ModelFeedback,
    RecoveryDirective, RetryDirective, UserPresentation,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A plugin failure with explicit public and diagnostic channels.
///
/// `failure` is safe to project to users or models. `diagnostic` exists only
/// for plugin transports and host logs and must never be copied into UI,
/// transcript, or model feedback fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginError {
    pub kind: PluginErrorKind,
    pub failure: Box<Failure>,
    pub diagnostic: Box<PluginDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginDiagnostic {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

pub const CONFIGURATION_REQUIRED_MARKER: &str = "configuration_required";
const PUBLIC_PROBLEM_DATA_KEY: &str = "agena_public_problem";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginErrorKind {
    Internal,
    NotImplemented,
    InvalidParams,
    Timeout,
    Disconnected,
    Panicked,
    HostUnavailable,
    PolicyDenied,
    UserDeclined,
    CapabilityUnavailable,
    ToolUnavailable,
}

impl PluginError {
    pub fn internal(diagnostic: impl std::fmt::Display) -> Self {
        Self::from_kind(PluginErrorKind::Internal, diagnostic)
    }

    pub fn not_implemented(hook: impl Into<String>) -> Self {
        let hook = hook.into();
        Self::from_kind(
            PluginErrorKind::NotImplemented,
            format_args!("hook not implemented: {hook}"),
        )
        .with_hook(hook)
    }

    pub fn invalid_params(diagnostic: impl std::fmt::Display) -> Self {
        Self::from_kind(PluginErrorKind::InvalidParams, diagnostic)
    }

    pub fn invalid_params_with_data(
        diagnostic: impl std::fmt::Display,
        data: serde_json::Value,
    ) -> Self {
        let mut error = Self::invalid_params(diagnostic);
        error.diagnostic.data = Some(data);
        error
    }

    pub fn from_kind(kind: PluginErrorKind, diagnostic: impl std::fmt::Display) -> Self {
        let diagnostic = diagnostic.to_string();
        let mut failure = failure_for_kind(kind);
        if !diagnostic.trim().is_empty() {
            failure.user = UserPresentation::validated(
                format!("{}-detail", failure.user.key),
                diagnostic.as_str(),
            );
        }
        Self {
            kind,
            failure: Box::new(failure),
            diagnostic: Box::new(PluginDiagnostic {
                message: diagnostic,
                hook: None,
                plugin: None,
                data: None,
            }),
        }
    }

    /// Report a safe, actionable configuration problem without exposing the
    /// underlying plugin diagnostic to transcripts or models.
    pub fn configuration_required(
        component: impl AsRef<str>,
        instruction: impl AsRef<str>,
    ) -> Self {
        let component = component.as_ref().trim();
        let instruction = instruction.as_ref().trim();
        let diagnostic = if instruction.is_empty() {
            format!("{component} configuration is required")
        } else {
            format!("{component} configuration is required: {instruction}")
        };
        let mut error = Self::from_kind(PluginErrorKind::InvalidParams, diagnostic);
        // This marker is an enum-like, safe semantic signal. The runtime
        // reprojects it into a fixed public Failure and never renders data.
        error.diagnostic.data = Some(serde_json::json!({
            PUBLIC_PROBLEM_DATA_KEY: CONFIGURATION_REQUIRED_MARKER,
        }));
        error
    }

    /// Attach a fixed runtime-recognised public-problem marker. The runtime
    /// maps this to its own static public Failure and never renders the data.
    pub fn with_public_problem(mut self, marker: &'static str) -> Self {
        let data = self
            .diagnostic
            .data
            .get_or_insert_with(|| serde_json::json!({}));
        if let Some(object) = data.as_object_mut() {
            object.insert(
                PUBLIC_PROBLEM_DATA_KEY.to_owned(),
                serde_json::Value::String(marker.to_owned()),
            );
        }
        self
    }

    pub fn with_hook(mut self, hook: impl Into<String>) -> Self {
        self.diagnostic.hook = Some(hook.into());
        self
    }

    pub fn with_plugin(mut self, plugin: impl Into<String>) -> Self {
        self.diagnostic.plugin = Some(plugin.into());
        self
    }

    pub fn diagnostic_message(&self) -> &str {
        self.diagnostic.message.as_str()
    }
}

fn failure_for_kind(kind: PluginErrorKind) -> Failure {
    let (code, category, responsibility, retry, recovery, fallback, model) = match kind {
        PluginErrorKind::InvalidParams => (
            "plugin.invalid_input",
            FailureCategory::InvalidInput,
            FailureResponsibility::Caller,
            RetryDirective::CorrectInput,
            RecoveryDirective::None,
            "The plugin input is invalid.",
            Some(ModelFeedback::invalid_input()),
        ),
        PluginErrorKind::NotImplemented => (
            "plugin.not_implemented",
            FailureCategory::NotFound,
            FailureResponsibility::Dependency,
            RetryDirective::UseAlternative,
            RecoveryDirective::ChooseAlternative,
            "The plugin does not support this operation.",
            None,
        ),
        PluginErrorKind::Timeout => (
            "plugin.timeout",
            FailureCategory::Timeout,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            "The plugin did not respond in time.",
            None,
        ),
        PluginErrorKind::Disconnected => (
            "plugin.disconnected",
            FailureCategory::DependencyUnavailable,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::RestartPlugin,
            "The plugin is disconnected. Restart it and try again.",
            None,
        ),
        PluginErrorKind::HostUnavailable => (
            "plugin.host_unavailable",
            FailureCategory::DependencyUnavailable,
            FailureResponsibility::System,
            RetryDirective::Backoff,
            RecoveryDirective::RestartRuntime,
            "The plugin host is unavailable. Restart the runtime and try again.",
            None,
        ),
        PluginErrorKind::PolicyDenied => (
            "plugin.policy_denied",
            FailureCategory::PermissionDenied,
            FailureResponsibility::Policy,
            RetryDirective::Never,
            RecoveryDirective::None,
            "The plugin denied the request due to policy.",
            None,
        ),
        PluginErrorKind::UserDeclined => (
            "plugin.user_declined",
            FailureCategory::PermissionDenied,
            FailureResponsibility::Caller,
            RetryDirective::AfterUserAction,
            RecoveryDirective::AskUser,
            "The user declined the request.",
            None,
        ),
        PluginErrorKind::CapabilityUnavailable => (
            "plugin.capability_unavailable",
            FailureCategory::NotFound,
            FailureResponsibility::Caller,
            RetryDirective::UseAlternative,
            RecoveryDirective::ChooseAlternative,
            "The capability is not available.",
            None,
        ),
        PluginErrorKind::ToolUnavailable => (
            "plugin.tool_unavailable",
            FailureCategory::NotFound,
            FailureResponsibility::Caller,
            RetryDirective::UseAlternative,
            RecoveryDirective::ChooseAlternative,
            "The tool is not available.",
            None,
        ),
        PluginErrorKind::Panicked | PluginErrorKind::Internal => (
            "plugin.internal",
            FailureCategory::Internal,
            FailureResponsibility::Dependency,
            RetryDirective::UseAlternative,
            RecoveryDirective::RestartPlugin,
            "Plugin execution failed without diagnostic details.",
            None,
        ),
    };
    let failure = Failure::new(
        FailureCode::new(code),
        category,
        responsibility,
        retry,
        recovery,
        FailureImpact::OperationFailed,
        UserPresentation::new(code, fallback),
    );
    match model {
        Some(model) => failure.with_model_feedback(model),
        None => failure,
    }
}

/// Display is deliberately safe-by-default. Use `diagnostic_message()` only
/// in a correlated host log.
impl std::fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.failure.user.fallback.as_str())?;
        if self.failure.is_unexpected() {
            write!(formatter, " Reference: {}", self.failure.id)?;
        }
        Ok(())
    }
}

impl std::error::Error for PluginError {}

impl From<serde_json::Error> for PluginError {
    fn from(value: serde_json::Error) -> Self {
        PluginError::invalid_params(value)
    }
}

#[derive(Debug, Error)]
#[error("plugin transport error: {message}")]
pub struct TransportErrorRepr {
    pub message: String,
}

pub type Result<T> = std::result::Result<T, PluginError>;

#[cfg(test)]
mod tests {
    use super::PluginError;

    #[test]
    fn display_and_failure_exclude_plugin_diagnostics() {
        let error = PluginError::internal("token=secret /private/plugin.sock panic backtrace");
        let public = serde_json::to_string(&error.failure).expect("serialize public failure");
        assert!(!error.to_string().contains("token=secret"));
        assert!(!public.contains("/private/plugin.sock"));
        assert!(error.diagnostic_message().contains("panic backtrace"));
    }
}
