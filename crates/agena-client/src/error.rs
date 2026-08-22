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
            Self::Decode(error) => Some(agena_failure::diagnostic::format_error_chain(error)),
            Self::Protocol(diagnostic) => Some(diagnostic.clone()),
            Self::Api(_) | Self::SubscriptionClosed => None,
        }
    }

    /// Best operator-facing diagnostic for process, log, and local RPC
    /// boundaries. `Display` remains safe-by-default for user transports, so
    /// callers that need the real cause must use this method explicitly.
    pub fn operator_diagnostic(&self) -> String {
        self.diagnostic_message()
            .unwrap_or_else(|| self.to_string())
    }

    /// Safe user-facing projection that still carries a scrubbed root cause.
    /// Transport details remain available verbatim through
    /// [`Self::operator_diagnostic`].
    pub fn user_message(&self) -> String {
        match self {
            Self::Api(error) => error.to_string(),
            Self::SubscriptionClosed => {
                "The live update connection was closed. Reconnect and try again.".to_owned()
            }
            Self::Transport(diagnostic) => {
                let message = agena_failure::diagnostic::user_message_with_context(diagnostic, 240);
                if message.is_empty() {
                    "The service could not be reached. Check the connection and try again."
                        .to_owned()
                } else {
                    message
                }
            }
            Self::Decode(error) => {
                let diagnostic = agena_failure::diagnostic::format_error_chain(error);
                let message =
                    agena_failure::diagnostic::user_message_with_context(&diagnostic, 240);
                if message.is_empty() {
                    "The service returned an invalid response. Try again.".to_owned()
                } else {
                    message
                }
            }
            Self::Protocol(diagnostic) => {
                let message = agena_failure::diagnostic::user_message_with_context(diagnostic, 240);
                if message.is_empty() {
                    "The service returned an invalid response. Try again.".to_owned()
                } else {
                    message
                }
            }
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.user_message().as_str())
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
        Self::Transport(agena_failure::diagnostic::format_error_chain(&err))
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for ClientError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::Transport(agena_failure::diagnostic::format_error_chain(&err))
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
        assert!(error.to_string().contains("connection failed"));
        assert_eq!(error.diagnostic_message().as_deref(), Some(diagnostic));
        assert_eq!(error.operator_diagnostic(), diagnostic);
    }
}
