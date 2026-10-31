//! Pagination policy and opaque cursor encoding shared by application services.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Serialize, de::DeserializeOwned};

use crate::ApplicationError;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
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
        total: value.total,
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
