//! Provider-specific state attached to a part (design 13.2).
//!
//! Persisted in the assistant run's per-round records. Repeated reasoning
//! bodies refer to their canonical think parts and are restored for provider
//! serialization through [`agena_provider::CompletionInputProviderState`].

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use agena_domain::AssistantReasoningField;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
/// Provider-specific state attached to a part.
pub struct PartProviderState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<AssistantReasoningField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub gemini_thought_signatures: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anthropic_thinking_blocks: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openai_reasoning_items: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_chat_reasoning_details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_reasoning_opaque: Option<String>,
}

impl PartProviderState {
    pub fn is_empty(&self) -> bool {
        self.assistant_reasoning_field.is_none()
            && self.response_id.is_none()
            && self.gemini_thought_signatures.is_empty()
            && self.anthropic_thinking_blocks.is_empty()
            && self.openai_reasoning_items.is_empty()
            && self.openai_chat_reasoning_details.is_none()
            && self.copilot_reasoning_opaque.is_none()
    }
}

impl From<PartProviderState> for agena_provider::CompletionInputProviderState {
    fn from(value: PartProviderState) -> Self {
        Self {
            assistant_reasoning_field: value.assistant_reasoning_field,
            response_id: value.response_id,
            gemini_thought_signatures: value.gemini_thought_signatures,
            anthropic_thinking_blocks: value.anthropic_thinking_blocks,
            openai_reasoning_items: value.openai_reasoning_items,
            openai_chat_reasoning_details: value.openai_chat_reasoning_details,
            copilot_reasoning_opaque: value.copilot_reasoning_opaque,
        }
    }
}

impl From<agena_provider::CompletionInputProviderState> for PartProviderState {
    fn from(value: agena_provider::CompletionInputProviderState) -> Self {
        Self {
            assistant_reasoning_field: value.assistant_reasoning_field,
            response_id: value.response_id,
            gemini_thought_signatures: value.gemini_thought_signatures,
            anthropic_thinking_blocks: value.anthropic_thinking_blocks,
            openai_reasoning_items: value.openai_reasoning_items,
            openai_chat_reasoning_details: value.openai_chat_reasoning_details,
            copilot_reasoning_opaque: value.copilot_reasoning_opaque,
        }
    }
}

const PART_TEXT_REFERENCE: &str = "agena.part_content";

#[derive(Debug, Clone)]
pub struct ReasoningTextSource {
    pub part_id: i64,
    pub fields: BTreeMap<String, String>,
}

impl ReasoningTextSource {
    pub fn from_content(part_id: i64, content: &serde_json::Value) -> Self {
        let mut fields = BTreeMap::new();
        for field in ["summary", "raw"] {
            let text = content
                .get(field)
                .and_then(serde_json::Value::as_array)
                .map(|chunks| {
                    chunks
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<String>()
                })
                .unwrap_or_default();
            if !text.is_empty() {
                fields.insert(field.to_owned(), text);
            }
        }
        if let Some(encrypted) = content
            .get("encrypted_content")
            .and_then(serde_json::Value::as_str)
            && !encrypted.is_empty()
        {
            fields.insert("encrypted_content".to_owned(), encrypted.to_owned());
        }
        Self { part_id, fields }
    }
}

/// References are internal storage values and must be restored before any
/// provider protocol serialization.
pub fn reference_reasoning_text(
    value: &serde_json::Value,
    sources: &[ReasoningTextSource],
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => serde_json::Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    if matches!(key.as_str(), "text" | "thinking" | "encrypted_content")
                        && let Some(text) = value.as_str().filter(|text| !text.is_empty())
                    {
                        for source in sources {
                            for (field, body) in &source.fields {
                                if (key == "encrypted_content") != (field == "encrypted_content") {
                                    continue;
                                }
                                if let Some(start) = body.find(text) {
                                    return (
                                        key.clone(),
                                        serde_json::json!({
                                            PART_TEXT_REFERENCE: {
                                                "part_id": source.part_id,
                                                "field": field,
                                                "start": start,
                                                "length": text.len()
                                            }
                                        }),
                                    );
                                }
                            }
                        }
                    }
                    (key.clone(), reference_reasoning_text(value, sources))
                })
                .collect(),
        ),
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .map(|value| reference_reasoning_text(value, sources))
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub fn restore_reasoning_text(
    value: &serde_json::Value,
    sources: &[ReasoningTextSource],
) -> Result<serde_json::Value, String> {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(reference) = object.get(PART_TEXT_REFERENCE) {
                if object.len() != 1 {
                    return Err("reasoning content reference contains unrelated fields".to_owned());
                }
                let part_id = reference
                    .get("part_id")
                    .and_then(serde_json::Value::as_i64)
                    .ok_or_else(|| "reasoning content reference has no part id".to_owned())?;
                let field = reference
                    .get("field")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| "reasoning content reference has no field".to_owned())?;
                let offset = |key: &str| {
                    reference
                        .get(key)
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| usize::try_from(value).ok())
                        .ok_or_else(|| format!("reasoning content reference has invalid {key}"))
                };
                let start = offset("start")?;
                let end = start
                    .checked_add(offset("length")?)
                    .ok_or_else(|| "reasoning content reference range overflows".to_owned())?;
                let body = sources
                    .iter()
                    .find(|source| source.part_id == part_id)
                    .and_then(|source| source.fields.get(field))
                    .ok_or_else(|| {
                        format!("reasoning content reference has no source part {part_id}")
                    })?;
                let text = body.get(start..end).ok_or_else(|| {
                    "reasoning content reference is outside its source".to_owned()
                })?;
                return Ok(serde_json::Value::String(text.to_owned()));
            }
            object
                .iter()
                .map(|(key, value)| Ok((key.clone(), restore_reasoning_text(value, sources)?)))
                .collect::<Result<serde_json::Map<String, serde_json::Value>, String>>()
                .map(serde_json::Value::Object)
        }
        serde_json::Value::Array(values) => values
            .iter()
            .map(|value| restore_reasoning_text(value, sources))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        _ => Ok(value.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PartProviderState, ReasoningTextSource, reference_reasoning_text, restore_reasoning_text,
    };
    use std::collections::BTreeMap;

    #[test]
    fn repeated_provider_bodies_are_references_and_restore_exactly() {
        let sources = vec![ReasoningTextSource::from_content(
            7,
            &serde_json::json!({
                "summary": ["first ", "你好 second"],
                "encrypted_content": "opaque-encrypted-content"
            }),
        )];
        let state = serde_json::json!({
            "response_id": "response-1",
            "anthropic_thinking_blocks": [{"type": "thinking", "thinking": "first 你好 second", "signature": "signed-1"}],
            "openai_reasoning_items": [{"type": "reasoning", "summary": [{"type": "summary_text", "text": "你好"}], "encrypted_content": "opaque-encrypted-content"}],
            "openai_chat_reasoning_details": [{"type": "reasoning.text", "text": "first 你好 second"}]
        });
        let stored = reference_reasoning_text(&state, &sources);
        let encoded = serde_json::to_string(&stored).unwrap();
        assert!(!encoded.contains("first 你好 second"));
        assert!(!encoded.contains("opaque-encrypted-content"));
        assert!(encoded.contains("signed-1"));
        assert_eq!(restore_reasoning_text(&stored, &sources).unwrap(), state);
    }

    #[test]
    fn unrelated_provider_facts_are_not_removed() {
        let sources = vec![ReasoningTextSource::from_content(
            7,
            &serde_json::json!({"summary": ["known reasoning"]}),
        )];
        let state = serde_json::json!({"text": "different original reasoning", "signature": "known reasoning", "response_id": "r-1"});
        let stored = reference_reasoning_text(&state, &sources);
        assert_eq!(stored, state);
        assert_eq!(restore_reasoning_text(&stored, &sources).unwrap(), state);
    }

    #[test]
    fn invalid_replay_references_cannot_be_sent_to_a_provider() {
        let sources = vec![ReasoningTextSource::from_content(
            7,
            &serde_json::json!({"summary": ["你好"]}),
        )];
        let state = serde_json::json!({"text": "你好"});
        let mut stored = reference_reasoning_text(&state, &sources);
        assert!(restore_reasoning_text(&stored, &[]).is_err());
        stored["text"][super::PART_TEXT_REFERENCE]["start"] = serde_json::json!(1);
        assert!(restore_reasoning_text(&stored, &sources).is_err());
    }

    #[test]
    fn provider_replay_state_round_trips_through_contract_value() {
        let state = PartProviderState {
            response_id: Some("response-1".to_owned()),
            gemini_thought_signatures: BTreeMap::from([("part".to_owned(), "sig".to_owned())]),
            ..Default::default()
        };
        let contract: agena_provider::CompletionInputProviderState = state.clone().into();
        let restored = PartProviderState::from(contract);
        assert_eq!(restored, state);
    }
}
