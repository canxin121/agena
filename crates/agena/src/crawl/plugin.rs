use std::collections::{HashSet, VecDeque};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_crawl::{
    BrowserRenderOptions, CrawlDocumentSummary, CrawlStore, FetchedPage, LocalBrowserOptions,
    SpiderFetchOptions, StoredDocument, fetch_page_with_spider, prepare_fetch_url, preview_text,
};
use agena_macros::StaticToolSurface;
use async_trait::async_trait;
use governor::{DefaultKeyedRateLimiter, Quota};
use moka::future::Cache;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::CrawlConfig;
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostNetworkPermissionCheckRequest};
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, NetworkRequest, PathRequest,
    Plugin, PluginManifest, PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
    ToolTag,
};

pub const CRAWL_PLUGIN_ID: &str = "agena.crawl";

pub struct CrawlPlugin {
    config: CrawlConfig,
    workspace_root: OnceLock<PathBuf>,
    host: OnceLock<Arc<dyn HostClient>>,
    fetch_cache: Cache<String, FetchedPage>,
    host_limiter: DefaultKeyedRateLimiter<String>,
    sync_lock: Mutex<()>,
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "crawl",
    description = "Preferred local web ingestion tool. Use action `fetch` for one page, `crawl` to build a local index, `search` to query that index, `get` to inspect one stored page, or `list` to inspect the local crawl catalog.",
    summary = "Preferred local crawler and embedded web index.",
    help = "This tool is fully local by default: it fetches pages directly, stores extracted markdown under the current workspace's Agena data directory, and searches the local Tantivy index without any Firecrawl-style remote service. Prefer it over `web.fetch` when you need multi-page retrieval, repeated lookups, or a persistent local web corpus.",
    tags(
        ToolTag::ReadOnly,
        ToolTag::Mutating,
        ToolTag::Network,
        ToolTag::Internet,
        ToolTag::Discovery
    ),
    host_capabilities(HostCapability::PermissionCheck),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum CrawlToolInput {
    #[tool(exec = "fetch")]
    Fetch {
        #[serde(flatten)]
        args: CrawlFetchInput,
    },
    #[tool(exec = "crawl")]
    Crawl {
        #[serde(flatten)]
        args: CrawlRunInput,
    },
    #[tool(exec = "search")]
    Search {
        #[serde(flatten)]
        args: CrawlSearchInput,
    },
    #[tool(exec = "get")]
    Get {
        #[serde(flatten)]
        args: CrawlGetInput,
    },
    #[tool(exec = "list")]
    List {
        #[serde(flatten)]
        args: CrawlListInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CrawlFetchInput {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    use_cache: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    render_js: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CrawlRunInput {
    start_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_pages: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_depth: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    same_host_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    use_cache: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    render_js: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CrawlSearchInput {
    query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CrawlGetInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CrawlListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct CrawlRunOutput {
    start_url: String,
    engine: String,
    rendered: bool,
    stored_count: usize,
    cached_count: usize,
    duplicate_count: usize,
    near_duplicate_count: usize,
    failure_count: usize,
    total_documents: usize,
    documents: Vec<CrawlDocumentSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    failures: Vec<String>,
}

impl CrawlPlugin {
    pub fn new(config: CrawlConfig) -> Self {
        Self {
            fetch_cache: Cache::builder()
                .time_to_live(Duration::from_secs(config.fetch_cache_ttl_secs))
                .max_capacity(config.fetch_cache_capacity)
                .build(),
            host_limiter: build_host_limiter(config.request_delay_ms),
            config,
            workspace_root: OnceLock::new(),
            host: OnceLock::new(),
            sync_lock: Mutex::new(()),
        }
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::new("crawl plugin invoked before init"))
    }

    fn host(&self) -> SdkResult<&Arc<dyn HostClient>> {
        self.host
            .get()
            .ok_or_else(|| PluginError::new("crawl plugin host not initialized"))
    }

    fn store(&self) -> SdkResult<CrawlStore> {
        Ok(CrawlStore::for_workspace(self.workspace_root()?))
    }

    fn spider_fetch_options(&self, rendered: bool) -> SpiderFetchOptions {
        let delay = (self.config.browser_wait_for_delay_ms > 0)
            .then(|| Duration::from_millis(self.config.browser_wait_for_delay_ms));
        SpiderFetchOptions {
            max_body_bytes: self.config.max_body_bytes as usize,
            timeout: Duration::from_secs(self.config.fetch_timeout_secs),
            delay_ms: self.config.request_delay_ms,
            user_agent: crate::provider::CLAUDE_USER_WEB_FETCH_USER_AGENT.to_string(),
            respect_robots_txt: self.config.respect_robots_txt,
            browser: BrowserRenderOptions {
                enabled: rendered,
                local_browser: LocalBrowserOptions {
                    executable_path: self
                        .config
                        .browser_executable_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(PathBuf::from),
                    startup_timeout: Duration::from_secs(self.config.browser_wait_timeout_secs),
                },
                wait_for_network_idle: self.config.browser_wait_for_network_idle,
                wait_for_selector: self.config.browser_wait_for_selector.clone(),
                wait_timeout: Duration::from_secs(self.config.browser_wait_timeout_secs),
                delay,
            },
        }
    }

    async fn ensure_network_permission(&self, url: &url::Url) -> SdkResult<()> {
        self.host()?
            .ensure_network_permission(HostNetworkPermissionCheckRequest::connect(url.as_str()))
            .await
    }

    async fn fetch_page(
        &self,
        url: &url::Url,
        use_cache: bool,
        render_js: bool,
    ) -> SdkResult<FetchedPage> {
        let cache_key = fetch_cache_key(url, render_js);
        if use_cache && let Some(hit) = self.fetch_cache.get(cache_key.as_str()).await {
            return Ok(hit);
        }
        if let Some(host) = url.host_str() {
            self.host_limiter.until_key_ready(&host.to_string()).await;
        }
        self.ensure_network_permission(url).await?;
        let options = self.spider_fetch_options(render_js);
        let page = fetch_page_with_spider(url, &options)
            .await
            .map_err(crawl_error_to_plugin)?;
        if use_cache {
            self.fetch_cache.insert(cache_key, page.clone()).await;
        }
        Ok(page)
    }

    async fn invoke_fetch(&self, input: &CrawlFetchInput) -> SdkResult<ToolInvokeOutput> {
        let url = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?;
        let render_js = input.render_js.unwrap_or(self.config.browser_enabled);
        let page = self
            .fetch_page(&url, input.use_cache.unwrap_or(true), render_js)
            .await?;
        let payload =
            serde_json::to_value(&page).map_err(|err| PluginError::new(err.to_string()))?;
        let text = format_fetched_page(&page);
        Ok(ToolInvokeOutput::text(text)
            .with_title("crawl fetch")
            .with_payload(payload))
    }

    async fn invoke_crawl(&self, input: &CrawlRunInput) -> SdkResult<ToolInvokeOutput> {
        let start_url =
            prepare_fetch_url(input.start_url.as_str()).map_err(crawl_error_to_plugin)?;
        let max_pages = clamp_limit(
            input.max_pages,
            self.config.default_max_pages as usize,
            self.config.max_pages_limit as usize,
        );
        let max_depth = input
            .max_depth
            .unwrap_or(self.config.default_max_depth)
            .clamp(0, self.config.max_depth_limit);
        let same_host_only = input
            .same_host_only
            .unwrap_or(self.config.default_same_host_only);
        let use_cache = input.use_cache.unwrap_or(true);
        let render_js = input.render_js.unwrap_or(self.config.browser_enabled);
        let store = self.store()?;

        let mut queue = VecDeque::from([(start_url.clone(), 0u32)]);
        let mut seen_urls = HashSet::from([start_url.to_string()]);
        let mut documents = Vec::new();
        let mut failures = Vec::new();
        let mut stored_count = 0usize;
        let mut cached_count = 0usize;
        let mut duplicate_count = 0usize;
        let mut near_duplicate_count = 0usize;
        let mut known_simhashes = store
            .list_documents()
            .map_err(crawl_error_to_plugin)?
            .into_iter()
            .map(|document| document.simhash)
            .collect::<Vec<_>>();

        let _guard = self.sync_lock.lock().await;
        while let Some((url, depth)) = queue.pop_front() {
            if documents.len() >= max_pages {
                break;
            }

            if use_cache
                && let Some(existing) = store
                    .find_by_url(url.as_str())
                    .map_err(crawl_error_to_plugin)?
                && document_matches_render_mode(&existing, render_js)
                && is_document_fresh(&existing, self.config.document_cache_ttl_secs)
            {
                if depth < max_depth {
                    enqueue_document_links(
                        &start_url,
                        &existing,
                        depth,
                        same_host_only,
                        &mut queue,
                        &mut seen_urls,
                    );
                }
                cached_count += 1;
                documents.push(existing.summary());
                continue;
            }

            match self.fetch_page(&url, use_cache, render_js).await {
                Ok(page) => {
                    if page.status >= 400 {
                        failures.push(format!("{url}: http {}", page.status));
                        continue;
                    }
                    let document = StoredDocument::from_fetched_page(
                        page.clone(),
                        depth,
                        self.config.default_chunk_chars as usize,
                    );
                    if store
                        .find_by_raw_hash(document.raw_html_hash.as_str())
                        .map_err(crawl_error_to_plugin)?
                        .is_some()
                    {
                        duplicate_count += 1;
                        continue;
                    }
                    if store
                        .find_by_markdown_hash(document.markdown_hash.as_str())
                        .map_err(crawl_error_to_plugin)?
                        .is_some()
                    {
                        duplicate_count += 1;
                        continue;
                    }
                    if is_near_duplicate(
                        document.simhash,
                        &known_simhashes,
                        self.config.near_duplicate_hamming_distance,
                    ) {
                        near_duplicate_count += 1;
                        continue;
                    }

                    store
                        .save_document(&document)
                        .map_err(crawl_error_to_plugin)?;
                    known_simhashes.push(document.simhash);
                    if depth < max_depth {
                        enqueue_links(
                            &start_url,
                            &page,
                            depth,
                            same_host_only,
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

        store.rebuild_index().map_err(crawl_error_to_plugin)?;
        let total_documents = store.list_documents().map_err(crawl_error_to_plugin)?.len();
        let payload = CrawlRunOutput {
            start_url: start_url.to_string(),
            engine: "spider".to_string(),
            rendered: render_js,
            stored_count,
            cached_count,
            duplicate_count,
            near_duplicate_count,
            failure_count: failures.len(),
            total_documents,
            documents: documents.clone(),
            failures: failures.clone(),
        };
        let text = format_crawl_run(&payload);
        let payload =
            serde_json::to_value(payload).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(text)
            .with_title("crawl run")
            .with_payload(payload))
    }

    async fn invoke_search(&self, input: &CrawlSearchInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.trim();
        if query.is_empty() {
            return Err(PluginError::invalid_params(
                "crawl search requires a non-empty query",
            ));
        }
        let limit = clamp_limit(
            input.limit,
            self.config.search_default_limit as usize,
            self.config.search_max_limit as usize,
        );
        let store = self.store()?;
        ensure_index_exists(&store).map_err(crawl_error_to_plugin)?;
        let hits = store.search(query, limit).map_err(crawl_error_to_plugin)?;
        let mut lines = vec![format!(
            "Found {} crawl hit(s) for '{}'.",
            hits.len(),
            query
        )];
        for hit in &hits {
            lines.push(format!(
                "- {} [{}] {} ({})",
                hit.title,
                hit.chunk_index,
                hit.url,
                hit.match_sources.join("+")
            ));
            lines.push(format!("  {}", hit.preview));
        }
        let payload =
            serde_json::to_value(&hits).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(lines.join("\n"))
            .with_title("crawl search")
            .with_payload(payload))
    }

    async fn invoke_get(&self, input: &CrawlGetInput) -> SdkResult<ToolInvokeOutput> {
        let store = self.store()?;
        let document = match (
            input
                .id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            input
                .url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        ) {
            (Some(id), None) => store.get_document(id).map_err(crawl_error_to_plugin)?,
            (None, Some(url)) => store
                .find_by_url(url)
                .map_err(crawl_error_to_plugin)?
                .ok_or_else(|| PluginError::new(format!("crawl document for '{url}' not found")))?,
            _ => {
                return Err(PluginError::invalid_params(
                    "crawl get requires exactly one of `id` or `url`",
                ));
            }
        };
        let payload =
            serde_json::to_value(&document).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(format_document(&document))
            .with_title("crawl get")
            .with_payload(payload))
    }

    async fn invoke_list(&self, input: &CrawlListInput) -> SdkResult<ToolInvokeOutput> {
        let limit = clamp_limit(
            input.limit,
            self.config.list_default_limit as usize,
            self.config.list_max_limit as usize,
        );
        let store = self.store()?;
        let documents = store.list_summaries(limit).map_err(crawl_error_to_plugin)?;
        let text = if documents.is_empty() {
            "No crawl documents stored.".to_string()
        } else {
            let mut lines = vec![format!("Stored {} crawl document(s).", documents.len())];
            for document in &documents {
                lines.push(format!(
                    "- {} [{} chunk(s), depth {}] {}",
                    document.title, document.chunk_count, document.depth, document.url
                ));
            }
            lines.join("\n")
        };
        let payload =
            serde_json::to_value(&documents).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(text)
            .with_title("crawl list")
            .with_payload(payload))
    }
}

#[async_trait]
impl Plugin for CrawlPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(CRAWL_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description(
                "Local crawl/search plugin with embedded storage, caching, and Tantivy retrieval.",
            )
            .hooks(HookSubscription::INIT | HookSubscription::TOOL_INVOKE)
            .tool(crawl_decl())
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let _ = self.workspace_root.set(ctx.workspace_root);
        let _ = self.host.set(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        if input.tool_name != "crawl" {
            return Err(PluginError::invalid_params(format!(
                "unknown crawl plugin tool '{}'",
                input.tool_name
            )));
        }
        match parse_crawl_input(input.input)? {
            CrawlToolInput::Fetch { args } => self.invoke_fetch(&args).await,
            CrawlToolInput::Crawl { args } => self.invoke_crawl(&args).await,
            CrawlToolInput::Search { args } => self.invoke_search(&args).await,
            CrawlToolInput::Get { args } => self.invoke_get(&args).await,
            CrawlToolInput::List { args } => self.invoke_list(&args).await,
        }
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        if tool != "crawl" {
            return Ok(Vec::new());
        }
        let parsed = parse_crawl_input(input.clone())?;
        let store = self.store()?;
        let path = store.dir().display().to_string();
        let request = match parsed {
            CrawlToolInput::Fetch { .. } => return Ok(Vec::new()),
            CrawlToolInput::Crawl { .. } => PathRequest::write(path),
            CrawlToolInput::Search { .. } => PathRequest::write(path),
            CrawlToolInput::Get { .. } | CrawlToolInput::List { .. } => PathRequest::read(path),
        };
        Ok(vec![request])
    }

    async fn permission_networks(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<NetworkRequest>> {
        if tool != "crawl" {
            return Ok(Vec::new());
        }
        let parsed = parse_crawl_input(input.clone())?;
        let requests = match parsed {
            CrawlToolInput::Fetch { args } => vec![NetworkRequest::connect(
                prepare_fetch_url(args.url.as_str())
                    .map_err(crawl_error_to_plugin)?
                    .to_string(),
            )],
            CrawlToolInput::Crawl { args } => vec![NetworkRequest::connect(
                prepare_fetch_url(args.start_url.as_str())
                    .map_err(crawl_error_to_plugin)?
                    .to_string(),
            )],
            CrawlToolInput::Search { .. }
            | CrawlToolInput::Get { .. }
            | CrawlToolInput::List { .. } => Vec::new(),
        };
        Ok(requests)
    }
}

fn crawl_decl() -> PluginToolDecl {
    CrawlToolInput::tool_decl()
}

fn parse_crawl_input(input: serde_json::Value) -> SdkResult<CrawlToolInput> {
    CrawlToolInput::parse_input(input)
}

fn crawl_error_to_plugin(err: agena_crawl::CrawlError) -> PluginError {
    PluginError::new(err.to_string())
}

fn clamp_limit(limit: Option<u32>, default_limit: usize, max_limit: usize) -> usize {
    limit
        .unwrap_or(default_limit as u32)
        .clamp(1, max_limit as u32) as usize
}

fn build_host_limiter(delay_ms: u64) -> DefaultKeyedRateLimiter<String> {
    let quota = Quota::with_period(Duration::from_millis(delay_ms.max(1)))
        .expect("crawl request delay must be non-zero")
        .allow_burst(NonZeroU32::new(1).expect("non-zero"));
    DefaultKeyedRateLimiter::keyed(quota)
}

fn is_document_fresh(document: &StoredDocument, ttl_secs: u64) -> bool {
    let ttl = chrono::Duration::seconds(ttl_secs.min(i64::MAX as u64) as i64);
    chrono::Utc::now() - document.fetched_at <= ttl
}

fn document_matches_render_mode(document: &StoredDocument, render_js: bool) -> bool {
    !render_js || document.rendered
}

fn is_near_duplicate(candidate: u64, existing: &[u64], max_distance: u32) -> bool {
    existing
        .iter()
        .any(|value| simhash::hamming_distance(candidate, *value) <= max_distance)
}

fn fetch_cache_key(url: &url::Url, render_js: bool) -> String {
    format!(
        "spider:{}:{}",
        if render_js { "rendered" } else { "plain" },
        url
    )
}

fn enqueue_links(
    start_url: &url::Url,
    page: &FetchedPage,
    current_depth: u32,
    same_host_only: bool,
    queue: &mut VecDeque<(url::Url, u32)>,
    seen_urls: &mut HashSet<String>,
) {
    for link in &page.links {
        let Ok(url) = url::Url::parse(link) else {
            continue;
        };
        if same_host_only && url.host_str() != start_url.host_str() {
            continue;
        }
        if seen_urls.insert(url.to_string()) {
            queue.push_back((url, current_depth + 1));
        }
    }
}

fn enqueue_document_links(
    start_url: &url::Url,
    document: &StoredDocument,
    current_depth: u32,
    same_host_only: bool,
    queue: &mut VecDeque<(url::Url, u32)>,
    seen_urls: &mut HashSet<String>,
) {
    for link in &document.links {
        let Ok(url) = url::Url::parse(link) else {
            continue;
        };
        if same_host_only && url.host_str() != start_url.host_str() {
            continue;
        }
        if seen_urls.insert(url.to_string()) {
            queue.push_back((url, current_depth + 1));
        }
    }
}

fn ensure_index_exists(store: &CrawlStore) -> Result<(), agena_crawl::CrawlError> {
    if !store.dir().join(".index").exists() {
        store.rebuild_index()?;
    }
    Ok(())
}

fn format_fetched_page(page: &FetchedPage) -> String {
    let mut lines = vec![format!("Title: {}", page.title)];
    lines.push(format!("URL: {}", page.canonical_url));
    lines.push(format!("Status: {}", page.status));
    lines.push(format!(
        "Rendered: {}",
        if page.rendered { "yes" } else { "no" }
    ));
    if let Some(etag) = &page.etag {
        lines.push(format!("ETag: {etag}"));
    }
    if let Some(last_modified) = &page.last_modified {
        lines.push(format!("Last-Modified: {last_modified}"));
    }
    lines.push(String::new());
    lines.push(preview_text(page.markdown.as_str(), 4000));
    lines.join("\n")
}

fn format_document(document: &StoredDocument) -> String {
    let mut lines = vec![format!("Title: {}", document.title)];
    lines.push(format!("URL: {}", document.canonical_url));
    lines.push(format!("Status: {}", document.status));
    lines.push(format!("Depth: {}", document.depth));
    lines.push(format!(
        "Rendered: {}",
        if document.rendered { "yes" } else { "no" }
    ));
    lines.push(format!("Fetched: {}", document.fetched_at.to_rfc3339()));
    lines.push(String::new());
    lines.push(preview_text(document.markdown.as_str(), 5000));
    lines.join("\n")
}

fn format_crawl_run(output: &CrawlRunOutput) -> String {
    let mut lines = vec![format!(
        "Crawled from {} via {} (rendered: {}). New pages stored: {}. Cached pages reused: {}. Exact duplicates skipped: {}. Near duplicates skipped: {}. Failures: {}. Total indexed documents: {}.",
        output.start_url,
        output.engine,
        if output.rendered { "yes" } else { "no" },
        output.stored_count,
        output.cached_count,
        output.duplicate_count,
        output.near_duplicate_count,
        output.failure_count,
        output.total_documents
    )];
    for document in &output.documents {
        lines.push(format!(
            "- {} [{} chunk(s), depth {}] {}",
            document.title, document.chunk_count, document.depth, document.url
        ));
    }
    if !output.failures.is_empty() {
        lines.push("Failures:".to_string());
        lines.extend(
            output
                .failures
                .iter()
                .take(5)
                .map(|failure| format!("- {failure}")),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{crawl_decl, document_matches_render_mode, fetch_cache_key};
    use crate::plugin::sdk::HostCapability;

    #[test]
    fn crawl_decl_declares_permission_check_host_capability() {
        let decl = crawl_decl();
        assert!(
            decl.host_capabilities
                .contains(&HostCapability::PermissionCheck)
        );
    }

    #[test]
    fn fetch_cache_key_tracks_render_mode() {
        let url = url::Url::parse("https://example.com/docs").expect("url parses");
        assert_eq!(
            fetch_cache_key(&url, true),
            "spider:rendered:https://example.com/docs"
        );
        assert_eq!(
            fetch_cache_key(&url, false),
            "spider:plain:https://example.com/docs"
        );
    }

    #[test]
    fn plain_fetch_can_reuse_rendered_documents_but_not_reverse() {
        let mut document = agena_crawl::StoredDocument {
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
            fetched_at: chrono::Utc::now(),
        };

        assert!(document_matches_render_mode(&document, false));
        assert!(!document_matches_render_mode(&document, true));

        document.rendered = true;
        assert!(document_matches_render_mode(&document, false));
        assert!(document_matches_render_mode(&document, true));
    }
}
