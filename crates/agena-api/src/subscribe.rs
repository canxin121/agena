//! Subscription request/identifier types shared by WS subscribe / unsubscribe.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::{EventFilter, EventKindTag, Scope};

/// Client-chosen subscription id. Unique per connection. The server echoes
/// this in every notification and the unsubscribe frame.
pub type SubscriptionId = SmolStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeRequest {
    /// Required scope (Global / Workspace / Session).
    pub scope: Scope,
    /// Optional whitelist of event kinds. `None` means "all kinds".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<std::collections::HashSet<EventKindTag>>,
    /// Resume from a specific `seq_global`. Server replays from the persisted
    /// store first, then switches to live broadcast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_seq_global: Option<i64>,
}

impl SubscribeRequest {
    pub fn into_filter(self) -> EventFilter {
        EventFilter {
            scope: self.scope,
            kinds: self.kinds,
            since_seq_global: self.since_seq_global,
        }
    }
}
