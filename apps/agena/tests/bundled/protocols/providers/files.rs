use super::{Vendor, peer::*};
use agena_testkit::{EditOptions, SessionOptions, SettingsLayer, TestResult};
use anyhow::{Context, ensure};
use axum::http::Method;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relative_provider_files_use_the_invoking_workspace_and_preserve_absolute_paths()
-> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let workspace = case.paths().root.join("invoking-provider-workspace");
            let initial_root = case
                .paths()
                .global_config
                .parent()
                .context("plugin configuration root")?
                .to_owned();
            ensure!(workspace != case.workspace() && workspace != initial_root);
            tokio::fs::create_dir_all(&workspace).await?;
            let user = case
                .user()
                .new_session_in_with(
                    &workspace,
                    SessionOptions {
                        title: "Provider workspace regression".into(),
                        ..Default::default()
                    },
                )
                .await?;
            super::super::super::allow_permissions(&user).await?;

            // Matching filenames make an incorrect initialization-root or process-
            // cwd lookup succeed with the wrong bytes, so an ENOENT alone cannot
            // make this regression appear covered.
            const DOCUMENT: &str = "invoking-workspace-document-bytes";
            const INITIAL: &str = "plugin-init-root-decoy-bytes";
            const DEFAULT: &str = "default-workspace-explicit-only-bytes";
            tokio::fs::write(workspace.join("scope.txt"), DOCUMENT).await?;
            tokio::fs::write(initial_root.join("scope.txt"), INITIAL).await?;
            let explicit = case.workspace().join("scope.txt");
            tokio::fs::write(&explicit, DEFAULT).await?;

            peer.json(Method::POST, "/v1/files", json!({"id":"file_secondary"}));
            user.call_tool(
                "chatgpt.files",
                json!({
                    "action":"upload","path":"scope.txt","parameters":{"purpose":"assistants"}
                }),
            )
            .await?
            .assert_succeeded()?;
            let upload = peer.last()?;
            assert_request(&upload, Vendor::ChatGpt, user.id())?;
            ensure!(upload.text().contains(DOCUMENT));
            ensure!(!upload.text().contains(INITIAL) && !upload.text().contains(DEFAULT));

            // Inline media preparation runs on a blocking worker. It must read the
            // same session-resolved file that the host just authorized.
            peer.json(
                Method::POST,
                "/v1/responses",
                json!({
                    "id":"resp_workspace_document","status":"completed",
                    "output_text":"workspace-document-confirmed"
                }),
            );
            user.call_tool(
                "chatgpt.document_understanding",
                json!({
                    "prompt":"Read this selected document.",
                    "inputs":[{"source":"local","path":"scope.txt"}]
                }),
            )
            .await?
            .assert_succeeded()?;
            let understanding = peer.last()?;
            assert_request(&understanding, Vendor::ChatGpt, user.id())?;
            ensure!(understanding.text().contains(DOCUMENT));
            ensure!(
                !understanding.text().contains(INITIAL) && !understanding.text().contains(DEFAULT)
            );

            peer.expect(
                Method::GET,
                "/v1/containers/cntr_scope/files/cfile_scope/content",
                ResponseSpec::bytes("text/plain", b"workspace-download-bytes".to_vec()),
            );
            let downloaded = user
                .call_tool(
                    "chatgpt.containers",
                    json!({
                        "action":"download_file","id":"cntr_scope","item_id":"cfile_scope",
                        "output_path":"exports/answer.txt"
                    }),
                )
                .await?;
            downloaded.assert_succeeded()?;
            let attachments = downloaded.attachments()?;
            ensure!(attachments.len() == 1, "download omitted its attachment");
            let attachment = downloaded.attachment(0)?;
            ensure!(attachment.mime == "text/plain");
            ensure!(attachment.filename.as_deref() == Some("answer.txt"));
            ensure!(attachment.size_bytes == Some(b"workspace-download-bytes".len() as u64));
            ensure!(
                matches!(&attachment.source, agena_domain::AttachmentSource::Url { url }
                if url.starts_with("agena:media:v1:"))
            );
            // The public media reference is opaque. Read it through the same
            // session-scoped HTTP API a real client uses, preserving membership
            // checks instead of decoding storage details from the token.
            let (filename, bytes) = user.download_attachment(attachment).await?;
            ensure!(filename == "answer.txt" && bytes == b"workspace-download-bytes");
            let foreign = case.user().new_session().await?;
            let wrong_owner = foreign
                .download_attachment(attachment)
                .await
                .err()
                .context("another session read opaque media it does not own")?;
            let diagnostic = format!("{wrong_owner:#}");
            ensure!(
                diagnostic.contains(&format!("session {}", foreign.id()))
                    && diagnostic.contains("answer.txt")
            );
            ensure!(
                tokio::fs::read(workspace.join("exports/answer.txt")).await?
                    == b"workspace-download-bytes"
            );
            ensure!(!initial_root.join("exports/answer.txt").exists());
            ensure!(!case.workspace().join("exports/answer.txt").exists());

            // An explicitly supplied absolute input remains explicit; session
            // rebasing applies only to relative paths.
            peer.json(Method::POST, "/v1/files", json!({"id":"file_absolute"}));
            user.call_tool(
                "chatgpt.files",
                json!({
                    "action":"upload","path":explicit,"parameters":{"purpose":"assistants"}
                }),
            )
            .await?
            .assert_succeeded()?;
            ensure!(peer.last()?.text().contains(DEFAULT));
            ensure!(tokio::fs::read_to_string(workspace.join("scope.txt")).await? == DOCUMENT);
            ensure!(tokio::fs::read_to_string(initial_root.join("scope.txt")).await? == INITIAL);
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uploaded_file_handles_are_bound_to_the_selected_provider() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            tokio::fs::write(
                case.paths().workspace.join("input.txt"),
                b"explicit-upload-bytes",
            )
            .await?;
            let mut handles = Vec::new();
            for vendor in [Vendor::ChatGpt, Vendor::Claude] {
                peer.json(
                    Method::POST,
                    "/v1/files",
                    json!({"id":"file-test","status":"processed"}),
                );
                let result = user
                    .call_tool(
                        format!("{}.file_upload", vendor.name()),
                        json!({"path":"input.txt"}),
                    )
                    .await?;
                result.assert_succeeded()?;
                let request = peer.last()?;
                assert_request(&request, vendor, user.id())?;
                ensure!(request.text().contains("explicit-upload-bytes"));
                ensure!(!request.text().contains(case.model().model_id()));
                handles.push((
                    vendor,
                    result.data()?["handle"]
                        .as_str()
                        .context("file handle")?
                        .to_owned(),
                ));
            }
            let config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            case.settings()
                .set(
                    SettingsLayer::Global,
                    "providers.second",
                    config["providers"]["testkit"].clone(),
                    EditOptions::default(),
                )
                .await?
                .require_success()?;
            user.client().update_session_selection(user.id(), serde_json::from_value(json!({
            "model":{"provider_id":"second","adapter_id":"openai_responses","model_id":"fake-model"}
        }))?).await?;
            for (vendor, handle) in handles {
                let result = user
                    .call_tool(
                        format!("{}.file_status", vendor.name()),
                        json!({"handle":handle}),
                    )
                    .await?;
                result.assert_failed()?;
                ensure!(
                    result.output.to_string().contains("not owned"),
                    "{}",
                    result.output
                );
            }
            ensure!(
                peer.requests().len() == 2,
                "a foreign handle reached the vendor"
            );
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn binary_and_native_multipart_uploads_keep_their_official_fields() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        let dataset = b"PK\x03\x04\x00binary-dataset";
        tokio::fs::write(case.paths().workspace.join("dataset.xlsx"), dataset).await?;
        peer.json(Method::POST, "/v1/files", json!({"id":"file_data"}));
        user.call_tool("chatgpt.files", json!({"action":"upload","path":"dataset.xlsx","parameters":{"purpose":"assistants"}}))
            .await?.assert_succeeded()?;
        let request = peer.last()?;
        ensure!(request.bytes.windows(dataset.len()).any(|bytes| bytes == dataset));
        ensure!(request.text().contains("assistants") && !request.text().contains(case.model().model_id()));
        for (tool, mut input, filename, field, endpoint) in [
            ("chatgpt.transcription", json!({"parameters":{"timestamp_granularities":["word","segment"],
                "chunking_strategy":{"type":"server_vad","threshold":0.5}}}), "input.wav", "file", "/v1/audio/transcriptions"),
            ("chatgpt.platform", json!({"action":"create","resource":"audio/voice_consents",
                "parameters":{"name":"speaker","language":"zh"}}), "consent.wav", "recording", "/v1/audio/voice_consents"),
            ("chatgpt.platform", json!({"action":"create","resource":"audio/voices",
                "parameters":{"name":"speaker","consent":"cons_1"}}), "sample.wav", "audio_sample", "/v1/audio/voices"),
        ] {
            tokio::fs::write(case.paths().workspace.join(filename), b"RIFF-explicit-recording").await?;
            input["path"] = json!(filename);
            peer.json(Method::POST, endpoint, json!({"id":"created","text":"transcribed"}));
            user.call_tool(tool, input).await?.assert_succeeded()?;
            let request = peer.last()?;
            ensure!(request.headers["content-type"].to_str()?.contains("multipart/form-data"));
            let body = request.text();
            ensure!(body.contains(&format!("name=\"{field}\"; filename=\"{filename}\"")), "{body}");
            ensure!(body.contains("Content-Type: audio/wav"));
            if tool == "chatgpt.transcription" {
                ensure!(body.matches("name=\"timestamp_granularities[]\"").count() == 2);
                ensure!(body.contains("name=\"chunking_strategy[type]\"") && body.contains("name=\"chunking_strategy[threshold]\""));
            }
        }
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_and_generated_audio_persist_the_exact_bytes() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            for (tool, input, method, path, mime, bytes) in [
                (
                    "chatgpt.containers",
                    json!({"action":"download_file","id":"cntr_1","item_id":"cfile_1"}),
                    Method::GET,
                    "/v1/containers/cntr_1/files/cfile_1/content",
                    "text/csv",
                    b"a,b\n1,2\n".to_vec(),
                ),
                (
                    "claude.files",
                    json!({"action":"download","id":"file_1"}),
                    Method::GET,
                    "/v1/files/file_1/content",
                    "application/pdf",
                    b"%PDF-test".to_vec(),
                ),
                (
                    "chatgpt.speech",
                    json!({"model":"speech-model","prompt":"hello","parameters":{"voice":"alloy"}}),
                    Method::POST,
                    "/v1/audio/speech",
                    "audio/wav",
                    b"RIFF-test-audio".to_vec(),
                ),
            ] {
                peer.expect(method, path, ResponseSpec::bytes(mime, bytes.clone()));
                let output = user.call_tool(tool, input).await?;
                output.assert_succeeded()?;
                ensure!(output.attachments()?.len() == 1);
                ensure!(attachment_bytes(&user, &output, 0, mime).await? == bytes);
            }
            peer.json(
                Method::POST,
                "/v1beta/interactions",
                json!({"id":"audio1","steps":[{"content":[
            {"type":"audio","mime_type":"audio/pcm;rate=24000","data":"AQIDBA=="},
            {"type":"audio","mime_type":"audio/wav","data":"d2F2"}]}]}),
            );
            let audio = user
                .call_tool(
                    "gemini.speech",
                    json!({"prompt":"hi","parameters":{
            "generation_config":{"speech_config":[{"voice":"Kore"}]}}}),
                )
                .await?;
            audio.assert_succeeded()?;
            let body = peer.last()?.json()?;
            ensure!(
                body["response_format"]["type"] == "audio"
                    && body["model"] == case.model().model_id()
            );
            ensure!(audio.attachments()?.len() == 2);
            let wav = attachment_bytes(&user, &audio, 0, "audio/wav").await?;
            ensure!(wav.starts_with(b"RIFF") && wav.ends_with(&[1, 2, 3, 4]));
            ensure!(attachment_bytes(&user, &audio, 1, "audio/wav").await? == b"wav");
            ensure!(!audio.output.to_string().contains("AQIDBA=="));
            ensure!(!serde_json::to_string(&audio.raw_output)?.contains("AQIDBA=="));
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumable_file_search_upload_keeps_metadata_and_operation_identity() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        tokio::fs::write(case.paths().workspace.join("doc.txt"), b"explicit document").await?;
        peer.expect(Method::POST, "/upload/v1beta/fileSearchStores/store:uploadToFileSearchStore",
            ResponseSpec::json(json!({})).header("x-goog-upload-url",
                "$BASE/upload/v1beta/fileSearchStores/store:uploadToFileSearchStore?upload_id=opaque"));
        peer.json(Method::POST, "/upload/v1beta/fileSearchStores/store:uploadToFileSearchStore?upload_id=opaque",
            json!({"name":"fileSearchStores/store/upload/operations/op"}));
        let output = user.call_tool("gemini.knowledge", json!({"action":"upload","id":"fileSearchStores/store","path":"doc.txt",
            "parameters":{"customMetadata":[{"key":"source","stringValue":"local"}],
                "chunkingConfig":{"whiteSpaceConfig":{"maxTokensPerChunk":200}}}})).await?;
        output.assert_succeeded()?;
        ensure!(output.data()?["pending"] == true);
        peer.json(Method::GET, "/v1beta/fileSearchStores/store/upload/operations/op",
            json!({"name":"fileSearchStores/store/upload/operations/op","done":true,
                "response":{"documentName":"fileSearchStores/store/documents/doc"}}));
        user.call_tool("gemini.operations", json!({"action":"wait","id":"fileSearchStores/store/upload/operations/op"}))
            .await?.assert_succeeded()?;
        let requests = peer.requests();
        ensure!(requests.len() == 3);
        ensure!(requests[0].json()?["customMetadata"][0]["key"] == "source");
        ensure!(requests[0].json()?["chunkingConfig"]["whiteSpaceConfig"]["maxTokensPerChunk"] == 200);
        ensure!(requests[1].bytes == b"explicit document");
        for request in requests { assert_request(&request, Vendor::Gemini, user.id())?; }
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_origin_upload_never_receives_the_document() -> TestResult {
    use axum::{Router, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            let hits = Arc::new(AtomicUsize::new(0));
            let captured = hits.clone();
            let foreign = case
                .serve_http(Router::new().route(
                    "/upload",
                    post(move || {
                        let captured = captured.clone();
                        async move {
                            captured.fetch_add(1, Ordering::SeqCst);
                            "{}"
                        }
                    }),
                ))
                .await?;
            tokio::fs::write(
                case.paths().workspace.join("doc.txt"),
                b"do-not-leak-document",
            )
            .await?;
            peer.expect(
                Method::POST,
                "/upload/v1beta/fileSearchStores/store:uploadToFileSearchStore",
                ResponseSpec::json(json!({})).header("x-goog-upload-url", foreign.url("/upload")),
            );
            let output = user
                .call_tool(
                    "gemini.knowledge",
                    json!({"action":"upload","id":"store","path":"doc.txt"}),
                )
                .await?;
            output.assert_failed()?;
            ensure!(
                output.output.to_string().contains("origin"),
                "{}",
                output.output
            );
            ensure!(
                hits.load(Ordering::SeqCst) == 0,
                "document reached foreign upload origin"
            );
            ensure!(!peer.last()?.text().contains("do-not-leak-document"));
            peer.assert_finished()
        })
        .await
}
