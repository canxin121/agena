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
//!   `MessageResource`, `WorkspaceResource`, …) — shared typed resources for
//!   the unified API surface so existing clients can be ported
//!   variant-for-variant.
//! - `commands`: Side-effectful operations the client can invoke
//!   (`Command::SubmitMessage`, `Command::CancelRun`, …). One enum, exhaustive,
//!   `#[serde(tag = "method", content = "params")]`.
//! - `queries`: Read-only requests (list sessions, fetch message, etc.).
//! - `notifications`: Server → client push messages — the stable
//!   [`EventResource`] envelope plus subscription lifecycle
//!   notifications.
//! - `ws`: The duplex WebSocket envelope ([`ClientMessage`] / [`ServerMessage`]).
//! - `pagination`: Cursor-based page/cursor types.
//! - `subscribe`: [`SubscribeRequest`] (scope + kind filter + resume cursor)
//!   and [`SubscriptionId`] for multiplexing many subscriptions over one WS.
//!
//! ## Development contract
//!
//! - `pub const PROTOCOL_VERSION: u32 = 1;` is fixed throughout development.
//! - Server and clients are built against the same current contract.
//! - Breaking changes replace the current contract directly; no older
//!   protocol generations or downgrade behavior are retained.

pub mod commands;
pub mod error;
pub mod message_part;
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
pub const PROTOCOL_VERSION: u32 = 1;

/// Stable, open-ended event-kind identifier used in wire filters.
pub type EventKindTag = smol_str::SmolStr;

/// Routing metadata for one public event envelope.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct EventMetaResource {
    pub id: uuid::Uuid,
    pub seq_global: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq_session: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<uuid::Uuid>,
    pub envelope_schema: u32,
}

/// API-owned event envelope. `kind` is deliberately open-ended and `payload`
/// preserves the current kind-specific JSON shape without exposing a runtime
/// enum to clients.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct EventResource {
    #[serde(flatten)]
    pub meta: EventMetaResource,
    pub kind: EventKindTag,
    pub payload: serde_json::Value,
}
