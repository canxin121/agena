//! Subscription request/identifier types shared by WS subscribe / unsubscribe.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::Scope;

/// Client-chosen subscription id. Unique per connection. The server echoes
/// this in every notification and the unsubscribe frame.
pub type SubscriptionId = SmolStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to subscribe to best-effort live changes.
pub struct SubscribeRequest {
    /// Required scope (Global / Workspace / Session). Persisted catch-up is a
    /// separate ordered-parts read; live notifications are never replayed.
    pub scope: Scope,
}
