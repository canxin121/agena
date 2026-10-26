use portable_atomic::AtomicU64;
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    net::{IpAddr, SocketAddr},
    sync::{Arc, LazyLock, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

use agena_web::{
    WebHttpRoute,
    network::{is_fake_ip, is_public_hostname, is_public_ip},
};
use reqwest::{StatusCode, header};
use tokio::{runtime::Handle, sync::Semaphore};
use url::{Host, Url};

use super::MAX_MARKDOWN_IMAGE_BYTES;

const MAX_REMOTE_IMAGE_URL_BYTES: usize = 8 * 1024;
const MAX_REMOTE_IMAGE_REDIRECTS: usize = 5;
const MAX_REMOTE_IMAGE_ENTRIES: usize = 128;
const MAX_REMOTE_IMAGE_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_REMOTE_IMAGE_DOWNLOADS: usize = 4;
const REMOTE_IMAGE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_IMAGE_REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const REMOTE_IMAGE_DNS_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_IMAGE_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Clone)]
enum RemoteImageState {
    Loading,
    Ready(Arc<Vec<u8>>),
    Failed {
        error: Arc<str>,
        retry_after: Instant,
    },
}

#[derive(Default)]
struct RemoteImageCache {
    entries: HashMap<String, RemoteImageState>,
    recency: VecDeque<String>,
    bytes: usize,
}

impl RemoteImageCache {
    fn get(&mut self, key: &str) -> Option<Result<Arc<Vec<u8>>, String>> {
        let result = match self.entries.get(key)?.clone() {
            RemoteImageState::Loading => Err("remote image is loading".to_string()),
            RemoteImageState::Ready(bytes) => Ok(bytes),
            RemoteImageState::Failed { retry_after, .. } if Instant::now() >= retry_after => {
                self.remove(key);
                return None;
            }
            RemoteImageState::Failed { error, .. } => Err(error.to_string()),
        };
        self.touch(key);
        Some(result)
    }

    fn begin(&mut self, key: String) -> bool {
        self.evict_until_entry_available();
        if self.entries.len() >= MAX_REMOTE_IMAGE_ENTRIES {
            return false;
        }
        self.entries.insert(key.clone(), RemoteImageState::Loading);
        self.touch(&key);
        true
    }

    fn complete(&mut self, key: &str, result: Result<Vec<u8>, String>) {
        if !matches!(self.entries.get(key), Some(RemoteImageState::Loading)) {
            return;
        }
        let state = match result {
            Ok(bytes) => {
                self.bytes = self.bytes.saturating_add(bytes.len());
                RemoteImageState::Ready(Arc::new(bytes))
            }
            Err(error) => RemoteImageState::Failed {
                error: Arc::from(error),
                retry_after: Instant::now() + REMOTE_IMAGE_FAILURE_RETRY_DELAY,
            },
        };
        self.entries.insert(key.to_string(), state);
        self.touch(key);
        self.evict_to_byte_budget();
    }

    fn touch(&mut self, key: &str) {
        self.recency.retain(|candidate| candidate != key);
        self.recency.push_back(key.to_string());
    }

    fn evict_until_entry_available(&mut self) {
        let candidates = self.recency.len();
        for _ in 0..candidates {
            if self.entries.len() < MAX_REMOTE_IMAGE_ENTRIES {
                break;
            }
            let Some(key) = self.recency.pop_front() else {
                break;
            };
            if matches!(self.entries.get(&key), Some(RemoteImageState::Loading)) {
                self.recency.push_back(key);
            } else {
                self.remove(&key);
            }
        }
    }

    fn evict_to_byte_budget(&mut self) {
        let candidates = self.recency.len();
        for _ in 0..candidates {
            if self.bytes <= MAX_REMOTE_IMAGE_CACHE_BYTES {
                break;
            }
            let Some(key) = self.recency.pop_front() else {
                break;
            };
            if matches!(self.entries.get(&key), Some(RemoteImageState::Loading)) {
                self.recency.push_back(key);
            } else {
                self.remove(&key);
            }
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some(RemoteImageState::Ready(bytes)) = self.entries.remove(key) {
            self.bytes = self.bytes.saturating_sub(bytes.len());
        }
        self.recency.retain(|candidate| candidate != key);
    }
}

static REMOTE_IMAGES: LazyLock<Mutex<RemoteImageCache>> =
    LazyLock::new(|| Mutex::new(RemoteImageCache::default()));
static REMOTE_IMAGE_GENERATION: AtomicU64 = AtomicU64::new(1);
static REMOTE_IMAGE_DOWNLOADS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_REMOTE_IMAGE_DOWNLOADS)));

struct RemoteImageLoadGuard {
    key: String,
    completed: bool,
}

impl RemoteImageLoadGuard {
    fn complete(mut self, result: Result<Vec<u8>, String>) {
        if let Err(error) = &result {
            let source = Url::parse(&self.key).ok();
            tracing::warn!(
                remote_host = source.as_ref().and_then(Url::host_str).unwrap_or_default(),
                diagnostic = error.as_str(),
                "remote Markdown image download failed"
            );
        }
        REMOTE_IMAGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .complete(&self.key, result);
        self.completed = true;
        REMOTE_IMAGE_GENERATION.fetch_add(1, Ordering::AcqRel);
    }
}

impl Drop for RemoteImageLoadGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        REMOTE_IMAGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .complete(
                &self.key,
                Err("remote image download was cancelled".to_string()),
            );
        REMOTE_IMAGE_GENERATION.fetch_add(1, Ordering::AcqRel);
    }
}

pub(super) fn generation() -> u64 {
    REMOTE_IMAGE_GENERATION.load(Ordering::Acquire)
}

pub(super) fn cached(url: &Url) -> Result<Arc<Vec<u8>>, String> {
    let url = canonical_remote_image_url(url)?;
    REMOTE_IMAGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(url.as_str())
        .unwrap_or_else(|| Err("remote image is outside the visible viewport".into()))
}

pub(super) fn load(url: &Url) -> Result<Arc<Vec<u8>>, String> {
    let url = canonical_remote_image_url(url)?;
    let key = url.as_str().to_string();
    let mut cache = REMOTE_IMAGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(result) = cache.get(&key) {
        return result;
    }

    let handle = Handle::try_current().map_err(|error| {
        agena_failure::diagnostic::format_error_chain_with_context(
            "remote image loading requires the TUI runtime",
            &error,
        )
    })?;
    if !cache.begin(key.clone()) {
        return Err("remote image cache is busy".to_string());
    }
    drop(cache);

    handle.spawn(async move {
        let guard = RemoteImageLoadGuard {
            key,
            completed: false,
        };
        let result = match Arc::clone(&REMOTE_IMAGE_DOWNLOADS).acquire_owned().await {
            Ok(_permit) => fetch(url).await,
            Err(error) => Err(agena_failure::diagnostic::format_error_chain_with_context(
                "remote image downloader is unavailable because its concurrency limiter was closed",
                &error,
            )),
        };
        guard.complete(result);
    });

    Err("remote image is loading".to_string())
}

fn canonical_remote_image_url(url: &Url) -> Result<Url, String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("remote images require HTTP or HTTPS".to_string());
    }
    if url.as_str().len() > MAX_REMOTE_IMAGE_URL_BYTES {
        return Err("remote image URL is too long".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("remote image URLs cannot contain credentials".to_string());
    }
    if url.host().is_none() {
        return Err("remote image URL has no host".to_string());
    }
    let mut normalized = url.clone();
    normalized.set_fragment(None);
    Ok(normalized)
}

async fn fetch(url: Url) -> Result<Vec<u8>, String> {
    fetch_with_proxy(url, None).await
}

async fn fetch_with_proxy(mut url: Url, proxy: Option<&str>) -> Result<Vec<u8>, String> {
    for redirects in 0..=MAX_REMOTE_IMAGE_REDIRECTS {
        validate_remote_destination(&url)?;
        let mut route = WebHttpRoute::for_url(&url, proxy).map_err(|error| error.to_string())?;
        if !route.remote_dns() {
            route = route.with_addresses(resolve_public_target(&url).await?);
        }
        let builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(REMOTE_IMAGE_CONNECT_TIMEOUT)
            .timeout(REMOTE_IMAGE_REQUEST_TIMEOUT)
            .user_agent("Agena-TUI/remote-image");
        let client = route.apply(builder, &url).build().map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "cannot configure remote image request",
                &error,
            )
        })?;
        let mut response = client
            .get(url.clone())
            .header(
                header::ACCEPT,
                "image/png,image/jpeg,image/gif,image/webp,image/bmp,image/svg+xml,*/*;q=0.1",
            )
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|error| {
                agena_failure::diagnostic::format_error_chain_with_context(
                    "remote image request failed",
                    &error.without_url(),
                )
            })?;

        if response.status().is_redirection() {
            if redirects == MAX_REMOTE_IMAGE_REDIRECTS {
                return Err("remote image has too many redirects".to_string());
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .ok_or_else(|| "remote image redirect has no location".to_string())?
                .to_str()
                .map_err(|error| {
                    agena_failure::diagnostic::format_error_chain_with_context(
                        "remote image redirect location is invalid",
                        &error,
                    )
                })?;
            let next = canonical_remote_image_url(&url.join(location).map_err(|error| {
                agena_failure::diagnostic::format_error_chain_with_context(
                    "remote image redirect URL is invalid",
                    &error,
                )
            })?)?;
            if url.scheme() == "https" && next.scheme() != "https" {
                return Err("remote image redirect cannot downgrade HTTPS".to_string());
            }
            url = next;
            continue;
        }

        if response.status() != StatusCode::OK {
            return Err(format!(
                "remote image returned HTTP {}",
                response.status().as_u16()
            ));
        }
        validate_response_headers(&response)?;

        let capacity = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(MAX_MARKDOWN_IMAGE_BYTES);
        let mut bytes = Vec::with_capacity(capacity);
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "cannot read remote image",
                &error.without_url(),
            )
        })? {
            if bytes.len().saturating_add(chunk.len()) > MAX_MARKDOWN_IMAGE_BYTES {
                return Err("remote image exceeds the encoded byte safety limit".to_string());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Err("remote image response is empty".to_string());
        }
        return Ok(bytes);
    }

    unreachable!("redirect loop always returns or continues within its fixed bound")
}

fn validate_response_headers(response: &reqwest::Response) -> Result<(), String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MARKDOWN_IMAGE_BYTES as u64)
    {
        return Err("remote image exceeds the encoded byte safety limit".to_string());
    }
    let Some(content_type) = response.headers().get(header::CONTENT_TYPE) else {
        return Ok(());
    };
    let content_type = content_type
        .to_str()
        .map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "remote image content type is invalid",
                &error,
            )
        })?
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if content_type.starts_with("image/") || content_type == "application/octet-stream" {
        Ok(())
    } else {
        Err(format!(
            "remote image returned unsupported content type {content_type}"
        ))
    }
}

fn validate_remote_destination(url: &Url) -> Result<(), String> {
    let public = match url.host() {
        Some(Host::Domain(host)) => is_public_hostname(host),
        Some(Host::Ipv4(address)) => is_public_ip(address.into()),
        Some(Host::Ipv6(address)) => is_public_ip(address.into()),
        None => false,
    };
    if !public {
        return Err(
            "remote images require a public destination hostname or IP address".to_string(),
        );
    }
    Ok(())
}

async fn resolve_public_target(url: &Url) -> Result<Vec<SocketAddr>, String> {
    validate_remote_destination(url)?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "remote image URL has no known port".to_string())?;
    let (addresses, domain_routed) = match url
        .host()
        .ok_or_else(|| "remote image URL has no host".to_string())?
    {
        Host::Ipv4(address) => (vec![SocketAddr::new(IpAddr::V4(address), port)], false),
        Host::Ipv6(address) => (vec![SocketAddr::new(IpAddr::V6(address), port)], false),
        Host::Domain(host) => {
            let resolved = tokio::time::timeout(
                REMOTE_IMAGE_DNS_TIMEOUT,
                tokio::net::lookup_host((host, port)),
            )
            .await
            .map_err(|error| {
                tracing::warn!(
                    remote_host = host,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "wait for remote image DNS resolution",
                        &error,
                    ),
                    "remote image DNS lookup timed out"
                );
                format!("remote image DNS lookup timed out for {host}")
            })?
            .map_err(|error| format!("cannot resolve remote image host {host}: {error}"))?;
            (
                resolved.collect::<BTreeSet<_>>().into_iter().collect(),
                true,
            )
        }
    };
    permitted_remote_targets(addresses, domain_routed)
}

fn permitted_remote_targets(
    addresses: Vec<SocketAddr>,
    domain_routed: bool,
) -> Result<Vec<SocketAddr>, String> {
    if addresses.is_empty() {
        return Err("remote image DNS lookup returned no addresses".to_string());
    }
    // The client is pinned to this list. An unusable IPv6 answer must not
    // suppress a valid IPv4 route (including the existing TUN Fake-IP route),
    // but it must never reach the connector or trigger a fresh DNS lookup.
    let permitted = addresses
        .iter()
        .copied()
        .filter(|address| is_permitted_remote_address(address.ip(), domain_routed))
        .collect::<Vec<_>>();
    if permitted.is_empty() {
        return Err(format!(
            "remote image host resolves to a non-public address ({})",
            addresses[0].ip()
        ));
    }
    Ok(permitted)
}

fn is_permitted_remote_address(address: IpAddr, domain_routed: bool) -> bool {
    is_public_ip(address) || domain_routed && is_fake_ip(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addresses(values: &[&str]) -> Vec<SocketAddr> {
        values.iter().map(|value| value.parse().unwrap()).collect()
    }

    #[test]
    fn dual_stack_fake_ip_dns_keeps_the_permitted_pinned_route() {
        let v4 = addresses(&["198.18.7.28:443"]);
        let dual = addresses(&["198.18.7.28:443", "[fdfe:dcba:9876::92]:443"]);
        assert_eq!(permitted_remote_targets(dual.clone(), true).unwrap(), dual);
        assert!(permitted_remote_targets(addresses(&["[fc00::92]:443"]), true).is_ok());
        assert!(permitted_remote_targets(v4, false).is_err());
    }

    #[test]
    fn private_answers_are_never_forwarded_to_the_connector() {
        let mixed = addresses(&[
            "127.0.0.1:443",
            "10.0.0.1:443",
            "169.254.169.254:443",
            "[::1]:443",
            "[fd00::1]:443",
            "[::ffff:192.168.1.1]:443",
            "1.1.1.1:443",
            "[2606:4700:4700::1111]:443",
        ]);
        assert_eq!(
            permitted_remote_targets(mixed, true).unwrap(),
            addresses(&["1.1.1.1:443", "[2606:4700:4700::1111]:443"])
        );
        assert!(permitted_remote_targets(Vec::new(), true).is_err());
        for address in ["127.0.0.1:443", "[fd00::92]:443"] {
            assert!(permitted_remote_targets(addresses(&[address]), true).is_err());
        }
    }

    #[tokio::test]
    async fn remote_images_use_proxy_dns_and_revalidate_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for location in [
            None,
            Some("http://127.0.0.1/private"),
            Some("http://[fc00::1]/private"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy = format!("http://{}", listener.local_addr().unwrap());
            let server = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                    assert!(request.len() < 8192);
                }
                assert!(String::from_utf8(request).unwrap().starts_with(
                    "GET http://remote-image.does-not-resolve.invalid/image HTTP/1.1"
                ));
                let response = match location {
                    Some(location) => format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    None => "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nConnection: close\r\n\r\nimage".into(),
                };
                stream.write_all(response.as_bytes()).await.unwrap();
            };
            let request = fetch_with_proxy(
                Url::parse("http://remote-image.does-not-resolve.invalid/image").unwrap(),
                Some(&proxy),
            );
            let (_, result) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(server, request)
            })
            .await
            .unwrap();
            if location.is_some() {
                assert!(result.unwrap_err().contains("public destination"));
            } else {
                assert_eq!(result.unwrap(), b"image");
            }
        }
    }
}
