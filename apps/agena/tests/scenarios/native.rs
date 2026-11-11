//! Native model protocol end-to-end tests use the *same strict script engine*
//! but send different HTTP wire formats to actual provider adapters.

use super::fixture;
use agena_testkit::{NativeProtocol, RequestMatch, TestResult};
use anyhow::ensure;
use serde_json::json;

async fn native_text(protocol: NativeProtocol) -> TestResult {
    fixture()
        .native_protocol(protocol)
        .run(move |case| async move {
            let path = protocol.request_path().to_owned();
            let model = case
                .model()
                .expect(RequestMatch::new(
                    format!("native {protocol:?} wire request"),
                    move |request| {
                        request.method == "POST"
                            && request.path == path
                            && request.body.to_string().contains("native-adapter-question")
                            && (protocol != NativeProtocol::Gemini
                                || request.query_values("alt") == ["sse"])
                    },
                ))
                .reply(protocol.text("native-protocol-answered"));
            let user = case.user().new_session().await?;
            let result = user
                .send("native-adapter-question")
                .await?
                .wait_settled()
                .await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("native-protocol-answered")?;
            let request = model.observed().await?;
            ensure!(request.path == protocol.request_path());
            ensure!(request.body.to_string().contains("native-adapter-question"));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anthropic_messages_is_a_real_native_provider_stream() -> TestResult {
    native_text(NativeProtocol::Anthropic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gemini_generate_content_is_a_real_native_provider_stream() -> TestResult {
    native_text(NativeProtocol::Gemini).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ollama_chat_is_a_real_native_provider_json_line_stream() -> TestResult {
    native_text(NativeProtocol::Ollama).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn openai_chat_completions_uses_its_own_native_sse_protocol() -> TestResult {
    native_text(NativeProtocol::OpenAiChat).await
}

async fn native_tool(protocol: NativeProtocol) -> TestResult {
    fixture()
        .native_protocol(protocol)
        .seed_file("workspace/native.txt", b"actual-native-tool-bytes".to_vec())
        .run(move |case| async move {
            let route = protocol.request_path().to_owned();
            let initial = case
                .model()
                .expect(RequestMatch::new(
                    "native provider declares the original user request",
                    move |request| {
                        request.method == "POST"
                            && request.path == route
                            && request.body.to_string().contains("native-tool-file-read")
                    },
                ))
                .reply(protocol.tool(
                    "native-call-1",
                    "fs.read",
                    json!({"file_path":"native.txt"}),
                ));
            let route = protocol.request_path().to_owned();
            case.model()
                .expect(RequestMatch::new(
                    "native provider receives the true tool output",
                    move |request| {
                        request.path == route
                            && request
                                .body
                                .to_string()
                                .contains("actual-native-tool-bytes")
                            && (protocol != NativeProtocol::Ollama
                                || request.body["messages"].as_array().is_some_and(|messages| {
                                    messages.iter().any(|item| {
                                        item["role"] == "assistant"
                                            && item["tool_calls"][0]["function"]["name"] == "tools_call"
                                            && item["tool_calls"][0]["function"]["arguments"]["tool"] == "fs.read"
                                    }) && messages.iter().any(|item| {
                                        item["role"] == "tool"
                                            && item["content"].as_str().is_some_and(|s| s.contains("actual-native-tool-bytes"))
                                    })
                                }))
                    },
                ))
                .after(&initial)
                .reply(protocol.text("native-tool-verified"));

            let user = case.user().new_session().await?;
            let run = user.send("native-tool-file-read").await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("native-tool-verified")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anthropic_native_tool_calls_execute_real_filesystem() -> TestResult {
    native_tool(NativeProtocol::Anthropic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gemini_native_tool_calls_execute_real_filesystem() -> TestResult {
    native_tool(NativeProtocol::Gemini).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ollama_native_tool_calls_execute_real_filesystem() -> TestResult {
    native_tool(NativeProtocol::Ollama).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn openai_chat_native_tool_calls_execute_real_filesystem() -> TestResult {
    native_tool(NativeProtocol::OpenAiChat).await
}

/// A native adapter must propagate a real provider rejection; the local
/// server must never manufacture a normal assistant reply in this case.
async fn native_provider_rejects(protocol: NativeProtocol) -> TestResult {
    fixture().native_protocol(protocol).run(move |case| async move {
        let path = protocol.request_path().to_owned();
        let rejected = case.model().expect(RequestMatch::new(
            format!("native {protocol:?} deliberately returns HTTP 400"),
            move |request| request.path == path && request.body.to_string().contains("native-provider-rejection"),
        )).reply(agena_testkit::Reply::raw_json(400, json!({
            "error":{"message":"native fixture rejects this request","type":"invalid_request_error"}
        })));
        let user = case.user().new_session().await?;
        let result = user.send("native-provider-rejection").await?.wait_settled().await?;
        result.assert_failed()?;
        rejected.finished().await?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_anthropic_provider_errors_are_real_http_failures() -> TestResult {
    native_provider_rejects(NativeProtocol::Anthropic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_gemini_provider_errors_are_real_http_failures() -> TestResult {
    native_provider_rejects(NativeProtocol::Gemini).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ollama_provider_errors_are_real_http_failures() -> TestResult {
    native_provider_rejects(NativeProtocol::Ollama).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_openai_chat_provider_errors_are_real_http_failures() -> TestResult {
    native_provider_rejects(NativeProtocol::OpenAiChat).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_anthropic_uses_signed_invoke_and_binary_eventstream() -> TestResult {
    native_text(NativeProtocol::BedrockAnthropic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_anthropic_native_tools_execute_real_filesystem() -> TestResult {
    native_tool(NativeProtocol::BedrockAnthropic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_anthropic_provider_http_errors_are_real_failures() -> TestResult {
    native_provider_rejects(NativeProtocol::BedrockAnthropic).await
}

/// The same request matchers, thinking/text builders, two parallel tool calls,
/// byte fragmentation, and strict tool-result checks traverse each real parser.
async fn uniform_native_round_trip(protocol: agena_testkit::ModelProtocol) -> TestResult {
    use agena_testkit::{Reply, Usage};
    fixture()
        .model_protocol(protocol)
        .seed_file("workspace/native-a.txt", b"first-native-evidence".to_vec())
        .seed_file("workspace/native-b.txt", b"second-native-evidence".to_vec())
        .run(move |case| async move {
            let first = case
                .model()
                .expect(
                    RequestMatch::user_text("uniform-native-round-trip")
                        .protocol(protocol)
                        .streaming(true)
                        .declares_function("tools_call"),
                )
                .reply(
                    Reply::text_chunks(["原生", "回答"])
                        .with_thinking_chunks(["inspect ", "both files"])
                        .with_tool_call("native-a", "fs.read", json!({"file_path":"native-a.txt"}))
                        .with_tool_call("native-b", "fs.read", json!({"file_path":"native-b.txt"}))
                        .usage(Usage::tokens(17, 11).cached(4).reasoning(3))
                        .fragment_bytes(7),
                );
            let second = case
                .model()
                .expect(
                    RequestMatch::tool_output("native-a")
                        .and_named("second correlated tool output", |request| {
                            request.tool_output("native-b").is_some()
                        })
                        .protocol(protocol),
                )
                .after(&first)
                .check_tool_output_contains("native-a", "first-native-evidence")
                .check_tool_output_contains("native-b", "second-native-evidence")
                .reply(Reply::text_chunks(["完成", "验证"]).fragment_bytes(5));
            let user = case.user().new_session().await?;
            let result = user
                .send("uniform-native-round-trip")
                .await?
                .wait_settled()
                .await?;
            result.assert_succeeded()?;
            result.assert_text_contains("原生回答")?;
            result.assert_text_contains("完成验证")?;
            let request = first.observed().await?;
            ensure!(request.protocol() == Some(protocol));
            ensure!(request.decoded_model().as_deref() == Some(protocol.model_id()));
            if protocol == agena_testkit::ModelProtocol::BedrockAnthropic {
                ensure!(
                    request
                        .header("authorization")
                        .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256 "))
                );
                ensure!(request.header("x-amz-date").is_some());
                ensure!(
                    request.body.get("model").is_none(),
                    "Bedrock native model is in the path"
                );
            }
            second.finished().await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anthropic_uniform_api_keeps_thinking_parallel_tools_and_utf8_fragments() -> TestResult {
    uniform_native_round_trip(agena_testkit::ModelProtocol::Anthropic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gemini_uniform_api_keeps_thinking_parallel_tools_and_utf8_fragments() -> TestResult {
    uniform_native_round_trip(agena_testkit::ModelProtocol::Gemini).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_uniform_api_keeps_thinking_parallel_tools_and_utf8_fragments() -> TestResult {
    uniform_native_round_trip(agena_testkit::ModelProtocol::BedrockAnthropic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binary_service_exception_fails_the_actual_run() -> TestResult {
    use agena_testkit::{ModelEvent, ModelProtocol};
    fixture()
        .model_protocol(ModelProtocol::BedrockAnthropic)
        .run(|case| async move {
            let failure = case
                .model()
                .expect(RequestMatch::user_text("native-binary-error"))
                .reply(
                    ModelProtocol::BedrockAnthropic
                        .events([ModelEvent::exception(
                            "validationException",
                            json!({"message":"fixture-native-validation-error"}),
                        )])
                        .fragment_bytes(3),
                );
            let user = case.user().new_session().await?;
            let result = user
                .send("native-binary-error")
                .await?
                .wait_settled()
                .await?;
            result.assert_failed()?;
            failure.finished().await?;
            Ok(())
        })
        .await
}

/// A clean HTTP EOF is not an application-level completion. In particular,
/// the adapter must not turn partially streamed text into a successful turn.
async fn incomplete_native_stream_fails(protocol: agena_testkit::ModelProtocol) -> TestResult {
    use agena_testkit::ModelEvent;
    let events = match protocol {
        agena_testkit::ModelProtocol::Anthropic
        | agena_testkit::ModelProtocol::BedrockAnthropic => vec![
            ModelEvent::json(json!({
                "type":"message_start",
                "message":{"id":"incomplete-response", "type":"message", "role":"assistant",
                    "model":protocol.model_id(), "content":[], "stop_reason":null,
                    "stop_sequence":null,"usage":{"input_tokens":5,"output_tokens":0}}
            })),
            ModelEvent::json(json!({"type":"content_block_start", "index":0,
                "content_block":{"type":"text","text":""}})),
            ModelEvent::json(json!({"type":"content_block_delta", "index":0,
                "delta":{"type":"text_delta","text":"unconfirmed-prefix"}})),
        ],
        agena_testkit::ModelProtocol::Gemini => vec![ModelEvent::json(json!({
            "modelVersion":"fake-model", "candidates":[{"index":0,
                "content":{"role":"model","parts":[{"text":"unconfirmed-prefix"}]}}]
        }))],
        agena_testkit::ModelProtocol::Ollama => vec![ModelEvent::json(json!({
            "model":"fake-model","created_at":"1970-01-01T00:00:00Z",
            "message":{"role":"assistant","content":"unconfirmed-prefix"}, "done":false
        }))],
        _ => unreachable!("only missing-terminal native streams use this fixture"),
    };
    fixture().model_protocol(protocol).run(move |case| async move {
        let step = case.model()
            .expect(RequestMatch::user_text("native-missing-terminal").protocol(protocol))
            .reply(protocol.events(events));
        let user = case.user().new_session().await?;
        let run = user.send("native-missing-terminal").await?;
        let result = run.wait_settled().await?;
        step.finished().await?;
        result.assert_failed()?;
        ensure!(
            !result.assert_succeeded().is_ok(),
            "{protocol:?} treated a clean HTTP EOF without an application terminal as a successful turn"
        );
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_anthropic_clean_eof_without_message_stop_fails() -> TestResult {
    incomplete_native_stream_fails(agena_testkit::ModelProtocol::Anthropic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_bedrock_clean_eof_without_message_stop_fails() -> TestResult {
    incomplete_native_stream_fails(agena_testkit::ModelProtocol::BedrockAnthropic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_gemini_clean_eof_without_finish_reason_fails() -> TestResult {
    incomplete_native_stream_fails(agena_testkit::ModelProtocol::Gemini).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ollama_clean_eof_without_done_fails() -> TestResult {
    incomplete_native_stream_fails(agena_testkit::ModelProtocol::Ollama).await
}

/// Split one Responses SSE event into an HTTP body chunk. The client must
/// tolerate arbitrary TCP chunk boundaries, so this only shapes the bytes.
fn sse_frame(event: serde_json::Value) -> String {
    format!("data: {event}\n\n")
}

/// A Responses gateway can corrupt its terminal frame after every output item
/// has already been delivered. Failing that turn would throw away an answer
/// the user fully received, so the adapter must close it successfully.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responses_corrupt_terminal_after_completed_items_keeps_the_answer() -> TestResult {
    use agena_testkit::Reply;
    let frames = [
        sse_frame(json!({"type":"response.created","response":{"id":"resp_corrupt"}})),
        sse_frame(json!({"type":"response.output_item.added","output_index":0,
            "item":{"type":"reasoning","id":"rs_1","summary":[]}})),
        sse_frame(
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1",
            "output_index":0,"summary_index":0,"delta":"checked the sources"}),
        ),
        sse_frame(json!({"type":"response.output_item.done","output_index":0,
            "item":{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"checked the sources"}]}})),
        sse_frame(json!({"type":"response.output_item.added","output_index":1,
            "item":{"type":"message","id":"msg_1","role":"assistant","content":[]}})),
        sse_frame(
            json!({"type":"response.output_text.delta","item_id":"msg_1",
            "output_index":1,"content_index":0,"delta":"corrupt-terminal-answer"}),
        ),
        sse_frame(json!({"type":"response.output_item.done","output_index":1,
            "item":{"type":"message","id":"msg_1","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":"corrupt-terminal-answer"}]}})),
        // Truncated JSON: the gateway dies mid-frame on the terminator.
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_corrupt\"".to_owned(),
    ];
    fixture()
        .run(move |case| async move {
            let step = case
            .model()
            .expect(RequestMatch::user_text("corrupt-terminal-marker"))
            // Raw bytes: the frames above are already SSE-framed, and the
            // last one is deliberately truncated JSON.
            .reply(Reply::raw_stream(200, frames.clone()));
            let user = case.user().new_session().await?;
            let result = user
                .send("corrupt-terminal-marker")
                .await?
                .wait_settled()
                .await?;
            step.finished().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("corrupt-terminal-answer")?;
            Ok(())
        })
        .await
}
