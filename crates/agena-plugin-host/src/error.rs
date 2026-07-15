use thiserror::Error;

#[derive(Debug, Error)]
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
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport cancelled")]
    Cancelled,
    #[error("transport disconnected")]
    Disconnected,
    #[error("transport timeout")]
    Timeout,
    #[error("transport panicked")]
    Panicked,
    #[error("io error: {0}")]
    Io(String),
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("plugin error: {0}")]
    Plugin(crate::sdk::PluginError),
}

impl From<std::io::Error> for TransportError {
    fn from(value: std::io::Error) -> Self {
        TransportError::Io(value.to_string())
    }
}

impl From<crate::sdk::PluginError> for TransportError {
    fn from(value: crate::sdk::PluginError) -> Self {
        TransportError::Plugin(value)
    }
}

impl From<serde_json::Error> for TransportError {
    fn from(value: serde_json::Error) -> Self {
        TransportError::Rpc(value.to_string())
    }
}
