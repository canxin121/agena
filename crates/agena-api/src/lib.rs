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
//! - `resource`: REST resource projections (`SessionResource`,
//!   `RunResource`, `WorkspaceResource`, …) — shared typed resources for
//!   the unified API surface so existing clients can be ported
//!   variant-for-variant.
//! - `commands`: Side-effectful operations the client can invoke
//!   (`Command::SubmitMessage`, `Command::CancelRun`, …). One enum, exhaustive,
//!   `#[serde(tag = "method", content = "params")]`.
//! - `queries`: Read-only requests (list sessions, fetch message, etc.).
//! - `notifications`: Server → client part patches and ephemeral runtime
//!   signals plus subscription lifecycle notifications.
//! - `ws`: The duplex WebSocket envelope ([`ClientMessage`] / [`ServerMessage`]).
//! - `pagination`: Cursor-based page/cursor types.
//! - `subscribe`: [`SubscribeRequest`] (scope + kind filter + resume cursor)
//!   and [`SubscriptionId`] for multiplexing many subscriptions over one WS.
//!
//! ## Development contract
//!
//! - `pub const PROTOCOL_VERSION: u32 = 2;` is the failure-semantics contract.
//! - Server and clients are built against the same current contract.
//! - Breaking changes replace the current contract directly; no older
//!   protocol generations or downgrade behavior are retained.

pub mod commands;
pub mod error;
pub mod live;
pub mod part;
pub mod notifications;
pub mod pagination;
pub mod queries;
pub mod resource;
pub mod scope;
pub mod subscribe;
pub mod ws;

pub use error::ApiError;
pub use scope::Scope;

/// Fixed development wire contract. Do not increment this during development;
/// change server and clients together against the one current format.
pub const PROTOCOL_VERSION: u32 = 2;
