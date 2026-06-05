use std::future::Future;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_macros::{StaticToolSurface, ToolInputShape, ToolSuite};
use agena_web::{
    BrowserRenderOptions, CrawlPageFetcher, CrawlRunOptions, CrawlRunReport, CrawlStore,
    CrawlStoreRetention, FetchedPage, LocalBrowserOptions, SpiderFetchOptions, StoredDocument,
    WebSearchEngine, WebSearchOptions, WebSearchResult, crawl_site, ensure_index_exists,
    fetch_page_with_spider, prepare_fetch_url, preview_text, results_to_text, search_web,
};
use governor::{DefaultKeyedRateLimiter, Quota};
use moka::future::Cache;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostNetworkPermissionCheckRequest};
use crate::plugin::sdk::{
    HookSubscription, HostCapability, NetworkRequest, PathRequest, Result as SdkResult,
    ToolInvokeOutput, ToolTag,
};

pub const WEB_PLUGIN_ID: &str = "agena.web";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebCrawlConfig {
    pub defaults: WebCrawlDefaultsConfig,
    pub limits: WebCrawlLimitsConfig,
    pub indexing: WebCrawlIndexingConfig,
}

impl Default for WebCrawlConfig {
    fn default() -> Self {
        Self {
            defaults: WebCrawlDefaultsConfig::default(),
            limits: WebCrawlLimitsConfig::default(),
            indexing: WebCrawlIndexingConfig::default(),
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebStoreConfig {
    pub retention: WebStoreRetentionConfig,
    pub listing: WebStoreListingConfig,
}

impl Default for WebStoreConfig {
    fn default() -> Self {
        Self {
            retention: WebStoreRetentionConfig::default(),
            listing: WebStoreListingConfig::default(),
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebStoreListingConfig {
    pub default_limit: u32,
    pub max_limit: u32,
}

impl Default for WebStoreListingConfig {
    fn default() -> Self {
        Self {
            default_limit: 20,
            max_limit: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebBrowserConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    pub wait: WebBrowserWaitConfig,
}

impl Default for WebBrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            executable_path: None,
            wait: WebBrowserWaitConfig::default(),
        }
    }
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

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            fetch: WebFetchConfig::default(),
            crawl: WebCrawlConfig::default(),
            search: WebSearchConfig::default(),
            store: WebStoreConfig::default(),
            browser: WebBrowserConfig::default(),
        }
    }
}

fn default_web_fetch_enabled() -> bool {
    true
}

fn web_config_schema() -> serde_json::Value {
    let mut schema = crate::tool::definition::json_schema_for_with_default(WebConfig::default());
    for (pointer, title, description) in [
        (
            "",
            "Web Plugin Config",
            "Fetch, crawl, search, storage, and browser defaults for the agena.web plugin.",
        ),
        (
            "/properties/fetch",
            "Fetch",
            "Controls direct page fetch operations, request throttling, and fetch cache behavior.",
        ),
        (
            "/properties/fetch/properties/enabled",
            "Enabled",
            "Allows agena.web/fetch and agena.web/crawl to run. Disable this to turn off network page retrieval.",
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
            "Store",
            "Retention and listing defaults for the embedded web document store.",
        ),
        (
            "/properties/store/properties/retention",
            "Retention",
            "Maximum document count and byte size retained in local web storage.",
        ),
        (
            "/properties/store/properties/retention/properties/max_documents",
            "Max Documents",
            "Maximum number of stored documents retained locally.",
        ),
        (
            "/properties/store/properties/retention/properties/max_bytes",
            "Max Bytes",
            "Maximum total byte size retained by the local document store.",
        ),
        (
            "/properties/store/properties/listing",
            "Listing",
            "Default and maximum limits for document listing operations.",
        ),
        (
            "/properties/store/properties/listing/properties/default_limit",
            "Default Limit",
            "Number of stored documents listed when callers omit a limit.",
        ),
        (
            "/properties/store/properties/listing/properties/max_limit",
            "Max Limit",
            "Largest number of stored documents a caller may request in one list call.",
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
    ] {
        crate::tool::definition::set_schema_metadata(
            &mut schema,
            pointer,
            Some(title),
            Some(description),
        );
    }
    schema
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
                .time_to_live(Duration::from_secs(config.fetch.cache.ttl_secs))
                .max_capacity(config.fetch.cache.capacity)
                .build(),
            host_limiter: build_host_limiter(config.fetch.request.delay_ms),
            config,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(
    trim("url", "prompt"),
    non_empty("url"),
    non_empty_if_present("prompt")
)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(trim("start_url"), non_empty("start_url"))]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(trim("query"), non_empty("query"))]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(trim("query"), non_empty("query"))]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(trim("id", "url"), exactly_one_of("id", "url"))]
#[serde(deny_unknown_fields)]
struct CrawlGetInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[serde(deny_unknown_fields)]
struct CrawlListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "search",
    description = "Search the public web through the configured search engine.",
    summary = "Search the public web.",
    handler_receiver = WebPlugin,
    handle = WebPlugin::invoke_search,
    handle_field = args,
    permission_paths_handle = WebPlugin::permission_paths_search,
    permission_networks_handle = WebPlugin::permission_networks_search,
    examples(
        r#"{"query":"Agena plugin architecture","limit":5}"#,
        r#"{"query":"Rust schemars derive examples"}"#
    ),
    display = detailed,
    tags(
        ToolTag::ReadOnly,
        ToolTag::Network,
        ToolTag::Internet,
        ToolTag::Discovery
    ),
    host_capabilities(HostCapability::PermissionCheck),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct SearchToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: CrawlWebSearchInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "fetch",
    description = "Fetch a web page and return readable page content.",
    summary = "Fetch one web page as readable content.",
    handler_receiver = WebPlugin,
    handle = WebPlugin::invoke_fetch,
    handle_field = args,
    permission_paths_handle = WebPlugin::permission_paths_fetch,
    permission_networks_handle = WebPlugin::permission_networks_fetch,
    examples(
        r#"{"url":"https://openai.com"}"#,
        r#"{"url":"https://example.com/docs","render_mode":"plain"}"#
    ),
    display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Network, ToolTag::Internet),
    host_capabilities(HostCapability::PermissionCheck),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct FetchToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: CrawlFetchInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "crawl.run",
    description = "Crawl a site into the local web document store.",
    summary = "Crawl a site into the local document store.",
    handler_receiver = WebPlugin,
    handle = WebPlugin::invoke_crawl,
    handle_field = args,
    permission_paths_handle = WebPlugin::permission_paths_crawl,
    permission_networks_handle = WebPlugin::permission_networks_crawl,
    ui_display = detailed,
    tags(
        ToolTag::Mutating,
        ToolTag::Network,
        ToolTag::Internet,
        ToolTag::Discovery
    ),
    host_capabilities(HostCapability::PermissionCheck),
    concurrency_safe = false
)]
#[serde(deny_unknown_fields)]
struct CrawlToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: CrawlRunInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "store.query",
    description = "Search locally stored crawl documents.",
    summary = "Search locally stored crawl documents.",
    handler_receiver = WebPlugin,
    handle = WebPlugin::invoke_query,
    handle_field = args,
    permission_paths_handle = WebPlugin::permission_paths_store_query,
    permission_networks_handle = WebPlugin::permission_networks_store_query,
    ui_display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    host_capabilities(HostCapability::PermissionCheck),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct StoreQueryToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: CrawlQueryInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "store.get",
    description = "Get one stored crawl document by id or url.",
    summary = "Get one stored crawl document.",
    handler_receiver = WebPlugin,
    handle = WebPlugin::invoke_get,
    handle_field = args,
    permission_paths_handle = WebPlugin::permission_paths_store_get,
    permission_networks_handle = WebPlugin::permission_networks_store_get,
    ui_display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    host_capabilities(HostCapability::PermissionCheck),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct StoreGetToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: CrawlGetInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "store.list",
    description = "List stored crawl documents.",
    summary = "List stored crawl documents.",
    handler_receiver = WebPlugin,
    handle = WebPlugin::invoke_list,
    handle_field = args,
    permission_paths_handle = WebPlugin::permission_paths_store_list,
    permission_networks_handle = WebPlugin::permission_networks_store_list,
    ui_display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::Discovery),
    host_capabilities(HostCapability::PermissionCheck),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct StoreListToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: CrawlListInput,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = WebPlugin)]
enum WebToolSuite {
    Search(SearchToolInput),
    Fetch(FetchToolInput),
    Crawl(CrawlToolInput),
    StoreQuery(StoreQueryToolInput),
    StoreGet(StoreGetToolInput),
    StoreList(StoreListToolInput),
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
        let delay = (config.browser.wait.delay_ms > 0)
            .then(|| Duration::from_millis(config.browser.wait.delay_ms));
        Ok(SpiderFetchOptions {
            max_body_bytes: config.fetch.request.max_body_bytes as usize,
            timeout: Duration::from_secs(config.fetch.request.timeout_secs),
            delay_ms: config.fetch.request.delay_ms,
            user_agent: crate::provider::CLAUDE_USER_WEB_FETCH_USER_AGENT.to_string(),
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
        if use_cache {
            state.fetch_cache.insert(cache_key, page.clone()).await;
        }
        Ok(page)
    }

    async fn invoke_fetch(&self, input: &CrawlFetchInput) -> SdkResult<ToolInvokeOutput> {
        let url = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?;
        let config = self.config()?;
        let render_js = input.render_js.unwrap_or(config.browser.enabled);
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
            use_cache: input.use_cache.unwrap_or(true),
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
        Ok(ToolInvokeOutput::text(text)
            .with_title("web crawl")
            .with_payload(payload))
    }

    async fn invoke_search(&self, input: &CrawlWebSearchInput) -> SdkResult<ToolInvokeOutput> {
        let query = input.query.as_str();
        let config = self.config()?;
        let limit = clamp_limit(
            input.limit.or(input.max_results),
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
            timeout: Duration::from_secs(config.fetch.request.timeout_secs),
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
        let query = input.query.as_str();
        let config = self.config()?;
        let limit = clamp_limit(
            input.limit.or(input.max_results),
            config.search.default_limit as usize,
            config.search.max_limit as usize,
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
            _ => unreachable!("store.get validation should guarantee exactly one selector"),
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
            config.store.listing.default_limit as usize,
            config.store.listing.max_limit as usize,
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

    fn store_permission_requests(
        &self,
        kind: PathKindForPermission,
    ) -> SdkResult<Vec<PathRequest>> {
        let store = self.store()?;
        let path = store.dir().display().to_string();
        let request = match kind {
            PathKindForPermission::Read => PathRequest::read(path),
            PathKindForPermission::Write => PathRequest::write(path),
        };
        Ok(vec![request])
    }

    async fn permission_paths_search(
        &self,
        _input: &CrawlWebSearchInput,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(Vec::new())
    }

    async fn permission_networks_search(
        &self,
        input: &CrawlWebSearchInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        Ok(search_engines(input.engine)
            .into_iter()
            .map(|engine| NetworkRequest::connect(engine.permission_url().to_string()))
            .collect())
    }

    async fn permission_paths_fetch(
        &self,
        _input: &CrawlFetchInput,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(Vec::new())
    }

    async fn permission_networks_fetch(
        &self,
        input: &CrawlFetchInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        Ok(vec![NetworkRequest::connect(
            prepare_fetch_url(input.url.as_str())
                .map_err(crawl_error_to_plugin)?
                .to_string(),
        )])
    }

    async fn permission_paths_crawl(&self, _input: &CrawlRunInput) -> SdkResult<Vec<PathRequest>> {
        self.store_permission_requests(PathKindForPermission::Write)
    }

    async fn permission_networks_crawl(
        &self,
        input: &CrawlRunInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        Ok(vec![NetworkRequest::connect(
            prepare_fetch_url(input.start_url.as_str())
                .map_err(crawl_error_to_plugin)?
                .to_string(),
        )])
    }

    async fn permission_paths_store_query(
        &self,
        _input: &CrawlQueryInput,
    ) -> SdkResult<Vec<PathRequest>> {
        self.store_permission_requests(PathKindForPermission::Write)
    }

    async fn permission_networks_store_query(
        &self,
        _input: &CrawlQueryInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        Ok(Vec::new())
    }

    async fn permission_paths_store_get(
        &self,
        _input: &CrawlGetInput,
    ) -> SdkResult<Vec<PathRequest>> {
        self.store_permission_requests(PathKindForPermission::Read)
    }

    async fn permission_networks_store_get(
        &self,
        _input: &CrawlGetInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        Ok(Vec::new())
    }

    async fn permission_paths_store_list(
        &self,
        _input: &CrawlListInput,
    ) -> SdkResult<Vec<PathRequest>> {
        self.store_permission_requests(PathKindForPermission::Read)
    }

    async fn permission_networks_store_list(
        &self,
        _input: &CrawlListInput,
    ) -> SdkResult<Vec<NetworkRequest>> {
        Ok(Vec::new())
    }
}

#[crate::plugin::sdk::plugin]
impl crate::plugin::sdk::Plugin for WebPlugin {
    #[agena_plugin_sdk::plugin_manifest_method(
        id = WEB_PLUGIN_ID,
        version = env!("CARGO_PKG_VERSION"),
        description = "Local web search/fetch/crawl plugin with embedded storage, caching, and Tantivy retrieval.",
        hooks = HookSubscription::INIT | HookSubscription::TOOL_INVOKE,
        config_schema = web_config_schema(),
        display = brief_detailed,
        tool_suite = WebToolSuite,
    )]
    fn manifest(&self) -> crate::plugin::sdk::PluginManifest {}

    #[agena_plugin_sdk::plugin_init_method(
        store = {
            field = self.state,
            value = WebPluginState::new(parse_web_config(ctx.config)?),
            already = "web plugin initialized more than once"
        },
        store = {
            field = self.host,
            value = host,
            already = "web plugin host initialized more than once"
        },
        workspace_root = {
            field = self.workspace_root,
            value = ctx.workspace_root,
            already = "web plugin workspace root initialized more than once"
        }
    )]
    async fn init(
        &self,
        ctx: crate::plugin::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<crate::plugin::sdk::InitOutcome> {
    }

    #[agena_plugin_sdk::plugin_tool_invoke_method(suite(WebToolSuite))]
    async fn tool_invoke(
        &self,
        input: crate::plugin::sdk::ToolInvokeInput,
    ) -> SdkResult<ToolInvokeOutput> {
    }

    #[agena_plugin_sdk::plugin_permission_paths_method(suite(WebToolSuite))]
    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        let _ = (tool, input);
    }

    #[agena_plugin_sdk::plugin_permission_networks_method(suite(WebToolSuite))]
    async fn permission_networks(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<NetworkRequest>> {
        let _ = (tool, input);
    }
}

enum PathKindForPermission {
    Read,
    Write,
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
        (
            "store.listing.default_limit",
            web.store.listing.default_limit,
        ),
        ("store.listing.max_limit", web.store.listing.max_limit),
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
    if web.store.listing.default_limit > web.store.listing.max_limit {
        return Err(PluginError::new(
            "web plugin config `store.listing.default_limit` must be less than or equal to `store.listing.max_limit`",
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
    use super::{
        FetchToolInput, SearchToolInput, StoreGetToolInput, StoreQueryToolInput, WebConfig,
        WebToolSuite, fetch_cache_key, parse_web_config, search_engines,
    };
    use crate::plugin::sdk::{HostCapability, Plugin as _, ToolDescriptionMode};
    use serde_json::json;

    #[test]
    fn web_decls_declare_permission_check_host_capability() {
        let decls = WebToolSuite::tool_decls();
        assert_eq!(decls.len(), 6);
        assert!(decls.iter().all(|decl| {
            decl.host_capabilities
                .contains(&HostCapability::PermissionCheck)
        }));
        assert_eq!(
            decls
                .iter()
                .map(|decl| decl.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "search",
                "fetch",
                "crawl.run",
                "store.query",
                "store.get",
                "store.list"
            ]
        );
    }

    #[test]
    fn web_manifest_defaults_to_brief_but_keeps_search_and_fetch_detailed() {
        let manifest = super::WebPlugin::new().manifest();
        assert_eq!(
            manifest.tool_description_mode,
            Some(ToolDescriptionMode::Brief)
        );

        for tool_name in ["search", "fetch"] {
            let tool = manifest
                .tools
                .iter()
                .find(|decl| decl.name == tool_name)
                .expect("tool should be declared");
            assert_eq!(tool.description_mode, Some(ToolDescriptionMode::Detailed));
        }

        for tool_name in ["crawl.run", "store.query", "store.get", "store.list"] {
            let tool = manifest
                .tools
                .iter()
                .find(|decl| decl.name == tool_name)
                .expect("tool should be declared");
            assert_eq!(tool.description_mode, None);
        }
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

    #[test]
    fn null_web_config_materializes_nested_defaults() {
        let config = parse_web_config(serde_json::Value::Null).expect("null config should parse");
        assert_eq!(config, WebConfig::default());
        assert_eq!(config.fetch.request.timeout_secs, 30);
        assert_eq!(config.crawl.defaults.max_pages, 10);
        assert_eq!(config.search.default_limit, 5);
        assert_eq!(config.store.listing.default_limit, 20);
        assert_eq!(config.browser.wait.timeout_secs, 10);
    }

    #[test]
    fn nested_web_config_override_parses_and_validates() {
        let config = parse_web_config(json!({
            "fetch": {
                "enabled": true,
                "request": {
                    "delay_ms": 250,
                    "timeout_secs": 45,
                    "max_body_bytes": 1048576,
                    "respect_robots_txt": false
                },
                "cache": {
                    "ttl_secs": 120,
                    "capacity": 32
                }
            },
            "crawl": {
                "defaults": {
                    "max_pages": 12,
                    "max_depth": 2,
                    "same_host_only": false
                },
                "limits": {
                    "max_pages": 50,
                    "max_depth": 6
                },
                "indexing": {
                    "document_cache_ttl_secs": 7200,
                    "chunk_chars": 1200,
                    "near_duplicate_hamming_distance": 4
                }
            },
            "search": {
                "default_limit": 8,
                "max_limit": 25
            },
            "store": {
                "retention": {
                    "max_documents": 400,
                    "max_bytes": 209715200
                },
                "listing": {
                    "default_limit": 30,
                    "max_limit": 120
                }
            },
            "browser": {
                "enabled": true,
                "executable_path": "/usr/bin/chromium",
                "wait": {
                    "for_network_idle": false,
                    "timeout_secs": 15,
                    "for_selector": "#app",
                    "delay_ms": 300
                }
            }
        }))
        .expect("nested config should parse");

        assert_eq!(config.fetch.request.delay_ms, 250);
        assert_eq!(config.fetch.cache.capacity, 32);
        assert_eq!(config.crawl.defaults.max_pages, 12);
        assert_eq!(config.crawl.limits.max_depth, 6);
        assert_eq!(config.search.max_limit, 25);
        assert_eq!(config.store.retention.max_documents, 400);
        assert_eq!(config.store.listing.max_limit, 120);
        assert_eq!(
            config.browser.executable_path.as_deref(),
            Some("/usr/bin/chromium")
        );
        assert_eq!(config.browser.wait.for_selector.as_deref(), Some("#app"));
    }

    #[test]
    fn flat_web_config_shape_is_rejected() {
        let err = parse_web_config(json!({
            "default_max_pages": 12,
            "browser_enabled": true
        }))
        .expect_err("legacy flat config should fail");

        let message = err.to_string();
        assert!(message.contains("invalid web plugin config"));
        assert!(message.contains("unknown field"));
    }

    #[test]
    fn web_tool_inputs_validate_empty_queries_and_store_get_selector_pairs() {
        let err = SearchToolInput::parse_input(json!({
            "query": "   "
        }))
        .expect_err("web search should reject empty query during parse");
        assert!(err.to_string().contains("field `query` must not be empty"));

        let err = StoreQueryToolInput::parse_input(json!({
            "query": ""
        }))
        .expect_err("web query should reject empty query during parse");
        assert!(err.to_string().contains("field `query` must not be empty"));

        let err = StoreGetToolInput::parse_input(json!({
            "id": "doc_1",
            "url": "https://example.com"
        }))
        .expect_err("store.get should reject duplicate selectors during parse");
        assert!(
            err.to_string()
                .contains("exactly one of `id` or `url` is required")
        );

        let parsed = SearchToolInput::parse_input(json!({
            "query": "  Rust schemars derive examples  "
        }))
        .expect("web search should trim query during parse");
        assert_eq!(parsed.args.query, "Rust schemars derive examples");

        let parsed = StoreQueryToolInput::parse_input(json!({
            "query": "  local docs  "
        }))
        .expect("store.query should trim query during parse");
        assert_eq!(parsed.args.query, "local docs");

        let fetch = FetchToolInput::parse_input(json!({
            "url": "  https://example.com/docs  ",
            "prompt": "  summarize key points  "
        }))
        .expect("fetch should trim nested fields during parse");
        assert_eq!(fetch.args.url, "https://example.com/docs");
        assert_eq!(fetch.args.prompt.as_deref(), Some("summarize key points"));

        let get = StoreGetToolInput::parse_input(json!({
            "url": "  https://example.com/post  "
        }))
        .expect("store.get should trim selector fields during parse");
        assert_eq!(get.args.url.as_deref(), Some("https://example.com/post"));
    }
}
