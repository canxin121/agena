//! Cursor-based pagination types.

use serde::{Deserialize, Serialize};

/// Default page size when the client doesn't specify one.
pub const DEFAULT_LIMIT: u64 = 100;
/// Maximum page size (server clamps anything above this).
pub const MAX_LIMIT: u64 = 1000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Pagination metadata for one page of results.
pub struct PageInfo {
    /// Opaque cursor pointing at the next page; absent when fully consumed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub returned: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A page of items together with its [`PageInfo`].
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub page: PageInfo,
}

#[derive(Debug, Clone, Default, Deserialize)]
/// Cursor-based pagination query parameters.
pub struct PaginationQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
}

pub fn normalize_limit(limit: Option<u64>) -> u64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}
