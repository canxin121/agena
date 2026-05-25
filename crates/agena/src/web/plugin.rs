use std::future::Future;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_macros::StaticToolSurface;
use agena_web::{
    BrowserRenderOptions, CrawlPageFetcher, CrawlRunOptions, CrawlRunReport, CrawlStore,
    CrawlStoreRetention, FetchedPage, LocalBrowserOptions, SpiderFetchOptions, StoredDocument,
    WebSearchEngine, WebSearchOptions, WebSearchResult, crawl_site, ensure_index_exists,
    fetch_page_with_spider, prepare_fetch_url, preview_text, results_to_text, search_web,
};
use async_trait::async_trait;
use governor::{DefaultKeyedRateLimiter, Quota};
use moka::future::Cache;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostNetworkPermissionCheckRequest};
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, NetworkRequest, PathRequest,
    Plugin, PluginManifest, PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
    ToolTag,
};

pub const WEB_PLUGIN_ID: &str = "agena.web";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebConfig {
    #[serde(default = "default_web_fetch_enabled")]
    pub fetch_enabled: bool,
    pub default_max_pages: u32,
    pub max_pages_limit: u32,
    pub default_max_depth: u32,
    pub max_depth_limit: u32,
    pub default_same_host_only: bool,
    pub request_delay_ms: u64,
    pub fetch_timeout_secs: u64,
    pub max_body_bytes: u64,
    pub respect_robots_txt: bool,
    pub document_cache_ttl_secs: u64,
    pub fetch_cache_ttl_secs: u64,
    pub fetch_cache_capacity: u64,
    pub store_max_documents: u32,
    pub store_max_bytes: u64,
    pub default_chunk_chars: u32,
    pub near_duplicate_hamming_distance: u32,
    pub search_default_limit: u32,
    pub search_max_limit: u32,
    pub list_default_limit: u32,
    pub list_max_limit: u32,
    pub browser_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_executable_path: Option<String>,
    pub browser_wait_for_network_idle: bool,
    pub browser_wait_timeout_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_wait_for_selector: Option<String>,
    pub browser_wait_for_delay_ms: u64,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            fetch_enabled: true,
            default_max_pages: 10,
            max_pages_limit: 100,
            default_max_depth: 1,
            max_depth_limit: 4,
            default_same_host_only: true,
            request_delay_ms: 400,
            fetch_timeout_secs: 30,
            max_body_bytes: 5 * 1024 * 1024,
            respect_robots_txt: true,
            document_cache_ttl_secs: 24 * 60 * 60,
            fetch_cache_ttl_secs: 15 * 60,
            fetch_cache_capacity: 128,
            store_max_documents: 200,
            store_max_bytes: 100 * 1024 * 1024,
            default_chunk_chars: 1800,
            near_duplicate_hamming_distance: 3,
            search_default_limit: 5,
            search_max_limit: 20,
            list_default_limit: 20,
            list_max_limit: 100,
            browser_enabled: false,
            browser_executable_path: None,
            browser_wait_for_network_idle: true,
            browser_wait_timeout_secs: 10,
            browser_wait_for_selector: None,
            browser_wait_for_delay_ms: 0,
        }
    }
}

fn default_web_fetch_enabled() -> bool {
    true
}

pub struct WebPlugin {
    state: OnceLock<WebPluginState>,
    workspace_root: OnceLock<PathBuf>,
    host: OnceLock<Arc<dyn HostClient>>,
    sync_lock: Mutex<()>,
}

struct WebPluginState {
    config: WebConfig,
    fetch_cache: Cache<String, FetchedPage>,
    host_limiter: DefaultKeyedRateLimiter<String>,
}

impl WebPluginState {
    fn new(config: WebConfig) -> Self {
        Self {
            fetch_cache: Cache::builder()
                .time_to_live(Duration::from_secs(config.fetch_cache_ttl_secs))
                .max_capacity(config.fetch_cache_capacity)
                .build(),
            host_limiter: build_host_limiter(config.request_delay_ms),
            config,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "web",
    description = "Preferred local web discovery and ingestion tool. Use action `search` for direct web search, `fetch` for one page, `crawl` to build a local index, `query` to search that index, `get` to inspect one stored page, or `list` to inspect the local crawl catalog. For `search`, choose engine `duckduckgo`, `bing`, `baidu`, or `auto` per query.",
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
enum WebToolInput {
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
    engine: Option<WebSearchEngineSelection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    blocked_domains: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WebSearchEngineSelection {
    Auto,
    Bing,
    #[serde(rename = "duckduckgo", alias = "duck_duck_go", alias = "ddg")]
    DuckDuckGo,
    Baidu,
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
    attempted_engines: Vec<String>,
    results: Vec<WebSearchResult>,
}

#[derive(Debug, Serialize)]
struct CrawlQueryOutput {
    query: String,
    results: Vec<agena_web::CrawlSearchHit>,
}

impl WebPlugin {
    pub fn new() -> Self {
        Self {
            state: OnceLock::new(),
            workspace_root: OnceLock::new(),
            host: OnceLock::new(),
            sync_lock: Mutex::new(()),
        }
    }

    fn state(&self) -> SdkResult<&WebPluginState> {
        self.state
            .get()
            .ok_or_else(|| PluginError::new("web plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&WebConfig> {
        Ok(&self.state()?.config)
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::new("web plugin invoked before init"))
    }

    fn host(&self) -> SdkResult<&Arc<dyn HostClient>> {
        self.host
            .get()
            .ok_or_else(|| PluginError::new("web plugin host not initialized"))
    }

    fn store(&self) -> SdkResult<CrawlStore> {
        Ok(CrawlStore::for_workspace(self.workspace_root()?))
    }

    fn spider_fetch_options(&self, rendered: bool) -> SdkResult<SpiderFetchOptions> {
        let config = self.config()?;
        let delay = (config.browser_wait_for_delay_ms > 0)
            .then(|| Duration::from_millis(config.browser_wait_for_delay_ms));
        Ok(SpiderFetchOptions {
            max_body_bytes: config.max_body_bytes as usize,
            timeout: Duration::from_secs(config.fetch_timeout_secs),
            delay_ms: config.request_delay_ms,
            user_agent: crate::provider::CLAUDE_USER_WEB_FETCH_USER_AGENT.to_string(),
            respect_robots_txt: config.respect_robots_txt,
            browser: BrowserRenderOptions {
                enabled: rendered,
                local_browser: LocalBrowserOptions {
                    executable_path: config
                        .browser_executable_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(PathBuf::from),
                    startup_timeout: Duration::from_secs(config.browser_wait_timeout_secs),
                },
                wait_for_network_idle: config.browser_wait_for_network_idle,
                wait_for_selector: config.browser_wait_for_selector.clone(),
                wait_timeout: Duration::from_secs(config.browser_wait_timeout_secs),
                delay,
            },
        })
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
        let state = self.state()?;
        let cache_key = fetch_cache_key(url, render_js);
        if use_cache && let Some(hit) = state.fetch_cache.get(cache_key.as_str()).await {
            return Ok(hit);
        }
        if let Some(host) = url.host_str() {
            state.host_limiter.until_key_ready(&host.to_string()).await;
        }
        self.ensure_network_permission(url).await?;
        let options = self.spider_fetch_options(render_js)?;
        let page = fetch_page_with_spider(url, &options)
            .await
            .map_err(crawl_error_to_plugin)?;
        if use_cache {
            state.fetch_cache.insert(cache_key, page.clone()).await;
        }
        Ok(page)
    }

    async fn invoke_fetch(&self, input: &CrawlFetchInput) -> SdkResult<ToolInvokeOutput> {
        let url = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?;
        let config = self.config()?;
        let render_js = input.render_js.unwrap_or(config.browser_enabled);
        let page = self
            .fetch_page(&url, input.use_cache.unwrap_or(true), render_js)
            .await?;
        let payload =
            serde_json::to_value(&page).map_err(|err| PluginError::new(err.to_string()))?;
        let text = format_fetched_page(&page);
        Ok(ToolInvokeOutput::text(text)
            .with_title("web fetch")
            .with_payload(payload))
    }

    async fn invoke_crawl(&self, input: &CrawlRunInput) -> SdkResult<ToolInvokeOutput> {
        let start_url =
            prepare_fetch_url(input.start_url.as_str()).map_err(crawl_error_to_plugin)?;
        let store = self.store()?;
        let _guard = self.sync_lock.lock().await;
        let config = self.config()?;
        let options = CrawlRunOptions {
            max_pages: clamp_limit(
                input.max_pages,
                config.default_max_pages as usize,
                config.max_pages_limit as usize,
            ),
            max_depth: input
                .max_depth
                .unwrap_or(config.default_max_depth)
                .clamp(0, config.max_depth_limit),
            same_host_only: input
                .same_host_only
                .unwrap_or(config.default_same_host_only),
            use_cache: input.use_cache.unwrap_or(true),
            render_js: input.render_js.unwrap_or(config.browser_enabled),
            document_cache_ttl: Duration::from_secs(config.document_cache_ttl_secs),
            max_chunk_chars: config.default_chunk_chars as usize,
            near_duplicate_hamming_distance: config.near_duplicate_hamming_distance,
            store_retention: Some(CrawlStoreRetention {
                max_documents: config.store_max_documents as usize,
                max_total_bytes: config.store_max_bytes,
            }),
        };
        let fetcher = PluginPageFetcher { plugin: self };
        let report = crawl_site(&start_url, &store, &options, &fetcher)
            .await
            .map_err(crawl_error_to_plugin)?;
        let text = format_crawl_run(&report);
        let payload =
            serde_json::to_value(report).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(text)
            .with_title("web crawl")
            .with_payload(payload))
    }

    async fn invoke_search(&self, input: &CrawlWebSearchInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.trim();
        if query.is_empty() {
            return Err(PluginError::invalid_params(
                "web search requires a non-empty query",
            ));
        }
        let config = self.config()?;
        let limit = clamp_limit(
            input.limit.or(input.max_results),
            config.search_default_limit as usize,
            config.search_max_limit as usize,
        );
        let engines = search_engines(input.engine);
        let explicit_engine = !matches!(input.engine, None | Some(WebSearchEngineSelection::Auto));
        let mut attempted_engines = Vec::new();
        let mut last_error = None;
        let mut selected_engine = WebSearchEngineSelection::Auto.label().to_string();
        let mut results = Vec::new();

        for engine in engines {
            attempted_engines.push(engine.as_str().to_string());
            match self.search_with_engine(query, limit, engine, input).await {
                Ok(engine_results) => {
                    if explicit_engine || !engine_results.is_empty() {
                        selected_engine = engine.as_str().to_string();
                        results = engine_results;
                        break;
                    }
                }
                Err(err) if explicit_engine => return Err(err),
                Err(err) => {
                    last_error = Some(format!("{}: {}", engine.as_str(), err));
                }
            }
        }

        if results.is_empty()
            && !explicit_engine
            && attempted_engines.len() == search_engines(input.engine).len()
            && let Some(error) = last_error
        {
            tracing::debug!(target: "agena::web", %error, "auto web search exhausted engines");
        }

        let output = CrawlWebSearchOutput {
            query: query.to_string(),
            engine: selected_engine,
            attempted_engines,
            results,
        };
        let text = format_web_search(&output);
        let payload =
            serde_json::to_value(output).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(text)
            .with_title("web search")
            .with_payload(payload))
    }

    async fn search_with_engine(
        &self,
        query: &str,
        limit: usize,
        engine: WebSearchEngine,
        input: &CrawlWebSearchInput,
    ) -> SdkResult<Vec<WebSearchResult>> {
        let engine_url = url::Url::parse(engine.permission_url())
            .map_err(|err| PluginError::new(err.to_string()))?;
        let state = self.state()?;
        if let Some(host) = engine_url.host_str() {
            state.host_limiter.until_key_ready(&host.to_string()).await;
        }
        self.ensure_network_permission(&engine_url).await?;
        let config = &state.config;
        let options = WebSearchOptions {
            engine,
            limit,
            timeout: Duration::from_secs(config.fetch_timeout_secs),
            user_agent: crate::provider::CLAUDE_USER_WEB_FETCH_USER_AGENT.to_string(),
        };
        Ok(search_web(query, &options)
            .await
            .map_err(crawl_error_to_plugin)?
            .into_iter()
            .filter(|result| {
                domain_allowed(&result.url, &input.allowed_domains, &input.blocked_domains)
            })
            .take(limit)
            .collect())
    }

    async fn invoke_query(&self, input: &CrawlQueryInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.trim();
        if query.is_empty() {
            return Err(PluginError::invalid_params(
                "web query requires a non-empty query",
            ));
        }
        let config = self.config()?;
        let limit = clamp_limit(
            input.limit.or(input.max_results),
            config.search_default_limit as usize,
            config.search_max_limit as usize,
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
            .with_title("web query")
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
                .ok_or_else(|| PluginError::new(format!("web document for '{url}' not found")))?,
            _ => {
                return Err(PluginError::invalid_params(
                    "web get requires exactly one of `id` or `url`",
                ));
            }
        };
        let payload =
            serde_json::to_value(&document).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(format_document(&document))
            .with_title("web get")
            .with_payload(payload))
    }

    async fn invoke_list(&self, input: &CrawlListInput) -> SdkResult<ToolInvokeOutput> {
        let config = self.config()?;
        let limit = clamp_limit(
            input.limit,
            config.list_default_limit as usize,
            config.list_max_limit as usize,
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
            .with_title("web list")
            .with_payload(payload))
    }
}

#[async_trait]
impl Plugin for WebPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(WEB_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description(
                "Local web search/fetch/crawl plugin with embedded storage, caching, and Tantivy retrieval.",
            )
            .hooks(HookSubscription::INIT | HookSubscription::TOOL_INVOKE)
            .config_schema(crate::entry::definition::json_schema_for::<WebConfig>())
            .tool(web_decl())
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config = parse_web_config(ctx.config)?;
        self.state
            .set(WebPluginState::new(config))
            .map_err(|_| PluginError::new("web plugin initialized more than once"))?;
        let _ = self.workspace_root.set(ctx.workspace_root);
        let _ = self.host.set(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        if input.tool_name != "web" {
            return Err(PluginError::invalid_params(format!(
                "unknown web plugin tool '{}'",
                input.tool_name
            )));
        }
        match parse_web_input(input.input)? {
            WebToolInput::Search { args } => self.invoke_search(&args).await,
            WebToolInput::Fetch { args } => self.invoke_fetch(&args).await,
            WebToolInput::Crawl { args } => self.invoke_crawl(&args).await,
            WebToolInput::Query { args } => self.invoke_query(&args).await,
            WebToolInput::Get { args } => self.invoke_get(&args).await,
            WebToolInput::List { args } => self.invoke_list(&args).await,
        }
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        if tool != "web" {
            return Ok(Vec::new());
        }
        let parsed = parse_web_input(input.clone())?;
        let store = self.store()?;
        let path = store.dir().display().to_string();
        let request = match parsed {
            WebToolInput::Search { .. } => return Ok(Vec::new()),
            WebToolInput::Fetch { .. } => return Ok(Vec::new()),
            WebToolInput::Crawl { .. } => PathRequest::write(path),
            WebToolInput::Query { .. } => PathRequest::write(path),
            WebToolInput::Get { .. } | WebToolInput::List { .. } => PathRequest::read(path),
        };
        Ok(vec![request])
    }

    async fn permission_networks(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<NetworkRequest>> {
        if tool != "web" {
            return Ok(Vec::new());
        }
        let parsed = parse_web_input(input.clone())?;
        let requests = match parsed {
            WebToolInput::Search { args } => search_engines(args.engine)
                .into_iter()
                .map(|engine| NetworkRequest::connect(engine.permission_url().to_string()))
                .collect(),
            WebToolInput::Fetch { args } => vec![NetworkRequest::connect(
                prepare_fetch_url(args.url.as_str())
                    .map_err(crawl_error_to_plugin)?
                    .to_string(),
            )],
            WebToolInput::Crawl { args } => vec![NetworkRequest::connect(
                prepare_fetch_url(args.start_url.as_str())
                    .map_err(crawl_error_to_plugin)?
                    .to_string(),
            )],
            WebToolInput::Query { .. } | WebToolInput::Get { .. } | WebToolInput::List { .. } => {
                Vec::new()
            }
        };
        Ok(requests)
    }
}

fn web_decl() -> PluginToolDecl {
    WebToolInput::tool_decl()
}

fn parse_web_input(input: serde_json::Value) -> SdkResult<WebToolInput> {
    if matches!(&input, serde_json::Value::Array(items) if items.is_empty()) {
        return Ok(WebToolInput::List {
            args: CrawlListInput { limit: None },
        });
    }
    WebToolInput::parse_input(input)
}

fn crawl_error_to_plugin(err: agena_web::CrawlError) -> PluginError {
    PluginError::new(err.to_string())
}

struct PluginPageFetcher<'a> {
    plugin: &'a WebPlugin,
}

impl CrawlPageFetcher for PluginPageFetcher<'_> {
    fn fetch_page<'a>(
        &'a self,
        url: &'a url::Url,
        use_cache: bool,
        render_js: bool,
    ) -> Pin<Box<dyn Future<Output = Result<FetchedPage, agena_web::CrawlError>> + Send + 'a>> {
        Box::pin(async move {
            self.plugin
                .fetch_page(url, use_cache, render_js)
                .await
                .map_err(|err| agena_web::CrawlError::InvalidInput(err.to_string()))
        })
    }
}

fn clamp_limit(limit: Option<u32>, default_limit: usize, max_limit: usize) -> usize {
    limit
        .unwrap_or(default_limit as u32)
        .clamp(1, max_limit as u32) as usize
}

fn parse_web_config(value: serde_json::Value) -> SdkResult<WebConfig> {
    let config = if value.is_null() {
        WebConfig::default()
    } else {
        serde_json::from_value(value)
            .map_err(|err| PluginError::new(format!("invalid web plugin config: {err}")))?
    };
    validate_web_config(&config)?;
    Ok(config)
}

fn validate_web_config(web: &WebConfig) -> SdkResult<()> {
    for (label, value) in [
        ("default_max_pages", web.default_max_pages),
        ("max_pages_limit", web.max_pages_limit),
        ("max_depth_limit", web.max_depth_limit),
        ("default_chunk_chars", web.default_chunk_chars),
        (
            "near_duplicate_hamming_distance",
            web.near_duplicate_hamming_distance,
        ),
        ("search_default_limit", web.search_default_limit),
        ("search_max_limit", web.search_max_limit),
        ("list_default_limit", web.list_default_limit),
        ("list_max_limit", web.list_max_limit),
    ] {
        if value == 0 {
            return Err(PluginError::new(format!(
                "web plugin config `{label}` must be greater than 0"
            )));
        }
    }
    for (label, value) in [
        ("request_delay_ms", web.request_delay_ms),
        ("fetch_timeout_secs", web.fetch_timeout_secs),
        ("max_body_bytes", web.max_body_bytes),
        ("browser_wait_timeout_secs", web.browser_wait_timeout_secs),
        ("document_cache_ttl_secs", web.document_cache_ttl_secs),
        ("fetch_cache_ttl_secs", web.fetch_cache_ttl_secs),
        ("fetch_cache_capacity", web.fetch_cache_capacity),
        ("store_max_bytes", web.store_max_bytes),
    ] {
        if value == 0 {
            return Err(PluginError::new(format!(
                "web plugin config `{label}` must be greater than 0"
            )));
        }
    }
    if web.store_max_documents == 0 {
        return Err(PluginError::new(
            "web plugin config `store_max_documents` must be greater than 0",
        ));
    }
    if web.default_max_pages > web.max_pages_limit {
        return Err(PluginError::new(
            "web plugin config `default_max_pages` must be less than or equal to `max_pages_limit`",
        ));
    }
    if web.default_max_depth > web.max_depth_limit {
        return Err(PluginError::new(
            "web plugin config `default_max_depth` must be less than or equal to `max_depth_limit`",
        ));
    }
    if web.search_default_limit > web.search_max_limit {
        return Err(PluginError::new(
            "web plugin config `search_default_limit` must be less than or equal to `search_max_limit`",
        ));
    }
    if web.list_default_limit > web.list_max_limit {
        return Err(PluginError::new(
            "web plugin config `list_default_limit` must be less than or equal to `list_max_limit`",
        ));
    }
    if web
        .browser_executable_path
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(PluginError::new(
            "web plugin config `browser_executable_path` must not be empty when set",
        ));
    }
    if web
        .browser_wait_for_selector
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(PluginError::new(
            "web plugin config `browser_wait_for_selector` must not be empty when set",
        ));
    }
    Ok(())
}

fn build_host_limiter(delay_ms: u64) -> DefaultKeyedRateLimiter<String> {
    let quota = Quota::with_period(Duration::from_millis(delay_ms.max(1)))
        .expect("crawl request delay must be non-zero")
        .allow_burst(NonZeroU32::new(1).expect("non-zero"));
    DefaultKeyedRateLimiter::keyed(quota)
}

fn search_engines(selection: Option<WebSearchEngineSelection>) -> Vec<WebSearchEngine> {
    match selection {
        Some(WebSearchEngineSelection::Bing) => vec![WebSearchEngine::Bing],
        Some(WebSearchEngineSelection::DuckDuckGo) => vec![WebSearchEngine::DuckDuckGo],
        Some(WebSearchEngineSelection::Baidu) => vec![WebSearchEngine::Baidu],
        Some(WebSearchEngineSelection::Auto) | None => vec![
            WebSearchEngine::DuckDuckGo,
            WebSearchEngine::Bing,
            WebSearchEngine::Baidu,
        ],
    }
}

impl WebSearchEngineSelection {
    fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Bing => "bing",
            Self::DuckDuckGo => "duckduckgo",
            Self::Baidu => "baidu",
        }
    }
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
        if output.engine == "auto" {
            return format!(
                "No web search result(s) for '{}' via auto. Tried: {}.",
                output.query,
                output.attempted_engines.join(", ")
            );
        }
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
        "Crawled from {} via {} (rendered: {}). New pages stored: {}. Cached pages reused: {}. Exact duplicates skipped: {}. Near duplicates skipped: {}. Old pages pruned: {} ({} bytes). Failures: {}. Total indexed documents: {}.",
        output.start_url,
        output.engine,
        if output.rendered { "yes" } else { "no" },
        output.stored_count,
        output.cached_count,
        output.duplicate_count,
        output.near_duplicate_count,
        output.pruned_document_count,
        output.pruned_document_bytes,
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
    use super::{WebToolInput, fetch_cache_key, parse_web_input, search_engines, web_decl};
    use crate::plugin::sdk::HostCapability;

    #[test]
    fn web_decl_declares_permission_check_host_capability() {
        let decl = web_decl();
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
    fn empty_array_input_defaults_to_list_action() {
        let parsed = parse_web_input(serde_json::json!([])).expect("empty array should parse");
        assert!(matches!(parsed, WebToolInput::List { .. }));
    }

    #[test]
    fn omitted_search_engine_uses_auto_fallback_order() {
        let engines = search_engines(None);
        assert_eq!(
            engines,
            vec![
                agena_web::WebSearchEngine::DuckDuckGo,
                agena_web::WebSearchEngine::Bing,
                agena_web::WebSearchEngine::Baidu,
            ]
        );
    }

    #[test]
    fn plain_fetch_can_reuse_rendered_documents_but_not_reverse() {
        let mut document = agena_web::StoredDocument {
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

        assert!(agena_web::document_matches_render_mode(&document, false));
        assert!(!agena_web::document_matches_render_mode(&document, true));

        document.rendered = true;
        assert!(agena_web::document_matches_render_mode(&document, false));
        assert!(agena_web::document_matches_render_mode(&document, true));
    }
}
