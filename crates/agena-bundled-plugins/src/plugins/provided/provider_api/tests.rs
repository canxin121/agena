use super::*;

fn input(value: Value) -> ResourceInput {
    serde_json::from_value(value).unwrap()
}

#[test]
fn resource_routes_cover_complete_file_search_and_artifact_workflows() {
    let cases = [
        (
            "chatgpt",
            Family::Knowledge,
            json!({"action":"create"}),
            "POST",
            "vector_stores",
        ),
        (
            "chatgpt",
            Family::Knowledge,
            json!({"action":"add_file","id":"vs_1"}),
            "POST",
            "vector_stores/vs_1/files",
        ),
        (
            "chatgpt",
            Family::Knowledge,
            json!({"action":"get_file","id":"vs_1","item_id":"file_1"}),
            "GET",
            "vector_stores/vs_1/files/file_1",
        ),
        (
            "chatgpt",
            Family::Containers,
            json!({"action":"download_file","id":"cntr_1","item_id":"cfile_1"}),
            "GET",
            "containers/cntr_1/files/cfile_1/content",
        ),
        (
            "claude",
            Family::Files,
            json!({"action":"download","id":"file_1"}),
            "GET",
            "files/file_1/content",
        ),
        (
            "gemini",
            Family::Knowledge,
            json!({"action":"import_file","id":"fileSearchStores/store"}),
            "POST",
            "fileSearchStores/store:importFile",
        ),
        (
            "gemini",
            Family::Knowledge,
            json!({"action":"get_file","id":"store","item_id":"doc"}),
            "GET",
            "fileSearchStores/store/documents/doc",
        ),
        (
            "gemini",
            Family::Operations,
            json!({"action":"get","id":"fileSearchStores/store/operations/import"}),
            "GET",
            "fileSearchStores/store/operations/import",
        ),
        (
            "gemini",
            Family::Files,
            json!({"action":"download","id":"files/video"}),
            "GET",
            "files/video:download",
        ),
        (
            "gemini",
            Family::Batches,
            json!({"action":"cancel","id":"batches/job"}),
            "POST",
            "batches/job:cancel",
        ),
        (
            "claude",
            Family::Skills,
            json!({"action":"create_version","id":"skill_1"}),
            "POST",
            "skills/skill_1/versions",
        ),
        (
            "claude",
            Family::Batches,
            json!({"action":"results","id":"batch_1"}),
            "GET",
            "messages/batches/batch_1/results",
        ),
        (
            "chatgpt",
            Family::Tasks,
            json!({"action":"get","id":"resp_1"}),
            "GET",
            "responses/resp_1",
        ),
        (
            "gemini",
            Family::Tasks,
            json!({"action":"cancel","id":"interaction_1"}),
            "POST",
            "interactions/interaction_1/cancel",
        ),
    ];
    for (provider, family, args, method, path) in cases {
        let route = routes::resolve(provider, family, &input(args)).unwrap();
        assert_eq!(route.method.as_str(), method);
        assert_eq!(route.path, path);
        assert!(!route.model, "resource operations must not require a model");
    }
}

#[test]
fn routes_reject_url_and_path_injection_and_unsupported_actions() {
    for id in [
        "../secrets",
        "https://attacker.example/files/x",
        "file?key=secret",
        "file#frag",
        "file%2fother",
        "/files/other",
        "..",
    ] {
        assert!(
            routes::resolve(
                "chatgpt",
                Family::Files,
                &input(json!({"action":"download","id":id}))
            )
            .is_err()
        );
    }
    assert!(
        routes::resolve(
            "claude",
            Family::Embeddings,
            &input(json!({"action":"create"}))
        )
        .is_err()
    );
    assert!(
        routes::resolve(
            "gemini",
            Family::Platform,
            &input(json!({"action":"create","resource":"api_keys"}))
        )
        .is_err()
    );
    for parameters in [
        json!({"model":"override"}),
        json!({"stream":true}),
        json!({"api_key":"secret"}),
    ] {
        assert!(validate_input(&input(json!({"parameters":parameters}))).is_err());
    }
    assert!(validate_input(&input(json!({"query":{"key":"secret"}}))).is_err());
    assert!(
        validate_input(&input(
            json!({"beta_headers":["foo\r\nAuthorization: bad"]})
        ))
        .is_err()
    );
}

#[test]
fn inline_media_is_extracted_and_redacted_without_following_remote_urls() {
    let mut response = json!({"steps":[{"content":[
        {"type":"audio","mime_type":"audio/wav","data":"YXVkaW8="},
        {"type":"video","mime_type":"video/mp4","data":"dmlkZW8="},
        {"type":"image","mime_type":"image/png","data":"aW1hZ2U="},
        {"type":"video","uri":"https://elsewhere.example/file"}
    ]}]});
    let mut binaries = Vec::new();
    collect_binary(&mut response, &mut binaries, 0);
    assert_eq!(binaries.len(), 3);
    assert_eq!(STANDARD.decode(&binaries[0].1).unwrap(), b"audio");
    assert!(!response.to_string().contains("YXVkaW8="));
    assert!(
        response
            .to_string()
            .contains("https://elsewhere.example/file")
    );
}

#[test]
fn pending_states_do_not_confuse_completion_errors_and_upload_processing() {
    for value in [
        json!({"done":false}),
        json!({"status":"queued"}),
        json!({"processing_status":"in_progress"}),
        json!({"state":"PROCESSING"}),
        json!({"status":"validating"}),
        json!({"status":"validating_files"}),
        json!({"status":"finalizing"}),
        json!({"status":"pausing"}),
    ] {
        assert!(pending(&json!({"response":value})));
    }
    for value in [
        json!({"done":true,"error":{"code":3}}),
        json!({"status":"completed"}),
        json!({"status":"failed"}),
        json!({"state":"ACTIVE"}),
        json!({"done":true,"metadata":{"state":"JOB_STATE_RUNNING"}}),
        json!({"name":"batches/job","state":"JOB_STATE_SUCCEEDED"}),
    ] {
        assert!(!pending(&value));
    }
}

#[test]
fn platform_and_agents_reject_invented_lifecycle_actions() {
    for (provider, family, args) in [
        (
            "chatgpt",
            Family::Platform,
            json!({"action":"list","resource":"conversations"}),
        ),
        (
            "chatgpt",
            Family::Platform,
            json!({"action":"get","resource":"uploads","id":"upload_1"}),
        ),
        (
            "claude",
            Family::Agents,
            json!({"action":"delete","id":"agent_1"}),
        ),
        (
            "gemini",
            Family::Agents,
            json!({"action":"update","id":"agent_1"}),
        ),
        (
            "chatgpt",
            Family::Agents,
            json!({"action":"create","resource":"sandboxes"}),
        ),
    ] {
        assert!(routes::resolve(provider, family, &input(args)).is_err());
    }
    for (family, args, path, multipart) in [
        (
            Family::Platform,
            json!({"action":"add_part","resource":"uploads","id":"upload_1"}),
            "uploads/upload_1/parts",
            true,
        ),
        (
            Family::Platform,
            json!({"action":"complete","resource":"uploads","id":"upload_1"}),
            "uploads/upload_1/complete",
            false,
        ),
        (
            Family::Platform,
            json!({"action":"create_run","resource":"evals","id":"eval_1"}),
            "evals/eval_1/runs",
            false,
        ),
        (
            Family::Agents,
            json!({"action":"create_event","resource":"sessions","id":"session_1"}),
            "agents/sessions/session_1/events",
            false,
        ),
    ] {
        let route = routes::resolve("chatgpt", family, &input(args)).unwrap();
        assert_eq!(route.path, path);
        assert_eq!(route.multipart, multipart);
    }
}

#[test]
fn sse_cursor_stays_at_last_retained_event_when_data_is_partial_or_bounded() {
    let partial = decode_events(b"id: first\nevent: item\ndata: {\ndata: \"ok\": true}\n\nid: second\ndata: {\"incomplete\":",false);
    assert_eq!(partial["response"]["last_event_id"], "first");
    assert_eq!(partial["response"]["events"][0]["ok"], true);
    assert_eq!(partial["response"]["sse_events"][0]["event_index"], 0);
    assert_eq!(partial["response"]["partial"], true);
    let stream = (0..260)
        .map(|i| format!("id: {i}\ndata: {{\"item\":{i}}}\n\n"))
        .collect::<String>();
    let bounded = decode_events(stream.as_bytes(), true);
    assert_eq!(bounded["response"]["events"].as_array().unwrap().len(), 256);
    assert_eq!(bounded["response"]["last_event_id"], "255");
    assert_eq!(bounded["response"]["partial"], true);
}
