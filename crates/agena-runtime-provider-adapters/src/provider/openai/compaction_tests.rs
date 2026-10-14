use super::*;
use agena_provider::{ProviderCompactionContext, ProviderCompactionOutput, ProviderErrorKind};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

async fn compact_fixture(
    request: CompletionRequest,
    response: Value,
) -> (
    Result<Option<ProviderCompactionOutput>, ProviderError>,
    String,
    Value,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                .await
                .expect("compact request reached fixture")
                .unwrap();
        let captured = super::tests::read_http_request(&mut stream).await;
        let body = response.to_string();
        let headers = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(headers.as_bytes()).await.unwrap();
        stream.write_all(body.as_bytes()).await.unwrap();
        captured
    });
    let adapter = OpenAiResponsesAdapter::new_managed_with_options(
        "compact-fixture",
        reqwest::Client::new(),
        ManagedCredential::static_value("fixture", "fixture-key"),
        format!("http://{address}/v1"),
        "gpt-6-luna",
        OpenAiResponsesAdapterOptions::default(),
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ModelRuntime::compact_conversation(&adapter, request),
    )
    .await
    .expect("bounded compact request");
    let (headers, body) = server.await.unwrap();
    (result, headers, body)
}

fn request() -> CompletionRequest {
    serde_json::from_value(json!({
        "model": "gpt-6-luna",
        "system": "Preserve the current coding task.",
        "messages": [{"role": "user", "parts": [{"type": "text", "text": "Continue the permission fix."}]}],
    }))
    .unwrap()
}

#[tokio::test]
async fn compact_uses_its_own_http_schema_and_replays_the_complete_native_window() {
    let previous = json!({"type": "compaction", "encrypted_content": "previous opaque state"});
    let retained = json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Retained constraint"}]});
    let checkpoint = json!({"type": "compaction", "id": "cmp_next", "encrypted_content": "next opaque state", "vendor_metadata": {"keep": true}});
    let output = vec![retained.clone(), checkpoint.clone()];
    let mut request = request();
    request.provider_compaction = Some(ProviderCompactionContext::OpenAiResponses {
        items: vec![retained.clone(), previous.clone()],
    });
    request.previous_response_id = Some("resp_stale".to_owned());
    request.model_supports_parallel_tool_calls = Some(true);
    request.thinking = Some(ThinkingRequest::Effort {
        effort: ReasoningEffort::High,
    });
    request.verbosity = Some("high".to_owned());
    request
        .tool_api_functions
        .push(agena_provider::ToolApiDefinition {
            handler_key: "fixture".to_owned(),
            plugin_name: "fixture".to_owned(),
            name: "fixture_tool".to_owned(),
            description: "Fixture tool".to_owned(),
            input_schema: json!({"type": "object", "properties": {}}),
            output_schema: json!({}),
            definition_identity: "fixture".to_owned(),
        });
    request
        .request_override
        .headers
        .insert("x-fixture-route".to_owned(), "sticky".to_owned());
    request.request_override.body_patch = serde_json::from_value(json!({
        "model": "wrong-model",
        "instructions": "Wrong instructions",
        "input": [],
        "previous_response_id": "resp_wrong",
        "tools": [{"type": "web_search"}],
        "parallel_tool_calls": true,
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": 900000,
        "reasoning": {"effort": "high"},
        "text": {"format": {"type": "json_object"}},
        "stream": true,
        "store": true,
        "temperature": 1,
        "prompt_cache_key": "compact-cache",
        "prompt_cache_options": {"mode": "implicit", "ttl": "30m"},
        "service_tier": "priority"
    }))
    .unwrap();
    let (result, headers, body) = compact_fixture(request, json!({"output": output})).await;
    assert!(headers.starts_with("POST /v1/responses/compact HTTP/1.1\r\n"));
    let headers = headers.to_ascii_lowercase();
    assert!(headers.contains("authorization: bearer fixture-key\r\n"));
    assert!(headers.contains("content-type: application/json\r\n"));
    assert!(headers.contains("x-fixture-route: sticky\r\n"));
    assert_eq!(
        body.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "input",
            "instructions",
            "model",
            "prompt_cache_key",
            "prompt_cache_options",
            "service_tier"
        ]
    );
    assert_eq!(body["model"], "gpt-6-luna");
    assert_eq!(body["instructions"], "Preserve the current coding task.");
    assert_eq!(body["input"][0], retained);
    assert_eq!(body["input"][1], previous);
    assert_eq!(
        body["input"][2]["content"][0]["text"],
        "Continue the permission fix."
    );
    assert_eq!(body["service_tier"], "priority");
    assert_eq!(body["prompt_cache_options"]["ttl"], "30m");
    assert_eq!(
        result.unwrap(),
        Some(ProviderCompactionOutput::OpenAiResponses { items: output })
    );
}

#[tokio::test]
async fn compact_rejects_success_envelopes_without_a_valid_checkpoint() {
    for output in [
        json!([]),
        json!([{}]),
        json!([{"type": "message", "role": "assistant", "content": []}]),
        json!([{"type": "compaction", "encrypted_content": " "}]),
        json!([{"type": "compaction", "encrypted_content": "opaque"}, null]),
    ] {
        let (result, _, _) = compact_fixture(request(), json!({"output": output})).await;
        assert_eq!(
            result.unwrap_err().provider_error_kind(),
            Some(ProviderErrorKind::MalformedResponse)
        );
    }
    let (result, _, _) = compact_fixture(request(), json!({"status": "completed"})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn a_tool_result_can_continue_a_call_retained_in_the_native_checkpoint() {
    let mut request = request();
    request.provider_compaction = Some(ProviderCompactionContext::OpenAiResponses {
        items: vec![
            json!({"type": "compaction", "encrypted_content": "opaque"}),
            json!({"type": "function_call", "call_id": "call_pending", "name": "fixture_tool", "arguments": "{}"}),
        ],
    });
    request.turns = vec![agena_provider::CompletionInputRun {
        role: Role::Tool,
        parts: vec![agena_provider::CompletionInputPart::ToolResult {
            attachments: Vec::new(),
            tool_call_id: "call_pending".to_owned(),
            function: agena_provider::ModelToolFunction::new("fixture_tool"),
            arguments_json: "{}".to_owned(),
            status: agena_provider::CompletionInputToolResultStatus::Completed,
            output_json: "tool result after compaction".to_owned(),
        }],
        provider_state: Default::default(),
    }];
    let (result, _, body) = compact_fixture(
        request,
        json!({"output": [{"type": "compaction", "encrypted_content": "next opaque"}]}),
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(body["input"][1]["type"], "function_call");
    assert_eq!(body["input"][2]["type"], "function_call_output");
    assert_eq!(body["input"][2]["call_id"], "call_pending");
}
