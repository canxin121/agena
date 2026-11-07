//! Completion builders share one logical reply, then encode the selected wire.
mod anthropic;
mod bedrock;
mod chat;
mod converse;
mod gemini;
mod ollama;

pub(super) use bedrock::{anthropic_frame, bedrock_error_frame, bedrock_frame};

use anyhow::{Result, ensure};
use bytes::Bytes;

use super::{ModelProtocol, ModelRequest, reply::Completion};

pub(super) fn encode(
    completion: &Completion,
    request: &ModelRequest,
    protocol: ModelProtocol,
    step_id: u64,
    hold_after_text: Option<usize>,
) -> Result<(Vec<Bytes>, Option<usize>)> {
    for call in &completion.functions {
        ensure!(!call.name.is_empty(), "function name must not be empty");
    }
    if let Some(usage) = &completion.usage {
        ensure!(
            usage.cached_tokens <= usage.input_tokens,
            "cached_tokens must be a subset of input_tokens"
        );
        ensure!(
            usage.reasoning_tokens <= usage.output_tokens,
            "reasoning_tokens must be a subset of output_tokens"
        );
    }
    match protocol {
        ModelProtocol::OpenAiResponses => {
            super::reply::encode_completion(completion, request, step_id, hold_after_text)
        }
        ModelProtocol::Anthropic | ModelProtocol::BedrockAnthropic => {
            anthropic::encode(completion, request, protocol, step_id, hold_after_text)
        }
        ModelProtocol::Gemini => gemini::encode(completion, request, step_id, hold_after_text),
        ModelProtocol::BedrockConverse => {
            converse::encode(completion, request, step_id, hold_after_text)
        }
        ModelProtocol::OpenAiChat => chat::encode(completion, request, step_id, hold_after_text),
        ModelProtocol::Ollama => ollama::encode(completion, request, step_id, hold_after_text),
    }
}

fn call_id(call: &super::FunctionCall, step: u64, index: usize) -> String {
    if call.call_id.is_empty() {
        format!("call_testkit_{step}_{index}")
    } else {
        call.call_id.clone()
    }
}

fn arguments(call: &super::FunctionCall) -> Result<serde_json::Value> {
    use anyhow::Context;
    let value: serde_json::Value = serde_json::from_str(&call.arguments_json).context(
        "native function arguments must be JSON; use ModelEvent or raw_json for malformed messages",
    )?;
    ensure!(
        value.is_object(),
        "native function arguments must be an object"
    );
    Ok(value)
}

fn native_stop<'a>(
    completion: &'a Completion,
    text: &'a str,
    tool: &'a str,
    limit: &'a str,
) -> &'a str {
    if let Some(reason) = completion.finish_reason.as_deref() {
        match reason {
            "stop" | "end_turn" => text,
            "tool_calls" | "tool_use" => tool,
            "length" | "max_tokens" | "max_output_tokens" | "MAX_TOKENS" => limit,
            other => other,
        }
    } else if completion.incomplete.is_some() {
        limit
    } else if completion.functions.is_empty() {
        text
    } else {
        tool
    }
}
