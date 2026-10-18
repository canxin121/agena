use std::collections::BTreeMap;

use agena_domain::{ContentEpoch, ContentId, ContentKind, ModelRef};
use agena_provider::{CompletionUsage, ProviderCompactionContext};
use agena_storage::kernel::{CaptureState, ContentStreamView, KernelError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const REFERENCE: &str = "agena.kernel_text";

/// Provider facts belong to one route and one round. The raw replay tree
/// preserves unknown provider fields; projection interprets only supported
/// protocol fields and never writes a decoded projection back to storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelProviderState {
    pub version: u8,
    pub model: ModelRef,
    pub replay: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<CompletionUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<ProviderCompactionContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<agena_domain::FinishReason>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub follow_up_requested: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::Failure>,
}

impl KernelProviderState {
    pub fn new(model: ModelRef, replay: Value) -> Self {
        Self {
            version: 1,
            model,
            replay,
            usage: None,
            compaction: None,
            finish_reason: None,
            follow_up_requested: false,
            failure: None,
        }
    }

    pub(super) fn decode(value: &Value) -> Result<Self> {
        let state: Self = serde_json::from_value(value.clone())
            .map_err(|e| KernelError::Invalid(format!("invalid provider-round state: {e}")))?;
        if state.version != 1 {
            return Err(KernelError::Invalid(
                "unsupported provider-round state version".into(),
            ));
        }
        Ok(state)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    content: ContentId,
    epoch: ContentEpoch,
    start: usize,
    length: usize,
}

/// Deduplicate exact reasoning substrings against already sealed canonical
/// text. Signatures, encrypted data and unknown fields otherwise stay intact.
/// A reference is always scoped to the frozen source head used to restore it.
pub fn reference_replay_text(
    value: &Value,
    sources: &[(ContentStreamView, String)],
) -> Result<Value> {
    transform(value, &mut |object| {
        if object.contains_key(REFERENCE) {
            return Err(KernelError::Invalid(
                "provider data uses a reserved replay-reference key".into(),
            ));
        }
        let mut output = serde_json::Map::new();
        for (key, value) in object {
            let mut replacement = None;
            if matches!(key.as_str(), "text" | "thinking")
                && let Some(text) = value.as_str().filter(|s| !s.is_empty())
            {
                for (head, body) in sources {
                    if head.kind != ContentKind::Text
                        || head.state != CaptureState::Complete
                        || head.retained_start != 0
                        || head.committed != body.len() as u64
                    {
                        return Err(KernelError::Invalid(
                            "replay reference requires a complete canonical source".into(),
                        ));
                    }
                    if let Some(start) = body.find(text) {
                        replacement = Some(
                            serde_json::json!({REFERENCE: Reference {content:head.id, epoch:head.epoch, start, length:text.len()}}),
                        );
                        break;
                    }
                }
            }
            output.insert(
                key.clone(),
                match replacement {
                    Some(value) => value,
                    None => reference_replay_text(value, sources)?,
                },
            );
        }
        Ok(Value::Object(output))
    })
}

pub(super) fn restore_replay_text(
    value: &Value,
    sources: &BTreeMap<ContentId, (ContentStreamView, String)>,
    remaining: &mut usize,
) -> Result<Value> {
    super::input::charge(remaining, 128)?;
    match value {
        Value::Array(values) => values
            .iter()
            .map(|value| restore_replay_text(value, sources, remaining))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        Value::String(text) => {
            super::input::charge(remaining, text.len().saturating_mul(2))?;
            Ok(value.clone())
        }
        Value::Object(object) => {
            if let Some(reference) = object.get(REFERENCE) {
                if object.len() != 1 {
                    return Err(KernelError::Invalid(
                        "replay reference contains unrelated fields".into(),
                    ));
                }
                let reference: Reference = serde_json::from_value(reference.clone())
                    .map_err(|e| KernelError::Invalid(format!("invalid replay reference: {e}")))?;
                let (head, body) = sources.get(&reference.content).ok_or_else(|| {
                    KernelError::Invalid(
                        "replay source is outside this round's reasoning input".into(),
                    )
                })?;
                if head.epoch != reference.epoch {
                    return Err(KernelError::ContentReset);
                }
                let end = reference
                    .start
                    .checked_add(reference.length)
                    .ok_or_else(|| KernelError::Invalid("replay reference overflow".into()))?;
                if end as u64 > head.committed {
                    return Err(KernelError::Invalid(
                        "replay reference exceeds frozen committed head".into(),
                    ));
                }
                let text = body.get(reference.start..end).ok_or_else(|| {
                    KernelError::Invalid(
                        "replay reference is outside UTF-8 source boundaries".into(),
                    )
                })?;
                super::input::charge(remaining, text.len().saturating_mul(2))?;
                return Ok(Value::String(text.to_owned()));
            }
            object
                .iter()
                .map(|(key, value)| {
                    super::input::charge(remaining, key.len().saturating_mul(2))?;
                    Ok((key.clone(), restore_replay_text(value, sources, remaining)?))
                })
                .collect::<Result<serde_json::Map<_, _>>>()
                .map(Value::Object)
        }
        _ => Ok(value.clone()),
    }
}

fn transform(
    value: &Value,
    object: &mut impl FnMut(&serde_json::Map<String, Value>) -> Result<Value>,
) -> Result<Value> {
    match value {
        Value::Object(value) => object(value),
        Value::Array(values) => values
            .iter()
            .map(|value| transform(value, object))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        _ => Ok(value.clone()),
    }
}
