//! Stable error envelope. Keeps the server free to map internal errors to
//! these structured codes.

use serde::{Deserialize, Serialize};

/// Categorised error code. Stable across versions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Request payload couldn't be parsed or violated invariants.
    BadRequest,
    /// Resource doesn't exist (or caller can't see it).
    NotFound,
    /// Optimistic concurrency conflict (e.g. session version mismatch).
    Conflict,
    /// Caller may not perform this operation.
    Forbidden,
    /// Caller is unauthenticated.
    Unauthorized,
    /// Required service (DB, provider) is unreachable.
    ServiceUnavailable,
    /// Catch-all for unexpected internal errors.
    Internal,
    /// Client violated the WS protocol (e.g. unknown frame type).
    Protocol,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    /// Optional structured details that the server may attach (e.g. validation
    /// errors per field). Always JSON; clients decide whether to render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::BadRequest,
            message: msg.into(),
            details: None,
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::NotFound,
            message: msg.into(),
            details: None,
        }
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Conflict,
            message: msg.into(),
            details: None,
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Internal,
            message: msg.into(),
            details: None,
        }
    }

    pub fn protocol(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Protocol,
            message: msg.into(),
            details: None,
        }
    }

    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::ServiceUnavailable,
            message: msg.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}
