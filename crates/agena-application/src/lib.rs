//! # agena-application
//!
//! Product use cases and application-facing state.
//!
//! This crate is intentionally transport-free: it exposes an in-process
//! application handle to CLI, TUI, and transport adapters. HTTP, WebSocket,
//! terminal rendering, and process initialization do not belong here.
//!
//! ## Key items
//!
//! - [`Application`] — the in-process application handle used by every
//!   frontend.
//! - [`ApplicationError`] — typed application failures.
//! - [`AuthLoginKind`] — kinds of provider login flows.
//! - [`dto`] — data-transfer objects for frontend resources.
//! - [`service`], [`session`], [`provider_queries`] — use-case services and
//!   projections. The current design dropped the global runtime event projection (D11):
//!   chat history is per-session ordered parts, never a global event log.

mod application;
mod application_archives;
mod application_config;
mod application_content;
mod application_conversations;
mod application_inline_ui;
mod application_models;
mod application_parts;
mod application_plugins;
mod application_provider_studio;
mod application_sessions;
mod application_skills;
mod application_tools;
pub mod dto;
mod error;
pub mod filesystem_discovery;
pub mod pagination;
pub mod part_locale;
pub mod provider_queries;
pub mod provider_studio;
pub mod service;
pub mod session;

pub use application::model_catalog_source_kind_from_domain;
pub use application::{Application, ApplicationSessionServices, AuthLoginKind};
pub use application_content::{ContentDelivery, ContentWatch};
pub use application_parts::OwnedToolPartsPage;
pub use error::ApplicationError;
pub use provider_queries::provider_model_resource_from_domain;
pub use service::{permission_config_domain_from_resource, permission_config_resource_from_domain};

/// Validate public numeric identities before they cross the capability boundary.
pub(crate) fn kernel_session_id(
    id: i64,
) -> Result<agena_storage::kernel::SessionId, ApplicationError> {
    id.try_into()
        .map_err(|error| ApplicationError::bad_request_error(&error))
}
pub(crate) fn kernel_error(error: agena_storage::kernel::KernelError) -> ApplicationError {
    use agena_storage::kernel::KernelError;
    match error {
        KernelError::Invalid(_) => ApplicationError::bad_request_error(&error),
        KernelError::NotFound(_) | KernelError::Ownership => {
            ApplicationError::not_found_with_diagnostic("The resource was not found.", &error)
        }
        KernelError::Conflict(_)
        | KernelError::QueuePaused
        | KernelError::QueueOutOfOrder
        | KernelError::StaleAttempt
        | KernelError::PendingDelivery(_)
        | KernelError::ContentReset
        | KernelError::ContentGap { .. } => ApplicationError::conflict_with_diagnostic(
            "The resource changed. Refresh and retry.",
            &error,
        ),
        KernelError::Busy | KernelError::Closed => {
            ApplicationError::service_unavailable(error.to_string())
        }
        _ => ApplicationError::internal_error(&error),
    }
}
