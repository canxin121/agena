//! Gemini generate-content response wire records.

use serde::Deserialize;
use serde_json::Value;

use crate::{GeminiContent, GeminiUsageMetadata};

#[derive(Debug, Deserialize)]
/// Wire shape of a Gemini generate response.
pub struct GeminiGenerateResponse {
    #[serde(default)]
    pub candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "usageMetadata")]
    pub usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of a Gemini response candidate.
pub struct GeminiCandidate {
    #[serde(default)]
    pub content: Option<GeminiContent>,
    #[serde(default, rename = "finishReason")]
    pub finish_reason: Option<String>,
    #[serde(default, rename = "safetyRatings")]
    pub safety_ratings: Option<Value>,
    #[serde(default, rename = "groundingMetadata")]
    pub grounding_metadata: Option<Value>,
}
