use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
    worker: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

async fn serve(handler: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static) -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let worker = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut bytes = vec![0; 8192];
            let n = socket.read(&mut bytes).await.unwrap_or_default();
            if n == 0 {
                continue;
            }
            count.fetch_add(1, Ordering::SeqCst);
            let request = String::from_utf8_lossy(&bytes[..n]);
            let target = request.split_whitespace().nth(1).unwrap_or("/");
            let _ = socket.write_all(&handler(target)).await;
        }
    });
    Fixture {
        address,
        requests,
        worker,
    }
}

fn options() -> SpiderFetchOptions {
    SpiderFetchOptions {
        respect_robots_txt: false,
        timeout: Duration::from_secs(2),
        ..Default::default()
    }
}

fn url(fixture: &Fixture, path: &str) -> url::Url {
    url::Url::parse(&format!(
        "http://fixture.invalid:{}{path}",
        fixture.address.port()
    ))
    .unwrap()
}

#[tokio::test]
async fn dns_is_pinned_and_chunked_bodies_have_exact_limit_semantics() {
    let fixture = serve(|_| b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n5\r\nhello\r\n5\r\nworld\r\n0\r\n\r\n".to_vec()).await;
    for (limit, expected, truncated) in [(5, "hello", true), (10, "helloworld", false)] {
        let mut options = options();
        options.max_body_bytes = limit;
        let page = fetch(&url(&fixture, "/?z=2&z=1&ref=page"), &options, |_| async {
            Ok(vec![fixture.address])
        })
        .await
        .unwrap();
        assert_eq!(page.body, expected);
        assert_eq!(page.truncated, truncated);
        assert_eq!(page.final_url.query(), Some("z=2&z=1&ref=page"));
    }
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn denied_redirect_never_connects_to_the_new_origin() {
    let fixture = serve(|_| b"HTTP/1.1 302 Found\r\nLocation: http://denied.invalid/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()).await;
    let checked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let error = fetch(&url(&fixture, "/start"), &options(), |target| {
        checked.lock().unwrap().push(target.to_string());
        async move {
            if target.host_str() == Some("denied.invalid") {
                Err(PluginError::from_kind(
                    agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                    "fixture denied",
                ))
            } else {
                Ok(vec![fixture.address])
            }
        }
    })
    .await
    .unwrap_err();
    assert_eq!(
        error.kind,
        agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
    );
    assert_eq!(checked.lock().unwrap().len(), 2);
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn robots_matching_blocks_disallowed_paths_before_page_requests() {
    let fixture = serve(|path| {
        let body = if path == "/robots.txt" {
            "User-agent: agena-web\nDisallow: /private\nAllow: /private/open\n"
        } else {
            "public fixture"
        };
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    })
    .await;
    let mut options = options();
    options.respect_robots_txt = true;
    let error = fetch(&url(&fixture, "/private"), &options, |_| async {
        Ok(vec![fixture.address])
    })
    .await
    .unwrap_err();
    assert!(error.diagnostic_message().contains("disallows"));
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
    let page = fetch(&url(&fixture, "/private/open"), &options, |_| async {
        Ok(vec![fixture.address])
    })
    .await
    .unwrap();
    assert_eq!(page.body, "public fixture");
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn total_deadline_includes_body_streaming_and_cancellation_drops_the_socket() {
    for cancel in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, started) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            assert!(socket.read(&mut buf).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\na")
                .await
                .unwrap();
            ready.send(()).unwrap();
            let read = socket.read(&mut buf).await;
            assert!(
                matches!(read, Ok(0) | Err(_)),
                "aborted fetch must close the connection"
            );
        });
        let client = tokio::spawn(async move {
            let mut options = options();
            options.timeout = Duration::from_millis(250);
            fetch(
                &url::Url::parse(&format!("http://fixture.invalid:{}/slow", address.port()))
                    .unwrap(),
                &options,
                |_| async { Ok(vec![address]) },
            )
            .await
        });
        started.await.unwrap();
        if cancel {
            client.abort();
            assert!(client.await.unwrap_err().is_cancelled());
        } else {
            assert!(client.await.unwrap().is_err());
        }
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn unsafe_url_and_binary_body_fail_explicitly() {
    for raw in ["file:///private", "http://user:password@example.invalid/"] {
        let calls = AtomicUsize::new(0);
        assert!(
            fetch(&url::Url::parse(raw).unwrap(), &options(), |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(Vec::new()) }
            })
            .await
            .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    let fixture = serve(|_| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\n%PDF-x".to_vec()
    })
    .await;
    assert!(
        fetch(&url(&fixture, "/binary"), &options(), |_| async {
            Ok(vec![fixture.address])
        })
        .await
        .unwrap_err()
        .diagnostic_message()
        .contains("binary")
    );
}
