mod browser;
mod error;
mod extract;
mod fetch;
mod index;
mod metadata;
mod model;
mod paths;
mod spider;
mod store;

pub use error::CrawlError;
pub use fetch::{
    DEFAULT_FETCH_TIMEOUT_SECS, DEFAULT_MAX_BODY_BYTES, FetchOptions, build_client,
    canonicalize_url, fetch_page, fetch_page_with_client, prepare_fetch_url, resolve_link_url,
};
pub use index::{rebuild_search_index, search_documents};
pub use model::{
    CrawlDocumentSummary, CrawlSearchHit, FetchedPage, StoredDocument, chunk_markdown, preview_text,
};
pub use paths::{CrawlDir, workspace_key};
pub use spider::{
    BrowserRenderOptions, LocalBrowserOptions, SpiderFetchOptions, fetch_page_with_spider,
};
pub use store::CrawlStore;
