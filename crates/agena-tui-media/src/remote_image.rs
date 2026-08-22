use portable_atomic::AtomicU64;
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, LazyLock, Mutex, atomic::Ordering},
    time::{Duration, Instant},
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

async fn fetch(mut url: Url) -> Result<Vec<u8>, String> {
    for redirects in 0..=MAX_REMOTE_IMAGE_REDIRECTS {
        let target = resolve_public_target(&url).await?;
        let host = url
            .host_str()
            .ok_or_else(|| "remote image URL has no host".to_string())?;
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(REMOTE_IMAGE_CONNECT_TIMEOUT)
            .timeout(REMOTE_IMAGE_REQUEST_TIMEOUT)
            .user_agent("Agena-TUI/remote-image");
        if matches!(url.host(), Some(Host::Domain(_))) {
            builder = builder.resolve_to_addrs(host, &target);
        }
        let client = builder.build().map_err(|error| {
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
                    &error,
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
                &error,
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

async fn resolve_public_target(url: &Url) -> Result<Vec<SocketAddr>, String> {
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
    if addresses.is_empty() {
        return Err("remote image DNS lookup returned no addresses".to_string());
    }
    if let Some(address) = addresses
        .iter()
        .map(SocketAddr::ip)
        .find(|address| !is_permitted_remote_address(*address, domain_routed))
    {
        return Err(format!(
            "remote image host resolves to a non-public address ({address})"
        ));
    }
    Ok(addresses)
}

fn is_permitted_remote_address(address: IpAddr, domain_routed: bool) -> bool {
    is_public_address(address) || domain_routed && is_synthetic_proxy_address(address)
}

/// Clash, Surge, and similar transparent DNS proxies commonly reserve
/// 198.18.0.0/15 as a synthetic address pool. The original hostname still
/// supplies TLS SNI and the HTTP Host header, and reqwest remains pinned to the
/// resolver result. Permit this range only for a hostname resolution; a URL
/// that directly names a benchmark-range IP remains blocked.
fn is_synthetic_proxy_address(address: IpAddr) -> bool {
    let address = match address {
        IpAddr::V4(address) => Some(address),
        IpAddr::V6(address) => address.to_ipv4_mapped(),
    };
    address.is_some_and(|address| {
        let [first, second, _, _] = address.octets();
        first == 198 && matches!(second, 18 | 19)
    })
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map_or_else(|| is_public_ipv6(address), is_public_ipv4),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    !address.is_private()
        && !address.is_loopback()
        && !address.is_link_local()
        && !address.is_broadcast()
        && !address.is_documentation()
        && !address.is_multicast()
        && !address.is_unspecified()
        && first != 0
        && !(first == 100 && (64..=127).contains(&second))
        && !(first == 192 && second == 0 && third == 0)
        && !(first == 198 && matches!(second, 18 | 19))
        && first < 240
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    !address.is_loopback()
        && !address.is_unspecified()
        && !address.is_unique_local()
        && !address.is_unicast_link_local()
        && !address.is_multicast()
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && !(segments[0] == 0x2001 && segments[1] == 0x0002)
        && (segments[0] & 0xe000) == 0x2000
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn seed(url: &str, bytes: Vec<u8>) {
    let url = Url::parse(url).expect("test remote image URL");
    let url = canonical_remote_image_url(&url).expect("test remote image URL is canonical");
    let key = url.as_str().to_string();
    let mut cache = REMOTE_IMAGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.remove(&key);
    assert!(cache.begin(key.clone()));
    cache.complete(&key, Ok(bytes));
    REMOTE_IMAGE_GENERATION.fetch_add(1, Ordering::AcqRel);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_url_policy_rejects_credentials_and_non_http_schemes() {
        assert!(canonical_remote_image_url(&Url::parse("file:///tmp/a.png").unwrap()).is_err());
        assert!(
            canonical_remote_image_url(
                &Url::parse("https://user:secret@example.com/a.png").unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn address_policy_rejects_ssrf_ranges_and_accepts_public_endpoints() {
        for address in [
            "127.0.0.1",
            "0.1.2.3",
            "10.0.0.1",
            "169.254.1.1",
            "192.0.2.1",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public_address(address.parse().unwrap()), "{address}");
        }
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(is_public_address(address.parse().unwrap()), "{address}");
        }

        for address in ["198.18.0.1", "198.19.255.254", "::ffff:198.18.0.1"] {
            let address = address.parse().unwrap();
            assert!(!is_public_address(address), "{address}");
            assert!(
                is_permitted_remote_address(address, true),
                "a hostname may use the system DNS proxy pool: {address}"
            );
            assert!(
                !is_permitted_remote_address(address, false),
                "a literal benchmark-range URL must remain blocked: {address}"
            );
        }
    }

    #[test]
    fn expired_failures_can_be_retried() {
        let key = "https://images.example.test/retry.png".to_string();
        let mut cache = RemoteImageCache::default();
        cache.entries.insert(
            key.clone(),
            RemoteImageState::Failed {
                error: Arc::from("temporary failure"),
                retry_after: Instant::now(),
            },
        );
        cache.touch(&key);

        assert!(cache.get(&key).is_none());
        assert!(cache.begin(key));
    }

    #[tokio::test]
    async fn loopback_remote_images_are_rejected_before_connecting() {
        let error = fetch(Url::parse("http://127.0.0.1/image.png").unwrap())
            .await
            .expect_err("loopback image must be rejected");
        assert!(error.contains("non-public"));
    }

    #[tokio::test]
    async fn asynchronous_failures_publish_a_cache_generation() {
        let url = Url::parse("http://127.0.0.1:9/generation-test.png").unwrap();
        let before = generation();
        assert!(
            load(&url)
                .expect_err("first access schedules a load")
                .contains("loading")
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match load(&url) {
                    Err(error) if error.contains("non-public") => break,
                    Err(error) if error.contains("loading") => {
                        tokio::task::yield_now().await;
                    }
                    other => panic!("unexpected remote image cache state: {other:?}"),
                }
            }
        })
        .await
        .expect("failed download should publish its completion");
        assert_ne!(generation(), before);
    }
}
