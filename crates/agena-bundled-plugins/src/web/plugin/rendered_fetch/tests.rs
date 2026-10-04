use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[ignore = "requires local Chrome; isolated HTTP and browser contexts only"]
async fn real_rendered_fetch_checks_requests_status_budgets_and_cancellation() {
    use agena_plugin_host::sdk::host_api::{
        HostCallbackContext, run_in_isolated_host_callback_context,
    };
    run_in_isolated_host_callback_context(
        HostCallbackContext {
            session_id: Some(27),
            call_id: Some(41),
            ..Default::default()
        },
        run_fixture(),
    )
    .await;
}

async fn run_fixture() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let blocked_hits = Arc::new(AtomicUsize::new(0));
    let slow_hits = Arc::new(AtomicUsize::new(0));
    let worker = {
        let blocked = blocked_hits.clone();
        let slow = slow_hits.clone();
        tokio::spawn(async move {
            let mut peers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    peer = listener.accept() => {
                        let (mut socket, _) = peer.unwrap();
                        let blocked = blocked.clone();
                        let slow = slow.clone();
                        peers.spawn(async move {
                            let mut bytes = [0;8192];
                            let count = socket.read(&mut bytes).await.unwrap();
                            let request = String::from_utf8_lossy(&bytes[..count]);
                            let path = request.split_whitespace().nth(1).unwrap_or("/");
                            let (status, headers, body) = match path {
                                "/redirect" => (302, "Location: /page\r\n", "".to_owned()),
                                "/denied-redirect" => (302, "Location: /blocked\r\n", "".to_owned()),
                                "/blocked" => {blocked.fetch_add(1, Ordering::SeqCst); (200,"","must never load".into())},
                                "/script" => (200,"Content-Type: application/javascript\r\n","document.body.innerHTML='<article id=ready><h1>RENDERED_MARKER 中文</h1><p>JavaScript produced this readable local fixture with enough content for extraction.</p></article>'".into()),
                                "/subresource" => (200,"","<script src='/blocked'></script>".into()),
                                "/missing" => (404,"","<h1 id=ready>Not found</h1>".into()),
                                "/large" => (200,"",format!("<p id=ready>{}</p>", "中文".repeat(1000))),
                                "/slow" => {slow.fetch_add(1,Ordering::SeqCst);tokio::time::sleep(Duration::from_secs(30)).await;(200,"","late".into())},
                                _ => (200,"","<!doctype html><title>render fixture</title><body><script src='/script'></script></body>".into()),
                            };
                            let content_type = if headers.contains("Content-Type:") { "" } else { "Content-Type: text/html; charset=utf-8\r\n" };
                            let response = format!("HTTP/1.1 {status} Fixture\r\n{headers}{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                            let _ = socket.write_all(response.as_bytes()).await;
                        });
                    }
                    _ = peers.join_next(), if !peers.is_empty() => {}
                }
            }
        })
    };
    struct Cleanup(tokio::task::JoinHandle<()>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.abort();
            let _ = shutdown_local_browser();
        }
    }
    let _cleanup = Cleanup(worker);
    let endpoint =
        tokio::task::spawn_blocking(|| local_browser_endpoint(&LocalBrowserOptions::default()))
            .await
            .unwrap()
            .expect("Chrome required, no silent skip");
    let observer = CdpClient::connect(&endpoint, None, None).await.unwrap();
    let before = contexts(&observer).await;
    let denied = Arc::new(AtomicUsize::new(0));
    let count = denied.clone();
    let policy: BrowserRequestPolicy = Arc::new(move |raw, _| {
        let count = count.clone();
        Box::pin(async move {
            assert_eq!(
                agena_plugin_host::sdk::host_api::current_host_callback_context()
                    .unwrap()
                    .session_id,
                Some(27)
            );
            let url = url::Url::parse(&raw).unwrap();
            if url.host_str() != Some("127.0.0.1")
                || url.port() != Some(address.port())
                || url.path() == "/blocked"
            {
                count.fetch_add(1, Ordering::SeqCst);
                Err("fixture policy denied before dispatch".into())
            } else {
                Ok(())
            }
        })
    });
    let mut options = SpiderFetchOptions {
        respect_robots_txt: false,
        timeout: Duration::from_secs(15),
        ..Default::default()
    };
    options.browser.enabled = true;
    options.browser.wait_for_selector = Some("#ready".into());
    options.browser.wait_timeout = Duration::from_secs(5);
    let url = |path: &str| url::Url::parse(&format!("http://{address}{path}")).unwrap();
    let source = render(&endpoint, &url("/redirect"), &options, policy.clone())
        .await
        .unwrap();
    assert_eq!(source.status, 200);
    assert_eq!(source.final_url.path(), "/page");
    assert!(source.body.contains("RENDERED_MARKER 中文"));
    assert_eq!(contexts(&observer).await, before);
    assert_eq!(
        render(&endpoint, &url("/missing"), &options, policy.clone())
            .await
            .unwrap()
            .status,
        404
    );
    for path in ["/denied-redirect", "/subresource"] {
        let error = render(&endpoint, &url(path), &options, policy.clone())
            .await
            .unwrap_err();
        assert!(
            error
                .diagnostic_message()
                .contains("blocked before dispatch"),
            "{}",
            error.diagnostic_message()
        );
        assert_eq!(blocked_hits.load(Ordering::SeqCst), 0);
        assert_eq!(contexts(&observer).await, before);
    }
    assert!(denied.load(Ordering::SeqCst) >= 2);
    options.max_body_bytes = 4096;
    let source = render(&endpoint, &url("/large"), &options, policy.clone())
        .await
        .unwrap();
    assert!(source.truncated);
    assert!(source.body.len() <= 4096);
    assert!(source.body.contains("中文"));
    for cancel in [false, true] {
        let endpoint = endpoint.clone();
        let url = url("/slow");
        let options = options.clone();
        let policy = policy.clone();
        let expected_hits = slow_hits.load(Ordering::SeqCst) + 1;
        let context = agena_plugin_host::sdk::host_api::current_host_callback_context().unwrap();
        let operation = tokio::spawn(
            agena_plugin_host::sdk::host_api::run_in_isolated_host_callback_context(
                context,
                async move {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        render(&endpoint, &url, &options, policy),
                    )
                    .await
                },
            ),
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while slow_hits.load(Ordering::SeqCst) < expected_hits {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if cancel {
            operation.abort();
            let _ = operation.await;
        } else {
            assert!(operation.await.unwrap().is_err());
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while contexts(&observer).await != before {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cancel/timeout must dispose the owned context");
    }
    eprintln!(
        "AUDIT_REAL_RENDER: JS, redirect and subresource denial, HTTP status, Unicode byte limit, timeout/cancel context cleanup passed"
    );
}

async fn contexts(root: &CdpClient) -> Vec<String> {
    root.command("Target.getBrowserContexts", serde_json::json!({}))
        .await
        .unwrap()["browserContextIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}
