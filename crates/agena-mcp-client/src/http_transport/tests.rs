use super::*;
use rmcp::transport::streamable_http_client::StreamableHttpClient;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

struct OriginalOnly;

#[async_trait]
impl McpHttpNetworkAuthorizer for OriginalOnly {
    async fn authorize(&self, _target: &str) -> McpResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn original_url_allow_without_numeric_support_fails_closed() {
    assert!(matches!(
        authorized_options("http://127.0.0.1:39123/mcp", &OriginalOnly).await,
        Err(McpError::PermissionDenied)
    ));
}

#[tokio::test]
async fn direct_transport_uses_approved_address_without_resolving_the_hostname() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0_u8; 4096];
        let count = stream.read(&mut buffer).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
        String::from_utf8_lossy(&buffer[..count]).to_string()
    });
    let url = Url::parse(&format!("http://pin-only.invalid:{}/mcp", address.port())).unwrap();
    let request = reqwest::Request::new(reqwest::Method::GET, url);
    let response = execute(
        request,
        HttpOptions {
            policy: SandboxPolicy {
                disabled: true,
                ..Default::default()
            },
            pinned_addresses: vec![address],
            connect_timeout: Duration::from_secs(2),
            total_timeout: Some(Duration::from_secs(2)),
            max_request_bytes: 1024,
            max_response_bytes: Some(1024),
            cancellation: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
    let request = server.await.unwrap();
    assert!(
        request
            .to_ascii_lowercase()
            .contains("host: pin-only.invalid:")
    );
}

struct LiveAuthority {
    allowed: AtomicBool,
    checks: AtomicUsize,
}

#[async_trait]
impl McpHttpNetworkAuthorizer for LiveAuthority {
    async fn sandbox_policy(&self, _target: &str) -> McpResult<SandboxPolicy> {
        Ok(SandboxPolicy {
            disabled: true,
            ..Default::default()
        })
    }

    async fn authorize(&self, _target: &str) -> McpResult<()> {
        Ok(())
    }

    async fn authorize_resolved(&self, _target: &str, _address: SocketAddr) -> McpResult<bool> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        Ok(self.allowed.load(Ordering::SeqCst))
    }
}

#[tokio::test]
async fn legacy_sse_sends_to_the_advertised_path_and_rechecks_authority() {
    use rmcp::transport::Transport;
    use serde_json::{Value, json};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/events", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let initial = read_http_headers(&mut BufReader::new(&mut stream)).await;
        assert!(initial.starts_with("GET /events "));
        assert!(initial.contains("authorization: Bearer fixture\r\n"));
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\nevent: endpoint\ndata: /post?session=fixture\n\n").await.unwrap();

        let (post, _) = listener.accept().await.unwrap();
        let mut post = BufReader::new(post);
        let headers = read_http_headers(&mut post).await;
        assert!(headers.starts_with("POST /post?session=fixture "));
        assert!(headers.contains("authorization: Bearer fixture\r\n"));
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; length];
        post.read_exact(&mut body).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["method"],
            "ping"
        );
        post.get_mut()
            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        stream
            .write_all(b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n")
            .await
            .unwrap();
        let mut end = [0];
        assert_eq!(stream.read(&mut end).await.unwrap(), 0);
    });
    let authority = Arc::new(LiveAuthority {
        allowed: AtomicBool::new(true),
        checks: AtomicUsize::new(0),
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            "Bearer fixture".parse().unwrap(),
        );
        let mut transport = crate::legacy_sse::connect(url, headers, authority.clone(), false)
            .await
            .unwrap();
        let ping: rmcp::model::ClientJsonRpcMessage =
            serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"method":"ping"})).unwrap();
        transport.send(ping.clone()).await.unwrap();
        assert!(transport.receive().await.is_some());
        authority.allowed.store(false, Ordering::SeqCst);
        assert!(matches!(
            transport.send(ping).await,
            Err(McpError::PermissionDenied)
        ));
        assert_eq!(authority.checks.load(Ordering::SeqCst), 3);
        transport.close().await.unwrap();
        server.await.unwrap();
    })
    .await
    .unwrap();
}

async fn read_http_headers(socket: &mut BufReader<impl tokio::io::AsyncRead + Unpin>) -> String {
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        assert_ne!(socket.read_line(&mut line).await.unwrap(), 0);
        if line == "\r\n" {
            return headers;
        }
        headers.push_str(&line);
    }
}

#[tokio::test]
async fn websocket_sends_json_rpc_and_closes_with_its_authority() {
    use futures::{SinkExt, StreamExt};
    use rmcp::transport::Transport;
    use serde_json::{Value, json};
    use tokio_tungstenite::{
        accept_hdr_async,
        tungstenite::{
            Message,
            handshake::server::{Request, Response},
        },
    };

    struct CancellableAuthority(WorkerCancellation);
    #[async_trait]
    impl McpHttpNetworkAuthorizer for CancellableAuthority {
        async fn authorize(&self, _: &str) -> McpResult<()> {
            Ok(())
        }
        async fn authorize_resolved(&self, _: &str, address: SocketAddr) -> McpResult<bool> {
            Ok(address.ip().is_loopback())
        }
        async fn sandbox_policy(&self, _: &str) -> McpResult<SandboxPolicy> {
            Ok(SandboxPolicy {
                disabled: true,
                ..Default::default()
            })
        }
        fn cancellation(&self) -> Option<WorkerCancellation> {
            Some(self.0.child_token())
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/ws", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(stream, |request: &Request, mut response: Response| {
            assert_eq!(request.uri().path(), "/ws");
            assert_eq!(request.headers()["authorization"], "Bearer fixture");
            assert_eq!(request.headers()["sec-websocket-protocol"], "mcp");
            response
                .headers_mut()
                .insert("sec-websocket-protocol", "mcp".parse().unwrap());
            Ok(response)
        })
        .await
        .unwrap();
        let message = socket.next().await.unwrap().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&message).unwrap()["method"],
            "ping"
        );
        socket
            .send(Message::text(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#))
            .await
            .unwrap();
        assert!(!matches!(socket.next().await, Some(Ok(frame)) if !frame.is_close()));
    });
    let authority = Arc::new(CancellableAuthority(WorkerCancellation::new()));
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            "Bearer fixture".parse().unwrap(),
        );
        let mut transport = crate::websocket::connect(url, headers, authority.clone(), false)
            .await
            .unwrap();
        transport
            .send(serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"method":"ping"})).unwrap())
            .await
            .unwrap();
        assert!(transport.receive().await.is_some());
        authority.0.cancel();
        assert!(transport.receive().await.is_none());
        server.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dynamic_headers_preserve_http_auth_rejections_for_refresh() {
    use rmcp::{service::ServiceError, transport::DynamicTransportError};
    use std::any::TypeId;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for status in [
            "401 Unauthorized",
            "403 Forbidden",
            "500 Internal Server Error",
        ] {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            assert!(
                read_http_headers(&mut stream)
                    .await
                    .starts_with("POST /mcp ")
            );
            // Servers can return a JSON-RPC body without a WWW-Authenticate
            // challenge. Refresh is based on the HTTP status before decoding it.
            let body = br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"fixture"}}"#;
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.get_mut().write_all(head.as_bytes()).await.unwrap();
            stream.get_mut().write_all(body).await.unwrap();
        }
    });
    let executor = McpRequestExecutor {
        authorizer: Arc::new(LiveAuthority {
            allowed: AtomicBool::new(true),
            checks: AtomicUsize::new(0),
        }),
        dynamic_headers: true,
        remote_session_open: Arc::default(),
        app_events: None,
    };
    let url = Url::parse(&format!("http://{address}/mcp")).unwrap();
    for _ in 0..2 {
        let request = reqwest::Request::new(reqwest::Method::POST, url.clone());
        let error = executor.execute(request).await.unwrap_err();
        let framed = ServiceError::TransportSend(DynamicTransportError::from_parts(
            "fixture",
            TypeId::of::<McpRequestExecutor>(),
            Box::new(error),
        ));
        assert!(crate::header_helper::http_auth_rejected(&framed));
    }
    let request = reqwest::Request::new(reqwest::Method::POST, url);
    assert_eq!(
        executor.execute(request).await.unwrap().status(),
        http::StatusCode::INTERNAL_SERVER_ERROR,
    );
    server.await.unwrap();
}

#[tokio::test]
async fn every_mcp_request_rechecks_numeric_authority_before_io() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0_u8; 4096];
        stream.read(&mut buffer).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let authority = Arc::new(LiveAuthority {
        allowed: AtomicBool::new(true),
        checks: AtomicUsize::new(0),
    });
    let client = authorized_mcp_client(authority.clone(), false, Arc::default(), None).unwrap();
    let url: Arc<str> = format!("http://{address}/mcp").into();
    client
        .delete_session(url.clone(), "first".into(), None, HashMap::new())
        .await
        .unwrap();
    server.await.unwrap();

    authority.allowed.store(false, Ordering::SeqCst);
    let result = client
        .delete_session(url, "second".into(), None, HashMap::new())
        .await;
    assert!(
        matches!(result, Err(StreamableHttpError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied)
    );
    assert_eq!(authority.checks.load(Ordering::SeqCst), 2);
}

struct NumericOnly;

#[async_trait]
impl McpHttpNetworkAuthorizer for NumericOnly {
    async fn authorize(&self, _target: &str) -> McpResult<()> {
        Ok(())
    }
    async fn authorize_resolved(&self, _target: &str, _address: SocketAddr) -> McpResult<bool> {
        Ok(true)
    }
}

#[tokio::test]
async fn numeric_authorization_without_execution_policy_fails_closed() {
    assert!(matches!(
        authorized_options("http://127.0.0.1:39123/mcp", &NumericOnly).await,
        Err(McpError::PermissionDenied)
    ));
}

#[derive(Default)]
struct NativeAuthority(Option<WorkerCancellation>);

#[async_trait]
impl McpHttpNetworkAuthorizer for NativeAuthority {
    fn cancellation(&self) -> Option<WorkerCancellation> {
        self.0.as_ref().map(|token| token.child_token())
    }

    async fn authorize(&self, _target: &str) -> McpResult<()> {
        Ok(())
    }
    async fn authorize_resolved(&self, _target: &str, _address: SocketAddr) -> McpResult<bool> {
        Ok(true)
    }
    async fn sandbox_policy(&self, _target: &str) -> McpResult<SandboxPolicy> {
        Ok(SandboxPolicy::default())
    }
}

#[tokio::test]
async fn native_policy_and_pinned_address_reach_the_http_worker_options() {
    let options = authorized_options("http://127.0.0.1:39123/mcp", &NativeAuthority::default())
        .await
        .unwrap();
    assert!(!options.policy.disabled);
    assert_eq!(
        options.pinned_addresses,
        vec!["127.0.0.1:39123".parse().unwrap()]
    );
    assert!(
        options.total_timeout.is_none(),
        "SSE lifetime belongs to its call"
    );
    assert!(
        options.max_response_bytes.is_none(),
        "SSE is not buffered to completion"
    );
}

#[tokio::test]
async fn cancellation_is_call_owned_and_reaches_each_worker_without_cancelling_its_owner() {
    let owner = WorkerCancellation::new();
    let authority = NativeAuthority(Some(owner.child_token()));
    let target = "http://127.0.0.1:39123/mcp";
    let first = authorized_options(target, &authority).await.unwrap();
    first.cancellation.as_ref().unwrap().cancel();
    assert!(!owner.is_cancelled());

    let second = authorized_options(target, &authority).await.unwrap();
    assert!(!second.cancellation.as_ref().unwrap().is_cancelled());
    owner.cancel();
    assert!(second.cancellation.as_ref().unwrap().is_cancelled());
    let error = execute(
        reqwest::Request::new(reqwest::Method::GET, Url::parse(target).unwrap()),
        second,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
}

#[tokio::test]
async fn completed_authority_fails_before_dns_or_socket_io() {
    let token = WorkerCancellation::new();
    token.cancel();
    assert!(matches!(
        authorized_options(
            "https://must-not-resolve.invalid/mcp",
            &NativeAuthority(Some(token)),
        )
        .await,
        Err(McpError::PermissionDenied)
    ));
}
