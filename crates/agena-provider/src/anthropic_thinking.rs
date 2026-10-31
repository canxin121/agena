use crate::{
    AnthropicCacheCreationUsage, AnthropicOutputConfig, AnthropicOutputTokensDetails,
    AnthropicTextBlock, AnthropicUsage, CompletionUsage,
};
use agena_domain::{ThinkingDisplay, ThinkingRequest};
use serde_json::Value;

#[derive(Debug, Default)]
/// State of an Anthropic extended-thinking block.
pub struct AnthropicThinkingBlockState {
    pub kind: String,
    pub thinking: String,
    pub signature: Option<String>,
    pub data: Option<String>,
}

impl AnthropicThinkingBlockState {
    pub fn merge_start(&mut self, update: Self) {
        if !self.kind.trim().is_empty()
            && !update.kind.trim().is_empty()
            && self.kind != update.kind
        {
            *self = update;
            return;
        }
        if self.kind.trim().is_empty() && !update.kind.trim().is_empty() {
            self.kind = update.kind;
        }
        merge_thinking_snapshot(&mut self.thinking, update.thinking);
        if update
            .signature
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.signature = update.signature;
        }
        if update
            .data
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.data = update.data;
        }
    }

    pub fn into_value(self) -> Option<Value> {
        thinking_block_value(
            &self.kind,
            &self.thinking,
            self.signature.as_deref(),
            self.data.as_deref(),
        )
    }
}

/// Only signed thinking and nonempty redacted payloads may be replayed.
fn thinking_block_value(
    kind: &str,
    thinking: &str,
    signature: Option<&str>,
    data: Option<&str>,
) -> Option<Value> {
    match kind {
        "thinking" => signature.filter(|value| !value.trim().is_empty()).map(|signature| {
            serde_json::json!({"type": "thinking", "thinking": thinking, "signature": signature})
        }),
        "redacted_thinking" => data.filter(|value| !value.trim().is_empty()).map(|data| {
            serde_json::json!({"type": "redacted_thinking", "data": data})
        }),
        _ => None,
    }
}

fn merge_thinking_snapshot(current: &mut String, update: String) {
    if update.trim().is_empty() || current.starts_with(update.as_str()) {
        return;
    }
    *current = update;
}

pub fn anthropic_thinking_metadata(blocks: &[AnthropicTextBlock]) -> Option<Value> {
    let thinking_blocks = blocks
        .iter()
        .filter_map(|block| {
            thinking_block_value(
                &block.kind,
                block.thinking.as_deref().unwrap_or_default(),
                block.signature.as_deref(),
                block.data.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    (!thinking_blocks.is_empty())
        .then(|| serde_json::json!({ "anthropic_thinking_blocks": thinking_blocks }))
}

pub fn map_anthropic_usage(u: AnthropicUsage) -> CompletionUsage {
    let cache_write_tokens = u.cache_creation_input_tokens.unwrap_or_else(|| {
        u.cache_creation
            .as_ref()
            .map(AnthropicCacheCreationUsage::total_input_tokens)
            .unwrap_or_default()
    });

    let reasoning_tokens = u
        .output_tokens_details
        .as_ref()
        .and_then(|details| details.thinking_tokens)
        .unwrap_or_default();
    let output_tokens = u.output_tokens.unwrap_or_default();

    let cache_write_5m_tokens = u
        .cache_creation
        .as_ref()
        .and_then(|value| value.ephemeral_5m_input_tokens)
        .unwrap_or_default();
    let cache_write_1h_tokens = u
        .cache_creation
        .as_ref()
        .and_then(|value| value.ephemeral_1h_input_tokens)
        .unwrap_or_default();
    CompletionUsage {
        requests: 1,
        input_tokens: u.input_tokens.unwrap_or_default(),
        output_tokens: output_tokens.saturating_sub(reasoning_tokens),
        reasoning_tokens,
        cache_write_tokens,
        cache_write_5m_tokens,
        cache_write_1h_tokens,
        cache_read_tokens: u.cache_read_input_tokens.unwrap_or_default(),
        ..CompletionUsage::default()
    }
}

#[derive(Debug, Default)]
/// Extracted thinking parts and output config from an Anthropic response.
pub struct AnthropicThinkingParts {
    pub thinking: Option<serde_json::Value>,
    pub output_config: Option<AnthropicOutputConfig>,
}

impl AnthropicThinkingParts {
    pub fn include_thinking(&self) -> bool {
        self.thinking
            .as_ref()
            .and_then(|thinking| thinking.get("type"))
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "disabled")
    }
}

pub fn anthropic_model_requires_adaptive_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-opus-4-7")
        || normalized.contains("claude-opus-4.7")
        || normalized.contains("claude-opus-4-8")
        || normalized.contains("claude-opus-4.8")
        || normalized.contains("claude-sonnet-5")
        || normalized.contains("claude-fable-5")
        || normalized.contains("claude-mythos-5")
}

pub fn anthropic_model_supports_adaptive_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    anthropic_model_requires_adaptive_thinking(model)
        || normalized.contains("claude-mythos-preview")
        || normalized.contains("claude-opus-4-6")
        || normalized.contains("claude-opus-4.6")
        || normalized.contains("claude-sonnet-4-6")
        || normalized.contains("claude-sonnet-4.6")
}

pub fn anthropic_model_supports_xhigh_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-fable-5")
        || normalized.contains("claude-mythos-5")
        || normalized.contains("claude-opus-4-8")
        || normalized.contains("claude-opus-4.8")
        || normalized.contains("claude-opus-4-7")
        || normalized.contains("claude-opus-4.7")
        || normalized.contains("claude-sonnet-5")
}

pub fn anthropic_model_supports_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    anthropic_model_supports_adaptive_thinking(model)
        || normalized.contains("claude-opus-4-5")
        || normalized.contains("claude-opus-4.5")
}

pub fn anthropic_model_supports_max_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-fable-5")
        || normalized.contains("claude-mythos-5")
        || normalized.contains("claude-mythos-preview")
        || normalized.contains("claude-opus-4-8")
        || normalized.contains("claude-opus-4.8")
        || normalized.contains("claude-opus-4-7")
        || normalized.contains("claude-opus-4.7")
        || normalized.contains("claude-opus-4-6")
        || normalized.contains("claude-opus-4.6")
        || normalized.contains("claude-sonnet-5")
        || normalized.contains("claude-sonnet-4-6")
        || normalized.contains("claude-sonnet-4.6")
}

pub fn anthropic_model_defaults_to_omitted_thinking(model: &str) -> bool {
    anthropic_model_requires_adaptive_thinking(model)
        || model.to_ascii_lowercase().contains("claude-mythos-preview")
}

pub fn anthropic_model_rejects_disabled_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-fable-5")
        || normalized.contains("claude-mythos-5")
        || normalized.contains("claude-mythos-preview")
}

pub fn anthropic_model_rejects_sampling(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-fable-5")
        || normalized.contains("claude-mythos-5")
        || normalized.contains("claude-mythos-preview")
        || normalized.contains("claude-opus-4-8")
        || normalized.contains("claude-opus-4.8")
        || normalized.contains("claude-opus-4-7")
        || normalized.contains("claude-opus-4.7")
        || normalized.contains("claude-sonnet-5")
}

pub fn anthropic_budget_for_effort(effort: agena_domain::ReasoningEffort) -> u32 {
    match effort {
        agena_domain::ReasoningEffort::Minimal => 1_024,
        agena_domain::ReasoningEffort::Low => 4_000,
        agena_domain::ReasoningEffort::Medium => 10_000,
        agena_domain::ReasoningEffort::High => 16_000,
        agena_domain::ReasoningEffort::Xhigh | agena_domain::ReasoningEffort::Max => 31_999,
    }
}

/// Map a token budget to effort without applying platform/model eligibility.
pub fn anthropic_budget_effort(budget_tokens: u32) -> agena_domain::ReasoningEffort {
    if budget_tokens <= 4_000 {
        agena_domain::ReasoningEffort::Low
    } else if budget_tokens <= 10_000 {
        agena_domain::ReasoningEffort::Medium
    } else if budget_tokens <= 16_000 {
        agena_domain::ReasoningEffort::High
    } else if budget_tokens < 31_999 {
        agena_domain::ReasoningEffort::Xhigh
    } else {
        agena_domain::ReasoningEffort::Max
    }
}

pub fn anthropic_effort_for_budget(
    model: &str,
    budget_tokens: u32,
) -> Option<agena_domain::ReasoningEffort> {
    anthropic_model_requires_adaptive_thinking(model)
        .then(|| anthropic_budget_effort(budget_tokens))
}

/// Reserve an output token while enforcing Anthropic's minimum thinking budget.
pub fn anthropic_thinking_budget(budget_tokens: u32, max_output_tokens: u32) -> Option<u32> {
    const MIN_THINKING_BUDGET: u32 = 1_024;
    (max_output_tokens > MIN_THINKING_BUDGET).then(|| {
        budget_tokens
            .max(MIN_THINKING_BUDGET)
            .min(max_output_tokens - 1)
    })
}

pub fn anthropic_default_display(
    model: &str,
    explicit: Option<ThinkingDisplay>,
) -> Option<ThinkingDisplay> {
    explicit.or_else(|| {
        anthropic_model_defaults_to_omitted_thinking(model).then_some(ThinkingDisplay::Summarized)
    })
}

fn anthropic_wire_effort(model: &str, effort: agena_domain::ReasoningEffort) -> &'static str {
    match effort {
        agena_domain::ReasoningEffort::Minimal | agena_domain::ReasoningEffort::Low => "low",
        agena_domain::ReasoningEffort::Medium => "medium",
        agena_domain::ReasoningEffort::High => "high",
        agena_domain::ReasoningEffort::Xhigh if anthropic_model_supports_xhigh_effort(model) => {
            "xhigh"
        }
        agena_domain::ReasoningEffort::Xhigh if anthropic_model_supports_max_effort(model) => "max",
        agena_domain::ReasoningEffort::Max if anthropic_model_supports_max_effort(model) => "max",
        agena_domain::ReasoningEffort::Xhigh | agena_domain::ReasoningEffort::Max => "high",
    }
}

fn anthropic_output_config(
    model: &str,
    effort: Option<agena_domain::ReasoningEffort>,
) -> Option<AnthropicOutputConfig> {
    effort
        .filter(|_| anthropic_model_supports_effort(model))
        .map(|effort| AnthropicOutputConfig {
            effort: Some(anthropic_wire_effort(model, effort)),
        })
}

pub fn anthropic_adaptive_parts(
    model: &str,
    effort: Option<agena_domain::ReasoningEffort>,
    display: Option<ThinkingDisplay>,
) -> AnthropicThinkingParts {
    let display = anthropic_default_display(model, display);
    let mut thinking = serde_json::Map::new();
    thinking.insert(
        "type".to_owned(),
        serde_json::Value::String("adaptive".to_owned()),
    );
    if let Some(display) = display {
        thinking.insert(
            "display".to_owned(),
            serde_json::Value::String(display.to_string()),
        );
    }

    AnthropicThinkingParts {
        thinking: Some(serde_json::Value::Object(thinking)),
        output_config: effort.and_then(|effort| anthropic_output_config(model, Some(effort))),
    }
}

pub fn anthropic_enabled_parts(
    budget_tokens: u32,
    max_output_tokens: u32,
) -> AnthropicThinkingParts {
    let Some(budget_tokens) = anthropic_thinking_budget(budget_tokens, max_output_tokens) else {
        return AnthropicThinkingParts::default();
    };
    AnthropicThinkingParts {
        thinking: Some(serde_json::json!({
            "type": "enabled",
            "budget_tokens": budget_tokens
        })),
        output_config: None,
    }
}

pub fn anthropic_thinking_parts(
    model: &str,
    thinking: Option<&ThinkingRequest>,
    max_output_tokens: u32,
) -> AnthropicThinkingParts {
    match thinking {
        None => AnthropicThinkingParts::default(),
        Some(ThinkingRequest::Disabled) if anthropic_model_rejects_disabled_thinking(model) => {
            AnthropicThinkingParts::default()
        }
        Some(ThinkingRequest::Disabled) => AnthropicThinkingParts {
            thinking: Some(serde_json::json!({ "type": "disabled" })),
            output_config: None,
        },
        Some(ThinkingRequest::Budget { budget_tokens }) => {
            if let Some(effort) = anthropic_effort_for_budget(model, *budget_tokens) {
                anthropic_adaptive_parts(model, Some(effort), None)
            } else {
                anthropic_enabled_parts(*budget_tokens, max_output_tokens)
            }
        }
        Some(ThinkingRequest::Adaptive { effort, display })
            if anthropic_model_supports_adaptive_thinking(model) =>
        {
            anthropic_adaptive_parts(model, *effort, *display)
        }
        Some(ThinkingRequest::Adaptive { effort, .. }) => {
            let mut parts = anthropic_enabled_parts(
                anthropic_budget_for_effort(effort.unwrap_or(agena_domain::ReasoningEffort::High)),
                max_output_tokens,
            );
            parts.output_config = anthropic_output_config(model, *effort);
            parts
        }
        Some(ThinkingRequest::Effort { effort })
            if anthropic_model_supports_adaptive_thinking(model) =>
        {
            anthropic_adaptive_parts(model, Some(*effort), None)
        }
        Some(ThinkingRequest::Effort { effort }) => {
            let mut parts =
                anthropic_enabled_parts(anthropic_budget_for_effort(*effort), max_output_tokens);
            parts.output_config = anthropic_output_config(model, Some(*effort));
            parts
        }
    }
}

pub fn merge_anthropic_usage(
    current: Option<AnthropicUsage>,
    update: AnthropicUsage,
) -> AnthropicUsage {
    let Some(current) = current else {
        return update;
    };

    AnthropicUsage {
        input_tokens: update.input_tokens.or(current.input_tokens),
        output_tokens: update.output_tokens.or(current.output_tokens),
        output_tokens_details: match (current.output_tokens_details, update.output_tokens_details) {
            (Some(current), Some(update)) => Some(AnthropicOutputTokensDetails {
                thinking_tokens: update.thinking_tokens.or(current.thinking_tokens),
            }),
            (None, Some(update)) => Some(update),
            (Some(current), None) => Some(current),
            (None, None) => None,
        },
        cache_creation_input_tokens: update
            .cache_creation_input_tokens
            .or(current.cache_creation_input_tokens),
        cache_read_input_tokens: update
            .cache_read_input_tokens
            .or(current.cache_read_input_tokens),
        cache_creation: merge_anthropic_cache_creation_usage(
            current.cache_creation,
            update.cache_creation,
        ),
    }
}

pub fn merge_anthropic_cache_creation_usage(
    current: Option<AnthropicCacheCreationUsage>,
    update: Option<AnthropicCacheCreationUsage>,
) -> Option<AnthropicCacheCreationUsage> {
    match (current, update) {
        (Some(current), Some(update)) => Some(AnthropicCacheCreationUsage {
            ephemeral_1h_input_tokens: update
                .ephemeral_1h_input_tokens
                .or(current.ephemeral_1h_input_tokens),
            ephemeral_5m_input_tokens: update
                .ephemeral_5m_input_tokens
                .or(current.ephemeral_5m_input_tokens),
        }),
        (None, Some(update)) => Some(update),
        (Some(current), None) => Some(current),
        (None, None) => None,
    }
}

pub fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub fn anthropic_wire_tool_name(name: &str) -> String {
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::ReasoningEffort;
    use serde_json::json;

    #[test]
    fn thinking_metadata_preserves_signed_block_order_and_omits_invalid_blocks() {
        let values = json!([
            {"type": "text", "text": "answer"},
            {"type": "thinking", "thinking": "unsigned"},
            {"type": "thinking", "signature": " ", "thinking": "invalid"},
            {"type": "thinking", "signature": "signed", "thinking": ""},
            {"type": "redacted_thinking", "data": "opaque"},
            {"type": "redacted_thinking", "data": ""}
        ]);
        let blocks: Vec<AnthropicTextBlock> = serde_json::from_value(values).unwrap();
        assert_eq!(
            anthropic_thinking_metadata(&blocks),
            Some(json!({
                "anthropic_thinking_blocks": [
                    {"type": "thinking", "thinking": "", "signature": "signed"},
                    {"type": "redacted_thinking", "data": "opaque"}
                ]
            }))
        );
        let state = AnthropicThinkingBlockState {
            kind: "thinking".into(),
            signature: Some("signed".into()),
            ..Default::default()
        };
        assert_eq!(
            state.into_value(),
            Some(json!({
                "type": "thinking", "thinking": "", "signature": "signed"
            }))
        );
    }

    #[test]
    fn thinking_state_keeps_longer_snapshot_and_resets_when_block_kind_changes() {
        let mut state = AnthropicThinkingBlockState {
            kind: "thinking".into(),
            thinking: "full thought".into(),
            signature: Some("signed".into()),
            ..Default::default()
        };
        state.merge_start(AnthropicThinkingBlockState {
            kind: "thinking".into(),
            thinking: "full".into(),
            ..Default::default()
        });
        assert_eq!(state.thinking, "full thought");
        state.merge_start(AnthropicThinkingBlockState {
            kind: "redacted_thinking".into(),
            data: Some("opaque".into()),
            ..Default::default()
        });
        assert_eq!(
            state.into_value(),
            Some(json!({"type": "redacted_thinking", "data": "opaque"}))
        );
    }

    #[test]
    fn budget_thresholds_and_output_reservation_are_shared_protocol_rules() {
        for (budget, effort) in [
            (0, ReasoningEffort::Low),
            (4000, ReasoningEffort::Low),
            (4001, ReasoningEffort::Medium),
            (10000, ReasoningEffort::Medium),
            (10001, ReasoningEffort::High),
            (16000, ReasoningEffort::High),
            (16001, ReasoningEffort::Xhigh),
            (31998, ReasoningEffort::Xhigh),
            (31999, ReasoningEffort::Max),
            (u32::MAX, ReasoningEffort::Max),
        ] {
            assert_eq!(anthropic_budget_effort(budget), effort);
        }
        assert_eq!(anthropic_thinking_budget(16000, 0), None);
        assert_eq!(anthropic_thinking_budget(16000, 1024), None);
        assert_eq!(anthropic_thinking_budget(0, 1025), Some(1024));
        assert_eq!(anthropic_thinking_budget(16000, 2048), Some(2047));
    }
}
