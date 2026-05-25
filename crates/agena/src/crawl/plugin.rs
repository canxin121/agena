use std::future::Future;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_crawl::{
    BrowserRenderOptions, CrawlPageFetcher, CrawlRunOptions, CrawlRunReport, CrawlStore,
    FetchedPage, LocalBrowserOptions, SpiderFetchOptions, StoredDocument, WebSearchEngine,
    WebSearchOptions, WebSearchResult, crawl_site, ensure_index_exists, fetch_page_with_spider,
    prepare_fetch_url, preview_text, results_to_text, search_web,
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
    description = "Preferred local web discovery and ingestion tool. Use action `search` for direct web search, `fetch` for one page, `crawl` to build a local index, `query` to search that index, `get` to inspect one stored page, or `list` to inspect the local crawl catalog.",
    summary = "Local web search, crawling, fetch, and embedded crawl index.",
    help = "This tool performs direct web search with embedded ferris-style search code, fetches pages with Spider, stores extracted markdown under the current workspace's Agena data directory, and queries the local Tantivy index. It does not use Firecrawl, Brave API, or any remote search API key service.",
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
    #[tool(exec = "search")]
    Search {
        #[serde(flatten)]
        args: CrawlWebSearchInput,
    },
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
    #[tool(exec = "query")]
    Query {
        #[serde(flatten)]
        args: CrawlQueryInput,
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
    prompt: Option<String>,
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
struct CrawlWebSearchInput {
    query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_results: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    engine: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    blocked_domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CrawlQueryInput {
    query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_results: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    blocked_domains: Vec<String>,
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
struct CrawlWebSearchOutput {
    query: String,
    engine: String,
    results: Vec<WebSearchResult>,
}

#[derive(Debug, Serialize)]
struct CrawlQueryOutput {
    query: String,
    results: Vec<agena_crawl::CrawlSearchHit>,
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
        let store = self.store()?;
        let _guard = self.sync_lock.lock().await;
        let options = CrawlRunOptions {
            max_pages: clamp_limit(
                input.max_pages,
                self.config.default_max_pages as usize,
                self.config.max_pages_limit as usize,
            ),
            max_depth: input
                .max_depth
                .unwrap_or(self.config.default_max_depth)
                .clamp(0, self.config.max_depth_limit),
            same_host_only: input
                .same_host_only
                .unwrap_or(self.config.default_same_host_only),
            use_cache: input.use_cache.unwrap_or(true),
            render_js: input.render_js.unwrap_or(self.config.browser_enabled),
            document_cache_ttl: Duration::from_secs(self.config.document_cache_ttl_secs),
            max_chunk_chars: self.config.default_chunk_chars as usize,
            near_duplicate_hamming_distance: self.config.near_duplicate_hamming_distance,
        };
        let fetcher = PluginPageFetcher { plugin: self };
        let report = crawl_site(&start_url, &store, &options, &fetcher)
            .await
            .map_err(crawl_error_to_plugin)?;
        let text = format_crawl_run(&report);
        let payload =
            serde_json::to_value(report).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(text)
            .with_title("crawl run")
            .with_payload(payload))
    }

    async fn invoke_search(&self, input: &CrawlWebSearchInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.trim();
        if query.is_empty() {
            return Err(PluginError::invalid_params(
                "crawl search requires a non-empty query",
            ));
        }
        let limit = clamp_limit(
            input.limit.or(input.max_results),
            self.config.search_default_limit as usize,
            self.config.search_max_limit as usize,
        );
        let engine = input
            .engine
            .as_deref()
            .unwrap_or(self.config.search_engine.as_str())
            .parse::<WebSearchEngine>()
            .map_err(crawl_error_to_plugin)?;
        let engine_url = url::Url::parse(engine.permission_url())
            .map_err(|err| PluginError::new(err.to_string()))?;
        if let Some(host) = engine_url.host_str() {
            self.host_limiter.until_key_ready(&host.to_string()).await;
        }
        self.ensure_network_permission(&engine_url).await?;
        let mut options = WebSearchOptions {
            engine,
            limit,
            timeout: Duration::from_secs(self.config.fetch_timeout_secs),
            user_agent: crate::provider::CLAUDE_USER_WEB_FETCH_USER_AGENT.to_string(),
        };
        options.limit = limit;
        let results = search_web(query, &options)
            .await
            .map_err(crawl_error_to_plugin)?
            .into_iter()
            .filter(|result| {
                domain_allowed(&result.url, &input.allowed_domains, &input.blocked_domains)
            })
            .take(limit)
            .collect::<Vec<_>>();
        let output = CrawlWebSearchOutput {
            query: query.to_string(),
            engine: engine.as_str().to_string(),
            results,
        };
        let text = format_web_search(&output);
        let payload =
            serde_json::to_value(output).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(text)
            .with_title("crawl search")
            .with_payload(payload))
    }

    async fn invoke_query(&self, input: &CrawlQueryInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.trim();
        if query.is_empty() {
            return Err(PluginError::invalid_params(
                "crawl query requires a non-empty query",
            ));
        }
        let limit = clamp_limit(
            input.limit.or(input.max_results),
            self.config.search_default_limit as usize,
            self.config.search_max_limit as usize,
        );
        let store = self.store()?;
        ensure_index_exists(&store).map_err(crawl_error_to_plugin)?;
        let hits = store
            .search(query, limit)
            .map_err(crawl_error_to_plugin)?
            .into_iter()
            .filter(|hit| domain_allowed(&hit.url, &input.allowed_domains, &input.blocked_domains))
            .take(limit)
            .collect::<Vec<_>>();
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
        let output = CrawlQueryOutput {
            query: query.to_string(),
            results: hits,
        };
        let payload =
            serde_json::to_value(output).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(lines.join("\n"))
            .with_title("crawl query")
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
            CrawlToolInput::Search { args } => self.invoke_search(&args).await,
            CrawlToolInput::Fetch { args } => self.invoke_fetch(&args).await,
            CrawlToolInput::Crawl { args } => self.invoke_crawl(&args).await,
            CrawlToolInput::Query { args } => self.invoke_query(&args).await,
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
            CrawlToolInput::Search { .. } => return Ok(Vec::new()),
            CrawlToolInput::Fetch { .. } => return Ok(Vec::new()),
            CrawlToolInput::Crawl { .. } => PathRequest::write(path),
            CrawlToolInput::Query { .. } => PathRequest::write(path),
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
            CrawlToolInput::Search { args } => {
                let engine = args
                    .engine
                    .as_deref()
                    .unwrap_or(self.config.search_engine.as_str())
                    .parse::<WebSearchEngine>()
                    .map_err(crawl_error_to_plugin)?;
                vec![NetworkRequest::connect(engine.permission_url().to_string())]
            }
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
            CrawlToolInput::Query { .. }
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

struct PluginPageFetcher<'a> {
    plugin: &'a CrawlPlugin,
}

impl CrawlPageFetcher for PluginPageFetcher<'_> {
    fn fetch_page<'a>(
        &'a self,
        url: &'a url::Url,
        use_cache: bool,
        render_js: bool,
    ) -> Pin<Box<dyn Future<Output = Result<FetchedPage, agena_crawl::CrawlError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.plugin
                .fetch_page(url, use_cache, render_js)
                .await
                .map_err(|err| agena_crawl::CrawlError::InvalidInput(err.to_string()))
        })
    }
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

fn fetch_cache_key(url: &url::Url, render_js: bool) -> String {
    format!(
        "spider:{}:{}",
        if render_js { "rendered" } else { "plain" },
        url
    )
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

fn format_web_search(output: &CrawlWebSearchOutput) -> String {
    if output.results.is_empty() {
        return format!(
            "No web search result(s) for '{}' via {}.",
            output.query, output.engine
        );
    }
    format!(
        "Found {} web search result(s) for '{}' via {}.\n\n{}",
        output.results.len(),
        output.query,
        output.engine,
        results_to_text(&output.results)
    )
}

fn format_crawl_run(output: &CrawlRunReport) -> String {
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

fn domain_allowed(url: &str, allow: &[String], block: &[String]) -> bool {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if !allow.is_empty() && !allow.iter().any(|domain| host_matches(&host, domain)) {
        return false;
    }
    if block.iter().any(|domain| host_matches(&host, domain)) {
        return false;
    }
    true
}

fn host_matches(host: &str, pattern: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    !pattern.is_empty() && (host == pattern || host.ends_with(&format!(".{pattern}")))
}

#[cfg(test)]
mod tests {
    use super::{crawl_decl, fetch_cache_key};
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

        assert!(agena_crawl::document_matches_render_mode(&document, false));
        assert!(!agena_crawl::document_matches_render_mode(&document, true));

        document.rendered = true;
        assert!(agena_crawl::document_matches_render_mode(&document, false));
        assert!(agena_crawl::document_matches_render_mode(&document, true));
    }
}
