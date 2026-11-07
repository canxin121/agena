use super::{Vendor, peer::*};
use agena_testkit::TestResult;
use anyhow::ensure;
use axum::http::Method;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn image_and_embedding_models_reach_their_native_protocols() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        for (tool, input, path, model) in [
            ("chatgpt.image_generate", json!({"action":"create","model":"gpt-image-2","prompt":"Draw a cat"}),
                "/v1/images/generations", "gpt-image-2"),
            ("chatgpt.responses_image_generation", json!({"model":"responses-chat-model","prompt":"Draw a cat",
                "tool_options":{"model":"gpt-image-2","action":"generate"},"previous_response_id":"resp_previous"}),
                "/v1/responses", "responses-chat-model"),
            ("chatgpt.embeddings", json!({"model":"embedding-model","parameters":{"input":"hello"}}),
                "/v1/embeddings", "embedding-model"),
        ] {
            peer.json(Method::POST, path, json!({"id":"resource","status":"completed","output_text":"native-protocol-result"}));
            let output = user.call_tool(tool, input).await?;
            output.assert_succeeded()?;
            let request = peer.last()?;
            assert_request(&request, Vendor::ChatGpt, user.id())?;
            let body = request.json()?;
            ensure!(body["model"] == model, "{body}");
            if tool == "chatgpt.image_generate" {
                ensure!(body["prompt"] == "Draw a cat" && body.get("tools").is_none());
            } else if tool == "chatgpt.responses_image_generation" {
                ensure!(body["tools"][0] == json!({"type":"image_generation","model":"gpt-image-2","action":"generate"}));
                ensure!(body["previous_response_id"] == "resp_previous");
                ensure!(output.data()?["public_tool_name"] == "chatgpt.responses_image_generation");
            }
        }
        let model = "models/team/alias?token=ignored#fragment";
        peer.json(Method::POST, "/v1beta/models/team%2Falias%3Ftoken%3Dignored%23fragment:embedContent",
            json!({"embedding":{"values":[0.1,0.2]}}));
        user.call_tool("gemini.embeddings", json!({"model":model,"prompt":"hello"})).await?.assert_succeeded()?;
        let request = peer.last()?;
        assert_request(&request, Vendor::Gemini, user.id())?;
        ensure!(request.json()?.get("model").is_none());
        ensure!(request.json()?["content"]["parts"][0]["text"] == "hello");
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn knowledge_inference_and_batches_preserve_vendor_fields() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        peer.json(Method::POST, "/v1/messages", json!({"id":"msg_native","content":[{"type":"text","text":"native-claude-reply"}],"stop_reason":"end_turn"}));
        let output = user.call_tool("claude.inference", json!({"prompt":"make a chart","parameters":{
            "container":{"skills":[{"type":"anthropic","skill_id":"xlsx","version":"latest"}]},
            "tools":[{"type":"code_execution_20260521","name":"code_execution"}]}})).await?;
        output.assert_succeeded()?;
        let request = peer.last()?;
        assert_request(&request, Vendor::Claude, user.id())?;
        let body = request.json()?;
        ensure!(body["model"] == case.model().model_id());
        ensure!(body["container"]["skills"][0]["skill_id"] == "xlsx");
        ensure!(body["stream"] == false);
        peer.json(Method::POST, "/v1beta/interactions", json!({"id":"research","status":"completed"}));
        user.call_tool("gemini.interactions", json!({"action":"create","parameters":{
            "agent":"deep-research-preview-04-2026","input":"research","background":true}})).await?.assert_succeeded()?;
        ensure!(peer.last()?.json()?.get("model").is_none());

        for (tool, input, path, response) in [
            ("chatgpt.knowledge", json!({"action":"add_file","id":"vs_test","parameters":{"file_id":"file_test"}}),
                "/v1/vector_stores/vs_test/files", json!({"id":"file_test","status":"in_progress"})),
            ("gemini.knowledge", json!({"action":"import_file","id":"store","parameters":{"fileName":"files/dataset"}}),
                "/v1beta/fileSearchStores/store:importFile", json!({"name":"fileSearchStores/store/operations/import"})),
        ] {
            peer.json(Method::POST, path, response);
            let output = user.call_tool(tool, input).await?;
            output.assert_succeeded()?;
            ensure!(output.data()?["pending"] == true, "{}", output.data()?);
            ensure!(peer.last()?.json()?.get("model").is_none());
        }
        peer.json(Method::POST, "/v1/messages/batches", json!({"id":"batch_test","processing_status":"in_progress"}));
        user.call_tool("claude.batches", json!({"action":"create","parameters":{"requests":[
            {"custom_id":"one","params":{"messages":[],"max_tokens":20}}]}})).await?.assert_succeeded()?;
        let body = peer.last()?.json()?;
        ensure!(body["requests"][0]["params"]["model"] == case.model().model_id());
        ensure!(body.get("model").is_none());
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gemini_batch_embeddings_keep_native_models_and_update_queries() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        for (tool, input, method, path) in [
            ("gemini.embeddings", json!({"action":"create_batch","model":"embedding-model","parameters":{
                "requests":[{"content":{"parts":[{"text":"a"}]}},{"model":"wrong-model","content":{"parts":[{"text":"b"}]}}]}}),
                Method::POST, "/v1beta/models/embedding-model:batchEmbedContents"),
            ("gemini.batches", json!({"action":"create","resource":"embeddings","model":"embedding-model","parameters":{
                "batch":{"displayName":"batch","inputConfig":{"fileName":"files/data"}}}}),
                Method::POST, "/v1beta/models/embedding-model:asyncBatchEmbedContent"),
            ("gemini.batches", json!({"action":"update","resource":"embeddings","id":"batches/b1","model":"embedding-model",
                "parameters":{"displayName":"updated"},"query":{"updateMask":"display_name"}}),
                Method::PATCH, "/v1beta/batches/b1:updateEmbedContentBatch?updateMask=display_name"),
        ] {
            peer.json(method.clone(), path, json!({"embeddings":[{"values":[1]}]}));
            user.call_tool(tool, input).await?.assert_succeeded()?;
            let request = peer.last()?;
            assert_request(&request, Vendor::Gemini, user.id())?;
            let body = request.json()?;
            if tool == "gemini.embeddings" {
                for item in body["requests"].as_array().unwrap() { ensure!(item["model"] == "models/embedding-model"); }
            } else if method == Method::PATCH { ensure!(body["model"] == "models/embedding-model"); }
        }
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nested_agent_resources_and_artifacts_keep_all_identifiers() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        for (tool, input, path) in [
            ("chatgpt.agents", json!({"resource":"sessions/artifacts","action":"download","id":"s1","item_id":"a1"}),
                "/v1/agents/sessions/s1/artifacts/a1/content"),
            ("chatgpt.agents", json!({"resource":"sessions/subagents/turns","action":"items","id":"s1","item_id":"agent1","sub_item_id":"turn1"}),
                "/v1/agents/sessions/s1/subagents/agent1/turns/turn1/items"),
            ("claude.agents", json!({"resource":"sessions/resources","action":"get","id":"s1","item_id":"r1"}),
                "/v1/sessions/s1/resources/r1"),
            ("claude.agents", json!({"resource":"sessions/threads","action":"events","id":"s1","item_id":"t1"}),
                "/v1/sessions/s1/threads/t1/events"),
            ("claude.agents", json!({"resource":"deployment_runs","action":"get","id":"run1"}), "/v1/deployment_runs/run1"),
        ] {
            let binary = input["action"] == "download";
            let reply = if binary { ResponseSpec::bytes("text/plain", b"generated artifact bytes".to_vec()) }
                else { ResponseSpec::json(json!({"id":"result"})) };
            peer.expect(Method::GET, path, reply);
            let output = user.call_tool(tool, input).await?;
            output.assert_succeeded()?;
            if binary { ensure!(attachment_bytes(&user, &output, 0, "text/plain").await? == b"generated artifact bytes"); }
        }
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn platform_resources_and_prompt_voices_use_official_bodies() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        for (input, path, method) in [
            (json!({"resource":"fine_tuning/jobs/checkpoints","action":"list","id":"ftjob_1"}), "fine_tuning/jobs/ftjob_1/checkpoints", Method::GET),
            (json!({"resource":"fine_tuning/checkpoints/permissions","action":"list","id":"ft:gpt-model:org:run:step-1"}), "fine_tuning/checkpoints/ft%3Agpt-model%3Aorg%3Arun%3Astep-1/permissions", Method::GET),
            (json!({"resource":"evals/runs/output_items","action":"get","id":"eval_1","item_id":"run_1","sub_item_id":"output_1"}), "evals/eval_1/runs/run_1/output_items/output_1", Method::GET),
            (json!({"resource":"fine_tuning/alpha/graders","action":"validate","parameters":{"grader":{"type":"label_model","model":"grader-model"}}}), "fine_tuning/alpha/graders/validate", Method::POST),
            (json!({"resource":"audio/voice_consents","action":"update","id":"cons_1","parameters":{"name":"new label"}}), "audio/voice_consents/cons_1", Method::POST),
            (json!({"resource":"audio/voices","action":"delete","id":"voice_1"}), "audio/voices/voice_1", Method::DELETE),
            (json!({"resource":"audio/voices","action":"create","parameters":{"type":"prompt","name":"narrator","prompt":"calm voice"}}), "audio/voices", Method::POST),
        ] {
            peer.json(method.clone(), format!("/v1/{path}"), json!({"id":"result"}));
            user.call_tool("chatgpt.platform", input).await?.assert_succeeded()?;
            if method == Method::POST { ensure!(peer.last()?.json()?.get("model").is_none()); }
        }
        peer.json(Method::POST, "/v1/responses/compact", json!({"id":"compacted"}));
        user.call_tool("chatgpt.responses", json!({"action":"compact","model":"explicit-openai",
            "parameters":{"input":[{"role":"user","content":"hello"}]}})).await?.assert_succeeded()?;
        let body = peer.last()?.json()?;
        ensure!(body["model"] == "explicit-openai" && body.get("stream").is_none());
        peer.assert_finished()
    }).await
}
