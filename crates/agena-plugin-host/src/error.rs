//! Host-side error types.

use thiserror::Error;

#[derive(Debug, Error)]
/// Error from the plugin host.
pub enum HostError {
    #[error("plugin `{plugin}` failed to load: {message}")]
    Load { plugin: String, message: String },
    #[error("plugin `{plugin}` failed during init: {message}")]
    Init { plugin: String, message: String },
    #[error("plugin `{1}` returned error: {0}")]
    Plugin(#[source] crate::sdk::PluginError, String),
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("invalid plugin config: {0}")]
    Config(String),
    #[error("{primary}; additionally, {cleanup_operation}: {cleanup_diagnostic}")]
    Cleanup {
        #[source]
        primary: Box<HostError>,
        cleanup_operation: &'static str,
        cleanup_diagnostic: String,
    },
}

impl HostError {
    pub(crate) fn with_cleanup_error(
        self,
        cleanup_operation: &'static str,
        cleanup_error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::Cleanup {
            primary: Box::new(self),
            cleanup_operation,
            cleanup_diagnostic: agena_failure::diagnostic::format_error_chain(cleanup_error),
        }
    }
}

#[derive(Debug, Error)]
/// Error from a plugin transport.
pub enum TransportError {
    #[error("transport cancelled")]
    Cancelled,
    #[error("transport disconnected: {0}")]
    Disconnected(String),
    #[error("transport timeout: {0}")]
    Timeout(String),
    #[error("transport panicked: {0}")]
    Panicked(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("plugin error: {0}")]
    Plugin(crate::sdk::PluginError),
}

impl From<std::io::Error> for TransportError {
    fn from(value: std::io::Error) -> Self {
        TransportError::Io(agena_failure::diagnostic::format_error_chain(&value))
    }
}

impl TransportError {
    pub(crate) fn disconnected(context: impl Into<String>) -> Self {
        Self::Disconnected(context.into())
    }

    pub(crate) fn disconnected_error(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::Disconnected(agena_failure::diagnostic::format_error_chain_with_context(
            context, error,
        ))
    }

    pub(crate) fn timeout_error(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::Timeout(agena_failure::diagnostic::format_error_chain_with_context(
            context, error,
        ))
    }

    pub(crate) fn panicked(payload: Box<dyn std::any::Any + Send>) -> Self {
        let message = if let Some(message) = payload.downcast_ref::<&str>() {
            (*message).to_owned()
        } else if let Some(message) = payload.downcast_ref::<String>() {
            message.clone()
        } else {
            format!("non-string panic payload of type {:?}", payload.type_id())
        };
        Self::Panicked(message)
    }
}

impl From<crate::sdk::PluginError> for TransportError {
    fn from(value: crate::sdk::PluginError) -> Self {
        TransportError::Plugin(value)
    }
}

impl From<serde_json::Error> for TransportError {
    fn from(value: serde_json::Error) -> Self {
        TransportError::Rpc(agena_failure::diagnostic::format_error_chain(&value))
    }
}
