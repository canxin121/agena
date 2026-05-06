use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Serialize, de::DeserializeOwned};

use super::error::ApiError;

pub const DEFAULT_PAGE_LIMIT: u64 = 50;
pub const MAX_PAGE_LIMIT: u64 = 200;

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

pub fn normalize_limit(limit: Option<u64>) -> u64 {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT)
}

pub fn encode_cursor<T>(value: &T) -> Result<String, ApiError>
where
    T: Serialize,
{
    let json = serde_json::to_vec(value)
        .map_err(|error| ApiError::bad_request(format!("failed to encode cursor: {error}")))?;
    Ok(URL_SAFE_NO_PAD.encode(json))
}

pub fn decode_cursor<T>(value: &str) -> Result<T, ApiError>
where
    T: DeserializeOwned,
{
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|error| ApiError::bad_request(format!("invalid cursor encoding: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ApiError::bad_request(format!("invalid cursor payload: {error}")))
}
