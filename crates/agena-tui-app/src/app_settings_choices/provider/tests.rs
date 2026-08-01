use serde_json::json;

use super::provider_defaults_with_model;
use crate::{JsonValue, ModelRef};

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
            "model": "next-model"
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
