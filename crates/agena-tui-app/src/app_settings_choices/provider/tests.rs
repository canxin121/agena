use serde_json::json;

use super::{provider_defaults_with_mode, provider_defaults_with_model};
use crate::{JsonValue, ModelRef, SessionModelModeStep};

#[test]
fn selecting_a_default_model_resets_model_specific_modes() {
    let existing = json!({
        "adapter": "old-adapter",
        "model": "old-model",
        "thinking_mode": "high",
        "speed_mode": "fast",
        "verbosity": "high",
        "parallel_tool_calls": false
    })
    .as_object()
    .expect("object")
    .clone();
    let model = ModelRef::new_with_adapter("next-provider", "next-adapter", "next-model");

    let updated = provider_defaults_with_model(existing, &model);

    assert_eq!(
        JsonValue::Object(updated),
        json!({
            "adapter": "next-adapter",
            "model": "next-model",
            "parallel_tool_calls": false
        })
    );
}

#[test]
fn selecting_an_adapterless_default_model_removes_the_old_adapter() {
    let existing = json!({
        "adapter": "old-adapter",
        "model": "old-model",
        "thinking_mode": "medium"
    })
    .as_object()
    .expect("object")
    .clone();
    let model = ModelRef::new("next-provider", "next-model");

    let updated = provider_defaults_with_model(existing, &model);

    assert_eq!(
        JsonValue::Object(updated),
        json!({
            "model": "next-model"
        })
    );
}

#[test]
fn selecting_default_modes_preserves_the_model_route() {
    let existing = json!({
        "adapter": "openai_responses",
        "model": "gpt-5",
        "parallel_tool_calls": false
    })
    .as_object()
    .expect("object")
    .clone();

    let with_thinking =
        provider_defaults_with_mode(existing, SessionModelModeStep::ThinkingMode, "high");
    let updated =
        provider_defaults_with_mode(with_thinking, SessionModelModeStep::SpeedMode, "fast");

    assert_eq!(
        JsonValue::Object(updated),
        json!({
            "adapter": "openai_responses",
            "model": "gpt-5",
            "thinking_mode": "high",
            "speed_mode": "fast",
            "parallel_tool_calls": false
        })
    );
}

#[test]
fn inheriting_a_default_mode_removes_only_that_mode() {
    let existing = json!({
        "adapter": "anthropic",
        "model": "claude-sonnet",
        "thinking_mode": "high",
        "speed_mode": "fast"
    })
    .as_object()
    .expect("object")
    .clone();

    let updated = provider_defaults_with_mode(existing, SessionModelModeStep::ThinkingMode, "");

    assert_eq!(
        JsonValue::Object(updated),
        json!({
            "adapter": "anthropic",
            "model": "claude-sonnet",
            "speed_mode": "fast"
        })
    );
}
