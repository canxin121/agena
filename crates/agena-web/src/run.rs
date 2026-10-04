use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    CrawlDocumentSummary, CrawlError, CrawlStore, CrawlStorePruneReport, CrawlStoreRetention,
    FetchedPage, StoredDocument, prepare_fetch_url,
};

#[derive(Debug, Clone)]
/// Options of a crawl run.
pub struct CrawlRunOptions {
    pub extraction_backend: crate::ExtractionBackend,
    pub max_pages: usize,
    pub max_depth: u32,
    pub same_host_only: bool,
    pub use_cache: bool,
    pub render_js: bool,
    pub document_cache_ttl: Duration,
    pub max_chunk_chars: usize,
    pub near_duplicate_hamming_distance: u32,
    pub store_retention: Option<CrawlStoreRetention>,
}

impl Default for CrawlRunOptions {
    fn default() -> Self {
        Self {
            extraction_backend: crate::ExtractionBackend::default(),
            max_pages: 10,
            max_depth: 1,
            same_host_only: true,
            use_cache: true,
            render_js: false,
            document_cache_ttl: Duration::from_secs(24 * 60 * 60),
            max_chunk_chars: 1800,
            near_duplicate_hamming_distance: 3,
            store_retention: Some(CrawlStoreRetention {
                max_documents: 200,
                max_total_bytes: 100 * 1024 * 1024,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Report of a crawl run.
pub struct CrawlRunReport {
    pub start_url: String,
    pub engine: String,
    pub rendered: bool,
    #[serde(default)]
    pub attempted_count: usize,
    #[serde(default)]
    pub discovered_count: usize,
    #[serde(default)]
    pub truncated: bool,
    pub stored_count: usize,
    pub cached_count: usize,
    pub duplicate_count: usize,
    pub near_duplicate_count: usize,
    pub pruned_document_count: usize,
    pub pruned_document_bytes: u64,
    pub failure_count: usize,
    pub total_documents: usize,
    pub documents: Vec<CrawlDocumentSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<agena_failure::UserProblem>,
}

/// Fetcher used by a crawl run.
pub trait CrawlPageFetcher {
    /// Revalidate access before exposing stored content, including its actual
    /// transport destination. This also runs on a document-cache hit.
    fn authorize_cached<'a>(
        &'a self,
        requested_url: &'a Url,
        document: &'a StoredDocument,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CrawlError>> + Send + 'a>>;

    fn fetch_page<'a>(
        &'a self,
        url: &'a Url,
        use_cache: bool,
        render_js: bool,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<FetchedPage, CrawlError>> + Send + 'a>,
    >;
}

pub async fn crawl_site(
    start_url: &Url,
    store: &CrawlStore,
    options: &CrawlRunOptions,
    fetcher: &impl CrawlPageFetcher,
) -> Result<CrawlRunReport, CrawlError> {
    if options.max_pages == 0 || options.max_pages > 1000 || options.max_depth > 16 {
        return Err(CrawlError::InvalidInput(
            "crawl requires 1–1000 pages and depth at most 16".into(),
        ));
    }
    let max_urls = options.max_pages.saturating_mul(32).clamp(128, 32_000);
    let mut truncated = false;
    let mut attempted_count = 0usize;
    let mut queue = VecDeque::from([(start_url.clone(), 0u32)]);
    let mut seen_urls = HashSet::from([start_url.to_string()]);
    let mut documents = Vec::new();
    let mut failures = Vec::new();
    let mut stored_count = 0usize;
    let mut cached_count = 0usize;
    let mut duplicate_count = 0usize;
    let mut near_duplicate_count = 0usize;
    let mut known_simhashes = store
        .list_documents()?
        .into_iter()
        .map(|document| document.simhash)
        .collect::<Vec<_>>();

    while let Some((url, depth)) = queue.pop_front() {
        if attempted_count + cached_count >= options.max_pages {
            truncated = true;
            break;
        }

        if options.use_cache
            && let Some(existing) = store.find_by_url(url.as_str())?
            && document_matches_render_mode(&existing, options.render_js)
            && existing.extraction_backend == options.extraction_backend
            && !existing.final_url.is_empty()
            && is_document_fresh(&existing, options.document_cache_ttl)
            && !existing.truncated
            && (200..300).contains(&existing.status)
        {
            if let Err(err) = fetcher.authorize_cached(&url, &existing).await {
                attempted_count += 1;
                tracing::warn!(diagnostic = %err, "cached crawl page authorization failed");
                failures.push(crawl_page_failure().into());
                continue;
            }
            if depth < options.max_depth {
                enqueue_document_links(
                    start_url,
                    &existing,
                    depth,
                    options.same_host_only,
                    &mut queue,
                    &mut seen_urls,
                    max_urls,
                    &mut truncated,
                );
            }
            cached_count += 1;
            documents.push(existing.summary());
            continue;
        }

        attempted_count += 1;
        match fetcher
            .fetch_page(&url, options.use_cache, options.render_js)
            .await
        {
            Ok(page) => {
                if !(200..300).contains(&page.status) {
                    let failure = crawl_page_failure();
                    tracing::warn!(
                        failure_id = %failure.id,
                        url = %url,
                        http_status = page.status,
                        "crawl page returned an unsuccessful HTTP status"
                    );
                    failures.push(failure.into());
                    continue;
                }
                truncated |= page.truncated;
                if page.truncated {
                    // A partial extraction must not become a reusable document.
                    failures.push(crawl_page_failure().into());
                    continue;
                }
                if depth < options.max_depth {
                    enqueue_links(
                        start_url,
                        &page,
                        depth,
                        options.same_host_only,
                        &mut queue,
                        &mut seen_urls,
                        max_urls,
                        &mut truncated,
                    );
                }
                let document =
                    StoredDocument::from_fetched_page(page.clone(), depth, options.max_chunk_chars);
                if store
                    .find_by_raw_hash(document.raw_html_hash.as_str())?
                    .is_some()
                {
                    duplicate_count += 1;
                    continue;
                }
                if store
                    .find_by_markdown_hash(document.markdown_hash.as_str())?
                    .is_some()
                {
                    duplicate_count += 1;
                    continue;
                }
                if is_near_duplicate(
                    document.simhash,
                    &known_simhashes,
                    options.near_duplicate_hamming_distance,
                ) {
                    near_duplicate_count += 1;
                    continue;
                }

                store.save_document(&document)?;
                known_simhashes.push(document.simhash);
                stored_count += 1;
                documents.push(document.summary());
            }
            Err(err) => {
                let failure = crawl_page_failure();
                tracing::warn!(
                    failure_id = %failure.id,
                    url = %url,
                    diagnostic = %err,
                    "crawl page fetch failed"
                );
                failures.push(failure.into());
            }
        }
    }

    let prune_report = match options.store_retention {
        Some(retention) => store.prune(retention)?,
        None => CrawlStorePruneReport::default(),
    };
    store.rebuild_index()?;
    let total_documents = store.list_documents()?.len();
    Ok(CrawlRunReport {
        start_url: start_url.to_string(),
        engine: if options.render_js { "spider" } else { "http" }.to_string(),
        rendered: options.render_js,
        attempted_count,
        discovered_count: seen_urls.len(),
        truncated,
        stored_count,
        cached_count,
        duplicate_count,
        near_duplicate_count,
        pruned_document_count: prune_report.removed_document_count,
        pruned_document_bytes: prune_report.removed_bytes,
        failure_count: failures.len(),
        total_documents,
        documents,
        failures,
    })
}

fn crawl_page_failure() -> agena_failure::Failure {
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };

    Failure::new(
        FailureCode::new("web.crawl_page_failed"),
        FailureCategory::DependencyUnavailable,
        FailureResponsibility::Dependency,
        RetryDirective::Backoff,
        RecoveryDirective::Retry,
        FailureImpact::PartialSuccess,
        UserPresentation::new(
            "web-crawl-page-failed",
            "A page could not be retrieved during the crawl.",
        ),
    )
}

pub fn ensure_index_exists(store: &CrawlStore) -> Result<(), CrawlError> {
    if !store.dir().join(".index").exists() {
        store.rebuild_index()?;
    }
    Ok(())
}

pub fn document_matches_render_mode(document: &StoredDocument, render_js: bool) -> bool {
    !render_js || document.rendered
}

fn is_document_fresh(document: &StoredDocument, ttl: Duration) -> bool {
    let Ok(ttl) = chrono::Duration::from_std(ttl) else {
        return false;
    };
    Utc::now() - document.fetched_at <= ttl
}

fn is_near_duplicate(candidate: u64, existing: &[u64], max_distance: u32) -> bool {
    existing
        .iter()
        .any(|value| simhash::hamming_distance(candidate, *value) <= max_distance)
}

#[allow(clippy::too_many_arguments)]
fn enqueue_links(
    start_url: &Url,
    page: &FetchedPage,
    current_depth: u32,
    same_host_only: bool,
    queue: &mut VecDeque<(Url, u32)>,
    seen_urls: &mut HashSet<String>,
    max_urls: usize,
    truncated: &mut bool,
) {
    for link in &page.links {
        enqueue_url(
            start_url,
            link,
            current_depth,
            same_host_only,
            queue,
            seen_urls,
            max_urls,
            truncated,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn enqueue_document_links(
    start_url: &Url,
    document: &StoredDocument,
    current_depth: u32,
    same_host_only: bool,
    queue: &mut VecDeque<(Url, u32)>,
    seen_urls: &mut HashSet<String>,
    max_urls: usize,
    truncated: &mut bool,
) {
    for link in &document.links {
        enqueue_url(
            start_url,
            link,
            current_depth,
            same_host_only,
            queue,
            seen_urls,
            max_urls,
            truncated,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn enqueue_url(
    start_url: &Url,
    raw: &str,
    current_depth: u32,
    same_host_only: bool,
    queue: &mut VecDeque<(Url, u32)>,
    seen_urls: &mut HashSet<String>,
    max_urls: usize,
    truncated: &mut bool,
) {
    let Ok(url) = prepare_fetch_url(raw) else {
        return;
    };
    if same_host_only && url.host_str() != start_url.host_str() {
        return;
    }
    if seen_urls.contains(url.as_str()) {
        return;
    }
    if seen_urls.len() >= max_urls {
        *truncated = true;
        return;
    }
    if seen_urls.insert(url.to_string()) {
        queue.push_back((url, current_depth + 1));
    }
}

#[cfg(test)]
mod tests;
