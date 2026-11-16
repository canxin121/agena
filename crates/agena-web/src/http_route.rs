//! Snapshot proxy selection once, so authorization and connection use the
//! same route. Proxy-side DNS must not be replaced with local DNS checks.
use std::net::SocketAddr;
use std::time::Duration;

use hyper_util::client::proxy::matcher::Matcher;
use url::Url;

use crate::CrawlError;

#[derive(Clone, Default)]
pub struct WebHttpRoute {
    proxy: Option<reqwest::Proxy>,
    remote_dns: bool,
    proxy_key: String,
    addresses: Vec<SocketAddr>,
    sandbox: Option<agena_process::sandbox::SandboxPolicy>,
    sandbox_origin: Option<url::Origin>,
    cancellation: Option<agena_process::sandbox_http::WorkerCancellation>,
}

impl std::fmt::Debug for WebHttpRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebHttpRoute")
            .field("proxied", &self.proxy.is_some())
            .field("remote_dns", &self.remote_dns)
            .field("addresses", &self.addresses)
            .field("sandboxed", &self.sandbox.is_some())
            .finish()
    }
}

impl WebHttpRoute {
    /// None uses environment/OS proxy settings; an empty explicit proxy forces
    /// direct connections. NO_PROXY applies to explicit proxies as well.
    pub fn for_url(url: &Url, explicit_proxy: Option<&str>) -> Result<Self, CrawlError> {
        let matcher = match explicit_proxy {
            Some("") => Matcher::builder().build(),
            Some(proxy) => Matcher::builder()
                .all(proxy)
                .no(std::env::var("NO_PROXY")
                    .or_else(|_| std::env::var("no_proxy"))
                    .unwrap_or_default())
                .build(),
            None => Matcher::from_system(),
        };
        Self::from_matcher(url, &matcher)
    }

    fn from_matcher(url: &Url, matcher: &Matcher) -> Result<Self, CrawlError> {
        let uri = url
            .as_str()
            .parse::<http::Uri>()
            .map_err(|_| CrawlError::InvalidInput("invalid HTTP destination".into()))?;
        let Some(intercept) = matcher.intercept(&uri) else {
            return Ok(Self::default());
        };
        let mut proxy_url = Url::parse(&intercept.uri().to_string())
            .map_err(|_| CrawlError::InvalidInput("invalid configured web proxy".into()))?;
        if let Some((username, password)) = intercept.raw_auth() {
            proxy_url
                .set_username(username)
                .map_err(|_| CrawlError::InvalidInput("invalid proxy credentials".into()))?;
            proxy_url
                .set_password(Some(password))
                .map_err(|_| CrawlError::InvalidInput("invalid proxy credentials".into()))?;
        }
        let mut key = blake3::Hasher::new();
        key.update(proxy_url.as_str().as_bytes());
        let mut proxy = reqwest::Proxy::all(proxy_url.as_str())
            .map_err(|_| CrawlError::InvalidInput("invalid configured web proxy".into()))?;
        if let Some(auth) = intercept.basic_auth() {
            key.update(auth.as_bytes());
            proxy = proxy.custom_http_auth(auth.clone());
        }
        Ok(Self {
            proxy: Some(proxy),
            remote_dns: matches!(proxy_url.scheme(), "http" | "https" | "socks4a" | "socks5h"),
            proxy_key: key.finalize().to_hex().to_string(),
            addresses: Vec::new(),
            sandbox: None,
            sandbox_origin: None,
            cancellation: None,
        })
    }

    pub fn remote_dns(&self) -> bool {
        self.remote_dns
    }

    pub fn with_addresses(mut self, addresses: Vec<SocketAddr>) -> Self {
        self.addresses = addresses;
        self
    }

    pub fn cache_key(&self) -> String {
        let policy = self.sandbox.as_ref().map(|policy| {
            blake3::hash(&serde_json::to_vec(policy).expect("sandbox policy is serializable"))
                .to_hex()
                .to_string()
        });
        format!("{}:{:?}:{policy:?}", self.proxy_key, self.addresses)
    }

    /// Bind the approved destination to the current invocation's MXC policy.
    /// The worker connects directly to the pinned addresses and never consults
    /// proxy environment or resolves an additional destination.
    pub fn with_sandbox_policy(
        mut self,
        url: &Url,
        policy: agena_process::sandbox::SandboxPolicy,
    ) -> Result<Self, CrawlError> {
        if policy.disabled {
            self.sandbox = None;
            self.sandbox_origin = None;
            return Ok(self);
        }
        if self.proxy.is_some() {
            return Err(CrawlError::InvalidInput(
                "this HTTP proxy route cannot enforce the approved destination IPs inside MXC; configure a direct route for sandboxed requests".into(),
            ));
        }
        if self.addresses.is_empty() {
            return Err(CrawlError::InvalidInput(
                "sandboxed HTTP requests require approved destination addresses".into(),
            ));
        }
        self.sandbox = Some(policy);
        self.sandbox_origin = Some(url.origin());
        Ok(self)
    }

    pub fn sandboxed(&self) -> bool {
        self.sandbox.is_some()
    }

    /// The originating call owns the worker even when nobody polls its body.
    pub fn with_cancellation(
        mut self,
        cancellation: Option<agena_process::sandbox_http::WorkerCancellation>,
    ) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Execute one authorized HTTP hop. The returned response streams from
    /// the worker, so dropping it cancels the connection and its process.
    pub async fn execute(
        &self,
        client: &reqwest::Client,
        request: reqwest::Request,
        connect_timeout: Duration,
        total_timeout: Option<Duration>,
        max_response_bytes: Option<u64>,
    ) -> Result<reqwest::Response, CrawlError> {
        if let Some(policy) = &self.sandbox {
            if self.sandbox_origin.as_ref() != Some(&request.url().origin()) {
                return Err(CrawlError::InvalidInput(
                    "HTTP destination changed after sandbox authorization".into(),
                ));
            }
            return agena_process::sandbox_http::execute(
                request,
                agena_process::sandbox_http::HttpOptions {
                    policy: policy.clone(),
                    pinned_addresses: self.addresses.clone(),
                    connect_timeout,
                    total_timeout,
                    // File payloads are capped separately at 512 MiB. Reserve
                    // a bounded extra MiB for multipart fields and framing.
                    max_request_bytes: 513 * 1024 * 1024,
                    max_response_bytes,
                    cancellation: self.cancellation.clone(),
                },
            )
            .await
            .map_err(CrawlError::Io);
        }
        client
            .execute(request)
            .await
            .map_err(|error| CrawlError::Http(error.without_url()))
    }

    /// Explicitly apply the previously selected route; never re-read proxy
    /// environment after the caller has authorized it.
    pub fn apply(&self, mut builder: reqwest::ClientBuilder, url: &Url) -> reqwest::ClientBuilder {
        builder = builder.no_proxy();
        if let Some(proxy) = &self.proxy {
            builder = builder.proxy(proxy.clone());
        }
        if !self.addresses.is_empty() {
            builder = builder.resolve_to_addrs(
                url.host_str().expect("authorized URL host"),
                &self.addresses,
            );
        }
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn sandbox_route_rejects_retargeting_before_any_connection() {
        let url = Url::parse("https://approved.example/resource").unwrap();
        let route = WebHttpRoute::default()
            .with_addresses(vec!["192.0.2.1:443".parse().unwrap()])
            .with_sandbox_policy(&url, agena_process::sandbox::SandboxPolicy::default())
            .unwrap();
        let client = reqwest::Client::new();
        let request = client
            .get("https://different.example/resource")
            .build()
            .unwrap();
        let error = route
            .execute(&client, request, Duration::from_secs(1), None, None)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("changed after sandbox authorization")
        );
        let disabled = route
            .with_sandbox_policy(
                &url,
                agena_process::sandbox::SandboxPolicy {
                    disabled: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!disabled.sandboxed());
    }

    #[test]
    fn sandbox_route_requires_pins_and_rejects_proxy_bypass() {
        let url = Url::parse("https://approved.example/resource").unwrap();
        assert!(
            WebHttpRoute::default()
                .with_sandbox_policy(&url, agena_process::sandbox::SandboxPolicy::default())
                .is_err()
        );
        let matcher = Matcher::builder().all("socks5://127.0.0.1:1080").build();
        let route = WebHttpRoute::from_matcher(&url, &matcher)
            .unwrap()
            .with_addresses(vec!["192.0.2.1:443".parse().unwrap()]);
        assert!(
            route
                .with_sandbox_policy(&url, agena_process::sandbox::SandboxPolicy::default())
                .unwrap_err()
                .to_string()
                .contains("cannot enforce")
        );
    }

    async fn headers(stream: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            let byte = stream.read_u8().await.unwrap();
            bytes.push(byte);
            assert!(bytes.len() < 8192);
        }
        String::from_utf8(bytes).unwrap()
    }

    async fn response(stream: &mut tokio::net::TcpStream) {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn http_proxy_auth_and_connect_do_not_resolve_the_destination_locally() {
        for https in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy = format!("http://user:secret@{}", listener.local_addr().unwrap());
            let url = Url::parse(if https {
                "https://does-not-resolve.invalid/page"
            } else {
                "http://does-not-resolve.invalid/page"
            })
            .unwrap();
            let matcher = Matcher::builder().all(proxy).build();
            let route = WebHttpRoute::from_matcher(&url, &matcher).unwrap();
            let client = route
                .apply(
                    reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)),
                    &url,
                )
                .build()
                .unwrap();
            let server = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = headers(&mut stream).await;
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("proxy-authorization: basic dxnlcjpzzwnyzxq=")
                );
                if https {
                    assert!(request.starts_with("CONNECT does-not-resolve.invalid:443 HTTP/1.1"));
                    stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                } else {
                    assert!(
                        request.starts_with("GET http://does-not-resolve.invalid/page HTTP/1.1")
                    );
                    response(&mut stream).await;
                }
            };
            let request = async {
                let result = client.get(url).send().await;
                if https {
                    assert!(result.is_err());
                } else {
                    assert_eq!(result.unwrap().text().await.unwrap(), "ok");
                }
            };
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                tokio::join!(server, request);
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn socks5_supports_authenticated_remote_dns_and_pinned_local_dns() {
        for remote in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let scheme = if remote { "socks5h" } else { "socks5" };
            let matcher = Matcher::builder()
                .all(format!(
                    "{scheme}://user:secret@{}",
                    listener.local_addr().unwrap()
                ))
                .build();
            let url = Url::parse("http://does-not-resolve.invalid/page").unwrap();
            let mut route = WebHttpRoute::from_matcher(&url, &matcher).unwrap();
            if !remote {
                route = route.with_addresses(vec!["1.1.1.1:80".parse().unwrap()]);
            }
            let client = route
                .apply(
                    reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)),
                    &url,
                )
                .build()
                .unwrap();
            let server = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                assert_eq!(stream.read_u8().await.unwrap(), 5);
                let count = stream.read_u8().await.unwrap() as usize;
                let mut methods = vec![0; count];
                stream.read_exact(&mut methods).await.unwrap();
                assert!(methods.contains(&2));
                stream.write_all(&[5, 2]).await.unwrap();
                assert_eq!(stream.read_u8().await.unwrap(), 1);
                let length = stream.read_u8().await.unwrap() as usize;
                let mut user = vec![0; length];
                stream.read_exact(&mut user).await.unwrap();
                let length = stream.read_u8().await.unwrap() as usize;
                let mut password = vec![0; length];
                stream.read_exact(&mut password).await.unwrap();
                assert_eq!(user, b"user");
                assert_eq!(password, b"secret");
                stream.write_all(&[1, 0]).await.unwrap();
                let mut connect = [0; 4];
                stream.read_exact(&mut connect).await.unwrap();
                assert_eq!(&connect[..3], &[5, 1, 0]);
                if remote {
                    assert_eq!(connect[3], 3);
                    let length = stream.read_u8().await.unwrap() as usize;
                    let mut host = vec![0; length];
                    stream.read_exact(&mut host).await.unwrap();
                    assert_eq!(host, b"does-not-resolve.invalid");
                } else {
                    assert_eq!(connect[3], 1);
                    let mut ip = [0; 4];
                    stream.read_exact(&mut ip).await.unwrap();
                    assert_eq!(ip, [1, 1, 1, 1]);
                }
                assert_eq!(stream.read_u16().await.unwrap(), 80);
                stream
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                    .await
                    .unwrap();
                assert!(headers(&mut stream).await.starts_with("GET /page HTTP/1.1"));
                response(&mut stream).await;
            };
            let request = async {
                assert_eq!(
                    client.get(url).send().await.unwrap().text().await.unwrap(),
                    "ok"
                );
            };
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                tokio::join!(server, request);
            })
            .await
            .unwrap();
        }
    }

    #[test]
    fn proxy_selection_and_no_proxy_use_the_same_matcher_as_reqwest() {
        let matcher = Matcher::builder()
            .http("http://user:secret@proxy.example:8080")
            .https("socks5h://user:secret@proxy.example:1080")
            .no(".internal.example,127.0.0.0/8,::1")
            .build();
        for url in ["http://search.example/", "https://search.example/"] {
            let route = WebHttpRoute::from_matcher(&Url::parse(url).unwrap(), &matcher).unwrap();
            assert!(route.proxy.is_some() && route.remote_dns());
            assert!(!format!("{route:?}").contains("secret"));
            assert!(!route.cache_key().contains("secret"));
        }
        for url in [
            "http://a.internal.example/",
            "http://127.0.0.9/",
            "http://[::1]/",
        ] {
            let route = WebHttpRoute::from_matcher(&Url::parse(url).unwrap(), &matcher).unwrap();
            assert!(route.proxy.is_none() && !route.remote_dns());
        }
        for scheme in ["socks4", "socks5"] {
            let matcher = Matcher::builder()
                .all(format!("{scheme}://proxy.example:1080"))
                .build();
            let route = WebHttpRoute::from_matcher(
                &Url::parse("https://search.example/").unwrap(),
                &matcher,
            )
            .unwrap();
            assert!(route.proxy.is_some() && !route.remote_dns());
        }
    }
}
