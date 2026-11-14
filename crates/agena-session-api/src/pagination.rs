//! Cursor-based pagination types.

use serde::{Deserialize, Serialize};

/// Default page size for notification pages.
pub const DEFAULT_LIMIT: u64 = 100;
/// Maximum page size (server clamps anything above this).
pub const MAX_LIMIT: u64 = 1000;

/// Public resource listings (catalog, sessions, workspaces and permissions)
/// retain their established 50-item default and 200-item ceiling. Notification
/// listings deliberately use the larger policy above.
pub const RESOURCE_DEFAULT_LIMIT: u64 = 50;
pub const RESOURCE_MAX_LIMIT: u64 = 200;

pub fn normalize_resource_limit(limit: Option<u64>) -> u64 {
    limit
        .unwrap_or(RESOURCE_DEFAULT_LIMIT)
        .clamp(1, RESOURCE_MAX_LIMIT)
}

/// Ordering of a page of results.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Pagination metadata for one page of results.
pub struct PageInfo {
    /// Effective page size for public resource lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// Resource ordering, when the endpoint exposes ordered pagination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<PageOrder>,
    /// Opaque cursor pointing at the next page; absent when fully consumed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub returned: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A page of items together with its [`PageInfo`].
pub struct PaginatedResponse<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_without_resource_metadata_keep_their_existing_shape() {
        let wire = serde_json::json!({"has_more": false, "returned": 1});
        let page: PageInfo = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(page.limit, None);
        assert_eq!(page.order, None);
        assert_eq!(serde_json::to_value(page).unwrap(), wire);
    }

    #[test]
    fn resource_and_notification_pagination_keep_their_public_limits() {
        for (requested, resource, notification) in [
            (None, 50, 100),
            (Some(0), 1, 1),
            (Some(150), 150, 150),
            (Some(500), 200, 500),
            (Some(u64::MAX), 200, 1000),
        ] {
            assert_eq!(normalize_resource_limit(requested), resource);
            assert_eq!(normalize_limit(requested), notification);
        }
    }
}
