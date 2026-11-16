use super::*;
use rmcp::transport::streamable_http_client::StreamableHttpClient;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    let client = authorized_mcp_client(authority.clone()).unwrap();
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
