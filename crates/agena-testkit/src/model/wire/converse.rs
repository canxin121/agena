use anyhow::Result;
use bytes::Bytes;
use serde_json::{Value, json};

use super::{arguments, bedrock_frame, call_id, native_stop};
use crate::model::{ModelRequest, reply::Completion};

pub(super) fn encode(
    completion: &Completion,
    request: &ModelRequest,
    step: u64,
    hold: Option<usize>,
) -> Result<(Vec<Bytes>, Option<usize>)> {
    let stop = native_stop(completion, "end_turn", "tool_use", "max_tokens");
    let usage = completion.usage.as_ref().map(|usage| {
        json!({
            "inputTokens":usage.input_tokens.saturating_sub(usage.cached_tokens),
            "outputTokens":usage.output_tokens,
            "totalTokens":usage.input_tokens.saturating_add(usage.output_tokens),
            "cacheReadInputTokens":usage.cached_tokens
        })
    });
    if !request.streaming() {
        let mut content = Vec::new();
        if !completion.thinking.is_empty() {
            content.push(json!({"reasoningContent":{"reasoningText":{
                "text":completion.thinking.concat(),"signature":"dGVzdGtpdC1zaWduYXR1cmU="}}}));
        }
        if !completion.text.is_empty() {
            content.push(json!({"text":completion.text.concat()}));
        }
        for (index, call) in completion.functions.iter().enumerate() {
            content.push(json!({"toolUse":{"toolUseId":call_id(call, step, index),
                "name":call.name,"input":arguments(call)?}}));
        }
        let mut response = json!({"output":{"message":{"role":"assistant","content":content}},
            "stopReason":stop,"metrics":{"latencyMs":0}});
        if let Some(usage) = usage {
            response["usage"] = usage;
        }
        return Ok((vec![Bytes::from(serde_json::to_vec(&response)?)], None));
    }
    let mut events: Vec<(&str, Value)> = vec![("messageStart", json!({"role":"assistant"}))];
    let mut block = 0;
    if !completion.thinking.is_empty() {
        for delta in &completion.thinking {
            events.push((
                "contentBlockDelta",
                json!({"contentBlockIndex":block,
                "delta":{"reasoningContent":{"text":delta}}}),
            ));
        }
        events.push((
            "contentBlockDelta",
            json!({"contentBlockIndex":block,
            "delta":{"reasoningContent":{"signature":"dGVzdGtpdC1zaWduYXR1cmU="}}}),
        ));
        events.push(("contentBlockStop", json!({"contentBlockIndex":block})));
        block += 1;
    }
    let mut boundary = (hold == Some(0)).then_some(events.len());
    if !completion.text.is_empty() {
        for (index, delta) in completion.text.iter().enumerate() {
            events.push((
                "contentBlockDelta",
                json!({"contentBlockIndex":block,"delta":{"text":delta}}),
            ));
            if hold == Some(index + 1) {
                boundary = Some(events.len());
            }
        }
        events.push(("contentBlockStop", json!({"contentBlockIndex":block})));
        block += 1;
    }
    for (index, call) in completion.functions.iter().enumerate() {
        events.push((
            "contentBlockStart",
            json!({"contentBlockIndex":block,
            "start":{"toolUse":{"toolUseId":call_id(call, step, index),"name":call.name}}}),
        ));
        let defaults = [call.arguments_json.clone()];
        for delta in call.argument_chunks.as_deref().unwrap_or(&defaults) {
            events.push((
                "contentBlockDelta",
                json!({"contentBlockIndex":block,
                "delta":{"toolUse":{"input":delta}}}),
            ));
        }
        events.push(("contentBlockStop", json!({"contentBlockIndex":block})));
        block += 1;
    }
    events.push(("messageStop", json!({"stopReason":stop})));
    if let Some(usage) = usage {
        events.push(("metadata", json!({"usage":usage,"metrics":{"latencyMs":0}})));
    }
    let chunks = events
        .iter()
        .map(|(name, data)| bedrock_frame(name, data, false))
        .collect::<Result<Vec<_>>>()?;
    Ok((chunks, boundary))
}
