use serde_json::json;

use super::model_selection_value;
use crate::ModelRef;

#[test]
fn default_model_selection_keeps_model_variants_together() {
    let model = ModelRef::new_with_adapter("next-provider", "next-adapter", "next-model");

    assert_eq!(
        model_selection_value(
            &model,
            Some("high".to_owned()),
            Some("fast".to_owned()),
            Some("compact".to_owned()),
        ),
        json!({
            "provider": "next-provider",
            "adapter": "next-adapter",
            "model": "next-model",
            "thinking_mode": "high",
            "speed_mode": "fast",
            "verbosity": "compact"
        })
    );
}

#[test]
fn adapterless_default_model_selection_omits_adapter() {
    let model = ModelRef::new("next-provider", "next-model");

    assert_eq!(
        model_selection_value(&model, None, None, None),
        json!({
            "provider": "next-provider",
            "model": "next-model"
        })
    );
}
