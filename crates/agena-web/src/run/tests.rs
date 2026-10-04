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
