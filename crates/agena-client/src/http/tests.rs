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
    request_bodies: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
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
        let request_bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_bodies = request_bodies.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let captured = captured.clone();
                let captured_bodies = captured_bodies.clone();
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
                    captured_bodies.lock().unwrap().push(body.clone());
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
            request_bodies,
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
async fn session_media_download_limits_reject_before_the_response_body_finishes() {
    // Keep each response unfinished until the client returns. A client that
    // buffers the whole body before checking its limit would time out here.
    for (framing, prefix, limit, expected) in [
        ("Content-Length: 9\r\n".to_owned(), "", 8, "8 byte"),
        (
            "Transfer-Encoding: chunked\r\n".to_owned(),
            "5\r\n12345\r\n",
            4,
            "4 byte",
        ),
        (
            format!("Content-Length: {}\r\n", MAX_BINARY_RESPONSE_BYTES + 1),
            "",
            usize::MAX,
            "100 MiB",
        ),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let mut server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
                assert!(request.len() < 16_384);
            }
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\n{framing}Connection: close\r\n\r\n{prefix}")
                        .as_bytes(),
                )
                .await
                .unwrap();
            let _ = held.await;
            request
        });
        let client = AgenaClient::new(format!("http://{address}")).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            client.download_session_media_file_with_limit(23, "agena:media:v1:opaque", limit),
        )
        .await;
        let _ = release.send(());
        match tokio::time::timeout(Duration::from_secs(2), &mut server).await {
            Ok(joined) => {
                joined.expect("media response fixture failed");
            }
            Err(_) => {
                server.abort();
                let _ = server.await;
                panic!("unfinished media response fixture did not stop");
            }
        }
        let error = result
            .expect("limit was checked only after consuming the entire body")
            .unwrap_err();
        assert!(
            matches!(error, ClientError::Protocol(ref message) if message.contains(expected)),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn session_media_download_limit_keeps_auth_opaque_query_and_safe_filename() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
            assert!(request.len() < 16_384);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Disposition: attachment; filename*=UTF-8''..%2Fsafe%2Bname.bin\r\nConnection: close\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n").await.unwrap();
        String::from_utf8(request).unwrap()
    });
    let client = AgenaClient::new(format!("http://{address}"))
        .unwrap()
        .with_bearer_token("media-test-token")
        .unwrap();
    let reference = "agena:media:v1:opaque+selector";
    let (filename, bytes) = client
        .download_session_media_file_with_limit(23, reference, 5)
        .await
        .unwrap();
    assert_eq!(filename, "safe+name.bin");
    assert_eq!(bytes, b"abcde");
    let request = server.await.unwrap();
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer media-test-token\r\n")
    );
    let target = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://{address}{target}")).unwrap();
    assert_eq!(url.path(), "/api/v1/sessions/23/media");
    let query = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    assert_eq!(query, vec![("path".to_owned(), reference.to_owned())]);
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

#[tokio::test]
async fn permission_commands_send_the_public_contract_without_null_fields() {
    use agena_api::commands::{Command as ApiCommand, UpsertPermissionRuleParams};
    let server = Fixture::new("127.0.0.1:0", "unused", Some(r#"{"id":7,"action_key":"tool:read","subject_kind":"tool","mode":"allow","scope":"session","source":"user","created_at":"2026-10-31T00:00:00Z","updated_at":"2026-10-31T00:00:00Z"}"#)).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let minimal = serde_json::json!({"mode":"ask"});
    let replacement = serde_json::json!({"tool_name":"read", "qualifier":"", "scope":"session", "session_id":7, "mode":"allow"});
    let result = client
        .command(ApiCommand::UpsertPermissionRule(
            serde_json::from_value::<UpsertPermissionRuleParams>(minimal.clone()).unwrap(),
        ))
        .await
        .unwrap();
    assert!(matches!(result, CommandResult::PermissionRule(rule) if rule.id == 7));
    client
        .command(ApiCommand::ReplacePermissionRule(
            ReplacePermissionRuleParams {
                rule_id: 7,
                rule: serde_json::from_value(replacement.clone()).unwrap(),
            },
        ))
        .await
        .unwrap();
    let requests = server.requests.lock().unwrap();
    assert!(requests[0].starts_with("POST /api/v1/permission-rules "));
    assert!(requests[1].starts_with("PUT /api/v1/permission-rules/7 "));
    let bodies = server.request_bodies.lock().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bodies[0]).unwrap(),
        minimal
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bodies[1]).unwrap(),
        replacement
    );
}

#[tokio::test]
async fn adapter_probe_query_uses_the_same_body_as_the_direct_http_method() {
    let server = Fixture::new(
        "127.0.0.1:0",
        "unused",
        Some(r#"{"provider_id":"fixture","adapters":[]}"#),
    )
    .await;
    let client = AgenaClient::new(&server.url).unwrap();
    let request: ListProviderAdapterModelsParams = serde_json::from_value(serde_json::json!({
        "base_url":"https://example.test", "protocol_paths":{"anthropic":"/messages"},
        "api_key":{"kind":"env","value":"PROVIDER_KEY"}, "adapter_ids":["anthropic"]
    }))
    .unwrap();
    let direct = client
        .list_provider_adapter_models(request.clone())
        .await
        .unwrap();
    assert_eq!(direct.provider_id, "fixture");
    let result = client
        .query(Query::ListProviderAdapterModels(request))
        .await
        .unwrap();
    assert!(
        matches!(result, QueryResult::ProviderAdapterModels(response) if response.provider_id == "fixture")
    );
    let bodies = server.request_bodies.lock().unwrap();
    let direct: serde_json::Value = serde_json::from_slice(&bodies[0]).unwrap();
    assert_eq!(
        direct,
        serde_json::from_slice::<serde_json::Value>(&bodies[1]).unwrap()
    );
    assert_eq!(
        direct["protocol_paths"],
        serde_json::json!({"openai":"/v1","anthropic":"/messages","gemini":"/v1beta"})
    );
    assert_eq!(
        direct["api_key"],
        serde_json::json!({"kind":"env","value":"PROVIDER_KEY"})
    );
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.starts_with("POST /api/v1/providers/models "))
    );
}

#[tokio::test]
async fn aws_profiles_are_typed_and_keep_missing_field_defaults_and_decode_errors() {
    for (wire, expected) in [
        (
            r#"{"profiles":["default","开发"]}"#,
            vec!["default", "开发"],
        ),
        ("{}", Vec::new()),
    ] {
        let server = Fixture::new("127.0.0.1:0", "unused", Some(wire)).await;
        let client = AgenaClient::new(&server.url).unwrap();
        let response = client.aws_profile_names().await.unwrap();
        assert_eq!(response.profiles, expected);
    }
    for wire in [r#"{"profiles":null}"#, r#"{"profiles":[12]}"#] {
        let server = Fixture::new("127.0.0.1:0", "unused", Some(wire)).await;
        let client = AgenaClient::new(&server.url).unwrap();
        assert!(matches!(
            client.aws_profile_names().await,
            Err(ClientError::Decode(_))
        ));
    }
}

#[tokio::test]
async fn patch_settings_returns_the_shared_edit_response() {
    let server = Fixture::new("127.0.0.1:0", "unused", Some(r#"{"config_path":"/fixture/config.json","config_found":true,"operation":"patch","dry_run":true,"changed":true,"created":false,"deleted":false,"validated":true,"reload_requested":false,"reload_required":true,"previous":{"enabled":false},"current":{"enabled":true}}"#)).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let response = client
        .patch_settings(
            "providers",
            serde_json::json!({"enabled":true}),
            true,
            false,
        )
        .await
        .unwrap();
    assert_eq!(response.operation, "patch");
    assert!(response.before_revision.is_empty() && response.after_revision.is_empty());
    assert!(response.reload.is_none() && response.path.is_none());
    assert!(response.dry_run && response.reload_required);
    assert_eq!(response.current, serde_json::json!({"enabled":true}));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&server.request_bodies.lock().unwrap()[0])
            .unwrap(),
        serde_json::json!({
            "path":"providers", "changes":{"enabled":true}, "dry_run":true, "validate":true, "reload":false
        })
    );
    assert!(server.requests.lock().unwrap()[0].starts_with("PATCH /api/v1/settings "));
}

#[tokio::test]
async fn typed_auth_provider_responses_keep_defaults_and_credential_request_boundaries() {
    const PROVIDER: &str = r#"{"provider_id":"fixture","configured":true,"credential_present":false,"api_key_write_supported":true,"refresh_supported":false}"#;
    const PROVIDERS: &str = r#"[{"provider_id":"fixture","configured":true,"credential_present":false,"api_key_write_supported":true,"refresh_supported":false}]"#;
    let list_server = Fixture::new("127.0.0.1:0", "unused", Some(PROVIDERS)).await;
    let providers = AgenaClient::new(&list_server.url)
        .unwrap()
        .auth_providers()
        .await
        .unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].provider_id, "fixture");
    assert!(!providers[0].expired);

    let server = Fixture::new("127.0.0.1:0", "unused", Some(PROVIDER)).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let provider = client.auth_provider("fixture").await.unwrap();
    assert!(!provider.expired && provider.credential_type.is_none());
    let stored = client
        .set_auth_api_key("fixture", "fixture-private-key".to_owned())
        .await
        .unwrap();
    assert_eq!(stored.provider_id, "fixture");
    let removed = client.delete_auth_provider("fixture").await.unwrap();
    assert!(!removed.credential_present);
    let requests = server.requests.lock().unwrap();
    assert!(requests[0].starts_with("GET /api/v1/auth/providers/fixture "));
    assert!(requests[1].starts_with("PUT /api/v1/auth/providers/fixture/api-key "));
    assert!(requests[2].starts_with("DELETE /api/v1/auth/providers/fixture "));
    let bodies = server.request_bodies.lock().unwrap();
    assert!(bodies[0].is_empty() && bodies[2].is_empty());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bodies[1]).unwrap(),
        serde_json::json!({"api_key": "fixture-private-key"})
    );
}

#[tokio::test]
async fn typed_auth_flows_keep_handshakes_and_pending_poll_states() {
    const BROWSER: &str = r#"{"provider_id":"fixture","authorize_url":"https://example.test/authorize","state":"fixture-state","pkce_verifier":"fixture-verifier"}"#;
    let server = Fixture::new("127.0.0.1:0", "unused", Some(BROWSER)).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let openai = client
        .start_openai_browser_auth("fixture", "http://localhost:9876/auth/callback")
        .await
        .unwrap();
    let gitlab = client
        .start_gitlab_browser_auth("fixture", "http://localhost:9876/auth/callback")
        .await
        .unwrap();
    assert_eq!(openai.state, "fixture-state");
    assert_eq!(gitlab.pkce_verifier, "fixture-verifier");
    assert!(openai.instance_url.is_none() && gitlab.instance_url.is_none());
    for body in server.request_bodies.lock().unwrap().iter() {
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(body).unwrap(),
            serde_json::json!({
                "provider_id":"fixture", "redirect_uri":"http://localhost:9876/auth/callback"
            })
        );
    }

    const DEVICE: &str = r#"{"provider_id":"fixture","verification_url":"https://example.test/device","user_code":"USER-CODE","device_code":"fixture-device","interval_seconds":3}"#;
    let server = Fixture::new("127.0.0.1:0", "unused", Some(DEVICE)).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let openai = client.start_openai_device_auth("fixture").await.unwrap();
    let copilot = client
        .start_copilot_device_auth("fixture", Some("github.example.test"))
        .await
        .unwrap();
    assert_eq!(openai.user_code, "USER-CODE");
    assert_eq!(copilot.device_code, "fixture-device");
    assert_eq!(copilot.interval_seconds, 3);
    {
        let bodies = server.request_bodies.lock().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bodies[0]).unwrap(),
            serde_json::json!({"provider_id":"fixture"})
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bodies[1]).unwrap(),
            serde_json::json!({
                "provider_id":"fixture", "enterprise_domain":"github.example.test"
            })
        );
    }

    let server = Fixture::new("127.0.0.1:0", "unused", Some(r#"{"completed":false}"#)).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let openai = client
        .poll_openai_device_auth("fixture", "fixture-device".into(), "USER-CODE".into())
        .await
        .unwrap();
    let copilot = client
        .poll_copilot_device_auth("fixture", "fixture-device".into(), None)
        .await
        .unwrap();
    assert!(!openai.completed && openai.provider.is_none());
    assert!(!copilot.completed && copilot.provider.is_none());
    {
        let bodies = server.request_bodies.lock().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bodies[0]).unwrap(),
            serde_json::json!({
                "provider_id":"fixture", "device_code":"fixture-device", "user_code":"USER-CODE"
            })
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bodies[1]).unwrap(),
            serde_json::json!({
                "provider_id":"fixture", "device_code":"fixture-device", "enterprise_domain":null
            })
        );
    }

    let server = Fixture::new("127.0.0.1:0", "unused", Some(r#"{"completed":true}"#)).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let openai = client
        .finish_openai_browser_auth(
            "fixture",
            "fixture-code".into(),
            "fixture-verifier".into(),
            "http://localhost:9876/auth/callback".into(),
        )
        .await
        .unwrap();
    let gitlab = client
        .finish_gitlab_browser_auth(
            "fixture",
            "fixture-code".into(),
            "fixture-verifier".into(),
            "http://localhost:9876/auth/callback".into(),
        )
        .await
        .unwrap();
    assert!(openai.completed && gitlab.completed);
    let bodies = server.request_bodies.lock().unwrap();
    for body in bodies.iter() {
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(body).unwrap(),
            serde_json::json!({
                "provider_id":"fixture", "code":"fixture-code", "pkce_verifier":"fixture-verifier",
                "redirect_uri":"http://localhost:9876/auth/callback"
            })
        );
    }
}

#[tokio::test]
async fn typed_auth_responses_keep_protocol_and_decode_error_classes() {
    let malformed = Fixture::new("127.0.0.1:0", "unused", Some("{")).await;
    let client = AgenaClient::new(&malformed.url).unwrap();
    assert!(matches!(
        client.auth_provider("fixture").await,
        Err(ClientError::Protocol(_))
    ));
    let invalid = Fixture::new("127.0.0.1:0", "unused", Some(r#"{"provider_id":17}"#)).await;
    let client = AgenaClient::new(&invalid.url).unwrap();
    assert!(matches!(
        client.auth_provider("fixture").await,
        Err(ClientError::Decode(_))
    ));
}

#[tokio::test]
async fn typed_auth_lists_reject_invalid_dates_and_unknown_credential_variants() {
    for wire in [
        r#"[{"provider_id":"fixture","configured":true,"credential_present":true,"api_key_write_supported":false,"refresh_supported":true,"expires_at":"not-rfc3339"}]"#,
        r#"[{"provider_id":"fixture","configured":true,"credential_present":true,"api_key_write_supported":false,"refresh_supported":true,"credential_type":"unknown-credential-type"}]"#,
        r#"[{"provider_id":"fixture","configured":true,"credential_present":true,"api_key_write_supported":false,"refresh_supported":true,"credential_issuer":"unknown-credential-issuer"}]"#,
    ] {
        let server = Fixture::new("127.0.0.1:0", "unused", Some(wire)).await;
        let client = AgenaClient::new(&server.url).unwrap();
        assert!(
            matches!(client.auth_providers().await, Err(ClientError::Decode(_))),
            "wire: {wire}"
        );
    }
}

#[tokio::test]
async fn settings_delete_transmits_false_instead_of_triggering_default_reload() {
    let server = Fixture::new(
        "127.0.0.1:0",
        "settings",
        Some(r#"{"config_path":"/fixture/config.json","config_found":true,"operation":"delete","path":"model","dry_run":false,"changed":true,"created":false,"deleted":true,"validated":true,"reload_requested":false,"reload_required":false,"previous":"old","current":null}"#),
    ).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let result = client
        .delete_settings_layer_value("workspace", "model", false, false)
        .await
        .unwrap();
    assert!(!result.reload_requested);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let line = requests[0].lines().next().unwrap();
    assert!(line.starts_with("DELETE /api/v1/settings/layers/workspace?"));
    let target = line.split_whitespace().nth(1).unwrap();
    let url = url::Url::parse(&format!("{}{target}", server.url)).unwrap();
    let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query.get("path").map(String::as_str), Some("model"));
    assert_eq!(query.get("reload").map(String::as_str), Some("false"));
}

#[tokio::test]
async fn attempt_wait_distinguishes_recovered_terminal_from_settled() {
    let server = Fixture::new(
        "127.0.0.1:0",
        "attempts",
        Some(r#"{"session_id":7,"execution_id":"00000000-0000-0000-0000-000000000001","message_id":11,"attempt_generation":2,"current_attempt_generation":2,"phase":"interrupted","settlement":"recovered","kernel_epoch":"00000000-0000-0000-0000-000000000003","owner_epoch":"00000000-0000-0000-0000-000000000002"}"#),
    ).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let execution = uuid::Uuid::from_u128(1);
    let terminal = client
        .wait_execution_attempt_terminal(7, execution, 2, Duration::from_secs(1))
        .await
        .unwrap();
    assert!(terminal.phase.is_terminal());
    assert_eq!(
        terminal.settlement,
        agena_api::resource::ExecutionAttemptSettlement::Recovered
    );
    let error = client
        .wait_execution_attempt_settled(7, execution, 2, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(error.operator_diagnostic().contains("Recovered"));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.starts_with(&format!(
        "GET /api/v1/sessions/7/executions/{execution}/attempts/2 ",
    ))));
}

#[tokio::test]
async fn exact_attempt_cancel_sends_generation_and_preserves_mismatch_outcome() {
    let server = Fixture::new(
        "127.0.0.1:0",
        "attempts",
        Some(r#"{"result":"execution_mismatch"}"#),
    )
    .await;
    let client = AgenaClient::new(&server.url).unwrap();
    let execution = uuid::Uuid::from_u128(1);
    let outcome = client
        .cancel_execution_attempt(7, execution, 2)
        .await
        .unwrap();
    assert_eq!(
        outcome.result,
        agena_domain::CancellationResult::ExecutionMismatch
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "DELETE /api/v1/sessions/7/executions/{execution}/attempts/2 ",
    )));
}

#[tokio::test]
async fn memory_delete_sends_the_observed_revision() {
    let server = Fixture::new("127.0.0.1:0", "memory", Some("{}")).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let revision = "a".repeat(64);
    client
        .delete_memory("report-style", &revision)
        .await
        .unwrap();
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].lines().next().unwrap(),
        format!("DELETE /api/v1/memories/report-style?expected_sha256={revision} HTTP/1.1")
    );
}
