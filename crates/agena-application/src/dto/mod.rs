//! Data-transfer objects (DTOs) exposed to frontends.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use agena_storage::MemoryType;

mod access;
mod auth;
mod marketplace;
mod memory;
mod model_catalog;
mod operator;
mod plugins;
mod runtime;
mod sessions;
mod workspaces;

pub use access::*;
pub use agena_api::resource::{
    HealthResponse, PermissionMode, PermissionRuleResource, RuntimeBackgroundTaskCancelResponse,
    RuntimeBackgroundTaskResource, RuntimeBackgroundTaskStartResponse, ScheduledJobResource,
    ScheduledJobRunResource, SessionAutomationResource, WorkspaceResource,
};
pub use auth::*;
pub use marketplace::*;
pub use memory::*;
pub use model_catalog::*;
pub use operator::*;
pub use plugins::*;
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
    // `axum::extract::Query` normally parses numeric form values directly,
    // but this DTO is flattened through `SearchPaginationQuery` and then
    // flattened again by session/workspace queries. At that second flatten
    // boundary serde presents the value as a string, so accept both the form
    // representation and the numeric representation used by tests/other
    // transports.
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    pub limit: Option<u64>,
}

fn deserialize_optional_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        Number(u64),
        String(String),
    }

    Option::<StringOrNumber>::deserialize(deserializer)?
        .map(|value| match value {
            StringOrNumber::Number(value) => Ok(value),
            StringOrNumber::String(value) => value.parse::<u64>().map_err(D::Error::custom),
        })
        .transpose()
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
