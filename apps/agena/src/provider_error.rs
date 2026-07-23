use std::{fmt, io};

#[derive(Debug)]
pub(crate) enum ProviderError {
    Cancelled,
    PermissionDenied(String),
    Unsupported(String),
    DependencyMissing(String),
    Timeout { operation: String, seconds: u64 },
    Protocol(String),
    Io(io::Error),
}

impl ProviderError {
    pub(crate) const fn allows_fallback(&self) -> bool {
        matches!(self, Self::Unsupported(_) | Self::DependencyMissing(_))
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("operation cancelled"),
            Self::PermissionDenied(message)
            | Self::Unsupported(message)
            | Self::DependencyMissing(message)
            | Self::Protocol(message) => formatter.write_str(message),
            Self::Timeout { operation, seconds } => {
                write!(formatter, "{operation} timed out after {seconds} seconds")
            }
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProviderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ProviderError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
