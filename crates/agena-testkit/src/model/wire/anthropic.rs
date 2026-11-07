use anyhow::Result;
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
    protocol: ModelProtocol,
    step: u64,
    hold: Option<usize>,
) -> Result<(Vec<Bytes>, Option<usize>)> {
    let model = request
        .decoded_model()
        .unwrap_or_else(|| protocol.model_id().into());
    let stop = native_stop(completion, "end_turn", "tool_use", "max_tokens");
    let mut message = json!({
        "id":format!("msg_testkit_{step}"), "type":"message", "role":"assistant",
        "model":model, "content":[], "stop_reason":stop, "stop_sequence":null
    });
    if let Some(usage) = &completion.usage {
        message["usage"] = usage_json(usage);
    }
    if let Some(end_turn) = completion.end_turn {
        message["end_turn"] = json!(end_turn);
    }
    if !request.streaming() {
        let mut blocks = Vec::new();
        if !completion.thinking.is_empty() {
            blocks.push(
                json!({"type":"thinking", "thinking":completion.thinking.concat(),
                "signature":"dGVzdGtpdC1zaWduYXR1cmU="}),
            );
        }
        if !completion.text.is_empty() {
            blocks.push(json!({"type":"text", "text":completion.text.concat()}));
        }
        for (index, call) in completion.functions.iter().enumerate() {
            blocks.push(json!({"type":"tool_use", "id":call_id(call, step, index),
                "name":call.name, "input":arguments(call)?}));
        }
        message["content"] = json!(blocks);
        return Ok((vec![Bytes::from(serde_json::to_vec(&message)?)], None));
    }

    let mut start = message.clone();
    start["stop_reason"] = Value::Null;
    if let Some(usage) = start.get_mut("usage") {
        usage["output_tokens"] = json!(0);
        if let Some(details) = usage.get_mut("output_tokens_details") {
            details["thinking_tokens"] = json!(0);
        }
    }
    let mut events = vec![json!({"type":"message_start","message":start})];
    let mut block_index = 0;
    if !completion.thinking.is_empty() {
        events.push(json!({"type":"content_block_start","index":block_index,
            "content_block":{"type":"thinking","thinking":""}}));
        for delta in &completion.thinking {
            events.push(json!({"type":"content_block_delta","index":block_index,
                "delta":{"type":"thinking_delta","thinking":delta}}));
        }
        events.push(json!({"type":"content_block_delta","index":block_index,
            "delta":{"type":"signature_delta","signature":"dGVzdGtpdC1zaWduYXR1cmU="}}));
        events.push(json!({"type":"content_block_stop","index":block_index}));
        block_index += 1;
    }
    let mut boundary = None;
    if !completion.text.is_empty() {
        events.push(json!({"type":"content_block_start","index":block_index,
            "content_block":{"type":"text","text":""}}));
        boundary = (hold == Some(0)).then_some(events.len());
        for (index, delta) in completion.text.iter().enumerate() {
            events.push(json!({"type":"content_block_delta","index":block_index,
                "delta":{"type":"text_delta","text":delta}}));
            if hold == Some(index + 1) {
                boundary = Some(events.len());
            }
        }
        events.push(json!({"type":"content_block_stop","index":block_index}));
        block_index += 1;
    }
    for (index, call) in completion.functions.iter().enumerate() {
        events.push(json!({"type":"content_block_start","index":block_index,
            "content_block":{"type":"tool_use","id":call_id(call, step, index),
                "name":call.name,"input":{}}}));
        let defaults = [call.arguments_json.clone()];
        for delta in call.argument_chunks.as_deref().unwrap_or(&defaults) {
            events.push(json!({"type":"content_block_delta","index":block_index,
                "delta":{"type":"input_json_delta","partial_json":delta}}));
        }
        events.push(json!({"type":"content_block_stop","index":block_index}));
        block_index += 1;
    }
    let mut delta =
        json!({"type":"message_delta","delta":{"stop_reason":stop,"stop_sequence":null}});
    if let Some(usage) = &completion.usage {
        let mut output = json!({"output_tokens":usage.output_tokens});
        if usage.reasoning_tokens != 0 {
            output["output_tokens_details"] = json!({"thinking_tokens":usage.reasoning_tokens});
        }
        delta["usage"] = output;
    }
    events.push(delta);
    events.push(json!({"type":"message_stop"}));
    let chunks = events
        .into_iter()
        .map(|event| encode_event(protocol, &ModelEvent::json(event)))
        .collect::<Result<Vec<_>>>()?;
    Ok((chunks, boundary))
}

fn usage_json(usage: &Usage) -> Value {
    let mut value = json!({
        "input_tokens":usage.input_tokens.saturating_sub(usage.cached_tokens),
        "output_tokens":usage.output_tokens,
        "cache_read_input_tokens":usage.cached_tokens
    });
    if usage.reasoning_tokens != 0 {
        value["output_tokens_details"] = json!({"thinking_tokens":usage.reasoning_tokens});
    }
    value
}
