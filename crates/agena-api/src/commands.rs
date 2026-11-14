//! Side-effectful operations the client can invoke.
//!
//! The request contract lives in [`agena_session_api`] so
//! REST, WebSocket and plugin transports share one declaration. Only the
//! transport-facing result enum lives here.

pub use agena_session_api::commands::*;

use serde::{Deserialize, Serialize};

use crate::resource::{
    BackgroundActivityResource, SessionExecutionResource, SessionResource, WorkspaceResource,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // protocol-shaped enum; boxing breaks wire format
/// Result of executing a [`Command`].
pub enum CommandResult {
    /// Domain handlers serialize their public resource at the wire boundary.
    Data(serde_json::Value),
    Workspace(WorkspaceResource),
    WorkspaceDeleted {
        id: i64,
    },
    Session(SessionResource),
    SessionDeleted {
        id: i64,
    },
    SessionTree(Vec<SessionResource>),
    Execution(SessionExecutionResource),
    Cancellation(agena_domain::CancellationOutcome),
    PermissionRule(crate::resource::PermissionRuleResource),
    PermissionRuleDeleted {
        id: i64,
    },
    Activity(BackgroundActivityResource),
    ActivityDeleted {
        id: String,
    },
    ActivitiesCleared {
        count: usize,
    },
    Ack,
}
