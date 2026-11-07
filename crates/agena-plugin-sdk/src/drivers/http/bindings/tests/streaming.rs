use super::*;

#[tokio::test]
async fn retained_stream_producer_and_chunk_forwarder_keep_the_original_binding() {
    let mut fixture = Fixture::start().await;
    let handle = ok(fixture
        .wire
        .call(
            "old",
            method::HOOK_TOOL_INVOKE_STREAM,
            invocation(71, json!({"hold":true})),
        )
        .await);
    assert!(handle["stream_id"].is_string());
    fixture.started(71).await;
    fixture.fork().await;
    assert_eq!(
        ok(fixture.wire.invoke("new", 72, json!({})).await)["output_text"],
        "new"
    );
    fixture.control.release.add_permits(1);
    fixture.old.wait_method(method::TOOL_STREAM_END).await;

    let records = fixture.old.requests.lock().unwrap().clone();
    for name in [
        method::HOST_CONFIG_READ,
        method::TOOL_STREAM_CHUNK,
        method::TOOL_STREAM_END,
    ] {
        let request = records
            .iter()
            .find(|request| request.method == name)
            .unwrap();
        let context = &request.params.as_ref().unwrap()["context"];
        assert_eq!(context["http_binding_id"], "old", "{name}");
        assert_eq!(context["authority_token"], "authority-old", "{name}");
        assert_eq!(context["call_id"], 71, "{name}");
    }
    let terminal = records
        .iter()
        .find(|request| request.method == method::TOOL_STREAM_END)
        .unwrap();
    assert_eq!(terminal.params.as_ref().unwrap()["output_text"], "old");
    assert!(
        !fixture
            .new
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == method::TOOL_STREAM_CHUNK
                || request.method == method::TOOL_STREAM_END)
    );
    assert!(!fixture.wire.retire("old").await.last);
    assert!(fixture.wire.retire("new").await.last);
    ok(fixture
        .wire
        .call("new", method::META_SHUTDOWN, json!({}))
        .await);
    fixture.finish().await;
}

#[tokio::test]
async fn retiring_a_stream_binding_drains_its_producer_without_stopping_other_streams() {
    let mut fixture = Fixture::start().await;
    fixture.fork().await;
    ok(fixture
        .wire
        .call(
            "old",
            method::HOOK_TOOL_INVOKE_STREAM,
            invocation(81, json!({"hold":true})),
        )
        .await);
    fixture.started(81).await;
    ok(fixture
        .wire
        .call(
            "new",
            method::HOOK_TOOL_INVOKE_STREAM,
            invocation(82, json!({"hold":true})),
        )
        .await);
    fixture.started(82).await;
    assert!(!fixture.wire.retire("old").await.last);
    fixture.control.release.add_permits(1);
    fixture.new.wait_method(method::TOOL_STREAM_END).await;
    assert!(
        !fixture
            .old
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == method::HOST_CONFIG_READ
                || request.method == method::TOOL_STREAM_END)
    );
    assert!(fixture.wire.retire("new").await.last);
    ok(fixture
        .wire
        .call("new", method::META_SHUTDOWN, json!({}))
        .await);
    fixture.finish().await;
}
