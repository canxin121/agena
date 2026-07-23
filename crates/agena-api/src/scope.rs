//! Wire-level event subscription scope.
//!
//! This is deliberately distinct from the runtime event-store scope. The API
//! owns its serialized contract; transport adapters map it to the concrete
//! event implementation at their boundary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

#[cfg(test)]
mod tests {
    use super::Scope;

    #[test]
    fn scope_serialization_is_a_stable_wire_shape() {
        assert_eq!(
            serde_json::to_value(Scope::Workspace { workspace_id: 42 })
                .expect("serialize workspace scope"),
            serde_json::json!({"kind": "workspace", "workspace_id": 42})
        );
        assert_eq!(
            serde_json::from_value::<Scope>(serde_json::json!({
                "kind": "session",
                "session_id": 9
            }))
            .expect("deserialize session scope"),
            Scope::Session { session_id: 9 }
        );
    }
}
