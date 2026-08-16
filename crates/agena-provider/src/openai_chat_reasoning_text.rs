use serde_json::Value;

use crate::{ChatDeltaOrMessage, openai_chat_extract_text};

pub fn openai_chat_extract_reasoning_text(
    reasoning_content: Option<&Value>,
    reasoning_details: Option<&Value>,
    reasoning_text: Option<&Value>,
) -> Option<String> {
    [
        reasoning_content.map(openai_chat_extract_text),
        reasoning_details.map(extract_reasoning_details_text),
        reasoning_text.map(openai_chat_extract_text),
    ]
    .into_iter()
    .flatten()
    .find(|text| !text.trim().is_empty())
}

pub fn openai_chat_reasoning_field(
    reasoning_content: Option<&Value>,
    reasoning_details: Option<&Value>,
) -> Option<&'static str> {
    if reasoning_content.is_some_and(reasoning_field_value_is_meaningful) {
        Some("reasoning_content")
    } else if reasoning_details.is_some_and(reasoning_field_value_is_meaningful) {
        Some("reasoning_details")
    } else {
        None
    }
}

fn reasoning_field_value_is_meaningful(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

pub fn openai_chat_reasoning_field_from_delta(value: &ChatDeltaOrMessage) -> Option<&'static str> {
    openai_chat_reasoning_field(
        value.reasoning_content.as_ref(),
        value.reasoning_details.as_ref(),
    )
}

fn extract_reasoning_details_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(extract_reasoning_details_text).collect(),
        Value::Object(item) => item
            .get("text")
            .or_else(|| item.get("summary"))
            .map(extract_reasoning_details_text)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::openai_chat_reasoning_field;

    #[test]
    fn empty_reasoning_carrier_does_not_mask_a_later_meaningful_field() {
        let blank = serde_json::json!("   ");
        let details = serde_json::json!([{
            "type": "reasoning.text",
            "text": "reasoning"
        }]);
        assert_eq!(
            openai_chat_reasoning_field(Some(&blank), Some(&details)),
            Some("reasoning_details")
        );
        assert_eq!(
            openai_chat_reasoning_field(Some(&serde_json::Value::Null), None),
            None
        );
    }
}
