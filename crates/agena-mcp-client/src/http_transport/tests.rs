use super::*;
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
        authorized_client("http://127.0.0.1:39123/mcp", &OriginalOnly).await,
        Err(McpError::PermissionDenied)
    ));
}

#[tokio::test]
async fn pinned_client_uses_approved_address_and_rejects_unpinned_dns() {
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
    let client = pinned_client(&url, &[address]).unwrap();
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
    let request = server.await.unwrap();
    assert!(
        request
            .to_ascii_lowercase()
            .contains("host: pin-only.invalid:")
    );
    let unpinned = client
        .get(format!("http://not-pinned.invalid:{}/mcp", address.port()))
        .send()
        .await;
    assert!(unpinned.is_err());
}

struct LiveAuthority {
    allowed: AtomicBool,
    checks: AtomicUsize,
}

#[async_trait]
impl McpHttpNetworkAuthorizer for LiveAuthority {
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
    let client = AuthorizedMcpHttpClient::new(authority.clone());
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
