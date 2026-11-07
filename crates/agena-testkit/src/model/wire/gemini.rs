use anyhow::{Result, ensure};
use bytes::Bytes;
use serde_json::{Value, json};

use super::{arguments, call_id, native_stop};
use crate::model::{
    ModelEvent, ModelProtocol, ModelRequest,
    protocol::encode_event,
    reply::{Completion, Usage},
};

pub(super) fn encode(
    completion: &Completion,
    request: &ModelRequest,
    step: u64,
    hold: Option<usize>,
) -> Result<(Vec<Bytes>, Option<usize>)> {
    let mut calls = Vec::new();
    for (index, call) in completion.functions.iter().enumerate() {
        ensure!(
            call.argument_chunks.is_none(),
            "Gemini functionCall arguments are complete JSON objects; use ModelEvent for partialArgs events"
        );
        calls.push(json!({"functionCall":{"id":call_id(call, step, index),
            "name":call.name,"args":arguments(call)?}}));
    }
    let model = request
        .decoded_model()
        .unwrap_or_else(|| "fake-model".into());
    let stop = native_stop(completion, "STOP", "STOP", "MAX_TOKENS");
    let mut terminal = json!({"modelVersion":model,
        "candidates":[{"index":0,"finishReason":stop}]});
    if let Some(usage) = &completion.usage {
        terminal["usageMetadata"] = usage_json(usage);
    }
    if !request.streaming() {
        let mut parts = Vec::new();
        if !completion.thinking.is_empty() {
            parts.push(json!({"text":completion.thinking.concat(),"thought":true,
                "thoughtSignature":"dGVzdGtpdC1zaWduYXR1cmU="}));
        }
        if !completion.text.is_empty() {
            parts.push(json!({"text":completion.text.concat()}));
        }
        parts.extend(calls);
        terminal["candidates"][0]["content"] = json!({"role":"model","parts":parts});
        return Ok((vec![Bytes::from(serde_json::to_vec(&terminal)?)], None));
    }
    let mut events = Vec::new();
    for (index, delta) in completion.thinking.iter().enumerate() {
        let mut part = json!({"text":delta,"thought":true});
        if index + 1 == completion.thinking.len() {
            part["thoughtSignature"] = json!("dGVzdGtpdC1zaWduYXR1cmU=");
        }
        events.push(chunk(&model, part));
    }
    let mut boundary = (hold == Some(0)).then_some(events.len());
    for (index, delta) in completion.text.iter().enumerate() {
        events.push(chunk(&model, json!({"text":delta})));
        if hold == Some(index + 1) {
            boundary = Some(events.len());
        }
    }
    for call in calls {
        events.push(chunk(&model, call));
    }
    events.push(terminal);
    let chunks = events
        .into_iter()
        .map(|event| encode_event(ModelProtocol::Gemini, &ModelEvent::json(event)))
        .collect::<Result<Vec<_>>>()?;
    Ok((chunks, boundary))
}

fn chunk(model: &str, part: Value) -> Value {
    json!({"modelVersion":model,"candidates":[{"index":0,"content":{"role":"model","parts":[part]}}]})
}

fn usage_json(usage: &Usage) -> Value {
    json!({"promptTokenCount":usage.input_tokens,
        "candidatesTokenCount":usage.output_tokens.saturating_sub(usage.reasoning_tokens),
        "cachedContentTokenCount":usage.cached_tokens,
        "thoughtsTokenCount":usage.reasoning_tokens,
        "totalTokenCount":usage.input_tokens.saturating_add(usage.output_tokens)})
}
