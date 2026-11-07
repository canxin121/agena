use anyhow::Result;
use bytes::Bytes;
use serde_json::{Value, json};

use super::{call_id, native_stop};
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
    let model = request
        .decoded_model()
        .unwrap_or_else(|| "fake-model".into());
    let id = format!("chatcmpl_testkit_{step}");
    let stop = native_stop(completion, "stop", "tool_calls", "length");
    if !request.streaming() {
        let mut message = json!({"role":"assistant","content":completion.text.concat()});
        if !completion.thinking.is_empty() {
            message["reasoning_content"] = json!(completion.thinking.concat());
        }
        if !completion.functions.is_empty() {
            message["tool_calls"] = json!(
                completion
                    .functions
                    .iter()
                    .enumerate()
                    .map(|(index, call)| {
                        json!({"id":call_id(call, step, index),"type":"function",
                    "function":{"name":call.name,"arguments":call.arguments_json}})
                    })
                    .collect::<Vec<_>>()
            );
        }
        let mut response = json!({"id":id,"object":"chat.completion","created":0,"model":model,
            "choices":[{"index":0,"message":message,"finish_reason":stop}]});
        if let Some(usage) = &completion.usage {
            response["usage"] = usage_json(usage);
        }
        return Ok((vec![Bytes::from(serde_json::to_vec(&response)?)], None));
    }
    let chunk = |delta: Value, reason: Option<&str>| {
        json!({
            "id":id,"object":"chat.completion.chunk","created":0,"model":model,
            "choices":[{"index":0,"delta":delta,"finish_reason":reason}]
        })
    };
    let mut events = vec![ModelEvent::json(chunk(json!({"role":"assistant"}), None))];
    for delta in &completion.thinking {
        events.push(ModelEvent::json(chunk(
            json!({"reasoning_content":delta}),
            None,
        )));
    }
    let mut boundary = (hold == Some(0)).then_some(events.len());
    for (index, delta) in completion.text.iter().enumerate() {
        events.push(ModelEvent::json(chunk(json!({"content":delta}), None)));
        if hold == Some(index + 1) {
            boundary = Some(events.len());
        }
    }
    for (index, call) in completion.functions.iter().enumerate() {
        events.push(ModelEvent::json(chunk(
            json!({"tool_calls":[{"index":index,
            "id":call_id(call, step, index),"type":"function",
            "function":{"name":call.name,"arguments":""}}]}),
            None,
        )));
        let defaults = [call.arguments_json.clone()];
        for delta in call.argument_chunks.as_deref().unwrap_or(&defaults) {
            events.push(ModelEvent::json(chunk(
                json!({"tool_calls":[{"index":index,
                "function":{"arguments":delta}}]}),
                None,
            )));
        }
    }
    events.push(ModelEvent::json(chunk(json!({}), Some(stop))));
    if let Some(usage) = &completion.usage {
        events.push(ModelEvent::json(
            json!({"id":id,"object":"chat.completion.chunk",
            "created":0,"model":model,"choices":[],"usage":usage_json(usage)}),
        ));
    }
    events.push(ModelEvent::Done);
    let chunks = events
        .iter()
        .map(|event| encode_event(ModelProtocol::OpenAiChat, event))
        .collect::<Result<Vec<_>>>()?;
    Ok((chunks, boundary))
}

fn usage_json(usage: &Usage) -> Value {
    json!({"prompt_tokens":usage.input_tokens,"completion_tokens":usage.output_tokens,
        "total_tokens":usage.input_tokens.saturating_add(usage.output_tokens),
        "prompt_tokens_details":{"cached_tokens":usage.cached_tokens},
        "completion_tokens_details":{"reasoning_tokens":usage.reasoning_tokens}})
}
