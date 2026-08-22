//! Error types for the marketplace.

use std::io;

#[derive(Debug, thiserror::Error)]
/// Error from the plugin marketplace.
pub enum MarketplaceError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("http error: {0}")]
    Http(String),
    #[error("invalid registry index: {0}")]
    Index(String),
    #[error("invalid plugin project: {0}")]
    Project(String),
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
    #[error("archive error for `{plugin}`: {message}")]
    Archive { plugin: String, message: String },
    #[error("dependency `{0}` requested by `{1}` is missing from the registry")]
    MissingDependency(String, String),
    #[error("circular dependency detected involving `{0}`")]
    CircularDependency(String),
    #[error("plugin `{plugin}` is required by {dependents:?}; pass --cascade to remove together")]
    RequiredByOthers {
        plugin: String,
        dependents: Vec<String>,
    },
}

impl MarketplaceError {
    pub(crate) fn http_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Http(agena_failure::diagnostic::format_error_chain(error))
    }

    pub(crate) fn index_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Index(agena_failure::diagnostic::format_error_chain(error))
    }

    pub(crate) fn project_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Project(agena_failure::diagnostic::format_error_chain(error))
    }
}

impl From<toml::de::Error> for MarketplaceError {
    fn from(err: toml::de::Error) -> Self {
        Self::project_error(&err)
    }
}

impl From<toml::ser::Error> for MarketplaceError {
    fn from(err: toml::ser::Error) -> Self {
        Self::project_error(&err)
    }
}

impl From<reqwest::Error> for MarketplaceError {
    fn from(err: reqwest::Error) -> Self {
        Self::http_error(&err)
    }
}

impl From<serde_json::Error> for MarketplaceError {
    fn from(err: serde_json::Error) -> Self {
        Self::index_error(&err)
    }
}
