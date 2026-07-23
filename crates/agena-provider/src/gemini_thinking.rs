//! Gemini thinking request policy and wire configuration.

use agena_domain::{ReasoningEffort, ThinkingDisplay, ThinkingRequest};
use serde::Serialize;

/// Gemini's provider-specific thinking configuration wire record.
#[derive(Debug, Serialize)]
pub struct GeminiThinkingConfig {
    #[serde(rename = "thinkingBudget", skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<i32>,
    #[serde(rename = "thinkingLevel", skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<&'static str>,
    #[serde(rename = "includeThoughts", skip_serializing_if = "Option::is_none")]
    pub include_thoughts: Option<bool>,
}

/// Map a provider-neutral thinking request into Gemini's model-specific wire
/// configuration. Unsupported model families intentionally receive no config.
pub fn gemini_thinking_config(
    model: &str,
    thinking: Option<&ThinkingRequest>,
) -> Option<GeminiThinkingConfig> {
    let thinking = thinking?;
    let normalized = model.to_ascii_lowercase();
    let include_thoughts = Some(!matches!(
        thinking,
        ThinkingRequest::Disabled
            | ThinkingRequest::Adaptive {
                display: Some(ThinkingDisplay::Omitted),
                ..
            }
    ));

    if normalized.contains("gemini-2.5") {
        let thinking_budget = match thinking {
            ThinkingRequest::Budget { budget_tokens } => {
                Some(gemini_25_clamp_thinking_budget(&normalized, *budget_tokens))
            }
            ThinkingRequest::Adaptive { effort: None, .. } => Some(-1),
            ThinkingRequest::Adaptive {
                effort: Some(effort),
                ..
            }
            | ThinkingRequest::Effort { effort } => {
                Some(gemini_25_thinking_budget(&normalized, *effort))
            }
            ThinkingRequest::Disabled
                if normalized.contains("pro") && !normalized.contains("flash") =>
            {
                Some(128)
            }
            ThinkingRequest::Disabled => Some(0),
        };
        return Some(GeminiThinkingConfig {
            thinking_budget,
            thinking_level: None,
            include_thoughts,
        });
    }

    if normalized.contains("gemini-3") {
        let thinking_level = match thinking {
            ThinkingRequest::Budget { budget_tokens } if *budget_tokens < 4_000 => Some(
                gemini_3_thinking_level(&normalized, ReasoningEffort::Minimal),
            ),
            ThinkingRequest::Budget { budget_tokens } if *budget_tokens < 12_000 => {
                Some(gemini_3_thinking_level(&normalized, ReasoningEffort::Low))
            }
            ThinkingRequest::Budget { budget_tokens } if *budget_tokens < 24_000 => Some(
                gemini_3_thinking_level(&normalized, ReasoningEffort::Medium),
            ),
            ThinkingRequest::Budget { .. } => {
                Some(gemini_3_thinking_level(&normalized, ReasoningEffort::High))
            }
            ThinkingRequest::Adaptive { effort, .. } => Some(gemini_3_thinking_level(
                &normalized,
                effort.unwrap_or(ReasoningEffort::High),
            )),
            ThinkingRequest::Effort { effort } => {
                Some(gemini_3_thinking_level(&normalized, *effort))
            }
            ThinkingRequest::Disabled => Some(gemini_3_thinking_level(
                &normalized,
                ReasoningEffort::Minimal,
            )),
        };
        return Some(GeminiThinkingConfig {
            thinking_budget: None,
            thinking_level,
            include_thoughts,
        });
    }

    None
}

fn gemini_25_clamp_thinking_budget(model: &str, requested: u32) -> i32 {
    if model.contains("pro") && !model.contains("flash") {
        requested.clamp(128, 32_768) as i32
    } else if model.contains("flash-lite") && requested != 0 {
        requested.clamp(512, 24_576) as i32
    } else {
        requested.min(24_576) as i32
    }
}

fn gemini_25_thinking_budget(model: &str, effort: ReasoningEffort) -> i32 {
    match effort {
        ReasoningEffort::Minimal => 1_024,
        ReasoningEffort::Low => 4_096,
        ReasoningEffort::Medium => 10_240,
        ReasoningEffort::High => 16_384,
        ReasoningEffort::Xhigh | ReasoningEffort::Max
            if model.contains("pro") && !model.contains("flash") =>
        {
            32_768
        }
        ReasoningEffort::Xhigh | ReasoningEffort::Max => 24_576,
    }
}

fn gemini_3_thinking_level(model: &str, effort: ReasoningEffort) -> &'static str {
    let flash_image = model.contains("flash-lite-image") || model.contains("flash-image");
    if model.contains("pro-image") {
        return "HIGH";
    }
    if flash_image {
        return match effort {
            ReasoningEffort::Minimal | ReasoningEffort::Low => "MINIMAL",
            ReasoningEffort::Medium
            | ReasoningEffort::High
            | ReasoningEffort::Xhigh
            | ReasoningEffort::Max => "HIGH",
        };
    }
    let pro = model.contains("pro");
    let legacy_pro = model.contains("gemini-3-pro") && !model.contains("gemini-3.1-pro");
    match effort {
        ReasoningEffort::Minimal if !pro => "MINIMAL",
        ReasoningEffort::Minimal | ReasoningEffort::Low => "LOW",
        ReasoningEffort::Medium if legacy_pro => "LOW",
        ReasoningEffort::Medium => "MEDIUM",
        ReasoningEffort::High | ReasoningEffort::Xhigh | ReasoningEffort::Max => "HIGH",
    }
}
