//! In-memory fetch caching and per-host pacing for concrete web adapters.
//!
//! Permission checks, request construction, redirect policy, and the actual
//! fetch transport remain at the caller boundary. This module only owns the
//! reusable Web capability mechanics around an already-authorized fetch.

use std::{
    future::Future,
    num::NonZeroU32,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use governor::{DefaultKeyedRateLimiter, Quota};
use moka::future::Cache;

use crate::FetchedPage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Configuration of the web fetch coordinator.
pub struct WebFetchCoordinatorConfig {
    pub cache_ttl: Duration,
    pub cache_capacity: u64,
    pub per_host_delay: Duration,
}

/// Concrete in-memory web-fetch coordination owned by `agena-web`.
pub struct WebFetchCoordinator {
    fetch_cache: Cache<String, FetchedPage>,
    host_limiter: DefaultKeyedRateLimiter<String>,
    paced_requests: AtomicU64,
}

impl WebFetchCoordinator {
    pub fn new(config: WebFetchCoordinatorConfig) -> Self {
        const CACHE_BYTES: u64 = 64 * 1024 * 1024;
        let minimum_weight = CACHE_BYTES
            .div_ceil(config.cache_capacity.max(1))
            .min(u64::from(u32::MAX)) as u32;
        Self {
            fetch_cache: Cache::builder()
                .time_to_live(config.cache_ttl)
                .max_capacity(if config.cache_capacity == 0 {
                    0
                } else {
                    CACHE_BYTES
                })
                .weigher(move |key: &String, page: &FetchedPage| {
                    let bytes = key.len()
                        + page.markdown.len()
                        + page.title.len()
                        + page.url.len()
                        + page.final_url.len()
                        + page.canonical_url.len()
                        + page
                            .links
                            .iter()
                            .map(|link| link.len() + std::mem::size_of::<String>())
                            .sum::<usize>()
                        + page.warnings.iter().map(String::len).sum::<usize>()
                        + 512;
                    minimum_weight.max(bytes.min(u32::MAX as usize) as u32)
                })
                .build(),
            host_limiter: build_host_limiter(config.per_host_delay),
            paced_requests: AtomicU64::new(0),
        }
    }

    /// Wait until another request to this URL's host may start.
    pub async fn wait_for_url_host(&self, url: &url::Url) {
        if self
            .paced_requests
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(128)
        {
            self.host_limiter.retain_recent();
        }
        if let Some(host) = url.host_str() {
            self.host_limiter.until_key_ready(&host.to_string()).await;
        }
    }

    /// Return a cached page when enabled; otherwise invoke the caller-owned
    /// fetch operation and cache its successful page. Transports pace each
    /// actual request so robots and redirect requests share the same limit.
    pub async fn fetch_or_cached<E, Fetch, FetchFuture>(
        &self,
        url: &url::Url,
        render_js: bool,
        use_cache: bool,
        fetch: Fetch,
    ) -> Result<FetchedPage, E>
    where
        Fetch: FnOnce() -> FetchFuture,
        FetchFuture: Future<Output = Result<FetchedPage, E>>,
    {
        let cache_key = fetch_cache_key(url, render_js);
        if use_cache && let Some(hit) = self.fetch_cache.get(cache_key.as_str()).await {
            return Ok(hit);
        }
        let page = fetch().await?;
        if use_cache
            && (200..300).contains(&page.status)
            && !page.truncated
            && page.content_status == crate::PageContentStatus::Readable
        {
            self.fetch_cache.insert(cache_key, page.clone()).await;
        }
        Ok(page)
    }
}

fn build_host_limiter(delay: Duration) -> DefaultKeyedRateLimiter<String> {
    let quota = Quota::with_period(delay.max(Duration::from_millis(1)))
        .expect("web fetch delay must be non-zero")
        .allow_burst(NonZeroU32::new(1).expect("non-zero"));
    DefaultKeyedRateLimiter::keyed(quota)
}

fn fetch_cache_key(url: &url::Url, render_js: bool) -> String {
    format!(
        "spider:{}:{}",
        if render_js { "rendered" } else { "plain" },
        url
    )
}

#[cfg(test)]
mod tests {
    use super::fetch_cache_key;

    #[tokio::test]
    async fn http_failures_and_partial_bodies_do_not_poison_the_cache() {
        use super::*;
        let coordinator = WebFetchCoordinator::new(WebFetchCoordinatorConfig {
            cache_ttl: Duration::from_secs(30),
            cache_capacity: 2,
            per_host_delay: Duration::from_millis(1),
        });
        let url = url::Url::parse("https://fixture.invalid/").unwrap();
        for (status, truncated, body) in [
            (503, false, "server failure"),
            (200, true, "incomplete"),
            (200, false, "complete"),
        ] {
            let page = coordinator
                .fetch_or_cached(&url, false, true, || async {
                    Ok::<_, ()>(crate::extract_page_from_body(
                        &url,
                        &url,
                        "text/plain",
                        status,
                        truncated,
                        false,
                        body,
                        None,
                        None,
                    ))
                })
                .await
                .unwrap();
            assert_eq!(page.markdown, body);
        }
        let cached = coordinator
            .fetch_or_cached(&url, false, true, || async { Err(()) })
            .await
            .unwrap();
        assert_eq!(cached.markdown, "complete");
    }

    #[test]
    fn rendered_and_plain_fetches_have_distinct_cache_keys() {
        let url = url::Url::parse("https://example.test/docs").expect("URL");
        assert_ne!(fetch_cache_key(&url, false), fetch_cache_key(&url, true));
    }

    #[tokio::test]
    async fn successful_http_challenges_and_shells_are_not_cached() {
        use super::*;
        let coordinator = WebFetchCoordinator::new(WebFetchCoordinatorConfig {
            cache_ttl: Duration::from_secs(10),
            cache_capacity: 4,
            per_host_delay: Duration::from_millis(1),
        });
        let url = url::Url::parse("https://fixture.invalid/").unwrap();
        for status in [
            crate::PageContentStatus::Blocked,
            crate::PageContentStatus::RequiresJavascript,
            crate::PageContentStatus::Empty,
            crate::PageContentStatus::TooComplex,
        ] {
            coordinator
                .fetch_or_cached(&url, false, true, || async {
                    let mut page = crate::extract_page_from_body(
                        &url,
                        &url,
                        "text/plain",
                        200,
                        false,
                        false,
                        "placeholder",
                        None,
                        None,
                    );
                    page.content_status = status;
                    Ok::<_, ()>(page)
                })
                .await
                .unwrap();
            assert!(
                coordinator
                    .fetch_or_cached(&url, false, true, || async {
                        Err::<crate::FetchedPage, _>(())
                    })
                    .await
                    .is_err()
            );
        }
    }
}
