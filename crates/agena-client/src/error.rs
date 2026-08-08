//! Client-side error types for the `agena-client` SDK.

use agena_api::error::ApiError;

#[derive(Debug)]
/// Error returned by the API client: transport, decoding, API, or protocol failures.
pub enum ClientError {
    Transport(String),
    Decode(serde_json::Error),
    Api(ApiError),
    SubscriptionClosed,
    Protocol(String),
}

impl ClientError {
    pub fn problem(&self) -> Option<&agena_failure::UserProblem> {
        match self {
            Self::Api(error) => Some(&error.problem),
            _ => None,
        }
    }

    pub fn diagnostic_message(&self) -> Option<String> {
        match self {
            Self::Transport(diagnostic) => Some(diagnostic.clone()),
            Self::Decode(error) => Some(error.to_string()),
            Self::Protocol(diagnostic) => Some(diagnostic.clone()),
            Self::Api(_) | Self::SubscriptionClosed => None,
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(error) => error.fmt(formatter),
            Self::Transport(_) => formatter
                .write_str("The service could not be reached. Check the connection and try again."),
            Self::Decode(_) | Self::Protocol(_) => {
                formatter.write_str("The service returned an invalid response. Try again.")
            }
            Self::SubscriptionClosed => formatter
                .write_str("The live update connection was closed. Reconnect and try again."),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Api(error) => Some(error),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for ClientError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(error)
    }
}

impl From<reqwest::Error> for ClientError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err.to_string())
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for ClientError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::Transport(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::ClientError;

    #[test]
    fn transport_diagnostic_is_not_the_display_message() {
        let diagnostic = "connection failed token=secret /private/agena.sock";
        let error = ClientError::Transport(diagnostic.to_owned());
        assert!(!error.to_string().contains("token=secret"));
        assert!(!error.to_string().contains("/private/agena.sock"));
        assert_eq!(error.diagnostic_message().as_deref(), Some(diagnostic));
    }
}
