//! Push messages from server → client.

use serde::{Deserialize, Serialize};

use crate::live::{RuntimeSignalResource, SessionChangeResource};
use crate::subscribe::SubscriptionId;

/// One subscription delivers committed part patches and ephemeral runtime
/// signals interleaved with optional control frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Notification {
    /// A committed session part/meta mutation arrived.
    SessionChanged {
        subscription: SubscriptionId,
        change: Box<SessionChangeResource>,
    },
    /// A non-persistent runtime signal arrived.
    RuntimeSignal {
        subscription: SubscriptionId,
        signal: Box<RuntimeSignalResource>,
    },
    /// The subscription's broadcast channel dropped `skipped` messages
    /// because this client was too slow. The client must re-read the current
    /// ordered session parts (and compare `sessions.version`) to catch up.
    Lagged {
        subscription: SubscriptionId,
        skipped: u64,
    },
    /// Server-initiated subscription termination (e.g. session deleted).
    SubscriptionClosed {
        subscription: SubscriptionId,
        reason: String,
    },
}
