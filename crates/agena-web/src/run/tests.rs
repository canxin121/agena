use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    calls: AtomicUsize,
}
impl CrawlPageFetcher for Fixture {
    fn authorize_cached<'a>(
        &'a self,
        _: &'a Url,
        _: &'a StoredDocument,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CrawlError>> + Send + 'a>>
    {
        Box::pin(async {
            Err(CrawlError::InvalidInput(
                "fixture denies cached access".into(),
            ))
        })
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            if url.path() != "/" {
                return Err(CrawlError::NotFound("synthetic failure".into()));
            }
            let mut page = crate::extract_page_from_body(
                url,
                url,
                "text/plain",
                200,
                false,
                false,
                "A unique root document with enough content to store.",
                None,
                None,
            );
            page.links = (0..500)
                .map(|i| format!("https://example.test/{i}"))
                .collect();
            Ok(page)
        })
    }
}

#[tokio::test]
async fn cached_documents_require_current_authorization() {
    let dir = tempfile::tempdir().unwrap();
    let store = CrawlStore::for_test(dir.path());
    let url = Url::parse("https://example.test/").unwrap();
    let fixture = Fixture {
        calls: AtomicUsize::new(0),
    };
    let page = fixture.fetch_page(&url, false, false).await.unwrap();
    store
        .save_document(&StoredDocument::from_fetched_page(page, 0, 1000))
        .unwrap();
    fixture.calls.store(0, Ordering::SeqCst);
    let options = CrawlRunOptions {
        max_pages: 1,
        ..Default::default()
    };
    let report = crawl_site(&url, &store, &options, &fixture).await.unwrap();
    assert_eq!(report.cached_count, 0);
    assert_eq!(report.attempted_count, 1);
    assert_eq!(report.failure_count, 1);
    assert!(report.documents.is_empty());
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn attempts_and_discovery_remain_bounded_when_child_pages_all_fail() {
    let dir = tempfile::tempdir().unwrap();
    let store = CrawlStore::for_test(dir.path());
    let fixture = Fixture {
        calls: AtomicUsize::new(0),
    };
    let options = CrawlRunOptions {
        max_pages: 3,
        use_cache: false,
        ..Default::default()
    };
    let report = crawl_site(
        &Url::parse("https://example.test/").unwrap(),
        &store,
        &options,
        &fixture,
    )
    .await
    .unwrap();
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 3);
    assert_eq!(report.attempted_count, 3);
    assert_eq!(report.stored_count, 1);
    assert_eq!(report.failure_count, 2);
    assert_eq!(report.discovered_count, 128);
    assert!(report.truncated);
}

#[tokio::test]
async fn another_extractor_does_not_reuse_stored_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = CrawlStore::for_test(dir.path());
    let url = Url::parse("https://example.test/").unwrap();
    let mut page = crate::extract_page_from_body(
        &url,
        &url,
        "text/plain",
        200,
        false,
        false,
        "Old alternate extraction.",
        None,
        None,
    );
    page.extraction_backend = crate::ExtractionBackend::Trafilatura;
    store
        .save_document(&StoredDocument::from_fetched_page(page, 0, 1000))
        .unwrap();
    let fixture = Fixture {
        calls: AtomicUsize::new(0),
    };
    let options = CrawlRunOptions {
        max_pages: 1,
        max_depth: 0,
        ..Default::default()
    };
    let report = crawl_site(&url, &store, &options, &fixture).await.unwrap();
    assert_eq!(report.cached_count, 0);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}

struct ConcurrentFixture {
    active: AtomicUsize,
    peak: AtomicUsize,
    calls: std::sync::Mutex<Vec<String>>,
    wait_forever: bool,
    ready: tokio::sync::Notify,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl CrawlPageFetcher for ConcurrentFixture {
    fn authorize_cached<'a>(
        &'a self,
        _: &'a Url,
        _: &'a StoredDocument,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CrawlError>> + Send + 'a>>
    {
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
            self.calls.lock().unwrap().push(url.to_string());
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            let _guard = Active(&self.active);
            self.peak.fetch_max(active, Ordering::SeqCst);
            if url.path() != "/" && self.wait_forever {
                if active == 2 {
                    self.ready.notify_one();
                }
                std::future::pending::<()>().await;
            }
            tokio::time::sleep(Duration::from_millis(if url.path() == "/0" {
                10
            } else {
                1
            }))
            .await;
            if url.path() == "/1" {
                return Err(CrawlError::NotFound("fixture".into()));
            }
            let mut page = crate::extract_page_from_body(
                url,
                url,
                "text/plain",
                200,
                false,
                false,
                &format!(
                    "Unique reference {}. Details: {}",
                    url,
                    url.path().repeat(20)
                ),
                None,
                None,
            );
            if url.path() == "/" {
                page.links = (0..20)
                    .map(|i| format!("https://example.test/{i}"))
                    .collect();
            }
            if url.path() == "/2" {
                page.content_status = crate::PageContentStatus::Blocked;
            }
            Ok(page)
        })
    }
}
fn concurrent_fixture(wait_forever: bool) -> ConcurrentFixture {
    ConcurrentFixture {
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        calls: std::sync::Mutex::new(Vec::new()),
        wait_forever,
        ready: tokio::sync::Notify::new(),
    }
}

#[tokio::test]
async fn concurrent_crawl_reserves_exact_budget_and_reports_unreadable_urls() {
    let dir = tempfile::tempdir().unwrap();
    let store = CrawlStore::for_test(dir.path());
    let fixture = concurrent_fixture(false);
    let url = Url::parse("https://example.test/").unwrap();
    let options = CrawlRunOptions {
        max_pages: 6,
        concurrency: 2,
        use_cache: false,
        ..Default::default()
    };
    let report = crawl_site(&url, &store, &options, &fixture).await.unwrap();
    assert_eq!(fixture.peak.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.active.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.calls.lock().unwrap().len(), 6);
    assert_eq!(report.attempted_count, 6);
    assert_eq!(report.page_errors.len(), 2);
    assert_eq!(report.page_errors[0].url, "https://example.test/1");
    assert_eq!(
        report.page_errors[1].content_status,
        Some(crate::PageContentStatus::Blocked)
    );
    assert!(
        store
            .find_by_url("https://example.test/2")
            .unwrap()
            .is_none()
    );
    assert!(report.truncated);
}

#[tokio::test]
async fn cancelling_crawl_drops_all_concurrent_requests() {
    let dir = tempfile::tempdir().unwrap();
    let store = CrawlStore::for_test(dir.path());
    let fixture = concurrent_fixture(true);
    let url = Url::parse("https://example.test/").unwrap();
    let options = CrawlRunOptions {
        concurrency: 2,
        use_cache: false,
        ..Default::default()
    };
    let mut crawl = Box::pin(crawl_site(&url, &store, &options, &fixture));
    tokio::select! {
        result = &mut crawl => panic!("unexpected completion: {result:?}"),
        _ = fixture.ready.notified() => {},
        _ = tokio::time::sleep(Duration::from_secs(5)) => panic!("concurrent batch never started"),
    }
    drop(crawl);
    assert_eq!(fixture.active.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.calls.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn forced_http_refresh_replaces_a_rendered_or_near_duplicate_old_document() {
    let dir = tempfile::tempdir().unwrap();
    let store = CrawlStore::for_test(dir.path());
    let fixture = concurrent_fixture(false);
    let url = Url::parse("https://example.test/").unwrap();
    let mut old_page = fixture.fetch_page(&url, false, false).await.unwrap();
    old_page.rendered = true;
    old_page.markdown.push_str(" obsolete note");
    let mut old = StoredDocument::from_fetched_page(old_page, 0, 1000);
    old.fetched_at = Utc::now() - chrono::Duration::minutes(5);
    store.save_document(&old).unwrap();
    let options = CrawlRunOptions {
        max_pages: 1,
        max_depth: 0,
        near_duplicate_hamming_distance: 64,
        ..Default::default()
    };
    let report = crawl_site(&url, &store, &options, &fixture).await.unwrap();
    assert_eq!(report.cached_count, 0);
    assert_eq!(report.stored_count, 1);
    let refreshed = store.get_document(&old.id).unwrap();
    assert!(!refreshed.rendered);
    assert!(!refreshed.markdown.contains("obsolete note"));
    assert!(refreshed.fetched_at > old.fetched_at);
    let report = crawl_site(
        &url,
        &store,
        &CrawlRunOptions {
            use_cache: false,
            ..options
        },
        &fixture,
    )
    .await
    .unwrap();
    assert_eq!(
        report.stored_count, 1,
        "identical content still refreshes its timestamp"
    );
    assert_eq!(report.duplicate_count, 0);
}
