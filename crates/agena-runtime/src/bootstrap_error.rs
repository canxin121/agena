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

#[derive(Debug, Clone, thiserror::Error)]
#[error("runtime bootstrap {kind:?} error: {message}")]
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
}
