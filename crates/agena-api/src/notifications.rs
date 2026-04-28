//! Push messages from server → client.

use serde::{Deserialize, Serialize};

use crate::DomainEvent;
use crate::subscribe::SubscriptionId;

/// One subscription delivers many [`Notification::Event`] messages
/// interleaved with optional control frames (lagged, completed, error).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Notification {
    /// A new domain event arrived for the subscription.
    Event {
        subscription: SubscriptionId,
        event: Box<DomainEvent>,
    },
    /// The subscription's broadcast channel dropped `skipped` messages
    /// because this client was too slow. The client may issue a fresh
    /// `Query::ListEvents` with `since_seq_global = last_seen_seq` to backfill.
    Lagged {
        subscription: SubscriptionId,
        skipped: u64,
    },
    /// The subscription has been resumed from the persisted store; the
    /// `up_to_seq_global` field marks the last historical event delivered
    /// before live events resume.
    Resumed {
        subscription: SubscriptionId,
        up_to_seq_global: i64,
    },
    /// Server-initiated subscription termination (e.g. session deleted).
    SubscriptionClosed {
        subscription: SubscriptionId,
        reason: String,
    },
}
