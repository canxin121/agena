//! Error types for the marketplace.

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum MarketplaceError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("http error: {0}")]
    Http(String),
    #[error("invalid registry index: {0}")]
    Index(String),
    #[error("plugin `{0}` not found in registry")]
    PluginNotFound(String),
    #[error("no version of `{0}` matches platform `{1}`")]
    NoMatchingVersion(String, String),
    #[error("sha256 mismatch for `{plugin}`: expected {expected}, got {got}")]
    Sha256Mismatch {
        plugin: String,
        expected: String,
        got: String,
    },
    #[error("missing sha256 for `{0}` and --allow-unverified not set")]
    MissingSha256(String),
    #[error("signature verification failed for `{plugin}`: {message}")]
    SignatureFailed { plugin: String, message: String },
    #[error("config error: {0}")]
    Config(String),
    #[error("invalid url `{0}`")]
    InvalidUrl(String),
    #[error("plugin `{0}` is already installed; use --force")]
    AlreadyInstalled(String),
}

impl From<reqwest::Error> for MarketplaceError {
    fn from(err: reqwest::Error) -> Self {
        Self::Http(err.to_string())
    }
}

impl From<serde_json::Error> for MarketplaceError {
    fn from(err: serde_json::Error) -> Self {
        Self::Index(err.to_string())
    }
}
