use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    CrawlDocumentSummary, CrawlError, CrawlStore, FetchedPage, StoredDocument, prepare_fetch_url,
};

#[derive(Debug, Clone)]
pub struct CrawlRunOptions {
    pub max_pages: usize,
    pub max_depth: u32,
    pub same_host_only: bool,
    pub use_cache: bool,
    pub render_js: bool,
    pub document_cache_ttl: Duration,
    pub max_chunk_chars: usize,
    pub near_duplicate_hamming_distance: u32,
}

impl Default for CrawlRunOptions {
    fn default() -> Self {
        Self {
            max_pages: 10,
            max_depth: 1,
            same_host_only: true,
            use_cache: true,
            render_js: false,
            document_cache_ttl: Duration::from_secs(24 * 60 * 60),
            max_chunk_chars: 1800,
            near_duplicate_hamming_distance: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlRunReport {
    pub start_url: String,
    pub engine: String,
    pub rendered: bool,
    pub stored_count: usize,
    pub cached_count: usize,
    pub duplicate_count: usize,
    pub near_duplicate_count: usize,
    pub failure_count: usize,
    pub total_documents: usize,
    pub documents: Vec<CrawlDocumentSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

pub trait CrawlPageFetcher {
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
        if documents.len() >= options.max_pages {
            break;
        }

        if options.use_cache
            && let Some(existing) = store.find_by_url(url.as_str())?
            && document_matches_render_mode(&existing, options.render_js)
            && is_document_fresh(&existing, options.document_cache_ttl)
        {
            if depth < options.max_depth {
                enqueue_document_links(
                    start_url,
                    &existing,
                    depth,
                    options.same_host_only,
                    &mut queue,
                    &mut seen_urls,
                );
            }
            cached_count += 1;
            documents.push(existing.summary());
            continue;
        }

        match fetcher
            .fetch_page(&url, options.use_cache, options.render_js)
            .await
        {
            Ok(page) => {
                if page.status >= 400 {
                    failures.push(format!("{url}: http {}", page.status));
                    continue;
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
                if depth < options.max_depth {
                    enqueue_links(
                        start_url,
                        &page,
                        depth,
                        options.same_host_only,
                        &mut queue,
                        &mut seen_urls,
                    );
                }
                stored_count += 1;
                documents.push(document.summary());
            }
            Err(err) => failures.push(format!("{url}: {err}")),
        }
    }

    store.rebuild_index()?;
    let total_documents = store.list_documents()?.len();
    Ok(CrawlRunReport {
        start_url: start_url.to_string(),
        engine: "spider".to_string(),
        rendered: options.render_js,
        stored_count,
        cached_count,
        duplicate_count,
        near_duplicate_count,
        failure_count: failures.len(),
        total_documents,
        documents,
        failures,
    })
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

fn enqueue_links(
    start_url: &Url,
    page: &FetchedPage,
    current_depth: u32,
    same_host_only: bool,
    queue: &mut VecDeque<(Url, u32)>,
    seen_urls: &mut HashSet<String>,
) {
    for link in &page.links {
        enqueue_url(
            start_url,
            link,
            current_depth,
            same_host_only,
            queue,
            seen_urls,
        );
    }
}

fn enqueue_document_links(
    start_url: &Url,
    document: &StoredDocument,
    current_depth: u32,
    same_host_only: bool,
    queue: &mut VecDeque<(Url, u32)>,
    seen_urls: &mut HashSet<String>,
) {
    for link in &document.links {
        enqueue_url(
            start_url,
            link,
            current_depth,
            same_host_only,
            queue,
            seen_urls,
        );
    }
}

fn enqueue_url(
    start_url: &Url,
    raw: &str,
    current_depth: u32,
    same_host_only: bool,
    queue: &mut VecDeque<(Url, u32)>,
    seen_urls: &mut HashSet<String>,
) {
    let Ok(url) = prepare_fetch_url(raw) else {
        return;
    };
    if same_host_only && url.host_str() != start_url.host_str() {
        return;
    }
    if seen_urls.insert(url.to_string()) {
        queue.push_back((url, current_depth + 1));
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::document_matches_render_mode;

    #[test]
    fn plain_fetch_can_reuse_rendered_documents_but_not_reverse() {
        let mut document = crate::StoredDocument {
            id: "doc-1".to_string(),
            url: "https://example.com/docs".to_string(),
            canonical_url: "https://example.com/docs".to_string(),
            title: "Docs".to_string(),
            markdown: "content".to_string(),
            chunks: vec!["content".to_string()],
            chunk_hashes: vec!["chunk".to_string()],
            links: Vec::new(),
            content_type: "text/html".to_string(),
            status: 200,
            truncated: false,
            rendered: false,
            hash: "hash".to_string(),
            raw_html_hash: "raw".to_string(),
            markdown_hash: "markdown".to_string(),
            simhash: 1,
            etag: None,
            last_modified: None,
            depth: 0,
            fetched_at: Utc::now(),
        };

        assert!(document_matches_render_mode(&document, false));
        assert!(!document_matches_render_mode(&document, true));

        document.rendered = true;
        assert!(document_matches_render_mode(&document, false));
        assert!(document_matches_render_mode(&document, true));
    }
}
