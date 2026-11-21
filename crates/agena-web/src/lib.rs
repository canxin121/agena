//! # agena-web
//!
//! Web fetching, crawling, and search for Agena.
//!
//! - [`fetch_page`] — fetch and normalize a single page.
//! - [`crawl_site`] / [`CrawlRunOptions`] — crawl a site with a local store
//!   and retention/pruning ([`CrawlStore`], [`CrawlStoreRetention`]).
//! - [`search_web`] / [`WebSearchEngine`] — web search backed by an engine
//!   client or the local index.
//! - [`rebuild_search_index`] / [`search_documents`] — local full-text
//!   indexing and search.
//! - [`WebFetchCoordinator`] — coordinates concurrent fetches under a shared
//!   fetch budget.

mod error;
mod extract;
mod fetch;
mod fetch_coordinator;
mod fetch_options;
mod index;
pub mod managed_preview;
mod metadata;
mod model;
pub mod network;
mod paths;
mod run;
mod search;
mod search_api;
mod snapshots;
mod store;

pub use error::CrawlError;
pub use extract::{
    ExtractionBackend, decode_response_body, extract_page_from_body, extract_with_backend,
    robots_allows,
};
pub use fetch::{
    DEFAULT_FETCH_TIMEOUT_SECS, DEFAULT_MAX_BODY_BYTES, FetchOptions, build_client,
    canonicalize_url, fetch_page, fetch_page_with_client, prepare_fetch_url, resolve_link_url,
};
pub use fetch_coordinator::{WebFetchCoordinator, WebFetchCoordinatorConfig};
pub use fetch_options::{BrowserRenderOptions, WebFetchOptions};
pub use index::{rebuild_search_index, search_documents};
pub use model::{
    CrawlDocumentSummary, CrawlSearchHit, FetchedPage, PageContentStatus, StoredDocument,
    chunk_markdown, preview_text,
};
pub use paths::{CrawlDir, workspace_key};
pub use run::{
    CrawlPageError, CrawlPageFetcher, CrawlRunOptions, CrawlRunReport, MAX_CRAWL_CONCURRENCY,
    crawl_site, document_matches_render_mode, ensure_index_exists,
};
pub use search::{
    SearchIssue, SearchIssueKind, WebSearchCoordinator, WebSearchEngine, WebSearchOptions,
    WebSearchPage, WebSearchResponse, WebSearchResult, normalize_web_search_engine,
    results_to_text, search_web,
};
pub use search_api::{
    SearchApiOptions, SearchApiProvider, SearchApiResponse, search_api, search_domain_allowed,
};
pub use snapshots::{PageSlice, PageSnapshots, page_slice};
pub use store::{CrawlStore, CrawlStorePruneReport, CrawlStoreRetention};
mod http_route;
pub use http_route::{WebHttpClientPool, WebHttpRoute};
