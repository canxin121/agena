use agena_domain::{ReasoningEffort, ThinkingRequest};

pub fn openai_chat_reasoning_effort(
    thinking: Option<&ThinkingRequest>,
    model: &str,
) -> Option<String> {
    if !openai_chat_supports_reasoning_effort(model) {
        return None;
    }
    match thinking {
        Some(ThinkingRequest::Effort { effort }) => {
            Some(openai_compatible_reasoning_effort(model, *effort).to_owned())
        }
        Some(ThinkingRequest::Adaptive { effort, .. }) => Some(
            openai_compatible_reasoning_effort(model, (*effort).unwrap_or(ReasoningEffort::High))
                .to_owned(),
        ),
        Some(ThinkingRequest::Budget { budget_tokens }) => Some(
            if *budget_tokens > 10_000 {
                "high"
            } else if *budget_tokens > 3_000 {
                "medium"
            } else {
                "low"
            }
            .to_owned(),
        ),
        Some(ThinkingRequest::Disabled) if supports_none_reasoning_effort(model) => {
            Some("none".to_owned())
        }
        _ => None,
    }
}

pub fn openai_chat_supports_reasoning_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized.split('/').any(|segment| {
        segment.starts_with("o1")
            || segment.starts_with("o3")
            || segment.starts_with("o4")
            || segment.starts_with("gpt-5")
    }) || normalized.contains("codex")
        || normalized.contains("deepseek-v4")
}

fn openai_compatible_reasoning_effort(model: &str, effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::Xhigh => "xhigh",
        ReasoningEffort::Max if model.to_ascii_lowercase().contains("deepseek-v4") => "max",
        ReasoningEffort::Max => "xhigh",
    }
}

fn supports_none_reasoning_effort(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase();
    normalized
        .split(['/', ':'])
        .find_map(|segment| segment.strip_prefix("gpt-5."))
        .and_then(|suffix| suffix.split(['-', '.']).next())
        .and_then(|version| version.parse::<u32>().ok())
        .is_some_and(|version| version >= 1)
}
