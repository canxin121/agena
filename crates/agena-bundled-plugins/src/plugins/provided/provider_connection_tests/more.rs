use super::*;
use agena_provider::{AttributedCompletionUsage, PROVIDER_TOOL_USAGE_METADATA_KEY};

async fn setup(
    index: usize,
    root: &Path,
    base: String,
    model: Option<&str>,
) -> (Box<dyn Plugin>, Arc<Host>) {
    let plugin = plugins().remove(index);
    let host = Arc::new(Host {
        connection: Mutex::new(connection(base, model)),
        resolutions: Mutex::new(0),
        network: Mutex::new(Vec::new()),
    });
    initialize(plugin.as_ref(), root, host.clone()).await;
    (plugin, host)
}

struct Reply {
    mime: &'static str,
    body: Vec<u8>,
    upload: Option<String>,
}
impl Reply {
    fn json(value: Value) -> Self {
        Self {
            mime: "application/json",
            body: value.to_string().into_bytes(),
            upload: None,
        }
    }
}
async fn scripted(
    replies: Vec<Reply>,
) -> (String, tokio::task::JoinHandle<Vec<(String, Vec<u8>)>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let replacement = base.clone();
    let task = tokio::spawn(async move {
        let mut captured = Vec::new();
        for reply in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let captured_request = loop {
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(offset) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8(request[..offset].to_vec()).unwrap();
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap_or_else(|| "0".into())
                        .parse()
                        .unwrap();
                    if request.len() >= offset + 4 + length {
                        break (header, request[offset + 4..offset + 4 + length].to_vec());
                    }
                }
            };
            captured.push(captured_request);
            let upload = reply
                .upload
                .map(|s| {
                    format!(
                        "X-Goog-Upload-URL: {}\r\n",
                        s.replace("$BASE", &replacement)
                    )
                })
                .unwrap_or_default();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n{upload}Connection: close\r\n\r\n",reply.mime,reply.body.len()).as_bytes()).await.unwrap();
            socket.write_all(&reply.body).await.unwrap();
        }
        captured
    });
    (base, task)
}

#[tokio::test]
async fn native_multipart_repeats_arrays_and_names_consent_recordings() {
    for (tool, input, filename, expected) in [
        (
            "transcription",
            json!({"parameters":{"timestamp_granularities":["word","segment"],"chunking_strategy":{"type":"server_vad","threshold":0.5}}}),
            "input.wav",
            "file",
        ),
        (
            "platform",
            json!({"action":"create","resource":"audio/voice_consents","parameters":{"name":"speaker","language":"zh"}}),
            "consent.wav",
            "recording",
        ),
        (
            "platform",
            json!({"action":"create","resource":"audio/voices","parameters":{"name":"speaker","consent":"cons_1"}}),
            "sample.wav",
            "audio_sample",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(filename), b"RIFF-explicit-recording").unwrap();
        let (base, captured) = server(json!({"id":"created"})).await;
        let (plugin, _) = setup(0, root.path(), format!("{base}/v1"), Some("transcriber")).await;
        let mut input = input;
        input["path"] = json!(filename);
        invoke(plugin.as_ref(), root.path(), tool, input)
            .await
            .unwrap();
        let (header, body) = captured.await.unwrap();
        assert!(header.to_lowercase().contains("multipart/form-data"));
        let body = String::from_utf8(body).unwrap();
        assert!(
            body.contains(&format!("name=\"{expected}\"; filename=\"{filename}\"")),
            "{body}"
        );
        assert!(body.contains("Content-Type: audio/wav"), "{body}");
        if tool == "transcription" {
            assert_eq!(
                body.matches("name=\"timestamp_granularities[]\"").count(),
                2
            );
            assert!(body.contains("name=\"chunking_strategy[type]\""));
            assert!(body.contains("name=\"chunking_strategy[threshold]\""));
        }
    }
}

#[tokio::test]
async fn prompt_voices_and_compaction_need_no_file_and_use_official_bodies() {
    for (tool, input, path) in [
        (
            "platform",
            json!({"action":"create","resource":"audio/voices","parameters":{"type":"prompt","name":"narrator","prompt":"calm voice"}}),
            "audio/voices",
        ),
        (
            "responses",
            json!({"action":"compact","model":"explicit-openai","parameters":{"input":[{"role":"user","content":"hello"}]}}),
            "responses/compact",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (base, captured) = server(json!({"id":"resource"})).await;
        let (plugin, _) = setup(0, root.path(), format!("{base}/v1"), Some("current-claude")).await;
        invoke(plugin.as_ref(), root.path(), tool, input)
            .await
            .unwrap();
        let (header, body) = captured.await.unwrap();
        assert!(header.starts_with(&format!("POST /v1/{path} HTTP")));
        let body: Value = serde_json::from_slice(&body).unwrap();
        if tool == "platform" {
            assert!(body.get("model").is_none());
            assert_eq!(body["prompt"], "calm voice");
        } else {
            assert_eq!(body["model"], "explicit-openai");
            assert!(body.get("stream").is_none());
        }
    }
}

#[tokio::test]
async fn agent_artifacts_download_bytes_and_nested_resources_preserve_ids() {
    for (index, input, path) in [
        (
            0,
            json!({"resource":"sessions/artifacts","action":"download","id":"s1","item_id":"a1"}),
            "/v1/agents/sessions/s1/artifacts/a1/content",
        ),
        (
            0,
            json!({"resource":"sessions/subagents/turns","action":"items","id":"s1","item_id":"agent1","sub_item_id":"turn1"}),
            "/v1/agents/sessions/s1/subagents/agent1/turns/turn1/items",
        ),
        (
            1,
            json!({"resource":"sessions/resources","action":"get","id":"s1","item_id":"r1"}),
            "/v1/sessions/s1/resources/r1",
        ),
        (
            1,
            json!({"resource":"sessions/threads","action":"events","id":"s1","item_id":"t1"}),
            "/v1/sessions/s1/threads/t1/events",
        ),
        (
            1,
            json!({"resource":"deployment_runs","action":"get","id":"run1"}),
            "/v1/deployment_runs/run1",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let binary = input["action"] == "download";
        let bytes = if binary {
            b"generated artifact bytes".to_vec()
        } else {
            b"{\"id\":\"result\"}".to_vec()
        };
        let (base, captured) = server_bytes(
            if binary {
                "text/plain"
            } else {
                "application/json"
            },
            bytes.clone(),
        )
        .await;
        let (plugin, _) = setup(index, root.path(), format!("{base}/v1"), None).await;
        let output = invoke(plugin.as_ref(), root.path(), "agents", input)
            .await
            .unwrap();
        assert!(
            captured
                .await
                .unwrap()
                .0
                .starts_with(&format!("GET {path} HTTP"))
        );
        if binary {
            let attachment = &output.attachments[0];
            if let agena_plugin_sdk::attachment::AttachmentSource::LocalPath { path } =
                &attachment.source
            {
                assert_eq!(std::fs::read(path).unwrap(), bytes);
            } else {
                panic!("expected saved artifact");
            }
        }
    }
}

#[tokio::test]
async fn gemini_batch_embeddings_use_selected_model_and_native_endpoints() {
    for (tool, input, path) in [
        (
            "embeddings",
            json!({"action":"create_batch","model":"embedding-model","parameters":{"requests":[{"content":{"parts":[{"text":"a"}]}},{"model":"wrong-model","content":{"parts":[{"text":"b"}]}}]}}),
            "/v1beta/models/embedding-model:batchEmbedContents",
        ),
        (
            "batches",
            json!({"action":"create","resource":"embeddings","model":"embedding-model","parameters":{"batch":{"displayName":"batch","inputConfig":{"fileName":"files/data"}}}}),
            "/v1beta/models/embedding-model:asyncBatchEmbedContent",
        ),
        (
            "batches",
            json!({"action":"update","resource":"embeddings","id":"batches/b1","model":"embedding-model","parameters":{"displayName":"updated"},"query":{"updateMask":"display_name"}}),
            "/v1beta/batches/b1:updateEmbedContentBatch?updateMask=display_name",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (base, captured) = server(json!({"embeddings":[{"values":[1]}]})).await;
        let (plugin, _) = setup(
            2,
            root.path(),
            format!("{base}/v1beta"),
            Some("wrong-chat-model"),
        )
        .await;
        invoke(plugin.as_ref(), root.path(), tool, input)
            .await
            .unwrap();
        let (header, body) = captured.await.unwrap();
        assert!(header.contains(&format!(" {path} HTTP")), "{header}");
        let body: Value = serde_json::from_slice(&body).unwrap();
        if tool == "embeddings" {
            for request in body["requests"].as_array().unwrap() {
                assert_eq!(request["model"], "models/embedding-model");
            }
        } else if header.starts_with("PATCH") {
            assert_eq!(body["model"], "models/embedding-model");
        }
    }
}

#[tokio::test]
async fn file_search_resumable_upload_retains_metadata_and_checks_each_network_target() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("doc.txt"), b"explicit document").unwrap();
    let replies = vec![
        Reply{mime:"application/json",body:b"{}".to_vec(),upload:Some("$BASE/prefix/upload/v1beta/fileSearchStores/store:uploadToFileSearchStore?upload_id=opaque".into())},
        Reply::json(json!({"name":"fileSearchStores/store/upload/operations/op"})),
        Reply::json(json!({"name":"fileSearchStores/store/upload/operations/op","done":true,"response":{"documentName":"fileSearchStores/store/documents/doc"}})),
    ];
    let (base, captured) = scripted(replies).await;
    let (plugin, host) = setup(2, root.path(), format!("{base}/prefix/v1beta"), None).await;
    let output = invoke(plugin.as_ref(),root.path(),"knowledge",json!({"action":"upload","id":"fileSearchStores/store","path":"doc.txt","parameters":{"customMetadata":[{"key":"source","stringValue":"local"}],"chunkingConfig":{"whiteSpaceConfig":{"maxTokensPerChunk":200}}}})).await.unwrap();
    assert!(
        output.payload.as_ref().unwrap()["pending"]
            .as_bool()
            .unwrap()
    );
    invoke(
        plugin.as_ref(),
        root.path(),
        "operations",
        json!({"action":"wait","id":"fileSearchStores/store/upload/operations/op"}),
    )
    .await
    .unwrap();
    let requests = captured.await.unwrap();
    assert!(requests[0].0.starts_with(
        "POST /prefix/upload/v1beta/fileSearchStores/store:uploadToFileSearchStore HTTP"
    ));
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].1).unwrap()["customMetadata"][0]["key"],
        "source"
    );
    assert!(requests[1].0.contains("upload_id=opaque"));
    assert_eq!(requests[1].1, b"explicit document");
    assert!(
        requests[2]
            .0
            .starts_with("GET /prefix/v1beta/fileSearchStores/store/upload/operations/op HTTP")
    );
    assert!(host.network.lock().unwrap().iter().any(|s| {
        s.ends_with("/prefix/upload/v1beta/fileSearchStores/store:uploadToFileSearchStore")
    }));
}

#[tokio::test]
async fn file_search_rejects_cross_origin_upload_before_sending_bytes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("doc.txt"), b"do not leak").unwrap();
    let (base, captured) = scripted(vec![Reply {
        mime: "application/json",
        body: b"{}".to_vec(),
        upload: Some(
            "http://127.0.0.1:1/upload/v1beta/fileSearchStores/store:uploadToFileSearchStore"
                .into(),
        ),
    }])
    .await;
    let (plugin, _) = setup(2, root.path(), format!("{base}/v1beta"), None).await;
    let error = invoke(
        plugin.as_ref(),
        root.path(),
        "knowledge",
        json!({"action":"upload","id":"store","path":"doc.txt"}),
    )
    .await
    .unwrap_err();
    assert!(error.failure.user.fallback.contains("origin"));
    assert!(!String::from_utf8_lossy(&captured.await.unwrap()[0].1).contains("do not leak"));
}

#[tokio::test]
async fn async_usage_is_recorded_once_after_completion_even_across_tool_aliases() {
    let root = tempfile::tempdir().unwrap();
    let completed = json!({"id":"resp_paid","status":"completed","model":"actual-model","usage":{"input_tokens":11,"output_tokens":7}});
    let (base, captured) = scripted(vec![
        Reply::json(json!({"id":"resp_paid","status":"queued"})),
        Reply::json(completed.clone()),
        Reply::json(completed),
    ])
    .await;
    let (plugin, _) = setup(0, root.path(), format!("{base}/v1"), Some("selected-model")).await;
    let started = invoke(
        plugin.as_ref(),
        root.path(),
        "inference",
        json!({"prompt":"work","parameters":{"background":true}}),
    )
    .await
    .unwrap();
    assert!(
        !started
            .metadata
            .contains_key(PROVIDER_TOOL_USAGE_METADATA_KEY)
    );
    let finished = invoke(
        plugin.as_ref(),
        root.path(),
        "responses",
        json!({"action":"wait","id":"resp_paid"}),
    )
    .await
    .unwrap();
    let usage: AttributedCompletionUsage =
        serde_json::from_str(&finished.metadata[PROVIDER_TOOL_USAGE_METADATA_KEY]).unwrap();
    assert_eq!(usage.model_id, "actual-model");
    assert_eq!(usage.usage.input_tokens, 11);
    assert_eq!(usage.usage.output_tokens, 7);
    let again = invoke(
        plugin.as_ref(),
        root.path(),
        "responses",
        json!({"action":"get","id":"resp_paid"}),
    )
    .await
    .unwrap();
    assert!(
        !again
            .metadata
            .contains_key(PROVIDER_TOOL_USAGE_METADATA_KEY)
    );
    assert_eq!(captured.await.unwrap().len(), 3);
}

#[tokio::test]
async fn inference_without_usage_does_not_claim_later_usage() {
    for initial in [
        json!({"id":"resp_delayed_usage"}),
        json!({"id":"resp_delayed_usage","status":"completed"}),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (base, captured) = scripted(vec![
            Reply::json(initial),
            Reply::json(json!({"id":"resp_delayed_usage","status":"completed","model":"actual-model","usage":{"input_tokens":11,"output_tokens":7}})),
        ])
        .await;
        let (plugin, _) = setup(0, root.path(), format!("{base}/v1"), Some("selected-model")).await;
        let started = invoke(
            plugin.as_ref(),
            root.path(),
            "inference",
            json!({"prompt":"work"}),
        )
        .await
        .unwrap();
        assert!(
            !started
                .metadata
                .contains_key(PROVIDER_TOOL_USAGE_METADATA_KEY)
        );
        let finished = invoke(
            plugin.as_ref(),
            root.path(),
            "responses",
            json!({"action":"get","id":"resp_delayed_usage"}),
        )
        .await
        .unwrap();
        let usage: AttributedCompletionUsage =
            serde_json::from_str(&finished.metadata[PROVIDER_TOOL_USAGE_METADATA_KEY]).unwrap();
        assert_eq!(usage.usage.input_tokens, 11);
        assert_eq!(usage.usage.output_tokens, 7);
        captured.await.unwrap();
    }
}

#[tokio::test]
async fn claude_batch_results_keep_downloads_and_attribute_each_model_once() {
    let root = tempfile::tempdir().unwrap();
    let bytes = [
        json!({"custom_id":"a","result":{"type":"succeeded","message":{"model":"claude-a","usage":{"input_tokens":10,"output_tokens":3}}}}),
        json!({"custom_id":"b","result":{"type":"succeeded","message":{"model":"claude-a","usage":{"input_tokens":20,"output_tokens":4}}}}),
        json!({"custom_id":"c","result":{"type":"succeeded","message":{"model":"claude-b","usage":{"input_tokens":30,"output_tokens":5}}}}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n").into_bytes();
    let replies = (0..2)
        .map(|_| Reply {
            mime: "application/x-ndjson",
            body: bytes.clone(),
            upload: None,
        })
        .collect();
    let (base, captured) = scripted(replies).await;
    let (plugin, _) = setup(1, root.path(), format!("{base}/v1"), None).await;
    let output = invoke(
        plugin.as_ref(),
        root.path(),
        "batches",
        json!({"action":"results","id":"batch1"}),
    )
    .await
    .unwrap();
    let usage: AttributedCompletionUsage =
        serde_json::from_str(&output.metadata[PROVIDER_TOOL_USAGE_METADATA_KEY]).unwrap();
    assert_eq!(usage.usage.flattened().input_tokens, 60);
    assert_eq!(usage.usage.flattened().output_tokens, 12);
    assert_eq!(usage.usage.flattened().requests, 3);
    assert_eq!(output.attachments.len(), 1);
    let output = invoke(
        plugin.as_ref(),
        root.path(),
        "batches",
        json!({"action":"results","id":"batch1"}),
    )
    .await
    .unwrap();
    assert!(
        !output
            .metadata
            .contains_key(PROVIDER_TOOL_USAGE_METADATA_KEY)
    );
    captured.await.unwrap();
}

#[tokio::test]
async fn inference_stream_retains_sse_cursors_and_final_usage() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"id: cursor-1\nevent: response.completed\nretry: 1200\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"model\":\"actual-model\",\"status\":\"completed\",\"usage\":{\"input_tokens\":9,\"output_tokens\":2}}}\n\n".to_vec();
    let (base, captured) = server_bytes("text/event-stream", payload).await;
    let (plugin, _) = setup(0, root.path(), format!("{base}/v1"), Some("selected-model")).await;
    let output = invoke(
        plugin.as_ref(),
        root.path(),
        "inference",
        json!({"prompt":"work","stream":true,"last_event_id":"cursor-0"}),
    )
    .await
    .unwrap();
    let (header, body) = captured.await.unwrap();
    assert!(header.to_lowercase().contains("last-event-id: cursor-0"));
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["stream"],
        true
    );
    let response = &output.payload.as_ref().unwrap()["response"]["response"];
    assert_eq!(response["last_event_id"], "cursor-1");
    assert_eq!(response["sse_events"][0]["event"], "response.completed");
    assert_eq!(response["sse_events"][0]["retry"], 1200);
    let usage: AttributedCompletionUsage =
        serde_json::from_str(&output.metadata[PROVIDER_TOOL_USAGE_METADATA_KEY]).unwrap();
    assert_eq!(usage.usage.input_tokens, 9);
}

#[tokio::test]
async fn platform_nested_resources_support_real_checkpoint_ids_and_official_json() {
    for (input, path, method) in [
        (
            json!({"resource":"fine_tuning/jobs/checkpoints","action":"list","id":"ftjob_1"}),
            "fine_tuning/jobs/ftjob_1/checkpoints",
            "GET",
        ),
        (
            json!({"resource":"fine_tuning/checkpoints/permissions","action":"list","id":"ft:gpt-model:org:run:step-1"}),
            "fine_tuning/checkpoints/ft%3Agpt-model%3Aorg%3Arun%3Astep-1/permissions",
            "GET",
        ),
        (
            json!({"resource":"evals/runs/output_items","action":"get","id":"eval_1","item_id":"run_1","sub_item_id":"output_1"}),
            "evals/eval_1/runs/run_1/output_items/output_1",
            "GET",
        ),
        (
            json!({"resource":"fine_tuning/alpha/graders","action":"validate","parameters":{"grader":{"type":"label_model","model":"grader-model"}}}),
            "fine_tuning/alpha/graders/validate",
            "POST",
        ),
        (
            json!({"resource":"audio/voice_consents","action":"update","id":"cons_1","parameters":{"name":"new label"}}),
            "audio/voice_consents/cons_1",
            "POST",
        ),
        (
            json!({"resource":"audio/voices","action":"delete","id":"voice_1"}),
            "audio/voices/voice_1",
            "DELETE",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (base, captured) = server(json!({"id":"result"})).await;
        let (plugin, _) = setup(0, root.path(), format!("{base}/v1"), None).await;
        invoke(plugin.as_ref(), root.path(), "platform", input)
            .await
            .unwrap();
        let (header, body) = captured.await.unwrap();
        assert!(
            header.starts_with(&format!("{method} /v1/{path} HTTP")),
            "{header}"
        );
        if method == "POST" {
            assert!(
                serde_json::from_slice::<Value>(&body)
                    .unwrap()
                    .get("model")
                    .is_none()
            );
        }
    }
}

#[tokio::test]
async fn completed_gemini_batch_operation_attributes_inline_usage_once() {
    let root = tempfile::tempdir().unwrap();
    let completed = json!({"name":"batches/batch1","done":true,"response":{"name":"batches/batch1","model":"models/gemini-test","output":{"inlinedResponses":{"inlinedResponses":[{"response":{"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":3}}},{"response":{"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":4}}}]}}}});
    let (base, captured) =
        scripted(vec![Reply::json(completed.clone()), Reply::json(completed)]).await;
    let (plugin, _) = setup(2, root.path(), format!("{base}/v1beta"), None).await;
    let output = invoke(
        plugin.as_ref(),
        root.path(),
        "batches",
        json!({"action":"get","id":"batch1"}),
    )
    .await
    .unwrap();
    let usage: AttributedCompletionUsage =
        serde_json::from_str(&output.metadata[PROVIDER_TOOL_USAGE_METADATA_KEY]).unwrap();
    assert_eq!(usage.usage.input_tokens, 30);
    assert_eq!(usage.usage.output_tokens, 7);
    assert_eq!(usage.usage.requests, 2);
    let again = invoke(
        plugin.as_ref(),
        root.path(),
        "batches",
        json!({"action":"get","id":"batches/batch1"}),
    )
    .await
    .unwrap();
    assert!(
        !again
            .metadata
            .contains_key(PROVIDER_TOOL_USAGE_METADATA_KEY)
    );
    captured.await.unwrap();
}

#[tokio::test]
async fn concurrent_response_polls_cannot_double_count_usage() {
    let root = tempfile::tempdir().unwrap();
    let completed = json!({"id":"resp_concurrent","model":"actual-model","status":"completed","usage":{"input_tokens":5,"output_tokens":2}});
    let (base, captured) =
        scripted(vec![Reply::json(completed.clone()), Reply::json(completed)]).await;
    let (plugin, _) = setup(0, root.path(), format!("{base}/v1"), None).await;
    let input = json!({"action":"get","id":"resp_concurrent"});
    let (a, b) = tokio::join!(
        invoke(plugin.as_ref(), root.path(), "responses", input.clone()),
        invoke(plugin.as_ref(), root.path(), "responses", input)
    );
    let count = [a.unwrap(), b.unwrap()]
        .iter()
        .filter(|out| out.metadata.contains_key(PROVIDER_TOOL_USAGE_METADATA_KEY))
        .count();
    assert_eq!(count, 1);
    captured.await.unwrap();
}
