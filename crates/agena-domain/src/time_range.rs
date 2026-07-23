use serde::{Deserialize, Serialize};

/// Inclusive operation lifecycle timestamps in Unix milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TimeRange {
    pub start_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::TimeRange;

    #[test]
    fn time_range_omits_an_unknown_end_time() {
        assert_eq!(
            serde_json::to_string(&TimeRange {
                start_ms: 42,
                end_ms: None,
            })
            .unwrap(),
            "{\"start_ms\":42}"
        );
    }
}
