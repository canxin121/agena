//! In-memory fetch caching and per-host pacing for concrete web adapters.
//!
//! Permission checks, request construction, redirect policy, and the actual
//! fetch transport remain at the caller boundary. This module only owns the
//! reusable Web capability mechanics around an already-authorized fetch.

use std::{future::Future, num::NonZeroU32, time::Duration};

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
}

impl WebFetchCoordinator {
    pub fn new(config: WebFetchCoordinatorConfig) -> Self {
        Self {
            fetch_cache: Cache::builder()
                .time_to_live(config.cache_ttl)
                .max_capacity(config.cache_capacity)
                .build(),
            host_limiter: build_host_limiter(config.per_host_delay),
        }
    }

    /// Wait until another request to this URL's host may start.
    pub async fn wait_for_url_host(&self, url: &url::Url) {
        if let Some(host) = url.host_str() {
            self.host_limiter.until_key_ready(&host.to_string()).await;
        }
    }

    /// Return a cached page when enabled; otherwise pace the host, invoke the
    /// caller-owned fetch operation, and cache its successful page.
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
        self.wait_for_url_host(url).await;
        let page = fetch().await?;
        if use_cache {
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

    #[test]
    fn rendered_and_plain_fetches_have_distinct_cache_keys() {
        let url = url::Url::parse("https://example.test/docs").expect("URL");
        assert_ne!(fetch_cache_key(&url, false), fetch_cache_key(&url, true));
    }
}
