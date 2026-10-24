use serde::{Deserialize, Serialize};

/// Inclusive operation lifecycle timestamps in Unix milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TimeRange {
    pub start_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<i64>,
}
