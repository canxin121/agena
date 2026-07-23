use std::fmt;

use serde::{Deserialize, Serialize};

/// A stable request for a model's extended reasoning capability.
///
/// Provider adapters translate this value to their native protocol; the value
/// itself intentionally contains no adapter- or SDK-specific semantics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingRequest {
    #[serde(rename = "budget")]
    Budget {
        budget_tokens: u32,
    },
    Adaptive {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<ReasoningEffort>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<ThinkingDisplay>,
    },
    Effort {
        effort: ReasoningEffort,
    },
    Disabled,
}

/// Ordered semantic effort levels shared by model catalogs and provider requests.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl AsRef<str> for ReasoningEffort {
    fn as_ref(&self) -> &str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

/// The requested visibility of extended reasoning in a user-facing response.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingDisplay {
    Summarized,
    Omitted,
}

impl AsRef<str> for ThinkingDisplay {
    fn as_ref(&self) -> &str {
        match self {
            Self::Summarized => "summarized",
            Self::Omitted => "omitted",
        }
    }
}

impl fmt::Display for ThinkingDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::{ReasoningEffort, ThinkingDisplay, ThinkingRequest};

    #[test]
    fn reasoning_values_preserve_their_wire_contract() {
        assert_eq!(ReasoningEffort::Xhigh.to_string(), "xhigh");
        assert_eq!(ThinkingDisplay::Omitted.as_ref(), "omitted");
        assert_eq!(
            serde_json::to_string(&ThinkingRequest::Adaptive {
                effort: Some(ReasoningEffort::High),
                display: None,
            })
            .unwrap(),
            r#"{"type":"adaptive","effort":"high"}"#
        );
    }
}
