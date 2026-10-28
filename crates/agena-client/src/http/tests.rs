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
