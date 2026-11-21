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

use governor::{DefaultKeyedRateLimiter, Quota, clock::Clock};
use moka::{
    future::Cache,
    ops::compute::{CompResult, Op},
};
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::FetchedPage;

static HTTP_REQUESTS: Semaphore = Semaphore::const_new(32);

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
        self.prune_host_limiter();
        if let Some(host) = url.host_str() {
            self.host_limiter.until_key_ready(&host.to_owned()).await;
        }
    }

    /// Admit a single HTTP hop. A paced host releases its global slot while it waits.
    pub async fn admit_http(&self, url: &url::Url) -> SemaphorePermit<'static> {
        self.prune_host_limiter();
        let host = url.host_str().expect("HTTP URL has a host").to_owned();
        loop {
            let slot = HTTP_REQUESTS
                .acquire()
                .await
                .expect("HTTP admission is never closed");
            match self.host_limiter.check_key(&host) {
                Ok(()) => return slot,
                Err(not_until) => {
                    drop(slot);
                    tokio::time::sleep(not_until.wait_time_from(self.host_limiter.clock().now()))
                        .await;
                }
            }
        }
    }

    fn prune_host_limiter(&self) {
        if self
            .paced_requests
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(128)
        {
            self.host_limiter.retain_recent();
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
        if !use_cache || self.fetch_cache.policy().max_capacity() == Some(0) {
            return fetch().await;
        }
        let cache_key = fetch_cache_key(url, render_js);
        if let Some(hit) = self.fetch_cache.get(cache_key.as_str()).await {
            return Ok(hit);
        }
        // Each caller keeps its own error/partial response and its own authority.
        let mut uncached = None;
        let outcome_slot = &mut uncached;
        let result = self
            .fetch_cache
            .entry(cache_key)
            .and_compute_with(|entry| {
                Box::pin(async move {
                    if entry.is_some() {
                        return Op::Nop;
                    }
                    match fetch().await {
                        Ok(page)
                            if (200..300).contains(&page.status)
                                && !page.truncated
                                && page.content_status == crate::PageContentStatus::Readable =>
                        {
                            Op::Put(page)
                        }
                        outcome => {
                            *outcome_slot = Some(outcome);
                            Op::Nop
                        }
                    }
                })
            })
            .await;
        match result {
            CompResult::Inserted(entry) | CompResult::Unchanged(entry) => Ok(entry.into_value()),
            CompResult::StillNone(_) => uncached.expect("uncached fetch outcome"),
            _ => unreachable!("fetch cache entries are only inserted"),
        }
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
        "fetch:{}:{}",
        if render_js { "rendered" } else { "plain" },
        url
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::future::join_all;

    #[tokio::test]
    async fn concurrent_fetches_reuse_one_success_and_keep_errors_with_their_caller() {
        let coordinator = WebFetchCoordinator::new(WebFetchCoordinatorConfig {
            cache_ttl: Duration::from_secs(30),
            cache_capacity: 64,
            per_host_delay: Duration::from_millis(1),
        });
        let url = url::Url::parse("https://example.org/fixture").unwrap();
        let started = tokio::sync::Notify::new();
        let attempts = AtomicU64::new(0);
        let fetch = || async {
            attempts.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok::<_, &'static str>(crate::extract_page_from_body(
                &url,
                &url,
                "text/plain",
                200,
                false,
                false,
                "fixture",
                None,
                None,
            ))
        };
        let rejected = coordinator.fetch_or_cached(&url, false, true, || async {
            started.notify_one();
            tokio::time::sleep(Duration::from_millis(20)).await;
            Err("call-specific permission refusal")
        });
        let followers = async {
            started.notified().await;
            join_all((0..16).map(|_| coordinator.fetch_or_cached(&url, false, true, fetch))).await
        };
        let (rejected, pages) = tokio::join!(rejected, followers);
        assert_eq!(rejected.unwrap_err(), "call-specific permission refusal");
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert!(
            pages
                .into_iter()
                .all(|page| page.unwrap().markdown == "fixture")
        );

        coordinator
            .fetch_or_cached(&url, false, false, fetch)
            .await
            .unwrap();
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
    }
}
