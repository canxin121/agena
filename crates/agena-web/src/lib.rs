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

mod browser;
mod error;
mod extract;
mod fetch;
mod fetch_coordinator;
mod index;
mod metadata;
mod model;
mod paths;
mod run;
mod search;
mod spider;
mod store;

pub use browser::{
    local_browser_endpoint, local_browser_running, local_browser_touch, shutdown_local_browser,
};
pub use error::CrawlError;
pub use fetch::{
    DEFAULT_FETCH_TIMEOUT_SECS, DEFAULT_MAX_BODY_BYTES, FetchOptions, build_client,
    canonicalize_url, fetch_page, fetch_page_with_client, prepare_fetch_url, resolve_link_url,
};
pub use fetch_coordinator::{WebFetchCoordinator, WebFetchCoordinatorConfig};
pub use index::{rebuild_search_index, search_documents};
pub use model::{
    CrawlDocumentSummary, CrawlSearchHit, FetchedPage, StoredDocument, chunk_markdown, preview_text,
};
pub use paths::{CrawlDir, workspace_key};
pub use run::{
    CrawlPageFetcher, CrawlRunOptions, CrawlRunReport, crawl_site, document_matches_render_mode,
    ensure_index_exists,
};
pub use search::{
    WebSearchEngine, WebSearchOptions, WebSearchResult, normalize_web_search_engine,
    results_to_text, search_web,
};
pub use spider::{
    BrowserRenderOptions, LocalBrowserOptions, SpiderFetchOptions, fetch_page_with_spider,
};
pub use store::{CrawlStore, CrawlStorePruneReport, CrawlStoreRetention};
