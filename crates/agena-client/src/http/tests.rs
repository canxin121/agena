use super::*;
use std::process::Command;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn only_local_server_addresses_bypass_proxies() {
    for host in [
        "127.0.0.1",
        "127.8.9.10",
        "0.0.0.0",
        "[::1]",
        "[::]",
        "[::ffff:127.0.0.1]",
        "localhost",
        "LOCALHOST.",
        "agena.localhost",
    ] {
        assert!(
            is_local_server(&url::Url::parse(&format!("http://{host}:3210")).unwrap()),
            "{host}"
        );
    }
    for host in [
        "agena.example",
        "localhost.example",
        "notlocalhost",
        "192.168.1.2",
        "10.0.0.1",
        "[fd00::1]",
        "[::ffff:192.168.1.2]",
    ] {
        assert!(
            !is_local_server(&url::Url::parse(&format!("http://{host}:3210")).unwrap()),
            "{host}"
        );
    }
}

struct Fixture {
    url: String,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    async fn new(address: &str, identity: &'static str, malformed: Option<&'static str>) -> Self {
        let listener = tokio::net::TcpListener::bind(address).await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let captured = captured.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    while !bytes.ends_with(b"\r\n\r\n") {
                        bytes.push(socket.read_u8().await.unwrap());
                        assert!(bytes.len() < 16384);
                    }
                    let headers = String::from_utf8(bytes).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    let mut body = vec![0; length];
                    socket.read_exact(&mut body).await.unwrap();
                    captured.lock().unwrap().push(headers.clone());
                    let target = headers
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap();
                    let body = if target.ends_with("/auth/session") {
                        assert_eq!(
                            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["password"],
                            "test-password"
                        );
                        r#"{"token":"test-token"}"#.to_owned()
                    } else if target.contains("/changes/stream") {
                        ": ready\n\n".to_owned()
                    } else if target.ends_with("/operator/tools") {
                        assert!(
                            headers
                                .to_ascii_lowercase()
                                .contains("authorization: bearer test-token")
                        );
                        "[]".to_owned()
                    } else if let Some(body) = malformed {
                        body.to_owned()
                    } else {
                        serde_json::json!({
                            "status": "ok", "generation": 1,
                            "loaded_at": "2026-10-25T00:00:00Z", "database_connected": true,
                            "server": { "instance_id": identity, "id": uuid::Uuid::nil(),
                                "pid": 123, "started_at": "2026-10-25T00:00:00Z",
                                "protocol_version": agena_api::PROTOCOL_VERSION }
                        })
                        .to_string()
                    };
                    socket.write_all(format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                    ).as_bytes()).await.unwrap();
                });
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
}

async fn child(mode: &str, urls: &str, proxy: &str) {
    // Proxy environment is process-global. Isolate it in a child instead of
    // racing unsafe set_var calls against concurrent Rust tests.
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "http::tests::proxy_environment_child",
        "--ignored",
        "--nocapture",
    ]);
    for name in [
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ] {
        command.env(name, proxy);
    }
    command
        .env_remove("no_proxy")
        .env_remove("NO_PROXY")
        .env("AGENA_CLIENT_TEST_MODE", mode)
        .env("AGENA_CLIENT_TEST_URLS", urls);
    let output = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn local_auth_api_and_live_streams_bypass_proxy_but_remote_servers_use_it() {
    let local = Fixture::new("127.0.0.1:0", "local", None).await;
    let proxy = Fixture::new("127.0.0.1:0", "proxy", None).await;
    let localhost = local.url.replace("127.0.0.1", "localhost");
    child("local", &format!("{}|{localhost}", local.url), &proxy.url).await;
    assert_eq!(local.requests.lock().unwrap().len(), 8);
    assert!(proxy.requests.lock().unwrap().is_empty());
    child(
        "remote",
        "http://agena-remote.fixture.invalid:3210",
        &proxy.url,
    )
    .await;
    assert_eq!(proxy.requests.lock().unwrap().len(), 1);
    assert!(
        proxy.requests.lock().unwrap()[0]
            .starts_with("GET http://agena-remote.fixture.invalid:3210/api/v1/health ")
    );
}

#[tokio::test]
async fn empty_and_invalid_health_responses_include_server_context() {
    for (body, expected) in [
        (" \r\n", "empty response"),
        ("<html>proxy error</html>", "invalid JSON"),
    ] {
        let local = Fixture::new("127.0.0.1:0", "local", Some(body)).await;
        child(expected, &local.url, "http://127.0.0.1:9").await;
    }
}

#[tokio::test]
async fn explicit_list_refresh_revalidates_even_with_a_live_revision_lease() {
    let server = Fixture::new(
        "127.0.0.1:0",
        "unused",
        Some(r#"{"items":[],"page":{"returned":0,"has_more":true,"next_cursor":"next"}}"#),
    )
    .await;
    let client = AgenaClient::new(&server.url).unwrap();
    let resource = "workspace:1:sessions";
    let url = format!("{}/api/v1/sessions?workspace_id=1", server.url);
    {
        let mut reads = client.reads.lock().await;
        reads.global_live_users.store(1, Ordering::Release);
        reads.checked_at = Some(Instant::now());
        reads.versions.insert(resource.to_owned(), "old".to_owned());
        reads.remember(
            format!("{resource}\n{url}"),
            CachedRead {
                resource: resource.to_owned(),
                token: "old".to_owned(),
                etag: "\"old\"".to_owned(),
                value: serde_json::json!({"items":[],"page":{"returned":0,"has_more":false}}),
                bytes: 64,
                used: 0,
                generation: 0,
            },
        );
    }
    let query = Query::ListSessions(ListSessionsParams {
        workspace_id: Some(1),
        ..Default::default()
    });
    let QueryResult::Sessions(cached) = client.query(query.clone()).await.unwrap() else {
        panic!("wrong query result")
    };
    assert!(!cached.page.has_more);
    assert!(server.requests.lock().unwrap().is_empty());
    let QueryResult::Sessions(fresh) = client.query_revalidated(query).await.unwrap() else {
        panic!("wrong query result")
    };
    assert!(fresh.page.has_more);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("if-none-match: \"old\"")
    );
}

#[tokio::test]
#[ignore = "invoked by proxy tests in a process with isolated environment"]
async fn proxy_environment_child() {
    let mode = std::env::var("AGENA_CLIENT_TEST_MODE").unwrap();
    let urls = std::env::var("AGENA_CLIENT_TEST_URLS").unwrap();
    for url in urls.split('|') {
        if mode == "local" {
            let client = AgenaClient::connect_server(url, None, Some("test-password"))
                .await
                .unwrap();
            assert_eq!(
                client.operator_tools().await.unwrap(),
                serde_json::json!([])
            );
            let mut subscription = client
                .stream_changes(agena_api::Scope::Global)
                .await
                .unwrap();
            subscription.close();
        } else if mode == "remote" {
            assert_eq!(
                AgenaClient::new(url)
                    .unwrap()
                    .server_identity()
                    .await
                    .unwrap()
                    .instance_id,
                "proxy"
            );
        } else {
            let error = AgenaClient::new(url)
                .unwrap()
                .health()
                .await
                .unwrap_err()
                .operator_diagnostic();
            assert!(error.contains(&mode), "{error}");
            assert!(error.contains("server API"), "{error}");
            assert!(error.contains("200 OK"), "{error}");
            assert!(
                !error.contains("<html>"),
                "response content must stay private: {error}"
            );
        }
    }
}

#[test]
fn attachment_download_names_prefer_utf8_and_keep_literal_plus() {
    assert_eq!(
        download_filename(
            "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E5%90%91%E6%97%A5%E8%91%B5%20a+b.txt"
        ),
        Some("向日葵 a+b.txt".into())
    );
    assert_eq!(
        download_filename("inline; filename=\"safe.txt\"; filename*=UTF-8''%ZZ"),
        Some("safe.txt".into())
    );
    assert_eq!(
        download_filename("attachment; filename*=UTF-8''..%2Fprivate.txt"),
        Some("private.txt".into())
    );
    assert_eq!(
        download_filename("attachment; filename=\"x.txt\""),
        Some("x.txt".into())
    );
    assert!(sanitize_download_filename(&format!("{}.pdf", "文件".repeat(200))).len() <= 200);
}

#[tokio::test]
async fn inline_ui_human_export_expands_body_references_but_raw_sdk_reads_preserve_them() {
    use agena_domain::{
        ContentChunk, ContentCursor, ContentKind, ContentPage, ContentPayload, ContentRef,
        ContentResource, ContentState, InlineDocument,
    };
    let reference = "ui_123_0123456789abcdef0123456789abcdef";
    let marker = format!("```inline-ui\n{reference}\n```");
    let document = InlineDocument::from_value(serde_json::json!({"version":1,"blocks":[{"type":"paragraph","id":"fact","text":"Exact embedded content"}]})).unwrap();
    let original = format!("Before\n\n{marker}\n\nAfter\n\n````\n{marker}\n````\n");
    let cursor = ContentCursor {
        epoch: uuid::Uuid::new_v4(),
        sequence: original.len() as u64,
    };
    let id = agena_domain::ContentId::new();
    let page = ContentPage {
        inline_ui: Some([(reference.into(), document.clone())].into()),
        resource: ContentResource {
            resource_id: id,
            owner_session_id: 1,
            part_id: 2,
            kind: ContentKind::Text,
            state: ContentState::Complete,
            cursor,
            committed_cursor: cursor,
            total_bytes: Some(cursor.sequence),
            dropped_bytes: 0,
            dropped_packets: 0,
            committed_dropped_bytes: 0,
            committed_dropped_packets: 0,
            retained_ranges: vec![],
            capture_error: None,
        },
        chunks: vec![ContentChunk {
            cursor,
            captured_at_ms: 0,
            payload: ContentPayload::Text {
                text: original.clone(),
            },
        }],
        next_cursor: cursor,
        has_more: false,
        gap: false,
    };
    let response = Box::leak(serde_json::to_string(&page).unwrap().into_boxed_str());
    let server = Fixture::new("127.0.0.1:0", "inline-ui", Some(response)).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let source = ContentRef {
        resource_id: id,
        kind: ContentKind::Text,
    };
    let mut raw = Vec::new();
    client
        .write_content_text(1, &source, &mut raw)
        .await
        .unwrap();
    assert_eq!(String::from_utf8(raw).unwrap(), original);
    let mut human = Vec::new();
    client
        .write_reply_content_text(1, &source, &mut human)
        .await
        .unwrap();
    let human = String::from_utf8(human).unwrap();
    assert_eq!(
        human,
        format!(
            "Before\n\n{}\n\nAfter\n\n````\n{marker}\n````\n",
            document.to_markdown()
        )
    );
    let part = agena_api::part::PartResource {
        part_id: 2,
        kind: "text".into(),
        role: "assistant".into(),
        content: serde_json::json!({"resources":[source]}),
        ..Default::default()
    };
    let mut part_copy = Vec::new();
    client
        .write_reply_part_text(1, &part, &mut part_copy)
        .await
        .unwrap();
    assert_eq!(String::from_utf8(part_copy).unwrap(), human);
}

#[tokio::test]
async fn model_catalog_client_returns_the_public_contract() {
    let server = Fixture::new(
        "127.0.0.1:0",
        "catalog",
        Some(
            r#"{
        "summary": {"model_count": 1},
        "items": [{"model_id":"fixture-model","source":"generated","input":["text"],
            "thinking_modes":{"default":"high","high":{"strategy":"effort","effort":"high"}}}],
        "total":1,"offset":0,"limit":50
    }"#,
        ),
    )
    .await;
    let client = AgenaClient::new(&server.url).unwrap();
    let page: agena_api::resource::ModelCatalogListResponse = client
        .model_catalog("fixture model", Some("fixture"), 0, 50)
        .await
        .unwrap();
    assert_eq!(page.items[0].model_id, "fixture-model");
    assert_eq!(page.items[0].thinking_modes.default.mode(), Some("high"));
    assert!(page.available_origins.is_empty());
    let requests = server.requests.lock().unwrap();
    assert!(requests[0].contains("q=fixture+model"));
    assert!(requests[0].contains("origin=fixture"));
    assert!(requests[0].contains("limit=50"));
}

#[tokio::test]
async fn malformed_catalog_remains_a_client_decode_error() {
    let server = Fixture::new("127.0.0.1:0", "catalog", Some(r#"{"items":[]}"#)).await;
    let client = AgenaClient::new(&server.url).unwrap();
    assert!(client.model_catalog("", None, 0, 50).await.is_err());
}

/// One-response fixture for the JSON decoding boundary, including error HTTP
/// statuses and oversized declared lengths without allocating an oversized body.
async fn json_decoder_response(
    status: reqwest::StatusCode,
    body: &str,
    declared_length: Option<usize>,
) -> reqwest::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        status.as_u16(),
        declared_length.unwrap_or(body.len()),
    );
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
            assert!(request.len() < 16384);
        }
        // The oversized-length fixture may close as soon as it reads headers.
        let _ = socket.write_all(response.as_bytes()).await;
    });
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{address}/"))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn typed_json_decoder_preserves_duplicate_field_compatibility() {
    let client = AgenaClient::new("http://127.0.0.1:1").unwrap();
    let response = json_decoder_response(
        reqwest::StatusCode::OK,
        r#"{
        "summary":{"model_count":1}, "total":1, "offset":0, "limit":50,
        "items":[{"model_id":"earlier","model_id":"last","source":"cache"}]
    }"#,
        None,
    )
    .await;
    let page: agena_api::resource::ModelCatalogListResponse =
        client.parse_json(response).await.unwrap();
    assert_eq!(page.items[0].model_id, "last");
    assert!(page.available_origins.is_empty());
}

#[tokio::test]
async fn typed_json_decoder_preserves_syntax_and_schema_error_categories() {
    let client = AgenaClient::new("http://127.0.0.1:1").unwrap();
    for (body, expected) in [
        (" \r\n", "empty response"),
        ("<html>private-fixture</html>", "invalid JSON"),
        // A typed decoder can fail on summary before discovering the invalid
        // remainder; the established error category must still be Protocol.
        (r#"{"summary":true,"items":]}"#, "invalid JSON"),
    ] {
        let response = json_decoder_response(reqwest::StatusCode::OK, body, None).await;
        let error = client
            .parse_json::<agena_api::resource::ModelCatalogListResponse>(response)
            .await
            .unwrap_err();
        match error {
            ClientError::Protocol(message) => {
                assert!(message.contains("server API"));
                assert!(message.contains("HTTP 200"));
                assert!(message.contains(expected));
                assert!(!message.contains("private-fixture"));
            }
            other => panic!("wrong JSON protocol error: {other:?}"),
        }
    }
    let response = json_decoder_response(reqwest::StatusCode::OK, r#"{"items":[]}"#, None).await;
    assert!(matches!(
        client
            .parse_json::<agena_api::resource::ModelCatalogListResponse>(response)
            .await
            .unwrap_err(),
        ClientError::Decode(_)
    ));
}

#[tokio::test]
async fn typed_json_decoder_preserves_api_error_envelope_and_response_limit() {
    let client = AgenaClient::new("http://127.0.0.1:1").unwrap();
    let expected = agena_api::error::ApiError::bad_request("The request is invalid.");
    let body = serde_json::to_string(&expected).unwrap();
    let response = json_decoder_response(reqwest::StatusCode::BAD_REQUEST, &body, None).await;
    match client
        .parse_json::<agena_api::resource::ModelCatalogListResponse>(response)
        .await
        .unwrap_err()
    {
        ClientError::Api(actual) => assert_eq!(actual, expected),
        other => panic!("wrong structured API error: {other:?}"),
    }

    let response = json_decoder_response(
        reqwest::StatusCode::FORBIDDEN,
        r#"{"access_token":"private-fixture"}"#,
        None,
    )
    .await;
    match client
        .parse_json::<agena_api::resource::ModelCatalogListResponse>(response)
        .await
        .unwrap_err()
    {
        ClientError::Protocol(message) => {
            assert!(message.contains("HTTP 403"));
            assert!(message.contains("shared API envelope"));
            assert!(!message.contains("private-fixture"));
        }
        other => panic!("wrong opaque error response: {other:?}"),
    }

    let response = json_decoder_response(
        reqwest::StatusCode::OK,
        "{}",
        Some(MAX_JSON_RESPONSE_BYTES + 1),
    )
    .await;
    assert!(
        matches!(client.parse_json::<agena_api::resource::ModelCatalogListResponse>(response)
        .await.unwrap_err(), ClientError::Protocol(message) if message.contains("64 MiB limit"))
    );
}

#[tokio::test]
async fn typed_json_decoder_rejects_overflow_numbers_in_unknown_fields() {
    let body = r#"{
        "summary":{"model_count":0}, "items":[], "total":0, "offset":0,
        "limit":50, "future_field":1e9999
    }"#;
    // A directly derived DTO skips unknown values through IgnoredAny. It
    // therefore accepts this document, whereas our established complete-JSON
    // boundary rejects an out-of-range number anywhere in the response.
    let direct: agena_api::resource::ModelCatalogListResponse = serde_json::from_str(body).unwrap();
    assert_eq!(direct.total, 0);

    let client = AgenaClient::new("http://127.0.0.1:1").unwrap();
    let response = json_decoder_response(reqwest::StatusCode::OK, body, None).await;
    match client
        .parse_json::<agena_api::resource::ModelCatalogListResponse>(response)
        .await
        .unwrap_err()
    {
        ClientError::Protocol(message) => {
            assert!(message.contains("server API returned invalid JSON"));
            assert!(message.contains("number out of range"));
        }
        other => panic!("unknown numeric field bypassed JSON validation: {other:?}"),
    }
}
