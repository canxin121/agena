//! Pagination policy and opaque cursor encoding shared by application services.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Serialize, de::DeserializeOwned};

use crate::ApplicationError;

pub const DEFAULT_PAGE_LIMIT: u64 = 50;
pub const MAX_PAGE_LIMIT: u64 = 200;

/// Normalizes a caller-provided page limit to the product-wide safe range.
pub fn normalize_limit(limit: Option<u64>) -> u64 {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT)
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageInfo {
    pub limit: u64,
    pub returned: usize,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub order: PageOrder,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub page: PageInfo,
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
