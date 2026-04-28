//! # agena-api
//!
//! Protocol crate for the v2 agena API. Defines the wire-format types shared
//! between the server (`agena-api-server`) and clients (`agena-client`,
//! third-party apps). **No transport**: this crate owns the message shapes,
//! envelopes, and resource projections; how they reach the wire (REST / WS /
//! SSE / Unix socket) is the server crate's job.
//!
//! ## Surface
//!
//! - [`resource`]: REST resource projections (`SessionResource`,
//!   `MessageResource`, `WorkspaceResource`, …) — almost all lifted verbatim
//!   from the legacy `agena-http-api::dto` so existing clients can be ported
//!   variant-for-variant.
//! - [`commands`]: Side-effectful operations the client can invoke
//!   (`Command::SubmitTurn`, `Command::CancelTurn`, …). One enum, exhaustive,
//!   `#[serde(tag = "method", content = "params")]`.
//! - [`queries`]: Read-only requests (list sessions, fetch message, etc.).
//! - [`notifications`]: Server → client push messages — the unified
//!   [`agena::event::DomainEvent`] envelope plus subscription lifecycle
//!   notifications.
//! - [`ws`]: The duplex WebSocket envelope ([`ClientMessage`] / [`ServerMessage`]).
//! - [`pagination`]: Cursor-based page/cursor types.
//! - [`subscribe`]: [`SubscribeRequest`] (scope + kind filter + resume cursor)
//!   and [`SubscriptionId`] for multiplexing many subscriptions over one WS.
//!
//! ## Versioning
//!
//! - `pub const PROTOCOL_VERSION: u32 = 2;` — clients announce this on
//!   connect; server may downgrade behavior if it sees an older version.
//! - Each event payload embeds an `envelope_schema` field
//!   ([`agena_event::ENVELOPE_SCHEMA_VERSION`]); breaking changes go through a
//!   new `EventKindV2(...)` variant rather than mutating an existing payload.

pub mod commands;
pub mod error;
pub mod notifications;
pub mod pagination;
pub mod queries;
pub mod resource;
pub mod subscribe;
pub mod ws;

pub use error::ApiError;

/// Wire protocol version. Bumped on incompatible changes to the WS framing,
/// command set, or query set. Payload-level evolution is handled per-event by
/// `envelope_schema`.
pub const PROTOCOL_VERSION: u32 = 2;

/// Re-export the concrete [`agena::event::DomainEvent`] type so client code
/// only has to depend on `agena-api`.
pub use agena::event::DomainEvent;
pub use agena::event::EventKind;
pub use agena_event::{EventFilter, EventKindTag, Scope};
