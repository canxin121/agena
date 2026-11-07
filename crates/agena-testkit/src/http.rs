//! Authenticated HTTP escape hatch for public routes, malformed request bodies,
//! headers, and negative assertions that the typed client intentionally rejects.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use reqwest::{Method, RequestBuilder, StatusCode, Url, header::HeaderMap};
use serde::de::DeserializeOwned;

const DEFAULT_RESPONSE_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct TestHttpClient {
    base_url: Url,
    client: reqwest::Client,
    bearer: Arc<Option<String>>,
    timeout: Duration,
    response_limit: usize,
}

impl TestHttpClient {
    pub(crate) async fn connect(
        base_url: &str,
        password: Option<&str>,
        timeout: Duration,
    ) -> Result<Self> {
        let base_url = validate_server_url(base_url)?;
        let mut client = Self {
            base_url,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(timeout)
                .build()?,
            bearer: Arc::new(None),
            timeout,
            response_limit: DEFAULT_RESPONSE_LIMIT,
        };
        if let Some(password) = password {
            let response = client
                .send_json(
                    Method::POST,
                    "/auth/session",
                    &serde_json::json!({"password": password}),
                )
                .await?
                .require_success()?;
            let value: serde_json::Value = response.json()?;
            let token = value
                .get("token")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .context("authentication returned no bearer token")?;
            client.bearer = Arc::new(Some(token.to_owned()));
        }
        Ok(client)
    }

    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    /// Same owned endpoint with no credentials, for authentication assertions.
    pub fn anonymous(&self) -> Self {
        let mut client = self.clone();
        client.bearer = Arc::new(None);
        client
    }

    /// Supply an explicit token, including an invalid token for a negative test.
    pub fn with_bearer_token(&self, token: impl Into<String>) -> Self {
        let mut client = self.clone();
        client.bearer = Arc::new(Some(token.into()));
        client
    }

    pub fn with_response_limit(&self, bytes: usize) -> Self {
        let mut client = self.clone();
        client.response_limit = bytes;
        client
    }

    pub fn request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        ensure!(
            path.starts_with('/') && !path.starts_with("//") && !path.contains('\\'),
            "HTTP fixture paths must be absolute paths on the owned server"
        );
        let url = self.base_url.join(path)?;
        self.check_origin(&url)?;
        let mut request = self.client.request(method, url).timeout(self.timeout);
        if let Some(bearer) = self.bearer.as_ref() {
            request = request.bearer_auth(bearer);
        }
        Ok(request)
    }

    pub async fn get(&self, path: &str) -> Result<TestHttpResponse> {
        self.execute(self.request(Method::GET, path)?).await
    }

    pub async fn send_json(
        &self,
        method: Method,
        path: &str,
        value: &impl serde::Serialize,
    ) -> Result<TestHttpResponse> {
        self.execute(self.request(method, path)?.json(value)).await
    }

    /// Does not parse, normalize, or validate the request body.
    pub async fn send_bytes(
        &self,
        method: Method,
        path: &str,
        content_type: Option<&str>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<TestHttpResponse> {
        let mut request = self.request(method, path)?.body(bytes.into());
        if let Some(content_type) = content_type {
            request = request.header(reqwest::header::CONTENT_TYPE, content_type);
        }
        self.execute(request).await
    }

    pub async fn execute(&self, request: RequestBuilder) -> Result<TestHttpResponse> {
        let request = request.build()?;
        self.check_origin(request.url())?;
        tokio::time::timeout(self.timeout, async {
            let response = self.client.execute(request).await?;
            let status = response.status();
            let headers = response.headers().clone();
            let mut stream = response.bytes_stream();
            let mut body = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                ensure!(
                    body.len().saturating_add(chunk.len()) <= self.response_limit,
                    "HTTP response exceeded the fixture's {} byte limit",
                    self.response_limit
                );
                body.extend_from_slice(&chunk);
            }
            Ok(TestHttpResponse {
                status,
                headers,
                body,
            })
        })
        .await
        .context("HTTP fixture request timed out")?
    }

    /// Open a same-origin authenticated HTTP response without buffering its
    /// body. SSE consumers must impose their own body/deadline limits and drop
    /// the response when finished; the handshake itself is bounded here.
    pub async fn open_stream(&self, request: RequestBuilder) -> Result<reqwest::Response> {
        let request = request.build()?;
        self.check_origin(request.url())?;
        tokio::time::timeout(self.timeout, self.client.execute(request))
            .await
            .context("HTTP stream handshake exceeded the fixture deadline")?
            .context("opening the authenticated HTTP stream failed")
    }

    fn check_origin(&self, url: &Url) -> Result<()> {
        ensure!(
            url.origin() == self.base_url.origin()
                && url.username().is_empty()
                && url.password().is_none(),
            "HTTP fixture request tried to leave its owned server"
        );
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct TestHttpResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl TestHttpResponse {
    /// Opt-in status assertion: unsuccessful HTTP responses remain available to
    /// tests by default, including their production error envelope.
    pub fn require_success(self) -> Result<Self> {
        if !self.status.is_success() {
            let snippet = &self.body[..self.body.len().min(4096)];
            bail!("HTTP {}: {}", self.status, String::from_utf8_lossy(snippet));
        }
        Ok(self)
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).context("response was not the expected JSON document")
    }

    pub fn text(&self) -> Result<&str> {
        std::str::from_utf8(&self.body).context("response body was not UTF-8")
    }
}

pub(crate) fn validate_server_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("invalid server endpoint record URL")?;
    let loopback = url
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|host| host.is_loopback());
    ensure!(
        url.scheme() == "http"
            && loopback
            && url.port().is_some_and(|port| port != 0)
            && url.username().is_empty()
            && url.password().is_none()
            && (url.path().is_empty() || url.path() == "/")
            && url.query().is_none()
            && url.fragment().is_none(),
        "server endpoint must be an HTTP loopback address with an explicit nonzero port"
    );
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_never_accepts_remote_or_ambiguous_endpoints() {
        for url in [
            "http://example.com:1234",
            "http://127.0.0.1",
            "http://127.0.0.1:0",
            "https://127.0.0.1:1234",
            "http://user@127.0.0.1:1234",
            "http://127.0.0.1:1234/api",
            "http://127.0.0.1:1234/?redirect=elsewhere",
        ] {
            assert!(validate_server_url(url).is_err(), "{url}");
        }
        assert!(validate_server_url("http://127.0.0.1:1234").is_ok());
    }

    #[tokio::test]
    async fn raw_client_keeps_invalid_bytes_and_auth_negative_responses() -> Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await?;
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0u8; 1024];
                let count = socket.read(&mut chunk).await?;
                ensure!(count != 0, "request closed before body arrived");
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n")
                    && bytes.ends_with(&[b'{', 0xff])
                {
                    break;
                }
            }
            socket.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
            ).await?;
            Ok::<_, anyhow::Error>(bytes)
        });
        let client =
            TestHttpClient::connect(&format!("http://{address}"), None, Duration::from_secs(2))
                .await?;
        assert!(client.request(Method::GET, "//example.com/").is_err());
        let response = client
            .send_bytes(
                Method::POST,
                "/api/v1/settings",
                Some("application/json"),
                vec![b'{', 0xff],
            )
            .await?;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        assert!(response.require_success().is_err());
        let request = server.await??;
        assert!(request.ends_with(&[b'{', 0xff]));
        Ok(())
    }
}
