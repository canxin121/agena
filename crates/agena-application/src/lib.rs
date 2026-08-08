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
//! - [`dispatch`] — command/query dispatch used by the API server and TUI
//!   backend.
//! - [`dto`] — data-transfer objects for frontend resources.
//! - [`service`], [`session`], [`provider_queries`], [`event_projection`] —
//!   use-case services and projections.

mod application;
pub mod dispatch;
pub mod dto;
mod error;
pub mod event_projection;
pub mod pagination;
pub mod provider_queries;
pub mod service;
pub mod session;

pub use application::{Application, AuthLoginKind};
pub use error::ApplicationError;
pub use provider_queries::provider_model_resource_from_domain;
pub use service::{permission_config_domain_from_resource, permission_config_resource_from_domain};
