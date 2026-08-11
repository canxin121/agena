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
//!   projections. v2 dropped the global runtime event projection (D11):
//!   chat history is per-session ordered parts, never a global event log.

mod application;
mod application_config;
mod application_models;
mod application_plugins;
mod application_provider_studio;
mod application_sessions;
pub mod dto;
mod error;
pub mod pagination;
pub mod provider_queries;
pub mod provider_studio;
pub mod service;
pub mod session;

pub use application::{Application, ApplicationSessionServices, AuthLoginKind};
pub use error::ApplicationError;
pub use provider_queries::provider_model_resource_from_domain;
pub use service::{permission_config_domain_from_resource, permission_config_resource_from_domain};
