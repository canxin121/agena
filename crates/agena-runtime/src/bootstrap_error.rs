//! Stable error contract for runtime bootstrap.
//!
//! Concrete schema and adapter failures are mapped here at the composition
//! boundary so process entrypoints do not need a concrete composition error
//! type merely to start a runtime.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Kind of a runtime bootstrap error.
pub enum RuntimeBootstrapErrorKind {
    Configuration,
    Database,
    Io,
    Internal,
}

impl std::fmt::Display for RuntimeBootstrapErrorKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Configuration => "configuration",
            Self::Database => "database",
            Self::Io => "I/O",
            Self::Internal => "internal",
        })
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("runtime bootstrap {kind} error: {message}")]
/// Error bootstrapping the runtime.
pub struct RuntimeBootstrapError {
    pub kind: RuntimeBootstrapErrorKind,
    pub message: String,
}

impl RuntimeBootstrapError {
    pub fn new(kind: RuntimeBootstrapErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn configuration(message: impl Into<String>) -> Self {
        Self::new(RuntimeBootstrapErrorKind::Configuration, message)
    }

    pub fn database(message: impl Into<String>) -> Self {
        Self::new(RuntimeBootstrapErrorKind::Database, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(RuntimeBootstrapErrorKind::Io, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(RuntimeBootstrapErrorKind::Internal, message)
    }

    /// Preserve every source when a typed error crosses the cloneable runtime
    /// bootstrap contract and therefore has to become diagnostic text.
    pub fn from_error(
        kind: RuntimeBootstrapErrorKind,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::new(kind, agena_failure::diagnostic::format_error_chain(error))
    }

    /// Preserve an operation label together with the complete source chain.
    pub fn from_error_with_context(
        kind: RuntimeBootstrapErrorKind,
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::new(
            kind,
            agena_failure::diagnostic::format_error_chain_with_context(context, error),
        )
    }

    pub fn configuration_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::from_error(RuntimeBootstrapErrorKind::Configuration, error)
    }

    pub fn database_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::from_error(RuntimeBootstrapErrorKind::Database, error)
    }

    pub fn io_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::from_error(RuntimeBootstrapErrorKind::Io, error)
    }

    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::from_error(RuntimeBootstrapErrorKind::Internal, error)
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeBootstrapError, RuntimeBootstrapErrorKind};

    #[derive(Debug, thiserror::Error)]
    #[error("failed to load runtime configuration")]
    struct OuterError {
        #[source]
        source: InnerError,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("providers.default is no longer supported")]
    struct InnerError;

    #[test]
    fn typed_bootstrap_mapping_keeps_kind_and_complete_chain() {
        let error = RuntimeBootstrapError::from_error_with_context(
            RuntimeBootstrapErrorKind::Configuration,
            "failed to build agena runtime",
            &OuterError { source: InnerError },
        );

        assert_eq!(error.kind, RuntimeBootstrapErrorKind::Configuration);
        assert!(error.message.contains("failed to build agena runtime"));
        assert!(
            error
                .message
                .contains("failed to load runtime configuration")
        );
        assert!(
            error
                .message
                .contains("providers.default is no longer supported")
        );
        assert!(error.to_string().contains("bootstrap configuration error"));
    }
}
