use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use chrono::Utc;
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, mpsc};
use url::Url;

use crate::{
    CrawlDocumentSummary, CrawlError, CrawlStore, CrawlStoreRetention, FetchedPage, StoredDocument,
    prepare_fetch_url,
};

#[derive(Debug, Clone)]
/// Options of a crawl run.
pub struct CrawlRunOptions {
    pub extraction_backend: crate::ExtractionBackend,
    pub max_pages: usize,
    pub concurrency: usize,
    pub max_depth: u32,
    pub same_host_only: bool,
    pub use_cache: bool,
    pub render_js: bool,
    /// Automatic mode may reuse either complete HTTP or rendered documents.
    pub allow_rendered_cache: bool,
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
            concurrency: 4,
            max_depth: 1,
            same_host_only: true,
            use_cache: true,
            render_js: false,
            allow_rendered_cache: false,
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
    pub concurrency: usize,
    pub page_errors: Vec<CrawlPageError>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A bounded, inspectable failure without transport diagnostics or credentials.
pub struct CrawlPageError {
    pub url: String,
    pub reason: String,
    pub status: Option<u16>,
    pub content_status: Option<crate::PageContentStatus>,
}

enum Retrieved {
    Cached(StoredDocument),
    Fetched(FetchedPage),
}

pub const MAX_CRAWL_CONCURRENCY: u32 = 32;

struct DocumentFingerprint {
    raw: String,
    markdown: String,
    simhash: u64,
}

impl From<&StoredDocument> for DocumentFingerprint {
    fn from(document: &StoredDocument) -> Self {
        Self {
            raw: document.raw_html_hash.clone(),
            markdown: document.markdown_hash.clone(),
            simhash: document.simhash,
        }
    }
}

async fn lock_crawl(path: &Path) -> OwnedMutexGuard<()> {
    type CrawlLocks = HashMap<PathBuf, Weak<AsyncMutex<()>>>;
    static RUNS: LazyLock<Mutex<CrawlLocks>> = LazyLock::new(Mutex::default);
    let lock = {
        let mut runs = RUNS.lock().expect("web crawl locks");
        runs.retain(|_, lock| lock.strong_count() > 0);
        let current = runs.entry(path.to_owned()).or_default();
        current.upgrade().unwrap_or_else(|| {
            let lock = Arc::new(AsyncMutex::new(()));
            *current = Arc::downgrade(&lock);
            lock
        })
    };
    lock.lock_owned().await
}

/// Fetcher used by a crawl run.
pub trait CrawlPageFetcher: Sync {
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

static CRAWL_CONTENT: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

type PageOutcome = (Url, Result<Retrieved, (&'static str, CrawlError)>);

async fn retrieve_page(
    url: &Url,
    cached_id: Option<String>,
    store: &CrawlStore,
    options: &CrawlRunOptions,
    fetcher: &impl CrawlPageFetcher,
) -> Result<Result<Retrieved, (&'static str, CrawlError)>, CrawlError> {
    if let Some(id) = cached_id {
        let existing = store
            .read_async(move |store| store.get_document(&id))
            .await?;
        if (document_matches_render_mode(&existing, options.render_js)
            || (options.allow_rendered_cache && existing.rendered))
            && existing.extraction_backend == options.extraction_backend
            && !existing.extraction_strategy.is_empty()
            && !existing.final_url.is_empty()
            && is_document_fresh(&existing, options.document_cache_ttl)
            && !existing.truncated
            && existing.content_status == crate::PageContentStatus::Readable
            && (200..300).contains(&existing.status)
        {
            return Ok(fetcher
                .authorize_cached(url, &existing)
                .await
                .map(|()| Retrieved::Cached(existing))
                .map_err(|error| ("authorization_denied", error)));
        }
    }
    Ok(fetcher
        .fetch_page(url, options.use_cache, options.render_js)
        .await
        .map(Retrieved::Fetched)
        .map_err(|error| ("fetch_failed", error)))
}

async fn retrieve_depth(
    pages: Vec<(Url, Option<String>)>,
    store: &CrawlStore,
    options: &CrawlRunOptions,
    fetcher: &impl CrawlPageFetcher,
    send: mpsc::Sender<PageOutcome>,
) -> Result<(), CrawlError> {
    let mut pages = pages.into_iter().enumerate();
    let mut active = FuturesUnordered::new();
    let mut ready = BTreeMap::new();
    let mut submitted = 0;
    let mut next = 0;
    loop {
        // Refill freed slots while limiting how many pages can wait behind a
        // slow one. The consumer still commits in BFS discovery order.
        while active.len() < options.concurrency && submitted < next + 2 * options.concurrency {
            let Some((index, (url, cached_id))) = pages.next() else {
                break;
            };
            active.push(async move {
                let result = retrieve_page(&url, cached_id, store, options, fetcher).await?;
                Ok::<_, CrawlError>((index, (url, result)))
            });
            submitted += 1;
        }
        let Some(outcome) = active.next().await else {
            break;
        };
        let (index, page) = outcome?;
        ready.insert(index, page);
        while let Some(page) = ready.remove(&next) {
            if send.send(page).await.is_err() {
                return Ok(());
            }
            next += 1;
        }
    }
    Ok(())
}

pub async fn crawl_site(
    start_url: &Url,
    store: &CrawlStore,
    options: &CrawlRunOptions,
    fetcher: &impl CrawlPageFetcher,
) -> Result<CrawlRunReport, CrawlError> {
    if options.max_pages == 0
        || options.max_pages > 1000
        || options.max_depth > 16
        || !(1..=MAX_CRAWL_CONCURRENCY as usize).contains(&options.concurrency)
    {
        return Err(CrawlError::InvalidInput(format!(
            "crawl requires 1–1000 pages, depth at most 16, and concurrency 1–{MAX_CRAWL_CONCURRENCY}"
        )));
    }
    let max_urls = options.max_pages.saturating_mul(32).clamp(128, 32_000);
    let mut truncated = false;
    let mut attempted_count = 0usize;
    let mut queue = VecDeque::from([(start_url.clone(), 0u32)]);
    let mut seen_urls = HashSet::from([start_url.to_string()]);
    let mut documents = Vec::new();
    let mut failures = Vec::new();
    let mut page_errors = Vec::new();
    let mut rendered_count = 0usize;
    let mut stored_count = 0usize;
    let mut cached_count = 0usize;
    let mut duplicate_count = 0usize;
    let mut near_duplicate_count = 0usize;
    store.read_async(CrawlStore::ensure_exists).await?;
    let path = tokio::fs::canonicalize(store.dir()).await?;
    let _crawl = lock_crawl(&path).await;
    let writer = agena_async::WriteQueue::named(path);
    // Join any write still finishing after a cancelled crawl before taking this snapshot.
    let (mut cached_urls, mut fingerprints) = store
        .mutate_async(&writer, |store| {
            let mut urls = HashMap::new();
            let mut fingerprints = HashMap::new();
            for document in store.list_documents()? {
                fingerprints.insert(document.id.clone(), DocumentFingerprint::from(&document));
                urls.entry(document.canonical_url).or_insert(document.id);
            }
            Ok((urls, fingerprints))
        })
        .await?;

    while !queue.is_empty() {
        let remaining = options
            .max_pages
            .saturating_sub(attempted_count + cached_count);
        if remaining == 0 {
            truncated = true;
            break;
        }
        // Failures and cache hits both consume the page budget. Finish a BFS
        // depth before starting the next so newly discovered links stay ordered.
        let depth = queue.front().expect("nonempty queue").1;
        let mut pages = Vec::new();
        while pages.len() < remaining && queue.front().is_some_and(|entry| entry.1 == depth) {
            let (url, _) = queue.pop_front().expect("checked front");
            let cached_id = if options.use_cache {
                cached_urls.get(url.as_str()).cloned()
            } else {
                None
            };
            pages.push((url, cached_id));
        }
        let (send, mut receive) = mpsc::channel(options.concurrency);
        // Drive both stages in the caller's task. Disk writes overlap network
        // reads, and both stop with the owning invocation.
        let retrieve = retrieve_depth(pages, store, options, fetcher, send);
        let process = async {
            while let Some((url, result)) = receive.recv().await {
                if let Ok(Retrieved::Cached(existing)) = result {
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
                    rendered_count += usize::from(existing.rendered);
                    cached_count += 1;
                    documents.push(existing.summary());
                    continue;
                }
                attempted_count += 1;
                match result {
                    Ok(Retrieved::Fetched(page)) => {
                        if !(200..300).contains(&page.status)
                            || page.content_status != crate::PageContentStatus::Readable
                            || page.truncated
                        {
                            page_errors.push(CrawlPageError {
                                url: url.to_string(),
                                reason: if page.truncated {
                                    "incomplete"
                                } else if !(200..300).contains(&page.status) {
                                    "http_error"
                                } else {
                                    "unreadable"
                                }
                                .into(),
                                status: Some(page.status),
                                content_status: Some(page.content_status),
                            });
                            truncated |= page.truncated;
                            let failure = crawl_page_failure();
                            tracing::warn!(
                                failure_id = %failure.id,
                                url = %url,
                                http_status = page.status,
                            "crawl page is not reusable content"
                            );
                            failures.push(failure.into());
                            continue;
                        }
                        rendered_count += usize::from(page.rendered);
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
                        let max_chunk_chars = options.max_chunk_chars;
                        let document = CRAWL_CONTENT
                            .run(move || {
                                StoredDocument::from_fetched_page(page, depth, max_chunk_chars)
                            })
                            .await
                            .map_err(|error| {
                                CrawlError::Io(std::io::Error::other(format!(
                                    "crawl content worker failed: {error}"
                                )))
                            })?;
                        // A refresh of the same document must replace old text and
                        // timestamps even when its hash is unchanged or similar.
                        let refreshing = fingerprints.contains_key(&document.id);
                        if !refreshing
                            && fingerprints.values().any(|known| {
                                known.raw == document.raw_html_hash
                                    || known.markdown == document.markdown_hash
                            })
                        {
                            duplicate_count += 1;
                            continue;
                        }
                        if !refreshing
                            && is_near_duplicate(
                                document.simhash,
                                fingerprints.values().map(|known| known.simhash),
                                options.near_duplicate_hamming_distance,
                            )
                        {
                            near_duplicate_count += 1;
                            continue;
                        }

                        let document = store
                            .mutate_async(&writer, move |store| {
                                store.save_document(&document)?;
                                Ok(document)
                            })
                            .await?;
                        cached_urls.insert(document.canonical_url.clone(), document.id.clone());
                        fingerprints
                            .insert(document.id.clone(), DocumentFingerprint::from(&document));
                        stored_count += 1;
                        documents.push(document.summary());
                    }
                    Ok(Retrieved::Cached(_)) => unreachable!("cache handled above"),
                    Err((_, err @ CrawlError::AccessRejected(_))) => return Err(err),
                    Err((reason, err)) => {
                        page_errors.push(CrawlPageError {
                            url: url.to_string(),
                            reason: reason.into(),
                            status: None,
                            content_status: None,
                        });
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
            Ok::<_, CrawlError>(())
        };
        tokio::try_join!(retrieve, process)?;
    }

    let retention = options.store_retention;
    let (prune_report, total_documents) = store
        .mutate_async(&writer, move |store| store.finish_crawl(retention))
        .await?;
    Ok(CrawlRunReport {
        start_url: start_url.to_string(),
        concurrency: options.concurrency,
        page_errors,
        engine: if rendered_count == 0 {
            "http"
        } else if options.render_js {
            "browser"
        } else {
            "http+browser"
        }
        .to_string(),
        rendered: rendered_count > 0,
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
    document.rendered == render_js
}

fn is_document_fresh(document: &StoredDocument, ttl: Duration) -> bool {
    let Ok(ttl) = chrono::Duration::from_std(ttl) else {
        return false;
    };
    Utc::now() - document.fetched_at <= ttl
}

fn is_near_duplicate(
    candidate: u64,
    mut existing: impl Iterator<Item = u64>,
    max_distance: u32,
) -> bool {
    existing.any(|value| simhash::hamming_distance(candidate, value) <= max_distance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    #[derive(Default)]
    struct Fetcher {
        third_started: Notify,
        fetched: AtomicUsize,
        authorized: AtomicUsize,
    }

    impl CrawlPageFetcher for Fetcher {
        fn authorize_cached<'a>(
            &'a self,
            _url: &'a Url,
            _document: &'a StoredDocument,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CrawlError>> + Send + 'a>>
        {
            self.authorized.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Ok(()) })
        }

        fn fetch_page<'a>(
            &'a self,
            url: &'a Url,
            _: bool,
            _: bool,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<FetchedPage, CrawlError>> + Send + 'a>,
        > {
            Box::pin(async move {
                self.fetched.fetch_add(1, Ordering::Relaxed);
                let body = match url.path() {
                    "/" => "Crawl entry point",
                    "/first" => {
                        self.third_started.notified().await;
                        "Snow leopard habitat"
                    }
                    "/second" => "Astronomy radio telescope",
                    "/third" => {
                        self.third_started.notify_one();
                        "Ceramic kiln temperature"
                    }
                    _ => unreachable!(),
                };
                let mut page = crate::extract_page_from_body(
                    url,
                    url,
                    "text/plain",
                    200,
                    false,
                    false,
                    body,
                    None,
                    None,
                );
                if url.path() == "/" {
                    page.links = ["first", "second", "third"]
                        .map(|link| url.join(link).unwrap().to_string())
                        .into();
                }
                Ok(page)
            })
        }
    }

    #[tokio::test]
    async fn crawl_refills_slots_and_stores_pages_in_breadth_first_order() {
        let root = tempfile::tempdir().unwrap();
        let store = CrawlStore::for_test_directory(root.path());
        let fetcher = Fetcher::default();
        let url = prepare_fetch_url("https://example.org/").unwrap();
        let options = CrawlRunOptions {
            concurrency: 2,
            max_pages: 4,
            near_duplicate_hamming_distance: 0,
            ..Default::default()
        };
        let first = tokio::time::timeout(
            Duration::from_secs(5),
            crawl_site(&url, &store, &options, &fetcher),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(first.stored_count, 4);
        assert_eq!(first.total_documents, 4);
        assert_eq!(
            first
                .documents
                .iter()
                .map(|document| document.url.as_str())
                .collect::<Vec<_>>(),
            [
                "https://example.org/",
                "https://example.org/first",
                "https://example.org/second",
                "https://example.org/third"
            ]
        );
        let hits = store.search_async("telescope".into(), 4).await.unwrap();
        assert_eq!(hits[0].0.url, "https://example.org/second");

        let second = crawl_site(&url, &store, &options, &fetcher).await.unwrap();
        assert_eq!(second.cached_count, 4);
        assert_eq!(fetcher.fetched.load(Ordering::Relaxed), 4);
        assert_eq!(fetcher.authorized.load(Ordering::Relaxed), 4);
    }
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
