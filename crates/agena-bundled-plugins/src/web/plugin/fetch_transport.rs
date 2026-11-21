//! Bounded ordinary HTTP retrieval. Every redirect and robots request is
//! authorized before connecting, using the selected proxy or checked DNS addresses.
use super::*;
use agena_web::{WebHttpClientPool, WebHttpRoute};
use moka::{
    future::Cache,
    ops::compute::{CompResult, Op},
};
use tokio::sync::SemaphorePermit;

const ROBOTS_BYTES: usize = 512 * 1024;
const REDIRECTS: usize = 10;

#[derive(Hash, PartialEq, Eq)]
struct RobotsKey {
    url: url::Url,
    route: String,
    user_agent: String,
}

pub(super) struct HttpTransport {
    clients: WebHttpClientPool,
    robots: Cache<RobotsKey, Option<Arc<str>>>,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self {
            clients: WebHttpClientPool::default(),
            robots: Cache::builder()
                .time_to_live(Duration::from_secs(5 * 60))
                .max_capacity(8 * 1024 * 1024)
                .weigher(|key: &RobotsKey, rules: &Option<Arc<str>>| {
                    let size = key.url.as_str().len()
                        + key.route.len()
                        + key.user_agent.len()
                        + rules.as_deref().map_or(0, str::len);
                    size.clamp(32 * 1024, u32::MAX as usize) as u32
                })
                .build(),
        }
    }
}

#[derive(Debug)]
pub(super) struct Source {
    pub final_url: url::Url,
    pub status: u16,
    pub content_type: String,
    pub body: String,
    pub truncated: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub decoding_errors: bool,
}

async fn checked_route<F, Fut>(url: &url::Url, resolve: &F) -> SdkResult<WebHttpRoute>
where
    F: Fn(url::Url) -> Fut,
    Fut: Future<Output = SdkResult<WebHttpRoute>>,
{
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(PluginError::invalid_params(
            "fetch URLs must use HTTP(S), without URL credentials",
        ));
    }
    resolve(url.clone()).await
}

impl HttpTransport {
    async fn send(
        &self,
        url: &url::Url,
        options: &WebFetchOptions,
        pacing: &WebFetchCoordinator,
        route: &WebHttpRoute,
    ) -> SdkResult<(reqwest::Response, SemaphorePermit<'static>)> {
        let client = self
            .clients
            .client(url, route, options.timeout)
            .map_err(crawl_error_to_plugin)?;
        let request = client
            .get(url.clone())
            .header(reqwest::header::USER_AGENT, &options.user_agent)
            .header(
                "Accept",
                "text/html, text/plain, application/xhtml+xml, application/json;q=0.9, */*;q=0.1",
            )
            .build()
            .map_err(|error| PluginError::internal_error(&error.without_url()))?;
        let slot = pacing.admit_http(url).await;
        let response = route
            .execute(
                &client,
                request,
                options.timeout.min(Duration::from_secs(30)),
                Some(options.timeout),
                None,
            )
            .await
            .map_err(crawl_error_to_plugin)?;
        Ok((response, slot))
    }
}

fn redirect(response: &reqwest::Response, current_url: &url::Url) -> SdkResult<Option<url::Url>> {
    if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| PluginError::internal("HTTP redirect has no valid Location header"))?;
    let mut next = current_url
        .join(location)
        .map_err(|error| PluginError::invalid_params_error(&error))?;
    next.set_fragment(None);
    Ok(Some(next))
}

async fn bounded_body(
    mut response: reqwest::Response,
    maximum: usize,
) -> SdkResult<(Vec<u8>, bool)> {
    let mut body = Vec::with_capacity(maximum.min(64 * 1024));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| PluginError::internal_error(&error.without_url()))?
    {
        let remaining = maximum.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            return Ok((body, true));
        }
    }
    Ok((body, false))
}

enum RobotsHop {
    Rules(Option<Arc<str>>),
    Redirect(url::Url),
}

async fn read_robots(response: reqwest::Response, url: &url::Url) -> SdkResult<RobotsHop> {
    if let Some(next) = redirect(&response, url)? {
        return Ok(RobotsHop::Redirect(next));
    }
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(PluginError::invalid_params(
            "robots.txt denies access to this origin",
        ));
    }
    if status.is_client_error() {
        return Ok(RobotsHop::Rules(None));
    }
    if !status.is_success() {
        return Err(PluginError::internal(format!(
            "robots.txt returned HTTP {}; fetch did not continue",
            status.as_u16()
        )));
    }
    let (bytes, truncated) = bounded_body(response, ROBOTS_BYTES).await?;
    if truncated {
        return Err(PluginError::internal(
            "robots.txt exceeds the 512 KiB parsing budget; fetch did not continue",
        ));
    }
    Ok(RobotsHop::Rules(Some(Arc::from(
        String::from_utf8_lossy(&bytes).as_ref(),
    ))))
}

impl HttpTransport {
    async fn robots_hop(
        &self,
        url: &url::Url,
        options: &WebFetchOptions,
        pacing: &WebFetchCoordinator,
        route: &WebHttpRoute,
    ) -> SdkResult<RobotsHop> {
        let key = RobotsKey {
            url: url.clone(),
            route: route.cache_key(),
            user_agent: options.user_agent.clone(),
        };
        if let Some(rules) = self.robots.get(&key).await {
            return Ok(RobotsHop::Rules(rules));
        }
        let mut uncached = None;
        let outcome_slot = &mut uncached;
        let result = self
            .robots
            .entry(key)
            .and_compute_with(|entry| {
                Box::pin(async move {
                    if entry.is_some() {
                        return Op::Nop;
                    }
                    let outcome = async {
                        let (response, _slot) = self.send(url, options, pacing, route).await?;
                        let status = response.status();
                        let rules = read_robots(response, url).await?;
                        Ok((
                            rules,
                            status.is_success() || matches!(status.as_u16(), 404 | 410),
                        ))
                    }
                    .await;
                    match outcome {
                        Ok((RobotsHop::Rules(rules), true)) => Op::Put(rules),
                        outcome => {
                            *outcome_slot = Some(outcome.map(|(hop, _)| hop));
                            Op::Nop
                        }
                    }
                })
            })
            .await;
        match result {
            CompResult::Inserted(entry) | CompResult::Unchanged(entry) => {
                Ok(RobotsHop::Rules(entry.into_value()))
            }
            CompResult::StillNone(_) => uncached.expect("uncached robots outcome"),
            _ => unreachable!("robots cache entries are only inserted"),
        }
    }

    pub(super) async fn robots<F, Fut>(
        &self,
        target: &url::Url,
        options: &WebFetchOptions,
        pacing: &WebFetchCoordinator,
        resolve: &F,
    ) -> SdkResult<Option<Arc<str>>>
    where
        F: Fn(url::Url) -> Fut,
        Fut: Future<Output = SdkResult<agena_web::WebHttpRoute>>,
    {
        let mut url = target
            .join("/robots.txt")
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        url.set_query(None);
        let mut visited = BTreeSet::new();
        for _ in 0..=REDIRECTS {
            if !visited.insert(url.to_string()) {
                break;
            }
            // Resolve and authorize even on a cache hit. Redirects are followed
            // only within this invocation, after authorizing their own URL and IPs.
            let route = checked_route(&url, resolve).await?;
            match self.robots_hop(&url, options, pacing, &route).await? {
                RobotsHop::Rules(rules) => return Ok(rules),
                RobotsHop::Redirect(next) => url = next,
            }
        }
        Err(PluginError::internal(
            "robots.txt redirect loop or redirect budget exceeded",
        ))
    }

    pub(super) async fn fetch<F, Fut>(
        &self,
        initial: &url::Url,
        options: &WebFetchOptions,
        pacing: &WebFetchCoordinator,
        resolve: F,
    ) -> SdkResult<Source>
    where
        F: Fn(url::Url) -> Fut,
        Fut: Future<Output = SdkResult<agena_web::WebHttpRoute>>,
    {
        let operation = async {
            let mut url = initial.clone();
            let mut visited = BTreeSet::new();
            let mut policies = BTreeMap::new();
            for _ in 0..=REDIRECTS {
                if !visited.insert(url.to_string()) {
                    break;
                }
                if options.respect_robots_txt {
                    let origin = url.origin().ascii_serialization();
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        policies.entry(origin.clone())
                    {
                        entry.insert(self.robots(&url, options, pacing, &resolve).await?);
                    }
                    if let Some(Some(parser)) = policies.get(&origin)
                        && !agena_web::robots_allows(parser, &options.user_agent, url.as_str())
                    {
                        return Err(PluginError::invalid_params(
                            "robots.txt disallows the requested page",
                        ));
                    }
                }
                let route = checked_route(&url, &resolve).await?;
                let (response, _slot) = self.send(&url, options, pacing, &route).await?;
                if let Some(next) = redirect(&response, &url)? {
                    url = next;
                    continue;
                }
                let status = response.status().as_u16();
                let header = |name| {
                    response
                        .headers()
                        .get(name)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned)
                };
                let content_type = header(reqwest::header::CONTENT_TYPE).unwrap_or_default();
                let etag = header(reqwest::header::ETAG);
                let last_modified = header(reqwest::header::LAST_MODIFIED);
                let (bytes, truncated) = bounded_body(response, options.max_body_bytes).await?;
                let (body, decoding_errors) =
                    agena_web::decode_response_body(&bytes, &content_type);
                if body.contains('\0') || bytes.starts_with(b"%PDF-") {
                    return Err(PluginError::invalid_params(
                        "page body is binary; download it explicitly and use an appropriate document reader",
                    ));
                }
                return Ok(Source {
                    final_url: url,
                    status,
                    content_type,
                    body,
                    truncated,
                    etag,
                    last_modified,
                    decoding_errors,
                });
            }
            Err(PluginError::internal(
                "fetch redirect loop or ten-redirect budget exceeded",
            ))
        };
        agena_plugin_host::sdk::permission_wait::timeout(options.timeout, operation)
            .await
            .map_err(|_| {
                PluginError::internal(
                    "fetch deadline exceeded, including robots, DNS, redirects and body streaming",
                )
            })?
    }
}

pub(super) async fn extract(
    source: Source,
    requested: url::Url,
    backend: agena_web::ExtractionBackend,
) -> SdkResult<FetchedPage> {
    let permit = crate::BLOCKING_PLUGIN_WORKERS
        .acquire()
        .await
        .map_err(|error| PluginError::internal_error(&error))?;
    let decoding_errors = source.decoding_errors;
    let (mut page, body) = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let page = agena_web::extract_page_from_body(
            &requested,
            &source.final_url,
            &source.content_type,
            source.status,
            source.truncated,
            false,
            &source.body,
            source.etag,
            source.last_modified,
        );
        (page, source.body)
    })
    .await
    .map_err(|error| PluginError::internal_error(&error))?;
    if decoding_errors {
        page.warnings.push(
            "Response decoding replaced invalid byte sequences (possibly at a truncated boundary)."
                .into(),
        );
    }
    agena_web::extract_with_backend(page, &body, backend)
        .await
        .map_err(crawl_error_to_plugin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn robots_and_keep_alive_are_reused_without_reusing_authorization() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = url::Url::parse(&format!("http://fixture.invalid:{}/", address.port())).unwrap();
        let transport = HttpTransport::default();
        let pacing = WebFetchCoordinator::new(WebFetchCoordinatorConfig {
            cache_ttl: Duration::ZERO,
            cache_capacity: 0,
            per_host_delay: Duration::from_millis(10),
        });
        let checks = std::sync::Mutex::new(Vec::new());
        let deny_robots = std::sync::atomic::AtomicBool::new(false);
        let resolve = |target: url::Url| {
            checks.lock().unwrap().push(target.path().to_owned());
            std::future::ready(
                if target.path() == "/robots.txt"
                    && deny_robots.load(std::sync::atomic::Ordering::Relaxed)
                {
                    Err(PluginError::from_kind(
                        agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                        "robots route denied",
                    ))
                } else {
                    Ok(WebHttpRoute::default().with_addresses(vec![address]))
                },
            )
        };
        let server = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            for (path, body) in [
                ("/robots.txt", "User-agent: *\nDisallow: /private\n"),
                ("/first", "first"),
                ("/second", "second"),
            ] {
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(stream.read_u8().await.unwrap());
                    assert!(headers.len() < 8192);
                }
                assert!(
                    String::from_utf8(headers)
                        .unwrap()
                        .starts_with(&format!("GET {path} "))
                );
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        };
        let calls = async {
            let options = WebFetchOptions::default();
            for path in ["first", "second"] {
                let page = transport
                    .fetch(&url.join(path).unwrap(), &options, &pacing, &resolve)
                    .await
                    .unwrap();
                assert_eq!(page.body, path);
            }
            let blocked = transport
                .fetch(&url.join("private").unwrap(), &options, &pacing, &resolve)
                .await
                .unwrap_err();
            assert!(
                blocked
                    .diagnostic_message()
                    .contains("robots.txt disallows")
            );
            deny_robots.store(true, std::sync::atomic::Ordering::Relaxed);
            let error = transport
                .fetch(&url.join("third").unwrap(), &options, &pacing, &resolve)
                .await
                .unwrap_err();
            assert_eq!(error.kind, agena_plugin_sdk::PluginErrorKind::PolicyDenied);
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(server, calls);
        })
        .await
        .unwrap();
        assert_eq!(
            *checks.lock().unwrap(),
            [
                "/robots.txt",
                "/first",
                "/robots.txt",
                "/second",
                "/robots.txt",
                "/robots.txt"
            ]
        );
    }

    #[tokio::test]
    async fn fetch_and_robots_reauthorize_redirects_before_the_next_connection() {
        for check_robots in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = listener.local_addr().unwrap();
            let denied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            denied.set_nonblocking(true).unwrap();
            let url = url::Url::parse(&format!(
                "http://approved.invalid:{}/start",
                endpoint.port()
            ))
            .unwrap();
            let next = format!(
                "http://denied.invalid:{}/private",
                denied.local_addr().unwrap().port()
            );
            let next_for_server = next.clone();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(stream.read_u8().await.unwrap());
                    assert!(headers.len() < 8192);
                }
                stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {next_for_server}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                String::from_utf8(headers).unwrap()
            });
            let seen = std::sync::Mutex::new(Vec::new());
            let resolve = |target: url::Url| {
                seen.lock().unwrap().push(target.to_string());
                std::future::ready(if target.host_str() == Some("approved.invalid") {
                    Ok(agena_web::WebHttpRoute::default().with_addresses(vec![endpoint]))
                } else {
                    Err(PluginError::from_kind(
                        agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                        "redirect destination denied",
                    ))
                })
            };
            let options = WebFetchOptions {
                timeout: Duration::from_secs(3),
                user_agent: "Agena-Worker-Header-Fixture".into(),
                respect_robots_txt: false,
                ..Default::default()
            };
            let transport = HttpTransport::default();
            let pacing = WebFetchCoordinator::new(WebFetchCoordinatorConfig {
                cache_ttl: Duration::ZERO,
                cache_capacity: 0,
                per_host_delay: Duration::from_millis(1),
            });
            let error = if check_robots {
                transport
                    .robots(&url, &options, &pacing, &resolve)
                    .await
                    .unwrap_err()
            } else {
                transport
                    .fetch(&url, &options, &pacing, resolve)
                    .await
                    .unwrap_err()
            };
            assert_eq!(error.kind, agena_plugin_sdk::PluginErrorKind::PolicyDenied);
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1], next);
            drop(requests);
            assert_eq!(
                denied.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
            let headers = tokio::time::timeout(Duration::from_secs(3), server)
                .await
                .unwrap()
                .unwrap()
                .to_ascii_lowercase();
            assert!(headers.contains("user-agent: agena-worker-header-fixture\r\n"));
            assert!(headers.contains("accept: text/html"));
        }
    }
}
