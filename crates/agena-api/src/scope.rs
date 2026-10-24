//! Wire-level live-change subscription scope.
//!
//! This is deliberately distinct from the runtime event-store scope. The API
//! owns its serialized contract; transport adapters map it to session-change
//! and runtime-signal notifications at their boundary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// Scope of a live subscription.
pub enum Scope {
    #[default]
    Global,
    Workspace {
        workspace_id: i64,
    },
    Session {
        session_id: i64,
    },
}
