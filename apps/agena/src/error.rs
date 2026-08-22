#[derive(Debug, thiserror::Error)]
pub(crate) enum AgenaProcessError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl AgenaProcessError {
    pub(crate) fn configuration_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Configuration(agena_failure::diagnostic::format_error_chain(error))
    }

    pub(crate) fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Internal(agena_failure::diagnostic::format_error_chain(error))
    }

    /// Convert an `anyhow` server/lifecycle failure at the process boundary.
    /// The complete chain is retained, and a typed Runtime bootstrap error
    /// keeps its original category instead of becoming an `Internal` error.
    pub(crate) fn from_anyhow(error: anyhow::Error) -> Self {
        let diagnostic = anyhow_diagnostic(&error);
        let kind = error
            .downcast_ref::<agena_runtime::RuntimeBootstrapError>()
            .map(|error| error.kind);
        match kind {
            Some(agena_runtime::RuntimeBootstrapErrorKind::Configuration) => {
                Self::Configuration(diagnostic)
            }
            Some(agena_runtime::RuntimeBootstrapErrorKind::Database) => Self::Database(diagnostic),
            Some(agena_runtime::RuntimeBootstrapErrorKind::Io) => Self::Io(diagnostic),
            Some(agena_runtime::RuntimeBootstrapErrorKind::Internal) | None => {
                Self::Internal(diagnostic)
            }
        }
    }

    /// Preserve the CLI error's source chain while keeping its public error
    /// category at the executable boundary.
    pub(crate) fn from_cli(error: agena_cli::AppError) -> Self {
        let diagnostic = agena_failure::diagnostic::format_error_chain(&error);
        match error {
            agena_cli::AppError::Config(_) => Self::Configuration(diagnostic),
            agena_cli::AppError::Io(_) => Self::Io(diagnostic),
            agena_cli::AppError::Provider(_)
            | agena_cli::AppError::SerdeJson(_)
            | agena_cli::AppError::Internal(_) => Self::Internal(diagnostic),
        }
    }
}

fn anyhow_diagnostic(error: &anyhow::Error) -> String {
    let mut diagnostic = agena_failure::diagnostic::format_error_chain(error.as_ref());

    // These boundary errors intentionally keep their operator diagnostic out
    // of Display so API/TUI user channels do not leak raw transport details.
    // At the local executable boundary, append that diagnostic explicitly.
    let hidden = error
        .downcast_ref::<agena_client::ClientError>()
        .and_then(agena_client::ClientError::diagnostic_message)
        .or_else(|| {
            error
                .downcast_ref::<agena_application::ApplicationError>()
                .and_then(|error| error.diagnostic_message().map(ToOwned::to_owned))
        });
    if let Some(hidden) = hidden
        && !hidden.trim().is_empty()
        && !diagnostic.contains(&hidden)
    {
        diagnostic.push_str(": ");
        diagnostic.push_str(&hidden);
    }
    diagnostic
}

impl From<std::io::Error> for AgenaProcessError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(agena_failure::diagnostic::format_error_chain(&error))
    }
}

pub(crate) type Result<T> = std::result::Result<T, AgenaProcessError>;

#[cfg(test)]
mod tests {
    use super::AgenaProcessError;
    use anyhow::Context as _;

    #[test]
    fn anyhow_conversion_keeps_bootstrap_category_and_root_cause() {
        let bootstrap = agena_runtime::RuntimeBootstrapError::configuration(
            "config validation failed: providers.default is no longer supported",
        );
        let error = anyhow::Error::new(bootstrap).context("failed to build agena runtime");

        let process = AgenaProcessError::from_anyhow(error);
        let rendered = process.to_string();

        assert!(matches!(process, AgenaProcessError::Configuration(_)));
        assert!(rendered.starts_with("configuration error:"));
        assert!(rendered.contains("failed to build agena runtime"));
        assert!(rendered.contains("providers.default is no longer supported"));
        assert!(!rendered.contains("Internal(\""));
    }

    #[test]
    fn cli_io_conversion_keeps_the_leaf_diagnostic() {
        let process = AgenaProcessError::from_cli(agena_cli::AppError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "read-only config file",
        )));

        assert_eq!(
            process.to_string(),
            "I/O error: io error: read-only config file"
        );
    }
}
