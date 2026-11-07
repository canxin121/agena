use std::collections::BTreeMap;

use agena_provider_bedrock_streaming::{BedrockAnthropicStreamDecodeError, decode_response};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::{Value, json};

use super::client;
use crate::model::{
    ModelEvent, ModelOptions, ModelProtocol, ModelRequest, Reply, RequestMatch, ScriptedModel,
    Usage,
};

const PROTOCOLS: [ModelProtocol; 7] = [
    ModelProtocol::OpenAiResponses,
    ModelProtocol::OpenAiChat,
    ModelProtocol::Anthropic,
    ModelProtocol::Gemini,
    ModelProtocol::BedrockAnthropic,
    ModelProtocol::BedrockConverse,
    ModelProtocol::Ollama,
];

fn body(protocol: ModelProtocol, model: &str, streaming: bool) -> Value {
    match protocol {
        ModelProtocol::OpenAiResponses => json!({"model":model,"stream":streaming,
            "input":[{"role":"system","content":"system-only"},{"role":"user","content":"only-user"}],
            "tools":[{"type":"function","name":"tools_call"}]}),
        ModelProtocol::Anthropic | ModelProtocol::BedrockAnthropic => {
            let mut body = json!({"messages":[{"role":"user","content":[{"type":"text","text":"only-user"}]}],
                "system":"system-only","tools":[{"name":"tools_call","input_schema":{"type":"object"}}]});
            if protocol == ModelProtocol::Anthropic {
                body["model"] = json!(model);
                body["stream"] = json!(streaming);
            }
            body
        }
        ModelProtocol::Gemini => {
            json!({"contents":[{"role":"user","parts":[{"text":"only-user"}]}],
            "systemInstruction":{"parts":[{"text":"system-only"}]},
            "tools":[{"functionDeclarations":[{"name":"tools_call","parameters":{"type":"object"}}]}]})
        }
        ModelProtocol::BedrockConverse => {
            json!({"messages":[{"role":"user","content":[{"text":"only-user"}]}],
            "system":[{"text":"system-only"}],
            "toolConfig":{"tools":[{"toolSpec":{"name":"tools_call","inputSchema":{"json":{"type":"object"}}}}]}})
        }
        ModelProtocol::OpenAiChat | ModelProtocol::Ollama => {
            json!({"model":model,"stream":streaming,
            "messages":[{"role":"system","content":"system-only"},{"role":"user","content":"only-user"}],
            "tools":[{"type":"function","function":{"name":"tools_call","parameters":{"type":"object"}}}]})
        }
    }
}

fn observed(protocol: ModelProtocol, payload: Value) -> ModelRequest {
    ModelRequest {
        sequence: 1,
        method: "POST".into(),
        path: protocol.request_path(protocol.model_id(), false),
        raw_uri: protocol.request_path(protocol.model_id(), false),
        query_pairs: Vec::new(),
        route_protocol: Some(protocol),
        headers: BTreeMap::new(),
        body: payload,
        body_bytes: 0,
        body_truncated: false,
        body_preview: String::new(),
        step_id: None,
    }
}

fn common_reply() -> Reply {
    Reply::text_chunks(["你好", " world"])
        .with_thinking_chunks(["checking", " carefully"])
        .with_function_call(
            "native-call",
            "tools_call",
            json!({"tool":"fs.read","input":{"file_path":"a"}}),
        )
        .with_function_call("parallel-call", "second", json!({"key":"value"}))
        .usage(Usage::tokens(17, 11).cached(4).reasoning(3))
}

#[tokio::test]
async fn one_reply_and_matcher_api_works_for_every_native_json_contract() -> Result<()> {
    for protocol in PROTOCOLS {
        let owner = ScriptedModel::start_with_options(ModelOptions::for_protocol(protocol)).await?;
        let model = owner.handle();
        // Include percent signs, slashes, spaces, and colons for path model IDs.
        let id = "region:provider/model %name";
        let step = model
            .expect(
                RequestMatch::user_text("only-user")
                    .protocol(protocol)
                    .model(id)
                    .streaming(false)
                    .declares_function("tools_call"),
            )
            .check(|request| {
                ensure!(request.user_text() == "only-user");
                ensure!(!request.user_text().contains("system-only"));
                Ok(())
            })
            .reply(common_reply());
        let url = format!(
            "{}{}",
            owner.base_url().trim_end_matches("/v1"),
            protocol.request_path(id, false)
        );
        let response = client()?
            .post(url)
            .json(&body(protocol, id, false))
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("{protocol:?} JSON response"))?;
        assert!(
            response.headers()["content-type"]
                .to_str()?
                .starts_with("application/json")
        );
        let message: Value = response.json().await?;
        match protocol {
            ModelProtocol::OpenAiResponses => {
                assert_eq!(message["output"][1]["content"][0]["text"], "你好 world");
                assert_eq!(message["output"][2]["call_id"], "native-call");
                assert_eq!(message["model"], id);
                assert_eq!(
                    message["usage"]["output_tokens_details"]["reasoning_tokens"],
                    3
                );
            }
            ModelProtocol::OpenAiChat => {
                assert_eq!(message["choices"][0]["message"]["content"], "你好 world");
                assert_eq!(
                    message["choices"][0]["message"]["tool_calls"][0]["id"],
                    "native-call"
                );
                assert_eq!(message["model"], id);
                assert_eq!(message["usage"]["prompt_tokens"], 17);
            }
            ModelProtocol::Anthropic | ModelProtocol::BedrockAnthropic => {
                assert_eq!(message["content"][0]["type"], "thinking");
                assert_eq!(message["content"][1]["text"], "你好 world");
                assert_eq!(message["content"][2]["id"], "native-call");
                assert_eq!(message["content"][3]["id"], "parallel-call");
                assert_eq!(message["stop_reason"], "tool_use");
                assert_eq!(message["model"], id);
                assert_eq!(message["usage"]["input_tokens"], 13);
                assert_eq!(message["usage"]["cache_read_input_tokens"], 4);
            }
            ModelProtocol::Gemini => {
                assert_eq!(
                    message["candidates"][0]["content"]["parts"][0]["thought"],
                    true
                );
                assert_eq!(
                    message["candidates"][0]["content"]["parts"][1]["text"],
                    "你好 world"
                );
                assert_eq!(
                    message["candidates"][0]["content"]["parts"][2]["functionCall"]["id"],
                    "native-call"
                );
                assert_eq!(message["modelVersion"], id);
                assert_eq!(message["usageMetadata"]["candidatesTokenCount"], 8);
                assert_eq!(message["usageMetadata"]["thoughtsTokenCount"], 3);
            }
            ModelProtocol::BedrockConverse => {
                assert_eq!(
                    message["output"]["message"]["content"][1]["text"],
                    "你好 world"
                );
                assert_eq!(
                    message["output"]["message"]["content"][2]["toolUse"]["toolUseId"],
                    "native-call"
                );
                assert_eq!(message["stopReason"], "tool_use");
            }
            ModelProtocol::Ollama => {
                assert_eq!(message["message"]["content"], "你好 world");
                assert_eq!(
                    message["message"]["tool_calls"][0]["function"]["name"],
                    "tools_call"
                );
                assert_eq!(message["model"], id);
                assert_eq!(message["done"], true);
                assert_eq!(message["eval_count"], 11);
            }
        }
        step.finished().await?;
        model.assert_finished()?;
        owner.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn native_streams_keep_utf8_parallel_calls_usage_and_terminal_events() -> Result<()> {
    for protocol in PROTOCOLS {
        let owner = ScriptedModel::start_with_options(ModelOptions::for_protocol(protocol)).await?;
        let model = owner.handle();
        let step = model
            .expect(
                RequestMatch::user_text("only-user")
                    .protocol(protocol)
                    .streaming(true),
            )
            .reply(common_reply().fragment_bytes(7));
        let path = protocol.request_path(protocol.model_id(), true);
        let response = client()?
            .post(format!(
                "{}{}",
                owner.base_url().trim_end_matches("/v1"),
                path
            ))
            .json(&body(protocol, protocol.model_id(), true))
            .send()
            .await?
            .error_for_status()?;
        assert_eq!(
            response.headers()["content-type"],
            protocol.stream_content_type()
        );
        if protocol == ModelProtocol::BedrockAnthropic {
            assert_eq!(
                response.headers()["x-amzn-bedrock-content-type"],
                "application/json"
            );
        }
        let events: Vec<Value> = if protocol == ModelProtocol::BedrockAnthropic {
            decode_response(response)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<std::result::Result<_, _>>()?
        } else {
            let raw = response.bytes().await?;
            if protocol == ModelProtocol::BedrockConverse {
                unpack_aws(&raw)?
                    .into_iter()
                    .map(|(name, data)| json!({"event":name,"data":data}))
                    .collect()
            } else if protocol == ModelProtocol::Ollama {
                std::str::from_utf8(&raw)?
                    .lines()
                    .map(serde_json::from_str)
                    .collect::<std::result::Result<_, _>>()?
            } else {
                sse_events(std::str::from_utf8(&raw)?)?
            }
        };
        let serialized = serde_json::to_string(&events)?;
        assert!(serialized.contains("你好"), "{protocol:?}");
        assert!(serialized.contains(" world"), "{protocol:?}");
        assert!(serialized.contains("checking"), "{protocol:?}");
        assert!(serialized.contains("tools_call"), "{protocol:?}");
        assert!(serialized.contains("second"), "{protocol:?}");
        match protocol {
            ModelProtocol::Anthropic | ModelProtocol::BedrockAnthropic => {
                assert_eq!(events.first().unwrap()["type"], "message_start");
                assert_eq!(events.last().unwrap()["type"], "message_stop");
                let calls: Vec<_> = events
                    .iter()
                    .filter_map(|event| event.get("content_block"))
                    .filter(|block| block["type"] == "tool_use")
                    .collect();
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[0]["id"], "native-call");
                assert_eq!(calls[1]["id"], "parallel-call");
                assert!(
                    events.iter().any(
                        |event| event.pointer("/delta/type") == Some(&json!("signature_delta"))
                    )
                );
                let delta = &events[events.len() - 2];
                assert_eq!(delta["delta"]["stop_reason"], "tool_use");
                assert_eq!(delta["usage"]["output_tokens"], 11);
            }
            ModelProtocol::Gemini => {
                assert_eq!(
                    events.last().unwrap()["candidates"][0]["finishReason"],
                    "STOP"
                );
                assert_eq!(
                    events.last().unwrap()["usageMetadata"]["thoughtsTokenCount"],
                    3
                );
                assert_eq!(serialized.matches("native-call").count(), 1);
                assert_eq!(serialized.matches("parallel-call").count(), 1);
            }
            ModelProtocol::BedrockConverse => {
                assert_eq!(events[0]["event"], "messageStart");
                assert_eq!(events[events.len() - 2]["event"], "messageStop");
                assert_eq!(events.last().unwrap()["event"], "metadata");
                assert_eq!(events.last().unwrap()["data"]["usage"]["outputTokens"], 11);
            }
            ModelProtocol::OpenAiChat => {
                assert_eq!(events.last().unwrap()["usage"]["completion_tokens"], 11);
                assert!(
                    events
                        .iter()
                        .any(|event| event.pointer("/choices/0/finish_reason")
                            == Some(&json!("tool_calls")))
                );
            }
            ModelProtocol::OpenAiResponses => {
                assert_eq!(events.last().unwrap()["type"], "response.completed")
            }
            ModelProtocol::Ollama => assert_eq!(events.last().unwrap()["done"], true),
        }
        step.finished().await?;
        model.assert_finished()?;
        owner.shutdown().await?;
    }
    Ok(())
}

#[test]
fn native_request_helpers_exclude_tool_text_and_reject_duplicate_correlation_ids() -> Result<()> {
    for protocol in PROTOCOLS {
        let mut payload = body(protocol, protocol.model_id(), false);
        let result = match protocol {
            ModelProtocol::OpenAiResponses => {
                json!({"type":"function_call_output","call_id":"call","output":"tool-only"})
            }
            ModelProtocol::Anthropic | ModelProtocol::BedrockAnthropic => {
                json!({"type":"tool_result","tool_use_id":"call","content":[{"type":"text","text":"tool-only"}]})
            }
            ModelProtocol::Gemini => {
                json!({"functionResponse":{"id":"call","name":"tools_call","response":{"output":"tool-only"}}})
            }
            ModelProtocol::BedrockConverse => {
                json!({"toolResult":{"toolUseId":"call","content":[{"text":"tool-only"}]}})
            }
            ModelProtocol::OpenAiChat | ModelProtocol::Ollama => {
                json!({"role":"tool","tool_call_id":"call","content":"tool-only"})
            }
        };
        let add = |payload: &mut Value, item: Value| match protocol {
            ModelProtocol::OpenAiResponses => payload["input"].as_array_mut().unwrap().push(item),
            ModelProtocol::Anthropic
            | ModelProtocol::BedrockAnthropic
            | ModelProtocol::BedrockConverse => payload["messages"]
                .as_array_mut()
                .unwrap()
                .push(json!({"role":"user","content":[item]})),
            ModelProtocol::Gemini => payload["contents"]
                .as_array_mut()
                .unwrap()
                .push(json!({"role":"user","parts":[item]})),
            ModelProtocol::OpenAiChat | ModelProtocol::Ollama => {
                payload["messages"].as_array_mut().unwrap().push(item)
            }
        };
        add(&mut payload, result.clone());
        let request = observed(protocol, payload.clone());
        assert_eq!(request.user_text(), "only-user", "{protocol:?}");
        request.assert_tool_output_contains("call", "tool-only")?;
        let mut malformed = result.clone();
        match protocol {
            ModelProtocol::OpenAiResponses => {
                malformed.as_object_mut().unwrap().remove("output");
            }
            ModelProtocol::Anthropic
            | ModelProtocol::BedrockAnthropic
            | ModelProtocol::OpenAiChat
            | ModelProtocol::Ollama => {
                malformed.as_object_mut().unwrap().remove("content");
            }
            ModelProtocol::Gemini => {
                malformed["functionResponse"]
                    .as_object_mut()
                    .unwrap()
                    .remove("response");
            }
            ModelProtocol::BedrockConverse => {
                malformed["toolResult"]
                    .as_object_mut()
                    .unwrap()
                    .remove("content");
            }
        }
        add(&mut payload, malformed);
        let error = observed(protocol, payload)
            .require_tool_output("call")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected exactly one"),
            "{protocol:?}: {error}"
        );
    }
    Ok(())
}

#[test]
fn text_gates_stay_at_native_text_event_boundaries_after_byte_fragmentation() -> Result<()> {
    for protocol in PROTOCOLS {
        let mut request = observed(protocol, body(protocol, protocol.model_id(), true));
        request.path = protocol.request_path(protocol.model_id(), true);
        let plain = common_reply().hold_after_text_chunks(1).prepare(
            &request,
            1,
            &ModelOptions::for_protocol(protocol),
        )?;
        let fragmented = common_reply()
            .hold_after_text_chunks(1)
            .fragment_bytes(7)
            .prepare(&request, 1, &ModelOptions::for_protocol(protocol))?;
        let before = plain.chunks[..plain.hold_after.context("text boundary")?].concat();
        let after =
            fragmented.chunks[..fragmented.hold_after.context("fragmented boundary")?].concat();
        assert_eq!(
            before, after,
            "{protocol:?} gate moved inside a native event"
        );
        let after_gate = fragmented.chunks[fragmented.hold_after.unwrap()..].concat();
        if !matches!(
            protocol,
            ModelProtocol::BedrockAnthropic | ModelProtocol::BedrockConverse
        ) {
            assert!(std::str::from_utf8(&after)?.contains("你好"));
            assert!(!std::str::from_utf8(&after)?.contains(" world"));
            assert!(std::str::from_utf8(&after_gate)?.contains(" world"));
        } else {
            // Both halves must consist of whole AWS frames despite byte fragmentation.
            ensure!(!unpack_aws(&after)?.is_empty());
            ensure!(!unpack_aws(&after_gate)?.is_empty());
        }
    }
    Ok(())
}

#[test]
fn full_native_messages_events_and_unsupported_fields_are_not_silently_rewritten() -> Result<()> {
    let complete = json!({"id":"custom","content":[{"type":"server_tool_use","id":"srv","name":"web_search","input":{}}],
        "usage":{"cache_creation_input_tokens":91},"future_metadata":{"kept":true}});
    let request = observed(
        ModelProtocol::Anthropic,
        body(ModelProtocol::Anthropic, "fake-model", false),
    );
    let raw = ModelProtocol::Anthropic.message(complete.clone()).prepare(
        &request,
        1,
        &ModelOptions::default(),
    )?;
    assert_eq!(
        serde_json::from_slice::<Value>(&raw.chunks.concat())?,
        complete
    );
    let event = json!({"type":"content_block_delta","index":4,
        "delta":{"type":"citations_delta","citation":{"type":"char_location","cited_text":"引用"}},"future":true});
    let prepared = ModelProtocol::Anthropic
        .events([
            ModelEvent::json(event.clone()),
            ModelEvent::named("ping", json!({"type":"ping"})),
        ])
        .prepare(&request, 1, &ModelOptions::default())?;
    let bytes = prepared.chunks.concat();
    let text = std::str::from_utf8(&bytes)?;
    assert!(text.starts_with("event: content_block_delta\n"));
    assert_eq!(sse_events(text)?, vec![event, json!({"type":"ping"})]);
    assert!(
        !text.contains("message_stop"),
        "raw event API must not fabricate success"
    );
    let bad = ModelProtocol::Anthropic.events([ModelEvent::named("ping\nfake: yes", json!({}))]);
    assert!(bad.prepare(&request, 1, &ModelOptions::default()).is_err());
    let gemini = observed(
        ModelProtocol::Gemini,
        body(ModelProtocol::Gemini, "fake-model", true),
    );
    assert!(
        Reply::function_call("call", "f", json!({}))
            .arguments_chunks(["{", "}"])
            .prepare(
                &gemini,
                1,
                &ModelOptions::for_protocol(ModelProtocol::Gemini)
            )
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn bedrock_binary_exceptions_and_crc_failures_reach_the_production_decoder() -> Result<()> {
    let protocol = ModelProtocol::BedrockAnthropic;
    for corrupt in [false, true] {
        let owner = ScriptedModel::start_with_options(ModelOptions::for_protocol(protocol)).await?;
        let model = owner.handle();
        let event = ModelEvent::exception(
            "validationException",
            json!({"message":"native validation failure"}),
        );
        let mut reply = protocol.events([event]);
        if corrupt {
            let request = observed(protocol, body(protocol, protocol.model_id(), false));
            let prepared = reply.prepare(&request, 1, &ModelOptions::for_protocol(protocol))?;
            let mut bytes = prepared.chunks.concat();
            let last = bytes.len() - 1;
            bytes[last] ^= 1;
            reply = Reply::raw_stream(200, [Bytes::from(bytes)])
                .header("content-type", "application/vnd.amazon.eventstream");
        }
        let step = model
            .expect(RequestMatch::user_text("only-user"))
            .reply(reply.fragment_bytes(3));
        let response = client()?
            .post(format!(
                "{}{}",
                owner.base_url().trim_end_matches("/v1"),
                protocol.request_path(protocol.model_id(), true)
            ))
            .json(&body(protocol, protocol.model_id(), true))
            .send()
            .await?
            .error_for_status()?;
        let results = decode_response(response).collect::<Vec<_>>().await;
        assert_eq!(results.len(), 1);
        match &results[0] {
            Err(BedrockAnthropicStreamDecodeError::Service(error)) if !corrupt => {
                assert_eq!(error.event_type, "validationException");
                assert_eq!(error.message, "native validation failure");
                assert!(!error.retryable);
            }
            Err(BedrockAnthropicStreamDecodeError::Decode(_)) if corrupt => {}
            other => anyhow::bail!("unexpected Bedrock decode result: {other:?}"),
        }
        step.finished().await?;
        model.assert_finished()?;
        owner.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn bedrock_unmodeled_error_headers_preserve_full_values_through_the_production_decoder()
-> Result<()> {
    let code = "Provider.UnmodeledQuotaError";
    let message = format!(
        "{}\0tail  ",
        "  雪花\r\nsecond line\t\"quoted\"  ".repeat(80)
    );
    for protocol in [
        ModelProtocol::BedrockAnthropic,
        ModelProtocol::BedrockConverse,
    ] {
        for message in [&message, &String::new()] {
            let options = ModelOptions::for_protocol(protocol);
            let request = observed(protocol, body(protocol, protocol.model_id(), false));
            let reply = protocol.events([ModelEvent::error(code, message)]);
            let prepared = reply.prepare(&request, 1, &options)?;
            assert_eq!(prepared.chunks.len(), 1);
            let frame = &prepared.chunks[0];
            let length = u32::from_be_bytes(frame[..4].try_into()?) as usize;
            let headers_length = u32::from_be_bytes(frame[4..8].try_into()?) as usize;
            assert_eq!(length, frame.len());
            assert_eq!(
                length,
                headers_length + 16,
                "unmodeled errors have no payload"
            );

            let owner = ScriptedModel::start_with_options(options).await?;
            let model = owner.handle();
            let step = model
                .expect(RequestMatch::user_text("only-user"))
                .reply(reply.fragment_bytes(3));
            let response = client()?
                .post(format!(
                    "{}{}",
                    owner.base_url().trim_end_matches("/v1"),
                    protocol.request_path(protocol.model_id(), true)
                ))
                .json(&body(protocol, protocol.model_id(), true))
                .send()
                .await?
                .error_for_status()?;
            let results = decode_response(response).collect::<Vec<_>>().await;
            assert_eq!(results.len(), 1);
            let Err(BedrockAnthropicStreamDecodeError::Service(error)) = &results[0] else {
                anyhow::bail!("expected unmodeled service error, got {:?}", results[0]);
            };
            assert_eq!(error.event_type, code);
            assert_eq!(&error.message, message);
            step.finished().await?;
            model.assert_finished()?;
            owner.shutdown().await?;
        }
    }
    Ok(())
}

#[test]
fn unmodeled_error_events_reject_non_bedrock_protocols_and_empty_codes() -> Result<()> {
    for protocol in PROTOCOLS {
        let request = observed(protocol, body(protocol, protocol.model_id(), false));
        if !matches!(
            protocol,
            ModelProtocol::BedrockAnthropic | ModelProtocol::BedrockConverse
        ) {
            let error = protocol
                .events([ModelEvent::error("UnmodeledError", "message")])
                .prepare(&request, 1, &ModelOptions::for_protocol(protocol))
                .err()
                .context("non-Bedrock error event must fail explicitly")?;
            assert!(error.to_string().contains("require a Bedrock protocol"));
        } else {
            let error = protocol
                .events([ModelEvent::error("", "message")])
                .prepare(&request, 1, &ModelOptions::for_protocol(protocol))
                .err()
                .context("empty AWS error code must be rejected")?;
            assert!(error.to_string().contains("error code must not be empty"));
        }
    }
    Ok(())
}

fn sse_events(text: &str) -> Result<Vec<Value>> {
    text.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| Ok(serde_json::from_str(data)?))
        .collect()
}

/// Inspect AWS payloads without reusing the fixture's encoder or CRC routine.
/// CRC interoperability is checked separately by the production Smithy decoder.
fn unpack_aws(mut bytes: &[u8]) -> Result<Vec<(String, Value)>> {
    let mut output = Vec::new();
    while !bytes.is_empty() {
        ensure!(bytes.len() >= 16, "incomplete AWS frame");
        let length = u32::from_be_bytes(bytes[..4].try_into()?) as usize;
        let headers = u32::from_be_bytes(bytes[4..8].try_into()?) as usize;
        ensure!(length <= bytes.len() && length >= 16 && headers <= length - 16);
        let mut position = 12;
        let mut kind = None;
        while position < 12 + headers {
            let count = bytes[position] as usize;
            position += 1;
            let name = std::str::from_utf8(&bytes[position..position + count])?;
            position += count;
            ensure!(bytes[position] == 7, "expected string AWS header");
            position += 1;
            let count = u16::from_be_bytes(bytes[position..position + 2].try_into()?) as usize;
            position += 2;
            let value = std::str::from_utf8(&bytes[position..position + count])?;
            position += count;
            if matches!(name, ":event-type" | ":exception-type") {
                kind = Some(value.to_owned());
            }
        }
        let mut payload: Value = serde_json::from_slice(&bytes[12 + headers..length - 4])?;
        let mut kind = kind.context("event type")?;
        if kind == "chunk" {
            payload = serde_json::from_slice(
                &STANDARD.decode(payload["bytes"].as_str().context("base64 payload")?)?,
            )?;
            kind = payload["type"]
                .as_str()
                .context("Anthropic event type")?
                .to_owned();
        }
        output.push((kind, payload));
        bytes = &bytes[length..];
    }
    Ok(output)
}
