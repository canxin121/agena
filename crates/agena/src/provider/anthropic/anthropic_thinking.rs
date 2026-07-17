use super::{
    AnthropicCacheCreationUsage, AnthropicOutputConfig, AnthropicTextBlock, AnthropicUsage,
    CompletionUsage, MessageUsage, ThinkingDisplay, ThinkingRequest, Value,
};

#[derive(Debug, Default)]
pub(crate) struct AnthropicThinkingBlockState {
    pub(crate) kind: String,
    pub(crate) thinking: String,
    pub(crate) signature: Option<String>,
    pub(crate) data: Option<String>,
}

impl AnthropicThinkingBlockState {
    pub(crate) fn into_value(self) -> Option<Value> {
        match self.kind.as_str() {
            "thinking" => self
                .signature
                .filter(|signature| !signature.is_empty())
                .map(|signature| {
                    serde_json::json!({
                        "type": "thinking",
                        "thinking": self.thinking,
                        "signature": signature,
                    })
                }),
            "redacted_thinking" => self.data.filter(|data| !data.is_empty()).map(|data| {
                serde_json::json!({
                    "type": "redacted_thinking",
                    "data": data,
                })
            }),
            _ => None,
        }
    }
}

pub(crate) fn anthropic_thinking_metadata(blocks: &[AnthropicTextBlock]) -> Option<Value> {
    let thinking_blocks = blocks
        .iter()
        .filter_map(|block| match block.kind.as_str() {
            "thinking"
                if block
                    .signature
                    .as_deref()
                    .is_some_and(|signature| !signature.is_empty()) =>
            {
                Some(serde_json::json!({
                    "type": "thinking",
                    "thinking": block.thinking.as_deref().unwrap_or_default(),
                    "signature": block.signature.as_deref().unwrap_or_default(),
                }))
            }
            "redacted_thinking" if block.data.as_deref().is_some_and(|data| !data.is_empty()) => {
                Some(serde_json::json!({
                    "type": "redacted_thinking",
                    "data": block.data.as_deref().unwrap_or_default(),
                }))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    (!thinking_blocks.is_empty())
        .then(|| serde_json::json!({ "anthropic_thinking_blocks": thinking_blocks }))
}

pub(crate) fn map_anthropic_usage(u: AnthropicUsage) -> CompletionUsage {
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

    MessageUsage {
        input_tokens: u.input_tokens.unwrap_or_default(),
        output_tokens: output_tokens.saturating_sub(reasoning_tokens),
        reasoning_tokens,
        cache_write_tokens,
        cache_read_tokens: u.cache_read_input_tokens.unwrap_or_default(),
        total_cost: 0.0,
    }
    .into()
}

#[derive(Debug, Default)]
pub(crate) struct AnthropicThinkingParts {
    pub(crate) thinking: Option<serde_json::Value>,
    pub(crate) output_config: Option<AnthropicOutputConfig>,
}

impl AnthropicThinkingParts {
    pub(crate) fn include_thinking(&self) -> bool {
        self.thinking
            .as_ref()
            .and_then(|thinking| thinking.get("type"))
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "disabled")
    }
}

pub(crate) fn anthropic_model_requires_adaptive_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-opus-4-7")
        || normalized.contains("claude-opus-4.7")
        || normalized.contains("claude-opus-4-8")
        || normalized.contains("claude-opus-4.8")
        || normalized.contains("claude-sonnet-5")
        || normalized.contains("claude-fable-5")
        || normalized.contains("claude-mythos-5")
}

pub(crate) fn anthropic_model_supports_adaptive_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    anthropic_model_requires_adaptive_thinking(model)
        || normalized.contains("claude-mythos-preview")
        || normalized.contains("claude-opus-4-6")
        || normalized.contains("claude-opus-4.6")
        || normalized.contains("claude-sonnet-4-6")
        || normalized.contains("claude-sonnet-4.6")
}

pub(crate) fn anthropic_model_supports_xhigh_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-fable-5")
        || normalized.contains("claude-mythos-5")
        || normalized.contains("claude-opus-4-8")
        || normalized.contains("claude-opus-4.8")
        || normalized.contains("claude-opus-4-7")
        || normalized.contains("claude-opus-4.7")
        || normalized.contains("claude-sonnet-5")
}

pub(crate) fn anthropic_model_supports_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    anthropic_model_supports_adaptive_thinking(model)
        || normalized.contains("claude-opus-4-5")
        || normalized.contains("claude-opus-4.5")
}

pub(crate) fn anthropic_model_supports_max_effort(model: &str) -> bool {
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

pub(crate) fn anthropic_model_defaults_to_omitted_thinking(model: &str) -> bool {
    anthropic_model_requires_adaptive_thinking(model)
        || model.to_ascii_lowercase().contains("claude-mythos-preview")
}

pub(crate) fn anthropic_model_rejects_disabled_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-fable-5")
        || normalized.contains("claude-mythos-5")
        || normalized.contains("claude-mythos-preview")
}

pub(crate) fn anthropic_model_rejects_sampling(model: &str) -> bool {
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

pub(crate) fn anthropic_budget_for_effort(effort: crate::provider::ReasoningEffort) -> u32 {
    match effort {
        crate::provider::ReasoningEffort::Minimal => 1_024,
        crate::provider::ReasoningEffort::Low => 4_000,
        crate::provider::ReasoningEffort::Medium => 10_000,
        crate::provider::ReasoningEffort::High => 16_000,
        crate::provider::ReasoningEffort::Xhigh | crate::provider::ReasoningEffort::Max => 31_999,
    }
}

pub(crate) fn anthropic_effort_for_budget(
    model: &str,
    budget_tokens: u32,
) -> Option<crate::provider::ReasoningEffort> {
    anthropic_model_requires_adaptive_thinking(model).then_some(if budget_tokens <= 4_000 {
        crate::provider::ReasoningEffort::Low
    } else if budget_tokens <= 10_000 {
        crate::provider::ReasoningEffort::Medium
    } else if budget_tokens <= 16_000 {
        crate::provider::ReasoningEffort::High
    } else if budget_tokens < 31_999 {
        crate::provider::ReasoningEffort::Xhigh
    } else {
        crate::provider::ReasoningEffort::Max
    })
}

pub(crate) fn anthropic_default_display(
    model: &str,
    explicit: Option<ThinkingDisplay>,
) -> Option<ThinkingDisplay> {
    explicit.or_else(|| {
        anthropic_model_defaults_to_omitted_thinking(model).then_some(ThinkingDisplay::Summarized)
    })
}

fn anthropic_wire_effort(model: &str, effort: crate::provider::ReasoningEffort) -> &'static str {
    match effort {
        crate::provider::ReasoningEffort::Minimal | crate::provider::ReasoningEffort::Low => "low",
        crate::provider::ReasoningEffort::Medium => "medium",
        crate::provider::ReasoningEffort::High => "high",
        crate::provider::ReasoningEffort::Xhigh if anthropic_model_supports_xhigh_effort(model) => {
            "xhigh"
        }
        crate::provider::ReasoningEffort::Xhigh if anthropic_model_supports_max_effort(model) => {
            "max"
        }
        crate::provider::ReasoningEffort::Max if anthropic_model_supports_max_effort(model) => {
            "max"
        }
        crate::provider::ReasoningEffort::Xhigh | crate::provider::ReasoningEffort::Max => "high",
    }
}

fn anthropic_output_config(
    model: &str,
    effort: Option<crate::provider::ReasoningEffort>,
) -> Option<AnthropicOutputConfig> {
    effort
        .filter(|_| anthropic_model_supports_effort(model))
        .map(|effort| AnthropicOutputConfig {
            effort: Some(anthropic_wire_effort(model, effort)),
        })
}

pub(crate) fn anthropic_adaptive_parts(
    model: &str,
    effort: Option<crate::provider::ReasoningEffort>,
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

pub(crate) fn anthropic_enabled_parts(
    budget_tokens: u32,
    max_output_tokens: u32,
) -> AnthropicThinkingParts {
    const MIN_THINKING_BUDGET: u32 = 1_024;
    if max_output_tokens <= MIN_THINKING_BUDGET {
        return AnthropicThinkingParts::default();
    }
    let budget_tokens = budget_tokens
        .max(MIN_THINKING_BUDGET)
        .min(max_output_tokens - 1);
    AnthropicThinkingParts {
        thinking: Some(serde_json::json!({
            "type": "enabled",
            "budget_tokens": budget_tokens
        })),
        output_config: None,
    }
}

pub(crate) fn anthropic_thinking_parts(
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
                anthropic_budget_for_effort(
                    effort.unwrap_or(crate::provider::ReasoningEffort::High),
                ),
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

pub(crate) fn merge_anthropic_usage(
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
            (Some(current), Some(update)) => Some(super::AnthropicOutputTokensDetails {
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

pub(crate) fn merge_anthropic_cache_creation_usage(
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

pub(crate) fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub(crate) fn anthropic_wire_tool_name(name: &str) -> String {
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ReasoningEffort;

    #[test]
    fn manual_thinking_budget_stays_within_anthropic_limits() {
        let minimum = anthropic_enabled_parts(1, 4096);
        assert_eq!(minimum.thinking.unwrap()["budget_tokens"], 1024);

        let clamped = anthropic_enabled_parts(20_000, 4096);
        assert_eq!(clamped.thinking.unwrap()["budget_tokens"], 4095);

        let impossible = anthropic_enabled_parts(1024, 1024);
        assert!(!impossible.include_thinking());
    }

    #[test]
    fn adaptive_effort_uses_current_claude_wire_values() {
        assert!(anthropic_model_requires_adaptive_thinking(
            "claude-opus-4-8"
        ));
        assert!(anthropic_model_supports_adaptive_thinking(
            "claude-sonnet-4-6"
        ));

        let max = anthropic_adaptive_parts(
            "claude-opus-4-8",
            Some(ReasoningEffort::Max),
            Some(ThinkingDisplay::Omitted),
        );
        assert_eq!(max.thinking.unwrap()["type"], "adaptive");
        assert_eq!(max.output_config.unwrap().effort, Some("max"));

        let xhigh = anthropic_adaptive_parts("claude-opus-4-8", Some(ReasoningEffort::Xhigh), None);
        assert_eq!(xhigh.output_config.unwrap().effort, Some("xhigh"));

        let legacy_xhigh =
            anthropic_adaptive_parts("claude-sonnet-4-6", Some(ReasoningEffort::Xhigh), None);
        assert_eq!(legacy_xhigh.output_config.unwrap().effort, Some("max"));

        let opus_45 = anthropic_thinking_parts(
            "claude-opus-4-5",
            Some(&ThinkingRequest::Effort {
                effort: ReasoningEffort::Max,
            }),
            32_000,
        );
        assert_eq!(opus_45.thinking.unwrap()["type"], "enabled");
        assert_eq!(opus_45.output_config.unwrap().effort, Some("high"));

        assert!(anthropic_model_rejects_sampling("claude-opus-4.8"));
        assert!(anthropic_model_rejects_sampling("claude-mythos-preview"));
        assert!(anthropic_model_rejects_sampling("claude-sonnet-5"));
        assert!(!anthropic_model_rejects_sampling("claude-sonnet-4.6"));
    }

    #[test]
    fn disabled_thinking_uses_each_models_official_behavior() {
        let sonnet_5 =
            anthropic_thinking_parts("claude-sonnet-5", Some(&ThinkingRequest::Disabled), 4_096);
        assert_eq!(sonnet_5.thinking.as_ref().unwrap()["type"], "disabled");
        assert!(!sonnet_5.include_thinking());

        let always_on =
            anthropic_thinking_parts("claude-fable-5", Some(&ThinkingRequest::Disabled), 4_096);
        assert!(always_on.thinking.is_none());

        let legacy =
            anthropic_thinking_parts("claude-opus-4-5", Some(&ThinkingRequest::Disabled), 4_096);
        assert_eq!(legacy.thinking.as_ref().unwrap()["type"], "disabled");
        assert!(!legacy.include_thinking());
    }

    #[test]
    fn usage_splits_inclusive_output_and_thinking_tokens() {
        let usage = map_anthropic_usage(AnthropicUsage {
            input_tokens: Some(100),
            output_tokens: Some(80),
            output_tokens_details: Some(super::super::AnthropicOutputTokensDetails {
                thinking_tokens: Some(30),
            }),
            cache_creation_input_tokens: Some(7),
            cache_read_input_tokens: Some(11),
            cache_creation: None,
        });

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.reasoning_tokens, 30);
        assert_eq!(usage.cache_write_tokens, 7);
        assert_eq!(usage.cache_read_tokens, 11);
    }
}
