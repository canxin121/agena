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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/// A missing mode default inherits, null clears, and a string selects a mode.
pub enum ConfiguredModeDefault {
    #[default]
    Inherit,
    Clear,
    Mode(String),
}

impl ConfiguredModeDefault {
    pub fn is_inherit(&self) -> bool {
        matches!(self, Self::Inherit)
    }

    pub fn mode(&self) -> Option<&str> {
        match self {
            Self::Mode(value) => Some(value),
            _ => None,
        }
    }
}

impl Serialize for ConfiguredModeDefault {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Inherit | Self::Clear => serializer.serialize_none(),
            Self::Mode(value) => serializer.serialize_str(value),
        }
    }
}

impl<'de> Deserialize<'de> for ConfiguredModeDefault {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<String>::deserialize(deserializer)? {
            Some(value) => Self::Mode(value),
            None => Self::Clear,
        })
    }
}

/// Stable strategy for interpreting a configured thinking mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfiguredThinkingStrategy {
    Disabled,
    Effort,
    Budget,
    Adaptive,
    RequestOnly,
}

impl ConfiguredThinkingStrategy {
    pub fn request(
        self,
        effort: Option<ReasoningEffort>,
        budget_tokens: Option<u32>,
        display: Option<ThinkingDisplay>,
    ) -> Option<ThinkingRequest> {
        match self {
            Self::Disabled => Some(ThinkingRequest::Disabled),
            Self::Effort => effort.map(|effort| ThinkingRequest::Effort { effort }),
            Self::Budget => {
                budget_tokens.map(|budget_tokens| ThinkingRequest::Budget { budget_tokens })
            }
            Self::Adaptive => Some(ThinkingRequest::Adaptive { effort, display }),
            Self::RequestOnly => None,
        }
    }
}
