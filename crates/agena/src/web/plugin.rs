use std::collections::BTreeSet;
use std::future::Future;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_macros::ToolInput;
use agena_web::{
    BrowserRenderOptions, CrawlPageFetcher, CrawlRunOptions, CrawlRunReport, CrawlStore,
    CrawlStoreRetention, FetchedPage, LocalBrowserOptions, SpiderFetchOptions, WebSearchEngine,
    WebSearchOptions, WebSearchResult, crawl_site, fetch_page_with_spider, prepare_fetch_url,
    preview_text, results_to_text, search_web,
};
use governor::{DefaultKeyedRateLimiter, Quota};
use moka::future::Cache;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostNetworkPermissionCheckRequest};
use crate::plugin::sdk::{HostCapability, Result as SdkResult, ToolInvokeOutput};

pub const WEB_PLUGIN_ID: &str = "agena.web";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebConfig {
    pub fetch: WebFetchConfig,
    pub crawl: WebCrawlConfig,
    pub search: WebSearchConfig,
    pub store: WebStoreConfig,
    pub browser: WebBrowserConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebFetchConfig {
    #[serde(default = "default_web_fetch_enabled")]
    pub enabled: bool,
    pub request: WebRequestConfig,
    pub cache: WebFetchCacheConfig,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            request: WebRequestConfig::default(),
            cache: WebFetchCacheConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebRequestConfig {
    pub delay_ms: u64,
    pub timeout_secs: u64,
    pub max_body_bytes: u64,
    pub respect_robots_txt: bool,
}

impl Default for WebRequestConfig {
    fn default() -> Self {
        Self {
            delay_ms: 400,
            timeout_secs: 30,
            max_body_bytes: 5 * 1024 * 1024,
            respect_robots_txt: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebFetchCacheConfig {
    pub ttl_secs: u64,
    pub capacity: u64,
}

impl Default for WebFetchCacheConfig {
    fn default() -> Self {
        Self {
            ttl_secs: 15 * 60,
            capacity: 128,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebCrawlConfig {
    pub defaults: WebCrawlDefaultsConfig,
    pub limits: WebCrawlLimitsConfig,
    pub indexing: WebCrawlIndexingConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebCrawlDefaultsConfig {
    pub max_pages: u32,
    pub max_depth: u32,
    pub same_host_only: bool,
}

impl Default for WebCrawlDefaultsConfig {
    fn default() -> Self {
        Self {
            max_pages: 10,
            max_depth: 1,
            same_host_only: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebCrawlLimitsConfig {
    pub max_pages: u32,
    pub max_depth: u32,
}

impl Default for WebCrawlLimitsConfig {
    fn default() -> Self {
        Self {
            max_pages: 100,
            max_depth: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebCrawlIndexingConfig {
    pub document_cache_ttl_secs: u64,
    pub chunk_chars: u32,
    pub near_duplicate_hamming_distance: u32,
}

impl Default for WebCrawlIndexingConfig {
    fn default() -> Self {
        Self {
            document_cache_ttl_secs: 24 * 60 * 60,
            chunk_chars: 1800,
            near_duplicate_hamming_distance: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebSearchConfig {
    pub default_limit: u32,
    pub max_limit: u32,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 5,
            max_limit: 20,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebStoreConfig {
    pub retention: WebStoreRetentionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebStoreRetentionConfig {
    pub max_documents: u32,
    pub max_bytes: u64,
}

impl Default for WebStoreRetentionConfig {
    fn default() -> Self {
        Self {
            max_documents: 200,
            max_bytes: 100 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebBrowserConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    pub wait: WebBrowserWaitConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebBrowserWaitConfig {
    pub for_network_idle: bool,
    pub timeout_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub for_selector: Option<String>,
    pub delay_ms: u64,
}

impl Default for WebBrowserWaitConfig {
    fn default() -> Self {
        Self {
            for_network_idle: true,
            timeout_secs: 10,
            for_selector: None,
            delay_ms: 0,
        }
    }
}

fn default_web_fetch_enabled() -> bool {
    true
}

fn web_config_schema() -> serde_json::Value {
    crate::tool::definition::json_schema_for_default_with_metadata(
        default_web_config(),
        &[
            (
                "",
                "Web Plugin Config",
                "Fetch, crawl, search, embedded cache, and browser defaults for the agena.web plugin.",
            ),
            (
                "/properties/fetch",
                "Fetch",
                "Controls direct page fetch operations, request throttling, and fetch cache behavior.",
            ),
            (
                "/properties/fetch/properties/enabled",
                "Enabled",
                "Allows web.fetch and web.crawl to run. Disable this to turn off network page retrieval.",
            ),
            (
                "/properties/fetch/properties/request",
                "Request",
                "Default timing, timeout, and body-size limits for HTTP page fetches.",
            ),
            (
                "/properties/fetch/properties/request/properties/delay_ms",
                "Delay (ms)",
                "Minimum delay between fetches to the same host.",
            ),
            (
                "/properties/fetch/properties/request/properties/timeout_secs",
                "Timeout (sec)",
                "Maximum time allowed for one fetch request before it fails.",
            ),
            (
                "/properties/fetch/properties/request/properties/max_body_bytes",
                "Max Body Bytes",
                "Largest response body accepted from a fetched page.",
            ),
            (
                "/properties/fetch/properties/request/properties/respect_robots_txt",
                "Respect robots.txt",
                "Honors robots.txt restrictions during fetch and crawl operations.",
            ),
            (
                "/properties/fetch/properties/cache",
                "Cache",
                "Short-lived cache for fetched page content and metadata.",
            ),
            (
                "/properties/fetch/properties/cache/properties/ttl_secs",
                "TTL (sec)",
                "How long cached fetch results stay valid.",
            ),
            (
                "/properties/fetch/properties/cache/properties/capacity",
                "Capacity",
                "Maximum number of cached fetch results kept in memory.",
            ),
            (
                "/properties/crawl",
                "Crawl",
                "Defaults, limits, and indexing behavior for site crawls.",
            ),
            (
                "/properties/crawl/properties/defaults",
                "Defaults",
                "Default crawl options used when callers omit them.",
            ),
            (
                "/properties/crawl/properties/defaults/properties/max_pages",
                "Max Pages",
                "Default page limit for one crawl run.",
            ),
            (
                "/properties/crawl/properties/defaults/properties/max_depth",
                "Max Depth",
                "Default traversal depth for one crawl run.",
            ),
            (
                "/properties/crawl/properties/defaults/properties/same_host_only",
                "Same Host Only",
                "Keeps crawls on the original host unless the caller opts out.",
            ),
            (
                "/properties/crawl/properties/limits",
                "Limits",
                "Hard upper bounds enforced for crawl requests.",
            ),
            (
                "/properties/crawl/properties/limits/properties/max_pages",
                "Max Pages Limit",
                "Largest page count any crawl request may ask for.",
            ),
            (
                "/properties/crawl/properties/limits/properties/max_depth",
                "Max Depth Limit",
                "Largest traversal depth any crawl request may ask for.",
            ),
            (
                "/properties/crawl/properties/indexing",
                "Indexing",
                "Controls how crawled documents are cached and chunked before storage.",
            ),
            (
                "/properties/crawl/properties/indexing/properties/document_cache_ttl_secs",
                "Document Cache TTL (sec)",
                "How long indexed crawl documents stay in the crawl document cache.",
            ),
            (
                "/properties/crawl/properties/indexing/properties/chunk_chars",
                "Chunk Size (chars)",
                "Target character size for indexed text chunks.",
            ),
            (
                "/properties/crawl/properties/indexing/properties/near_duplicate_hamming_distance",
                "Near-Duplicate Distance",
                "Similarity threshold used when suppressing near-duplicate crawl content.",
            ),
            (
                "/properties/search",
                "Search",
                "Default and maximum limits for web search result lists.",
            ),
            (
                "/properties/search/properties/default_limit",
                "Default Limit",
                "Number of web search results returned when callers omit a limit.",
            ),
            (
                "/properties/search/properties/max_limit",
                "Max Limit",
                "Largest number of web search results a caller may request.",
            ),
            (
                "/properties/store",
                "Cache",
                "Retention defaults for the embedded crawl document cache.",
            ),
            (
                "/properties/store/properties/retention",
                "Retention",
                "Maximum document count and byte size retained in the local crawl cache.",
            ),
            (
                "/properties/store/properties/retention/properties/max_documents",
                "Max Documents",
                "Maximum number of cached crawl documents retained locally.",
            ),
            (
                "/properties/store/properties/retention/properties/max_bytes",
                "Max Bytes",
                "Maximum total byte size retained by the local crawl cache.",
            ),
            (
                "/properties/browser",
                "Browser",
                "Optional browser rendering support for JavaScript-heavy pages.",
            ),
            (
                "/properties/browser/properties/enabled",
                "Enabled",
                "Allows rendered fetches and crawls to use a local browser.",
            ),
            (
                "/properties/browser/properties/executable_path",
                "Executable Path",
                "Optional browser executable path. Leave unset to use the default browser resolution logic.",
            ),
            (
                "/properties/browser/properties/wait",
                "Wait",
                "Browser rendering wait strategy applied before capturing page content.",
            ),
            (
                "/properties/browser/properties/wait/properties/for_network_idle",
                "Wait for Network Idle",
                "Waits for the page network to go idle before reading rendered content.",
            ),
            (
                "/properties/browser/properties/wait/properties/timeout_secs",
                "Timeout (sec)",
                "Maximum browser wait time before rendered capture fails.",
            ),
            (
                "/properties/browser/properties/wait/properties/for_selector",
                "Wait for Selector",
                "Optional CSS selector that must appear before rendered capture continues.",
            ),
            (
                "/properties/browser/properties/wait/properties/delay_ms",
                "Extra Delay (ms)",
                "Additional delay after wait conditions succeed before capturing the page.",
            ),
        ],
    )
}

fn default_web_config() -> WebConfig {
    WebConfig {
        fetch: WebFetchConfig::default(),
        crawl: WebCrawlConfig::default(),
        search: WebSearchConfig::default(),
        store: WebStoreConfig::default(),
        browser: WebBrowserConfig::default(),
    }
}

#[derive(Default)]
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
                .time_to_live(Duration::from_secs(config.fetch.cache.ttl_secs))
                .max_capacity(config.fetch.cache.capacity)
                .build(),
            host_limiter: build_host_limiter(config.fetch.request.delay_ms),
            config,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("url", "prompt"),
    non_empty("url"),
    non_empty_if_present("prompt")
)]
#[serde(deny_unknown_fields)]
struct CrawlFetchInput {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    #[serde(
        default = "crate::serde_helpers::default_true",
        skip_serializing_if = "crate::serde_helpers::is_true"
    )]
    use_cache: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    render_js: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("start_url"), non_empty("start_url"))]
#[serde(deny_unknown_fields)]
struct CrawlRunInput {
    start_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_pages: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_depth: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    same_host_only: Option<bool>,
    #[serde(
        default = "crate::serde_helpers::default_true",
        skip_serializing_if = "crate::serde_helpers::is_true"
    )]
    use_cache: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    render_js: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("query"), non_empty("query"))]
#[serde(deny_unknown_fields)]
struct CrawlWebSearchInput {
    query: String,
    /// Maximum number of results to return. `limit` remains accepted as a
    /// backwards-compatible input alias, but is deliberately omitted from
    /// the advertised schema so callers see one unambiguous control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(alias = "limit")]
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

#[derive(Debug, Serialize)]
struct CrawlWebSearchOutput {
    query: String,
    engine: String,
    attempted_engines: Vec<String>,
    results: Vec<WebSearchResult>,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "web",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Local web search/fetch/crawl plugin with an embedded crawl cache, deduplication, and optional browser rendering.",
    config_schema = web_config_schema(),
    display = brief_detailed
)]
impl WebPlugin {
    pub fn new() -> Self {
        Self {
            state: OnceLock::new(),
            workspace_root: OnceLock::new(),
            host: OnceLock::new(),
            sync_lock: Mutex::new(()),
        }
    }

    #[hook(init)]
    async fn init(
        &self,
        ctx: crate::plugin::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<crate::plugin::sdk::InitOutcome> {
        self.state
            .set(WebPluginState::new(parse_web_config(ctx.config)?))
            .map_err(|_| PluginError::new("web plugin initialized more than once"))?;
        self.host
            .set(host)
            .map_err(|_| PluginError::new("web plugin host initialized more than once"))?;
        self.workspace_root.set(ctx.workspace_root).map_err(|_| {
            PluginError::new("web plugin workspace root initialized more than once")
        })?;
        Ok(crate::plugin::sdk::InitOutcome::ack(
            crate::plugin::sdk::Plugin::manifest(self),
        ))
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
        let delay = (config.browser.wait.delay_ms > 0)
            .then(|| Duration::from_millis(config.browser.wait.delay_ms));
        Ok(SpiderFetchOptions {
            max_body_bytes: config.fetch.request.max_body_bytes as usize,
            timeout: Duration::from_secs(config.fetch.request.timeout_secs),
            delay_ms: config.fetch.request.delay_ms,
            user_agent: crate::provider::claude_user_web_fetch_user_agent(),
            respect_robots_txt: config.fetch.request.respect_robots_txt,
            browser: BrowserRenderOptions {
                enabled: rendered,
                local_browser: LocalBrowserOptions {
                    executable_path: config
                        .browser
                        .executable_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(PathBuf::from),
                    startup_timeout: Duration::from_secs(config.browser.wait.timeout_secs),
                },
                wait_for_network_idle: config.browser.wait.for_network_idle,
                wait_for_selector: config.browser.wait.for_selector.clone(),
                wait_timeout: Duration::from_secs(config.browser.wait.timeout_secs),
                delay,
            },
        })
    }

    async fn ensure_network_permission(&self, url: &url::Url) -> SdkResult<()> {
        let host = url
            .host_str()
            .ok_or_else(|| PluginError::invalid_params("web URL has no host"))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| PluginError::invalid_params("web URL has no known port"))?;
        let host_client = self.host()?;

        // Check the requested hostname first so explicit host rules remain
        // usable, then check every resolved address. The latter prevents a
        // hostname rule from silently becoming a private/loopback connection
        // through DNS resolution.
        host_client
            .ensure_network_permission(HostNetworkPermissionCheckRequest::connect(url.as_str()))
            .await?;

        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| PluginError::new(format!("failed to resolve {host}: {error}")))?
            .map(|address| address.ip())
            .collect::<BTreeSet<_>>();
        if addresses.is_empty() {
            return Err(PluginError::new(format!(
                "DNS resolution returned no addresses for {host}"
            )));
        }
        for address in addresses {
            if is_public_address(address) {
                continue;
            }
            let target = match address {
                std::net::IpAddr::V4(address) => format!("{address}:{port}"),
                std::net::IpAddr::V6(address) => format!("[{address}]:{port}"),
            };
            host_client
                .ensure_network_permission(HostNetworkPermissionCheckRequest::connect(target))
                .await?;
        }
        Ok(())
    }

    async fn fetch_page(
        &self,
        url: &url::Url,
        use_cache: bool,
        render_js: bool,
    ) -> SdkResult<FetchedPage> {
        let state = self.state()?;
        if !state.config.fetch.enabled {
            return Err(PluginError::new(
                "web fetching is disabled by plugin config `fetch.enabled`",
            ));
        }
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
        let final_url = url::Url::parse(page.canonical_url.as_str())
            .map_err(|error| PluginError::new(format!("invalid final fetch URL: {error}")))?;
        // Spider follows HTTP redirects internally. Do not return a response
        // whose final destination would fail the same network policy.
        self.ensure_network_permission(&final_url).await?;
        if use_cache {
            state.fetch_cache.insert(cache_key, page.clone()).await;
        }
        Ok(page)
    }

    #[tool(
        summary = "Fetch one web page and inspect its actual content.",
        help = "Use this tool after search when you need evidence from the actual page rather than search snippets. If you already know what facts you need, set `prompt` so Agena prioritizes the most relevant excerpts from the page in the returned text output.",
        read_only,
        network,
        internet,
        display = detailed,
        capabilities(HostCapability::PermissionCheck),
        examples(
            r#"{"url":"https://openai.com"}"#,
            r#"{"url":"https://example.com/docs","prompt":"extract the release date and breaking changes"}"#
        ),
        network(connect = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?.to_string()),
        concurrency_safe
    )]
    async fn invoke_fetch(&self, input: &CrawlFetchInput) -> SdkResult<ToolInvokeOutput> {
        let url = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?;
        let config = self.config()?;
        let render_js = input.render_js.unwrap_or(config.browser.enabled);
        let page = self.fetch_page(&url, input.use_cache, render_js).await?;
        let payload =
            serde_json::to_value(&page).map_err(|err| PluginError::new(err.to_string()))?;
        let text = format_fetched_page(&page, input.prompt.as_deref());
        Ok(ToolInvokeOutput::from_parts(
            format!("web fetch {}", url),
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Crawl a site and cache indexed pages locally.",
        mutating,
        network,
        internet,
        discovery,
        ui_display = detailed,
        capabilities(HostCapability::PermissionCheck),
        path(write = self.store_write_permission_path()?),
        network(connect = prepare_fetch_url(input.start_url.as_str()).map_err(crawl_error_to_plugin)?.to_string())
    )]
    async fn invoke_crawl(&self, input: &CrawlRunInput) -> SdkResult<ToolInvokeOutput> {
        let start_url =
            prepare_fetch_url(input.start_url.as_str()).map_err(crawl_error_to_plugin)?;
        let store = self.store()?;
        let _guard = self.sync_lock.lock().await;
        let config = self.config()?;
        let options = CrawlRunOptions {
            max_pages: clamp_limit(
                input.max_pages,
                config.crawl.defaults.max_pages as usize,
                config.crawl.limits.max_pages as usize,
            ),
            max_depth: input
                .max_depth
                .unwrap_or(config.crawl.defaults.max_depth)
                .clamp(0, config.crawl.limits.max_depth),
            same_host_only: input
                .same_host_only
                .unwrap_or(config.crawl.defaults.same_host_only),
            use_cache: input.use_cache,
            render_js: input.render_js.unwrap_or(config.browser.enabled),
            document_cache_ttl: Duration::from_secs(config.crawl.indexing.document_cache_ttl_secs),
            max_chunk_chars: config.crawl.indexing.chunk_chars as usize,
            near_duplicate_hamming_distance: config.crawl.indexing.near_duplicate_hamming_distance,
            store_retention: Some(CrawlStoreRetention {
                max_documents: config.store.retention.max_documents as usize,
                max_total_bytes: config.store.retention.max_bytes,
            }),
        };
        let fetcher = PluginPageFetcher { plugin: self };
        let report = crawl_site(&start_url, &store, &options, &fetcher)
            .await
            .map_err(crawl_error_to_plugin)?;
        let text = format_crawl_run(&report);
        let payload =
            serde_json::to_value(report).map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::from_parts(
            format!("web crawl {}", start_url),
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Find candidate public-web pages to fetch.",
        help = "Use this tool to discover candidate pages, not to answer from result snippets alone. After searching, fetch 1-3 relevant result URLs before answering when the user needs facts, summaries, comparisons, or latest information. Use allowed_domains and blocked_domains to steer source quality.",
        read_only,
        network,
        internet,
        discovery,
        display = detailed,
        capabilities(HostCapability::PermissionCheck),
        examples(
            r#"{"query":"Agena plugin architecture","max_results":5}"#,
            r#"{"query":"Rust schemars derive examples","allowed_domains":["docs.rs","github.com"]}"#
        ),
        network(connects = search_network_targets(input.engine)),
        concurrency_safe
    )]
    async fn invoke_search(&self, input: &CrawlWebSearchInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.as_str();
        let config = self.config()?;
        let limit = clamp_limit(
            input.max_results,
            config.search.default_limit as usize,
            config.search.max_limit as usize,
        );
        let engines = search_engines(input.engine);
        let explicit_engine = !matches!(input.engine, None | Some(WebSearchEngineSelection::Auto));
        let mut attempted_engines = Vec::new();
        let mut last_error = None;
        let mut selected_engine = WebSearchEngineSelection::Auto.label().to_string();
        let mut results = Vec::new();

        for engine in engines {
            attempted_engines.push(engine.to_string());
            match self.search_with_engine(query, limit, engine, input).await {
                Ok(engine_results) => {
                    if explicit_engine || !engine_results.is_empty() {
                        selected_engine = engine.to_string();
                        results = engine_results;
                        break;
                    }
                }
                Err(err) if explicit_engine => return Err(err),
                Err(err) => {
                    last_error = Some(format!("{engine}: {err}"));
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
        Ok(ToolInvokeOutput::from_parts(
            format!("web search {query}"),
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
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
            timeout: Duration::from_secs(config.fetch.request.timeout_secs),
            user_agent: crate::provider::claude_user_web_fetch_user_agent(),
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

    fn store_write_permission_path(&self) -> SdkResult<String> {
        let store = self.store()?;
        Ok(store.dir().display().to_string())
    }
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            !address.is_private()
                && !address.is_loopback()
                && !address.is_link_local()
                && address.octets()[0] != 0
                && address.octets()[0] < 224
        }
        IpAddr::V6(address) => {
            !address.is_loopback()
                && !address.is_unique_local()
                && !address.is_unicast_link_local()
                && !address.is_unspecified()
        }
    }
}

fn crawl_error_to_plugin(err: agena_web::CrawlError) -> PluginError {
    PluginError::new(err.to_string())
}

fn search_network_targets(engine: Option<WebSearchEngineSelection>) -> Vec<String> {
    search_engines(engine)
        .into_iter()
        .map(|engine| engine.permission_url().to_string())
        .collect()
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
        ("crawl.defaults.max_pages", web.crawl.defaults.max_pages),
        ("crawl.limits.max_pages", web.crawl.limits.max_pages),
        ("crawl.limits.max_depth", web.crawl.limits.max_depth),
        ("crawl.indexing.chunk_chars", web.crawl.indexing.chunk_chars),
        (
            "crawl.indexing.near_duplicate_hamming_distance",
            web.crawl.indexing.near_duplicate_hamming_distance,
        ),
        ("search.default_limit", web.search.default_limit),
        ("search.max_limit", web.search.max_limit),
    ] {
        if value == 0 {
            return Err(PluginError::new(format!(
                "web plugin config `{label}` must be greater than 0"
            )));
        }
    }
    for (label, value) in [
        ("fetch.request.delay_ms", web.fetch.request.delay_ms),
        ("fetch.request.timeout_secs", web.fetch.request.timeout_secs),
        (
            "fetch.request.max_body_bytes",
            web.fetch.request.max_body_bytes,
        ),
        ("browser.wait.timeout_secs", web.browser.wait.timeout_secs),
        (
            "crawl.indexing.document_cache_ttl_secs",
            web.crawl.indexing.document_cache_ttl_secs,
        ),
        ("fetch.cache.ttl_secs", web.fetch.cache.ttl_secs),
        ("fetch.cache.capacity", web.fetch.cache.capacity),
        ("store.retention.max_bytes", web.store.retention.max_bytes),
    ] {
        if value == 0 {
            return Err(PluginError::new(format!(
                "web plugin config `{label}` must be greater than 0"
            )));
        }
    }
    if web.store.retention.max_documents == 0 {
        return Err(PluginError::new(
            "web plugin config `store.retention.max_documents` must be greater than 0",
        ));
    }
    if web.crawl.defaults.max_pages > web.crawl.limits.max_pages {
        return Err(PluginError::new(
            "web plugin config `crawl.defaults.max_pages` must be less than or equal to `crawl.limits.max_pages`",
        ));
    }
    if web.crawl.defaults.max_depth > web.crawl.limits.max_depth {
        return Err(PluginError::new(
            "web plugin config `crawl.defaults.max_depth` must be less than or equal to `crawl.limits.max_depth`",
        ));
    }
    if web.search.default_limit > web.search.max_limit {
        return Err(PluginError::new(
            "web plugin config `search.default_limit` must be less than or equal to `search.max_limit`",
        ));
    }
    if web
        .browser
        .executable_path
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(PluginError::new(
            "web plugin config `browser.executable_path` must not be empty when set",
        ));
    }
    if web
        .browser
        .wait
        .for_selector
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(PluginError::new(
            "web plugin config `browser.wait.for_selector` must not be empty when set",
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

fn format_fetched_page(page: &FetchedPage, focus: Option<&str>) -> String {
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
    let focus = focus.map(str::trim).filter(|value| !value.is_empty());
    if let Some(focus) = focus {
        lines.push(format!("Focus: {focus}"));
        lines.push(String::new());
        let excerpts = focused_page_excerpts(page.markdown.as_str(), focus);
        if excerpts.is_empty() {
            lines.push(
                "No strongly matching excerpt was found for that focus; returning a general page preview."
                    .to_string(),
            );
            lines.push(String::new());
            lines.push(preview_text(page.markdown.as_str(), 3000));
        } else {
            lines.push("Relevant excerpts:".to_string());
            for (index, excerpt) in excerpts.iter().enumerate() {
                lines.push(format!("{}. {}", index + 1, excerpt));
            }
        }
    } else {
        lines.push(preview_text(page.markdown.as_str(), 4000));
    }
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
        "Found {} web search result(s) for '{}' via {}.\n\nThese are candidate links, not final evidence. For questions that need real facts, comparisons, or latest information, fetch 1-3 of the most relevant URLs before answering. If you already know what to extract, use fetch with `prompt`.\n\n{}",
        output.results.len(),
        output.query,
        output.engine,
        results_to_text(&output.results)
    )
}

fn focused_page_excerpts(markdown: &str, focus: &str) -> Vec<String> {
    let focus = focus.trim();
    if focus.is_empty() {
        return Vec::new();
    }
    let terms = focus_terms(focus);
    if terms.is_empty() {
        return Vec::new();
    }
    let mut scored_blocks: Vec<(usize, usize, String)> = markdown
        .split("\n\n")
        .enumerate()
        .filter_map(|(index, block)| {
            let trimmed = block.trim();
            if trimmed.is_empty() {
                return None;
            }
            let score = score_focus_block(trimmed, terms.as_slice());
            (score > 0).then(|| (score, index, preview_text(trimmed, 700)))
        })
        .collect();
    scored_blocks.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    scored_blocks.truncate(3);
    scored_blocks
        .into_iter()
        .map(|(_, _, block)| block)
        .collect()
}

fn focus_terms(focus: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let normalized_focus = focus.trim().to_lowercase();
    if !normalized_focus.is_empty() {
        terms.push(normalized_focus);
    }

    let mut token = String::new();
    for ch in focus.chars() {
        if ch.is_alphanumeric() {
            token.extend(ch.to_lowercase());
        } else if !token.is_empty() {
            push_focus_term(&mut terms, &mut token);
        }
    }
    if !token.is_empty() {
        push_focus_term(&mut terms, &mut token);
    }

    terms.sort();
    terms.dedup();
    terms
}

fn push_focus_term(terms: &mut Vec<String>, token: &mut String) {
    let keep = token.chars().count() >= 3
        || token
            .chars()
            .any(|ch| !ch.is_ascii() && !ch.is_whitespace());
    if keep {
        terms.push(std::mem::take(token));
    } else {
        token.clear();
    }
}

fn score_focus_block(block: &str, terms: &[String]) -> usize {
    let normalized = block.to_lowercase();
    let mut score = 0;
    for term in terms {
        if normalized.contains(term) {
            score += if term.contains(' ') || term.chars().count() > 12 {
                4
            } else {
                1
            };
        }
    }
    score
}

fn format_crawl_run(output: &CrawlRunReport) -> String {
    let mut lines = vec![format!(
        "Crawled from {} via {} (rendered: {}). New pages indexed: {}. Cached pages reused: {}. Exact duplicates skipped: {}. Near duplicates skipped: {}. Old pages pruned: {} ({} bytes). Failures: {}. Total cached documents: {}.",
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
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::is_public_address;

    #[test]
    fn public_dns_addresses_do_not_need_a_second_permission_check() {
        assert!(is_public_address(Ipv4Addr::new(104, 18, 33, 45).into()));
        assert!(is_public_address(
            "2606:4700::6812:212d"
                .parse::<Ipv6Addr>()
                .expect("valid IPv6 address")
                .into()
        ));
        assert!(!is_public_address(Ipv4Addr::LOCALHOST.into()));
        assert!(!is_public_address(Ipv4Addr::new(10, 0, 0, 1).into()));
        assert!(!is_public_address(Ipv6Addr::LOCALHOST.into()));
    }
}
