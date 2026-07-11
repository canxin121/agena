use super::{
    AnthropicCacheCreationUsage, AnthropicOutputConfig, AnthropicUsage, CompletionUsage,
    MessageUsage, ThinkingDisplay, ThinkingRequest, Value,
};

pub(crate) fn parse_json_or_string(raw: String) -> Value {
    serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw))
}

pub(crate) fn map_anthropic_usage(u: AnthropicUsage) -> CompletionUsage {
    let cache_write_tokens = u.cache_creation_input_tokens.unwrap_or_else(|| {
        u.cache_creation
            .as_ref()
            .map(AnthropicCacheCreationUsage::total_input_tokens)
            .unwrap_or_default()
    });

    MessageUsage {
        input_tokens: u.input_tokens.unwrap_or_default(),
        output_tokens: u.output_tokens.unwrap_or_default(),
        reasoning_tokens: 0,
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
        self.thinking.is_some()
    }
}

pub(crate) fn anthropic_model_requires_adaptive_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.contains("claude-opus-4-7")
        || normalized.contains("claude-opus-4.7")
        || normalized.contains("claude-mythos-preview")
}

pub(crate) fn anthropic_model_supports_adaptive_thinking(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    anthropic_model_requires_adaptive_thinking(model)
        || normalized.contains("claude-opus-4-6")
        || normalized.contains("claude-opus-4.6")
        || normalized.contains("claude-sonnet-4-6")
        || normalized.contains("claude-sonnet-4.6")
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
        anthropic_model_requires_adaptive_thinking(model).then_some(ThinkingDisplay::Summarized)
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
        output_config: effort.map(|effort| AnthropicOutputConfig {
            effort: Some(match effort {
                crate::provider::ReasoningEffort::Minimal => "minimal",
                crate::provider::ReasoningEffort::Low => "low",
                crate::provider::ReasoningEffort::Medium => "medium",
                crate::provider::ReasoningEffort::High => "high",
                crate::provider::ReasoningEffort::Xhigh => "xhigh",
                crate::provider::ReasoningEffort::Max => "max",
            }),
        }),
    }
}

pub(crate) fn anthropic_enabled_parts(budget_tokens: u32) -> AnthropicThinkingParts {
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
) -> AnthropicThinkingParts {
    match thinking {
        None | Some(ThinkingRequest::Disabled) => AnthropicThinkingParts::default(),
        Some(ThinkingRequest::Budget { budget_tokens }) => {
            if let Some(effort) = anthropic_effort_for_budget(model, *budget_tokens) {
                anthropic_adaptive_parts(model, Some(effort), None)
            } else {
                anthropic_enabled_parts(*budget_tokens)
            }
        }
        Some(ThinkingRequest::Adaptive { effort, display })
            if anthropic_model_supports_adaptive_thinking(model) =>
        {
            anthropic_adaptive_parts(model, *effort, *display)
        }
        Some(ThinkingRequest::Adaptive { effort, .. }) => anthropic_enabled_parts(
            anthropic_budget_for_effort(effort.unwrap_or(crate::provider::ReasoningEffort::High)),
        ),
        Some(ThinkingRequest::Effort { effort })
            if anthropic_model_supports_adaptive_thinking(model) =>
        {
            anthropic_adaptive_parts(model, Some(*effort), None)
        }
        Some(ThinkingRequest::Effort { effort }) => {
            anthropic_enabled_parts(anthropic_budget_for_effort(*effort))
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
    name.trim().to_string()
}
