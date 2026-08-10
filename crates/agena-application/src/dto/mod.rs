//! Data-transfer objects (DTOs) exposed to frontends.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena_storage::MemoryType;

mod access;
mod auth;
mod marketplace;
mod memory;
mod model_catalog;
mod plugins;
mod providers;
mod runtime;
mod sessions;
mod workspaces;

pub use access::*;
pub use agena_api::resource::{
    HealthResponse, PermissionRuleResource, RuntimeBackgroundTaskResource,
    RuntimeBackgroundTaskCancelResponse, RuntimeBackgroundTaskStartResponse,
    ScheduledJobResource, ScheduledJobRunResource, SessionAutomationResource, WorkspaceResource,
};
pub use auth::*;
pub use marketplace::*;
pub use memory::*;
pub use model_catalog::*;
pub use plugins::*;
pub use providers::*;
pub use runtime::*;
pub use sessions::*;
pub use workspaces::*;

#[derive(Debug, Clone, Serialize)]
/// Simple non-paginated item list.
pub struct ItemsResponse<T> {
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Cursor-based pagination query.
pub struct CursorPaginationQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Pagination query with an optional search term.
pub struct SearchPaginationQuery {
    #[serde(flatten)]
    pub pagination: CursorPaginationQuery,
    #[serde(default)]
    pub search: Option<String>,
}

impl CursorPaginationQuery {
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }
}

impl SearchPaginationQuery {
    pub fn cursor(&self) -> Option<&str> {
        self.pagination.cursor()
    }

    pub const fn limit(&self) -> Option<u64> {
        self.pagination.limit()
    }

    pub fn search(&self) -> Option<&str> {
        self.search.as_deref()
    }
}
