//! Pagination policy and opaque cursor encoding shared by application services.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Serialize, de::DeserializeOwned};

use crate::ApplicationError;

/// Opaque cursor payload for the session-part transcript endpoint. The
/// transport encodes this value and validates the session id before passing
/// the storage position to the persistence facade.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SessionPartCursor {
    pub session_id: i64,
    pub created_at_ms: i64,
    pub part_id: i64,
}

/// Opaque cursor for the presentation-oriented transcript surface. It points
/// to the beginning of a logical run/message boundary, so a later page can
/// skip the folded raw prefix without asking the client to walk it.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SessionTranscriptCursor {
    pub session_id: i64,
    pub created_at_ms: i64,
    pub part_id: i64,
}

/// Cursor used when expanding one server-side folded assistant reply. The
/// run ids are part of the signed/opaque cursor so a client cannot broaden an
/// expansion request to unrelated session content.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionTranscriptFoldCursor {
    pub session_id: i64,
    pub run_ids: Vec<i64>,
    pub created_at_ms: i64,
    pub part_id: i64,
}

/// Default page limit used when no limit is requested.
pub const DEFAULT_PAGE_LIMIT: u64 = 50;
/// Maximum allowed page limit.
pub const MAX_PAGE_LIMIT: u64 = 200;

/// Normalizes a caller-provided page limit to the product-wide safe range.
pub fn normalize_limit(limit: Option<u64>) -> u64 {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT)
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
/// Ordering of a page of results.
pub enum PageOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize)]
/// Pagination metadata for one page of results.
pub struct PageInfo {
    pub limit: u64,
    pub returned: usize,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub order: PageOrder,
}

#[derive(Debug, Clone, Serialize)]
/// A page of items together with its [`PageInfo`].
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub page: PageInfo,
}

/// Converts an application page to the `agena_api::pagination::PaginatedResponse`
/// wire shape used by the WS/IPC protocol. The per-item conversion is a plain
/// field projection; callers pass the item mapping explicitly.
pub fn api_page_from_application<T, U>(
    value: PaginatedResponse<T>,
    mut map_item: impl FnMut(T) -> U,
) -> agena_api::pagination::PaginatedResponse<U> {
    agena_api::pagination::PaginatedResponse {
        items: value.items.into_iter().map(&mut map_item).collect(),
        page: agena_api::pagination::PageInfo {
            next_cursor: value.page.next_cursor,
            has_more: value.page.has_more,
            returned: value.page.returned as u64,
        },
    }
}

pub fn encode_cursor<T>(value: &T) -> Result<String, ApplicationError>
where
    T: Serialize,
{
    let json = serde_json::to_vec(value)
        .map_err(|error| ApplicationError::internal(format!("failed to encode cursor: {error}")))?;
    Ok(URL_SAFE_NO_PAD.encode(json))
}

pub fn decode_cursor<T>(value: &str) -> Result<T, ApplicationError>
where
    T: DeserializeOwned,
{
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|error| {
        ApplicationError::bad_request_with_diagnostic("The page cursor is invalid.", error)
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        ApplicationError::bad_request_with_diagnostic("The page cursor is invalid.", error)
    })
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, normalize_limit};

    #[test]
    fn limits_are_defaulted_and_bounded() {
        assert_eq!(normalize_limit(None), DEFAULT_PAGE_LIMIT);
        assert_eq!(normalize_limit(Some(0)), 1);
        assert_eq!(normalize_limit(Some(MAX_PAGE_LIMIT + 1)), MAX_PAGE_LIMIT);
    }
}
