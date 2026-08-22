//! # agena-runtime-provider
//!
//! Provider-facing contracts reserved for the runtime provider layer.
//!
//! Owns the provider adapter contract ([`provider`]), configuration support
//! ([`config_support`]), SSE handling ([`provider_sse`]), the provider error
//! type ([`ProviderError`]), request context ([`ProviderRequestContext`]),
//! model selection and priorities, and the runtime Codex user-agent.

mod codex_user_agent;
pub mod config_support;
pub mod provider;
mod provider_client_versions;
mod provider_model_selection;
mod provider_priorities;
pub mod provider_sse;

pub use codex_user_agent::{RUNTIME_CODEX_ORIGINATOR, runtime_codex_user_agent};
pub use provider::{
    CatalogedModelsProvider, ManagedCredential, ModelRuntime, MultiAdapterProvider,
    ProjectedSessionPart, ProviderModelRoute, ProviderRegistry, ProviderRequestHeaderHook,
    catalog_decoration_source, completion_input_part_from_wire, completion_input_provider_state,
    install_request_header_hook, parse_sap_ai_core_service_key, project_completion_input,
    project_operation_output, project_persisted as project_session_parts,
    project_persisted_text_lossy as project_session_text_lossy, with_request_cancellation,
};
pub use provider_client_versions::*;
pub use provider_model_selection::*;
pub use provider_priorities::*;
pub use provider_sse::*;

use agena_provider::ProviderErrorKind;

#[derive(Debug, thiserror::Error)]
/// Error from the provider runtime.
pub enum ProviderError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("serde json error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "{provider} api request failed with status {status}: {body} (kind={kind:?}, retryable={retryable})"
    )]
    HttpStatus {
        provider: String,
        status: reqwest::StatusCode,
        body: String,
        kind: ProviderErrorKind,
        retryable: bool,
    },
    #[error("{provider} provider error: {message} (kind={kind:?}, retryable={retryable})")]
    ProviderClassified {
        provider: String,
        message: String,
        kind: ProviderErrorKind,
        retryable: bool,
    },
    #[error("internal error: {0}")]
    Internal(String),
}

impl ProviderError {
    pub fn retryable(&self) -> bool {
        match self {
            Self::HttpStatus { retryable, .. } | Self::ProviderClassified { retryable, .. } => {
                *retryable
            }
            Self::Http(error) => {
                // Body/decode failures (connection dropped mid-response, invalid
                // chunk encoding) are transient transport conditions exactly like
                // timeouts and connect failures: they must enter the retry loop
                // instead of failing the run immediately.
                error.is_timeout() || error.is_connect() || error.is_body() || error.is_decode()
            }
            _ => false,
        }
    }

    pub fn provider_error_kind(&self) -> Option<ProviderErrorKind> {
        match self {
            Self::HttpStatus { kind, .. } | Self::ProviderClassified { kind, .. } => Some(*kind),
            _ => None,
        }
    }
}

impl From<agena_provider::ToolStreamError> for ProviderError {
    fn from(error: agena_provider::ToolStreamError) -> Self {
        Self::Provider(agena_failure::diagnostic::format_error_chain(&error))
    }
}

impl From<agena_provider::ProviderToolModeViolation> for ProviderError {
    fn from(error: agena_provider::ProviderToolModeViolation) -> Self {
        Self::Provider(agena_failure::diagnostic::format_error_chain(&error))
    }
}

impl From<ProviderJsonStreamError> for ProviderError {
    fn from(error: ProviderJsonStreamError) -> Self {
        match error {
            ProviderJsonStreamError::Http(error) => Self::Http(error),
            // A malformed SSE/JSON-lines payload is normally a transient stream
            // corruption (truncated chunk, proxy mangling) worth resampling,
            // not a permanent rejection of the request. Adapters additionally
            // classify through `utils::json_stream_error` with the real
            // provider id; this fallback keeps the conversion retryable for
            // any remaining `?` call sites.
            ProviderJsonStreamError::InvalidJson { format, source } => Self::ProviderClassified {
                provider: "stream".to_owned(),
                message: format!("invalid {format} payload: {source}"),
                kind: ProviderErrorKind::MalformedResponse,
                retryable: true,
            },
        }
    }
}

impl From<agena_runtime_config::ConfigError> for ProviderError {
    fn from(error: agena_runtime_config::ConfigError) -> Self {
        Self::Config(agena_failure::diagnostic::format_error_chain(&error))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Context of a provider request.
pub struct ProviderRequestContext {
    pub provider_id: String,
    pub model_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_json_stream_error_is_retryable_malformed_response() {
        let err: ProviderError = ProviderJsonStreamError::InvalidJson {
            format: "SSE",
            source: serde_json::from_str::<serde_json::Value>("{broken").unwrap_err(),
        }
        .into();
        assert!(err.retryable());
        assert_eq!(
            err.provider_error_kind(),
            Some(ProviderErrorKind::MalformedResponse)
        );
    }
}
