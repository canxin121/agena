//! Stable payload for an interactive request awaiting a client reply.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{PendingInteractiveRequestKind, PermissionRequest, UserInputRequest};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// A request awaiting user interaction (permission or user input).
pub enum PendingInteractiveRequest {
    Permission {
        #[serde(flatten)]
        request: PermissionRequest,
    },
    UserInput {
        #[serde(flatten)]
        request: UserInputRequest,
    },
}

/// One pending interactive request together with the stable session lineage
/// metadata required by a caller to present or route it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingInteractiveRequestContext {
    pub session_id: i64,
    pub parent_session_id: Option<i64>,
    pub task_id: Option<String>,
    pub request: PendingInteractiveRequest,
}

impl PendingInteractiveRequest {
    pub const fn kind(&self) -> PendingInteractiveRequestKind {
        match self {
            Self::Permission { .. } => PendingInteractiveRequestKind::Permission,
            Self::UserInput { .. } => PendingInteractiveRequestKind::UserInput,
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::Permission { request } => request.request_id.as_str(),
            Self::UserInput { request } => request.request_id.as_str(),
        }
    }

    pub const fn session_id(&self) -> Option<i64> {
        match self {
            Self::Permission { request } => request.session_id,
            Self::UserInput { request } => request.session_id,
        }
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        match self {
            Self::Permission { request } => request.created_at,
            Self::UserInput { request } => request.created_at,
        }
    }

    pub fn as_permission(&self) -> Option<&PermissionRequest> {
        match self {
            Self::Permission { request } => Some(request),
            Self::UserInput { .. } => None,
        }
    }

    pub fn as_user_input(&self) -> Option<&UserInputRequest> {
        match self {
            Self::Permission { .. } => None,
            Self::UserInput { request } => Some(request),
        }
    }
}

impl From<PermissionRequest> for PendingInteractiveRequest {
    fn from(request: PermissionRequest) -> Self {
        Self::Permission { request }
    }
}

impl From<UserInputRequest> for PendingInteractiveRequest {
    fn from(request: UserInputRequest) -> Self {
        Self::UserInput { request }
    }
}
