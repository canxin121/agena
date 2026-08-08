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
    catalog_decoration_source, install_request_header_hook, parse_sap_ai_core_service_key,
    project_completion_input, project_operation_output as project_session_tool_result_output,
    project_persisted as project_session_parts,
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
            Self::Http(error) => error.is_timeout() || error.is_connect(),
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
        Self::Provider(error.to_string())
    }
}

impl From<agena_provider::ProviderToolModeViolation> for ProviderError {
    fn from(error: agena_provider::ProviderToolModeViolation) -> Self {
        Self::Provider(error.to_string())
    }
}

impl From<ProviderJsonStreamError> for ProviderError {
    fn from(error: ProviderJsonStreamError) -> Self {
        Self::Provider(error.to_string())
    }
}

impl From<agena_runtime_config::ConfigError> for ProviderError {
    fn from(error: agena_runtime_config::ConfigError) -> Self {
        Self::Config(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Context of a provider request.
pub struct ProviderRequestContext {
    pub provider_id: String,
    pub model_id: String,
}
