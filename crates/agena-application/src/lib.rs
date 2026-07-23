//! Product use cases and application-facing state.
//!
//! This crate is intentionally transport-free: it exposes an in-process
//! application handle to CLI, TUI, and transport adapters. HTTP, WebSocket,
//! terminal rendering, and process initialization do not belong here.

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
pub use service::message_part_resource_from_runtime;
