use super::*;
use agena_api::live::SessionChangeResource;
#[cfg(feature = "sse")]
use agena_client::SubscriptionEvent;
use agena_storage::kernel::{RequestId, ToolPhase, ToolResult};
use std::time::Duration;

#[tokio::test]
async fn bounded_reconciliation_filters_memberships_and_validates_real_entry_ids_over_http() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let source = client
        .create_session(server.workspace_id, "source", None)
        .await
        .unwrap();
    let unrelated = client
        .create_session(server.workspace_id, "unrelated", None)
        .await
        .unwrap();
    let application = application_for_test(&server.runtime);
    let kernel = application.session_kernel().unwrap();
    let first = kernel_fixture::completed_input(&application, source.id, "first").await;
    let second = kernel_fixture::completed_input(&application, source.id, "second").await;
    let ids = [
        second.message_id.get(),
        first.message_id.get(),
        first.message_id.get(),
        i64::MAX,
    ];
    let page = client.session_parts_by_ids(source.id, &ids).await.unwrap();
    assert_eq!(page.parts.len(), 2);
    assert!(
        page.parts
            .iter()
            .all(|part| part.message_boundary && part.run_id == Some(part.part_id))
    );
    assert!(!page.page.has_more);
    assert!(
        client
            .session_parts_by_ids(unrelated.id, &ids)
            .await
            .unwrap()
            .parts
            .is_empty()
    );
    for query in [
        "ids=0".to_owned(),
        "ids=-1".into(),
        "ids=abc".into(),
        "ids=1&limit=2".into(),
        format!("ids={}", vec!["1"; 257].join(",")),
    ] {
        let response = reqwest::get(format!(
            "{}/api/v1/sessions/{}/parts?{query}",
            server.url, source.id
        ))
        .await
        .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    let response = reqwest::get(format!(
        "{}/api/v1/sessions/{}/parts?locale=zh-CN",
        server.url, source.id
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    kernel
        .sessions
        .delete(RequestId::new(), source.id.try_into().unwrap())
        .await
        .unwrap();
    for endpoint in ["parts?ids=1", "parts", "runs", "state"] {
        assert_eq!(
            reqwest::get(format!(
                "{}/api/v1/sessions/{}/{endpoint}",
                server.url, source.id
            ))
            .await
            .unwrap()
            .status(),
            reqwest::StatusCode::NOT_FOUND,
            "{endpoint}"
        );
    }
}

#[cfg(feature = "sse")]
#[tokio::test]
async fn resource_stream_delivers_live_text_without_entry_rewrites_and_workspace_deletion() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let source = client
        .create_session(server.workspace_id, "source", None)
        .await
        .unwrap();
    let empty = client
        .create_session(server.workspace_id, "empty", None)
        .await
        .unwrap();
    let application = application_for_test(&server.runtime);
    let kernel = application.session_kernel().unwrap();
    let (_sent, _attempt, round) =
        kernel_fixture::attempt(&application, source.id, "stream fixture").await;
    let writer = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    let part = kernel
        .reader
        .entry(source.id.try_into().unwrap(), writer.entry_id())
        .await
        .unwrap();
    let mut stream = client
        .stream_changes(agena_api::Scope::Workspace {
            workspace_id: server.workspace_id,
        })
        .await
        .unwrap();
    let mut content = client
        .stream_content(
            source.id,
            writer.id(),
            Some(agena_domain::ContentCursor {
                epoch: writer.epoch().into(),
                sequence: 0,
            }),
            64 * 1024,
        )
        .await
        .unwrap();
    for _ in 0..10 {
        writer.append("x").await.unwrap();
    }
    let text = tokio::time::timeout(Duration::from_secs(5), async {
        let mut text = String::new();
        while let Some(Ok(page)) = content.recv().await {
            for chunk in page.chunks {
                if let agena_domain::ContentPayload::Text { text: chunk } = chunk.payload {
                    text.push_str(&chunk);
                }
            }
            if page.next_cursor.sequence == 10 {
                return text;
            }
        }
        panic!("content stream closed before live cursor 10");
    })
    .await
    .expect("bytes arrive while entry is running");
    assert_eq!(text, "xxxxxxxxxx");
    let unchanged = kernel
        .reader
        .entry(source.id.try_into().unwrap(), writer.entry_id())
        .await
        .unwrap();
    assert_eq!(
        *part, *unchanged,
        "accepted byte appends never rewrite entry facts"
    );
    writer.finish(RequestId::new()).await.unwrap();
    kernel
        .sessions
        .delete(RequestId::new(), empty.id.try_into().unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Ok(SubscriptionEvent::SessionChanged(
                SessionChangeResource::SessionDeleted {
                    session_id,
                    workspace_id,
                },
            ))) = stream.recv().await
                && session_id == empty.id
            {
                assert_eq!(workspace_id, server.workspace_id);
                break;
            }
        }
    })
    .await
    .expect("empty deletion survives workspace filtering");
}

#[tokio::test]
async fn a_live_session_stream_localizes_tool_headlines_for_the_asked_language() {
    use futures_util::StreamExt as _;
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let http = reqwest::Client::new();
    let session = client
        .create_session(server.workspace_id, "live headline locale", None)
        .await
        .unwrap();
    let application = application_for_test(&server.runtime);
    let kernel = application.session_kernel().unwrap();
    let (_attempt, _round, tool, entry) = kernel_fixture::tool(
        &application,
        session.id,
        "shell.exec",
        serde_json::json!({"command":"echo hello"}),
    )
    .await;
    // The stored headline stays English; a reader that names a language gets the
    // mapped vocabulary on both the snapshot and every later push.
    let endpoint = format!(
        "{}/api/v1/sessions/{}/parts/{}/tool-sections/presentation",
        server.url,
        session.id,
        entry.get()
    );
    let english: serde_json::Value = http
        .get(endpoint)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let english_title = english["value"]["title"]
        .as_str()
        .expect("the tool presentation carries a headline")
        .to_owned();
    let english_action = english_title
        .split(" · ")
        .next()
        .unwrap_or_default()
        .to_owned();
    assert!(english_title.contains("echo hello"), "{english_title}");
    assert!(!english_action.is_empty(), "{english_title}");

    let response = http
        .get(format!(
            "{}/api/v1/sessions/{}/changes/stream?locale=zh-CN",
            server.url, session.id
        ))
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let mut stream = response.bytes_stream();
    let mut received = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !String::from_utf8_lossy(&received).contains("session_snapshot") {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for the localized snapshot: {}",
            String::from_utf8_lossy(&received)
        );
        let chunk = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("snapshot chunk arrives")
            .expect("session stream stays open")
            .expect("read snapshot bytes");
        received.extend_from_slice(&chunk);
    }
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({}),
        },
    )
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !String::from_utf8_lossy(&received).contains("part_updated") {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for the localized push: {}",
            String::from_utf8_lossy(&received)
        );
        let chunk = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("push chunk arrives")
            .expect("session stream stays open")
            .expect("read push bytes");
        received.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(received).expect("session SSE is UTF-8");
    assert!(text.contains("echo hello"), "{text}");
    assert!(
        !text.contains(english_action.as_str()),
        "the streamed headline must be localized: {text}"
    );
    assert!(
        text.chars()
            .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character)),
        "the Chinese vocabulary should reach the live reader: {text}"
    );
    kernel
        .sessions
        .delete(RequestId::new(), session.id.try_into().unwrap())
        .await
        .unwrap();
}
