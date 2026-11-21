mod content;
mod fetch_transport;
mod rendered_fetch;
mod search_aggregate;
mod search_provider;
mod search_selection;
use search_provider::WebSearchBackend;
use search_selection::WebSearchEngineSelection;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_web::{
    BrowserRenderOptions, CrawlPageFetcher, CrawlRunOptions, CrawlRunReport, CrawlStore,
    CrawlStoreRetention, FetchedPage, WebFetchCoordinator, WebFetchCoordinatorConfig,
    WebFetchOptions, WebSearchCoordinator, WebSearchEngine, WebSearchOptions, WebSearchResponse,
    crawl_site, prepare_fetch_url, preview_text, results_to_text,
};
use futures_util::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore, mpsc};

use crate::browser::runtime::{ChromeOptions, ChromeRuntime, ChromeSandbox};
use crate::network_policy::{self, authorized_network_addresses, resolve_public_network_target};

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, ToolStreamSink,
};

pub(crate) const WEB_PLUGIN_ID: &str = "agena.web";
const MAX_FETCH_BATCH: u32 = 16;
const DEFAULT_FETCH_BATCH_CONCURRENCY: u32 = 8;
static HTTP_PAGE_FETCHES: Semaphore = Semaphore::const_new(64);

fn web_storage_root(context: &ToolInvokeContext<'_>, project: &Path) -> PathBuf {
    if context.is_global() {
        agena_storage_layout::StorageLayout::current().session_assets_dir(context.session_id)
    } else {
        project.to_path_buf()
    }
}

fn plugin_internal_error_with_context(
    context: &str,
    error: &(dyn std::error::Error + 'static),
) -> PluginError {
    PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
        context, error,
    ))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WebConfig {
    pub network: network_policy::WebNetworkConfig,
    pub fetch: WebFetchConfig,
    pub crawl: WebCrawlConfig,
    pub search: WebSearchConfig,
    pub store: WebStoreConfig,
    pub browser: WebRenderConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Fetch configuration of the web plugin.
pub struct WebFetchConfig {
    pub extractor: agena_web::ExtractionBackend,
    #[serde(default = "default_web_fetch_enabled")]
    pub enabled: bool,
    pub request: WebRequestConfig,
    pub cache: WebFetchCacheConfig,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            extractor: agena_web::ExtractionBackend::default(),
            request: WebRequestConfig::default(),
            cache: WebFetchCacheConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Request behavior of the web plugin.
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
/// Cache configuration of the web plugin.
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
/// Crawl configuration of the web plugin.
pub struct WebCrawlConfig {
    pub defaults: WebCrawlDefaultsConfig,
    pub limits: WebCrawlLimitsConfig,
    pub indexing: WebCrawlIndexingConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Default crawl settings of the web plugin.
pub struct WebCrawlDefaultsConfig {
    pub max_pages: u32,
    pub concurrency: u32,
    pub max_depth: u32,
    pub same_host_only: bool,
}

impl Default for WebCrawlDefaultsConfig {
    fn default() -> Self {
        Self {
            max_pages: 10,
            concurrency: 4,
            max_depth: 1,
            same_host_only: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Crawl limits of the web plugin.
pub struct WebCrawlLimitsConfig {
    pub max_pages: u32,
    pub concurrency: u32,
    pub max_depth: u32,
}

impl Default for WebCrawlLimitsConfig {
    fn default() -> Self {
        Self {
            max_pages: 100,
            concurrency: 16,
            max_depth: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Indexing settings of the web plugin.
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
/// Search settings of the web plugin.
pub struct WebSearchConfig {
    pub default_limit: u32,
    pub max_limit: u32,
    /// Default candidate budget for each HTML engine, independent of the final limit.
    pub default_results_per_engine: u32,
    pub max_results_per_engine: u32,
    /// Default page budget for each HTML engine, including its first page.
    pub default_pages_per_engine: u32,
    pub max_pages_per_engine: u32,
    /// Overall active budget for an automatic one-page HTML search.
    #[schemars(range(min = 1, max = 120))]
    pub auto_timeout_secs: u64,
    /// Time to collect additional engines once enough distinct, filtered results are available.
    /// Zero waits for all engines within the overall budget.
    #[schemars(range(max = 30000))]
    pub auto_grace_ms: u64,
    pub provider: WebSearchBackend,
    /// Full API search endpoint. SearXNG requires an explicit endpoint.
    pub endpoint: Option<String>,
    /// Environment variable name only; credentials never belong in settings.
    pub api_key_env: Option<String>,
    /// Allow a configured SearXNG endpoint on a private network.
    pub allow_private_endpoint: bool,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 5,
            max_limit: 20,
            default_results_per_engine: 10,
            max_results_per_engine: 50,
            default_pages_per_engine: 1,
            max_pages_per_engine: 5,
            auto_timeout_secs: 8,
            auto_grace_ms: 750,
            provider: WebSearchBackend::default(),
            endpoint: None,
            api_key_env: None,
            allow_private_endpoint: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Store settings of the web plugin.
pub struct WebStoreConfig {
    pub retention: WebStoreRetentionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Retention settings of the web plugin store.
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
/// Chrome settings for ephemeral page rendering.
pub struct WebRenderConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    pub wait: WebRenderWaitConfig,
    /// Seconds of inactivity after which the managed browser process is shut
    /// down automatically. `0` disables idle auto-close.
    #[serde(default = "default_web_browser_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
}

impl Default for WebRenderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            executable_path: None,
            wait: WebRenderWaitConfig::default(),
            idle_timeout_secs: default_web_browser_idle_timeout_secs(),
        }
    }
}

impl WebRenderConfig {
    fn chrome_options(&self) -> ChromeOptions {
        ChromeOptions::new(
            self.executable_path.as_deref(),
            self.wait.timeout_secs,
            self.idle_timeout_secs,
            ChromeSandbox::Automatic,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Readiness conditions for rendered fetches.
pub struct WebRenderWaitConfig {
    pub for_network_idle: bool,
    pub timeout_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub for_selector: Option<String>,
    pub delay_ms: u64,
}

impl Default for WebRenderWaitConfig {
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

fn default_web_browser_idle_timeout_secs() -> u64 {
    300
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn web_settings_metadata() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            "",
            "Web Plugin Config",
            "Fetch, crawl, search, embedded cache, and browser defaults for the agena.web plugin.",
        ),
        (
            "/fetch",
            "Fetch",
            "Controls direct page fetch operations, request throttling, and fetch cache behavior.",
        ),
        (
            "/network",
            "Network",
            "HTTP/SOCKS proxy selection, public-address validation and trusted local Fake-IP DNS routing.",
        ),
        (
            "/network/proxy",
            "HTTP / SOCKS Proxy",
            "Optional proxy URL for HTTP(S) requests. Omit to use HTTP_PROXY, HTTPS_PROXY, ALL_PROXY and OS settings; NO_PROXY remains effective. An empty string forces direct access. Supports HTTP(S), SOCKS4/4a/5/5h.",
        ),
        (
            "/network/fake_ip_ranges",
            "Additional Fake-IP Ranges",
            "Common TUN Fake-IP pools (198.18.0.0/15, fc00::/18 and fdfe:dcba:9876::/64) work automatically. Add exact custom synthetic CIDRs here if needed. Accepts subnets of 198.18.0.0/15, fc00::/18 and IPv6 ULA /64 or narrower. Applies to public hostname DNS answers.",
        ),
        (
            "/fetch/extractor",
            "HTML Extractor",
            "Readability (local Rust default) or optional local Trafilatura via AGENA_EXTRACT_PYTHON. No remote extraction or automatic dependency installation.",
        ),
        (
            "/fetch/enabled",
            "Enabled",
            "Allows web.fetch and web.crawl to run. Disable this to turn off network page retrieval.",
        ),
        (
            "/fetch/request",
            "Request",
            "Default timing, timeout, and body-size limits for HTTP page fetches.",
        ),
        (
            "/fetch/request/delay_ms",
            "Delay (ms)",
            "Minimum delay between fetches to the same host.",
        ),
        (
            "/fetch/request/timeout_secs",
            "Timeout (sec)",
            "Maximum time allowed for one fetch request before it fails.",
        ),
        (
            "/fetch/request/max_body_bytes",
            "Max Body Bytes",
            "Largest response body accepted from a fetched page.",
        ),
        (
            "/fetch/request/respect_robots_txt",
            "Respect robots.txt",
            "Honors robots.txt restrictions during fetch and crawl operations.",
        ),
        (
            "/fetch/cache",
            "Cache",
            "Short-lived cache for fetched page content and metadata.",
        ),
        (
            "/fetch/cache/ttl_secs",
            "TTL (sec)",
            "How long cached fetch results stay valid.",
        ),
        (
            "/fetch/cache/capacity",
            "Capacity",
            "Maximum number of cached fetch results kept in memory.",
        ),
        (
            "/crawl",
            "Crawl",
            "Defaults, limits, and indexing behavior for site crawls.",
        ),
        (
            "/crawl/defaults",
            "Defaults",
            "Default crawl options used when callers omit them.",
        ),
        (
            "/crawl/defaults/concurrency",
            "Concurrent Pages",
            "Default simultaneous page reads per crawl, capped by the configured crawl limit.",
        ),
        (
            "/crawl/limits/concurrency",
            "Maximum Concurrent Pages",
            "Upper bound on crawl concurrency (default 16, maximum 32).",
        ),
        (
            "/crawl/defaults/max_pages",
            "Max Pages",
            "Default page limit for one crawl run.",
        ),
        (
            "/crawl/defaults/max_depth",
            "Max Depth",
            "Default traversal depth for one crawl run.",
        ),
        (
            "/crawl/defaults/same_host_only",
            "Same Host Only",
            "Keeps crawls on the original host unless the caller opts out.",
        ),
        (
            "/crawl/limits",
            "Limits",
            "Hard upper bounds enforced for crawl requests.",
        ),
        (
            "/crawl/limits/max_pages",
            "Max Pages Limit",
            "Largest page count any crawl request may ask for.",
        ),
        (
            "/crawl/limits/max_depth",
            "Max Depth Limit",
            "Largest traversal depth any crawl request may ask for.",
        ),
        (
            "/crawl/indexing",
            "Indexing",
            "Controls how crawled documents are cached and chunked before storage.",
        ),
        (
            "/crawl/indexing/document_cache_ttl_secs",
            "Document Cache TTL (sec)",
            "How long indexed crawl documents stay in the crawl document cache.",
        ),
        (
            "/crawl/indexing/chunk_chars",
            "Chunk Size (chars)",
            "Target character size for indexed text chunks.",
        ),
        (
            "/crawl/indexing/near_duplicate_hamming_distance",
            "Near-Duplicate Distance",
            "Similarity threshold used when suppressing near-duplicate crawl content.",
        ),
        (
            "/search",
            "Search",
            "Search backend, credentials and result limits.",
        ),
        (
            "/search/provider",
            "Search Provider",
            "Use the free HTML engines, a configured Brave/Tavily/Exa API, or SearXNG. API failures do not silently switch providers.",
        ),
        (
            "/search/endpoint",
            "Search API Endpoint",
            "Full search URL. Required for SearXNG; hosted providers use their official endpoint by default.",
        ),
        (
            "/search/api_key_env",
            "API Key Environment Variable",
            "Name of the server environment variable holding the credential, never the credential itself.",
        ),
        (
            "/search/allow_private_endpoint",
            "Allow Private SearXNG Endpoint",
            "Permit only the configured SearXNG search endpoint on a private network. Page fetching keeps its public-network policy.",
        ),
        (
            "/search/default_limit",
            "Default Output Limit",
            "Final result limit when max_results is omitted (default 5). Independent of HTML retrieval budgets.",
        ),
        (
            "/search/max_limit",
            "Max Output Limit",
            "Largest final result limit a caller may request (default 20, ceiling 50). Does not trigger extra HTML pages.",
        ),
        (
            "/search/default_results_per_engine",
            "Default Candidates Per Engine",
            "Distinct candidates to collect from each HTML engine before domain filtering (default 10). Independent of the final result limit.",
        ),
        (
            "/search/max_results_per_engine",
            "Max Candidates Per Engine",
            "Largest candidate budget callers may request for each HTML engine (default and ceiling 50).",
        ),
        (
            "/search/default_pages_per_engine",
            "Default Pages Per Engine",
            "Search result pages to fetch from each HTML engine, including its first page (default 1). Stops earlier at the candidate budget or a page with no new URLs.",
        ),
        (
            "/search/max_pages_per_engine",
            "Max Pages Per Engine",
            "Largest page budget callers may request for each HTML engine (default and ceiling 5). API providers do not use HTML budgets.",
        ),
        (
            "/search/auto_timeout_secs",
            "Automatic HTML Search Timeout (sec)",
            "Active time budget for automatic one-page HTML searches (default 8 seconds), capped by the fetch timeout.",
        ),
        (
            "/search/auto_grace_ms",
            "Extra Time for Search Sources (ms)",
            "Time to collect more HTML engines once enough distinct results pass the domain filters in automatic one-page mode (default 750 ms). Zero waits until all engines finish or the search times out.",
        ),
        (
            "/store",
            "Cache",
            "Retention defaults for the embedded crawl document cache.",
        ),
        (
            "/store/retention",
            "Retention",
            "Maximum document count and byte size retained in the local crawl cache.",
        ),
        (
            "/store/retention/max_documents",
            "Max Documents",
            "Maximum number of cached crawl documents retained locally.",
        ),
        (
            "/store/retention/max_bytes",
            "Max Bytes",
            "Maximum total byte size retained by the local crawl cache.",
        ),
        (
            "/browser",
            "Rendering",
            "Optional browser rendering support for JavaScript-heavy pages.",
        ),
        (
            "/browser/enabled",
            "Enabled",
            "Allows rendered fetches and crawls to use a local browser.",
        ),
        (
            "/browser/executable_path",
            "Executable Path",
            "Optional browser executable path. Leave unset to use the default browser resolution logic.",
        ),
        (
            "/browser/idle_timeout_secs",
            "Idle Timeout (sec)",
            "Seconds of inactivity before the managed browser process is closed automatically. 0 disables auto-close.",
        ),
        (
            "/browser/wait",
            "Wait",
            "Browser rendering wait strategy applied before capturing page content.",
        ),
        (
            "/browser/wait/for_network_idle",
            "Wait for Network Idle",
            "Waits for the page network to go idle before reading rendered content.",
        ),
        (
            "/browser/wait/timeout_secs",
            "Timeout (sec)",
            "Maximum browser wait time before rendered capture fails.",
        ),
        (
            "/browser/wait/for_selector",
            "Wait for Selector",
            "Optional CSS selector that must appear before rendered capture continues.",
        ),
        (
            "/browser/wait/delay_ms",
            "Extra Delay (ms)",
            "Additional delay after wait conditions succeed before capturing the page.",
        ),
    ]
}

fn default_web_config() -> WebConfig {
    WebConfig {
        network: network_policy::WebNetworkConfig::default(),
        fetch: WebFetchConfig::default(),
        crawl: WebCrawlConfig::default(),
        search: WebSearchConfig::default(),
        store: WebStoreConfig::default(),
        browser: WebRenderConfig::default(),
    }
}

pub(crate) struct WebPlugin {
    state: OnceLock<WebPluginState>,
    workspace_root: OnceLock<PathBuf>,
    renderer: ChromeRuntime,
    user_agent: String,
}

impl Default for WebPlugin {
    fn default() -> Self {
        Self::new()
    }
}

struct WebPluginState {
    config: WebConfig,
    fetch_coordinator: Arc<WebFetchCoordinator>,
    http_transport: Arc<fetch_transport::HttpTransport>,
    search_coordinator: WebSearchCoordinator,
    snapshots: agena_web::PageSnapshots,
    host: Arc<dyn HostClient>,
}

impl WebPluginState {
    fn new(config: WebConfig, host: Arc<dyn HostClient>) -> Self {
        Self {
            fetch_coordinator: Arc::new(WebFetchCoordinator::new(WebFetchCoordinatorConfig {
                cache_ttl: Duration::from_secs(config.fetch.cache.ttl_secs),
                cache_capacity: config.fetch.cache.capacity,
                per_host_delay: Duration::from_millis(config.fetch.request.delay_ms),
            })),
            http_transport: Arc::default(),
            snapshots: agena_web::PageSnapshots::default(),
            search_coordinator: WebSearchCoordinator::new(Duration::from_millis(
                config.fetch.request.delay_ms,
            )),
            config,
            host,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
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
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    use_cache: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Omit for HTTP first and one JavaScript-shell retry if browser.enabled; false forces HTTP, true forces an isolated browser.
    render_js: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extractor: Option<agena_web::ExtractionBackend>,
    /// Initial Unicode-character preview. Default 8000, maximum 24000; use web.read for continuation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 24000))]
    max_chars: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
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
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    use_cache: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Omit for HTTP first and one JavaScript-shell retry if browser.enabled; false forces HTTP, true forces an isolated browser.
    render_js: Option<bool>,
    /// Concurrent page reads; default 4, maximum 32, capped by crawl.limits.concurrency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = agena_web::MAX_CRAWL_CONCURRENCY))]
    concurrency: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[serde(deny_unknown_fields)]
struct FetchManyInput {
    /// One to sixteen known HTTP(S) URLs; normalized duplicates are fetched once, in first-occurrence order.
    #[schemars(length(min = 1, max = MAX_FETCH_BATCH))]
    urls: Vec<String>,
    /// In-flight page reads, default 8, maximum 16. Per-host pacing still applies.
    #[serde(default)]
    #[schemars(range(min = 1, max = MAX_FETCH_BATCH))]
    concurrency: Option<u32>,
    /// Per-page Unicode-character preview, default 4000, maximum 8000. Continue each page with web.read.
    #[serde(default)]
    #[schemars(range(min = 1, max = 8000))]
    max_chars: Option<u32>,
    #[serde(default = "default_true")]
    use_cache: bool,
    /// Omit for HTTP first with conditional rendering when browser.enabled; false forces HTTP; true forces browser.
    #[serde(default)]
    render_js: Option<bool>,
    #[serde(default)]
    extractor: Option<agena_web::ExtractionBackend>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("page_id"), non_empty("page_id"))]
#[serde(deny_unknown_fields)]
struct ReadPageInput {
    /// Immutable snapshot handle returned by fetch, fetch_many, or query. Expires after 15 minutes or memory eviction.
    page_id: String,
    /// Unicode character offset from next_offset; default 0. No network refetch.
    #[serde(default)]
    offset: usize,
    /// Characters to return, default 8000, maximum 24000.
    #[serde(default)]
    #[schemars(range(min = 1, max = 24000))]
    max_chars: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("query"), non_empty("query"))]
#[serde(deny_unknown_fields)]
struct QueryCrawlInput {
    /// Search the local crawl index; does not search the public web or refresh stored pages.
    #[schemars(length(min = 1, max = 4096))]
    query: String,
    #[serde(default)]
    #[schemars(range(min = 1, max = 20))]
    max_results: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("query"), non_empty("query"))]
#[serde(deny_unknown_fields)]
struct CrawlWebSearchInput {
    #[schemars(length(max = 8192))]
    query: String,
    /// Maximum total results after domain filtering, URL deduplication and ranking.
    /// Default 5, capped by search.max_limit (default 20). Does not increase HTML retrieval budgets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 50))]
    max_results: Option<u32>,
    /// HTML only: distinct candidates to collect per engine before domain filtering.
    /// Default 10 (configurable); independent of max_results and capped by search.max_results_per_engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 50))]
    max_results_per_engine: Option<u32>,
    /// HTML only: maximum pages per engine, including the first page; not a starting page or offset.
    /// Default 1 (configurable), capped by search.max_pages_per_engine. Stops early when the candidate
    /// budget is reached or a page has no new URLs. Increase both HTML budgets for deeper retrieval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 5))]
    max_pages_per_engine: Option<u32>,
    /// Omit or use auto for the configured provider. HTML auto searches all eight engines concurrently.
    /// Use one name or a comma-separated list, e.g. "baidu,google", to query only those free HTML
    /// websites concurrently, overriding the configured API provider. Names: duckduckgo, bing, baidu,
    /// yandex, google, yahoo, brave, naver. Case and surrounding spaces are ignored; duplicates run
    /// once, in first-occurrence order. auto must stand alone; empty or unknown names are errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>", length(min = 1, max = 256))]
    engine: Option<WebSearchEngineSelection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 64))]
    allowed_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 64))]
    blocked_domains: Vec<String>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "web",
    version = env!("CARGO_PKG_VERSION"),
    display_name = "Web",
    icon = crate::plugin_metadata::icon(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/plugin-icons/web.svg"))),
    authors = ["canxin121"],
    help = "Search, fetch and index web pages. Fetch and extract content from JavaScript-heavy pages with optional rendering.",
    tags(agena_plugin_host::sdk::ToolTag::Network, agena_plugin_host::sdk::ToolTag::Fetch, agena_plugin_host::sdk::ToolTag::Discovery),
    summary = "Web search, fetch and crawl with a local index and optional page rendering.",
    translations(
        locale("zh-CN", summary = "搜索、抓取和索引网页，支持渲染依赖 JavaScript 的页面。", help = "搜索和抓取网页，将正文存入本地索引。可渲染依赖 JavaScript 的页面并提取内容。"),
        locale("zh-TW", summary = "本機網頁搜尋、擷取與爬取工具，內建爬取快取與去重，並可選擇使用瀏覽器呈現。"),
        locale("ja-JP", summary = "ローカルのWeb検索・取得・クロール機能です。クロール結果をキャッシュして重複を除去し、必要に応じてブラウザーで描画します。"),
        locale("ko-KR", summary = "로컬 웹 검색, 가져오기, 크롤링 도구입니다. 크롤링 캐시와 중복 제거 기능이 있으며 브라우저 렌더링을 선택할 수 있습니다."),
        locale("fr-FR", summary = "Recherche, récupération et exploration Web locales, avec cache intégré, déduplication et rendu par navigateur en option."),
        locale("de-DE", summary = "Lokale Websuche, Abruf und Crawling mit integriertem Crawling-Cache, Duplikaterkennung und optionalem Browser-Rendering."),
        locale("es-ES", summary = "Búsqueda, descarga y rastreo web local con caché integrada, eliminación de duplicados y renderizado opcional en navegador."),
        locale("hi-IN", summary = "स्थानीय वेब खोज, फ़ेच और क्रॉल टूल; इसमें क्रॉल कैश, डुप्लिकेट हटाना और वैकल्पिक ब्राउज़र रेंडरिंग है।"),
        locale("ar-SA", summary = "أدوات محلية للبحث في الويب وجلب الصفحات والزحف إليها، مع ذاكرة زحف مدمجة وإزالة للتكرار وخيار العرض عبر المتصفح."),
        locale("pt-BR", summary = "Busca, obtenção e rastreamento web locais, com cache integrado, remoção de duplicatas e renderização opcional no navegador.")
    ),
    settings = WebConfig,
    settings_default = default_web_config(),
    settings_metadata = web_settings_metadata(),
)]
impl WebPlugin {
    pub(crate) fn new() -> Self {
        Self {
            state: OnceLock::new(),
            workspace_root: OnceLock::new(),
            renderer: ChromeRuntime::default(),
            user_agent: "agena-web".to_owned(),
        }
    }

    #[hook(
        init,
        summary = "Initialize web tools",
        help = "Loads browser configuration and initializes the network-authorized web tools.",
        translations(
            locale(
                "zh-CN",
                summary = "初始化网页工具",
                help = "准备网页工具功能并连接所需的 Host 服务。"
            ),
            locale(
                "zh-TW",
                summary = "初始化網頁工具",
                help = "準備網頁工具功能並連接所需的 Host 服務。"
            ),
            locale(
                "fr-FR",
                summary = "Initialiser les outils web",
                help = "Prépare les outils web et connecte les services hôtes nécessaires."
            ),
            locale(
                "es-ES",
                summary = "Inicializar las herramientas web",
                help = "Prepara las herramientas web y conecta los servicios del host necesarios."
            ),
            locale(
                "pt-BR",
                summary = "Inicializar as ferramentas web",
                help = "Prepara as ferramentas web e conecta os serviços necessários do host."
            ),
            locale(
                "de-DE",
                summary = "Die Webwerkzeuge initialisieren",
                help = "Bereitet die Webwerkzeuge vor und verbindet die erforderlichen Host-Dienste."
            ),
            locale(
                "ja-JP",
                summary = "Web ツールを初期化",
                help = "必要なホストサービスに接続し、Web ツールを利用できるようにします。"
            ),
            locale(
                "ko-KR",
                summary = "웹 도구 초기화",
                help = "필요한 호스트 서비스에 연결하고 웹 도구 기능을 준비합니다."
            ),
            locale(
                "hi-IN",
                summary = "वेब टूल प्रारंभ करें",
                help = "“वेब टूल” के लिए ज़रूरी होस्ट सेवाओं से जुड़कर संबंधित कार्यक्षमता तैयार करता है।"
            ),
            locale(
                "ar-SA",
                summary = "تهيئة أدوات الويب",
                help = "يُعِدّ أدوات الويب ويربط خدمات المضيف اللازمة."
            )
        )
    )]
    async fn init(
        &self,
        ctx: agena_plugin_host::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        self.state
            .set(WebPluginState::new(
                parse_web_config(ctx.settings)?,
                host.clone(),
            ))
            .map_err(|_| PluginError::internal("web plugin initialized more than once"))?;
        self.workspace_root.set(ctx.workspace_root).map_err(|_| {
            PluginError::internal("web plugin workspace root initialized more than once")
        })?;
        let mut manifest = agena_plugin_host::sdk::Plugin::manifest(self);
        crate::plugin_tool_docs::localize_builtin_tool_docs(&mut manifest);
        Ok(agena_plugin_host::sdk::InitOutcome::ack(manifest))
    }

    #[hook(
        shutdown,
        summary = "Close the web renderer",
        help = "Stop the Chrome process used by rendered fetches and searches.",
        translations(
            locale(
                "zh-CN",
                summary = "关闭网页渲染器",
                help = "停止网页抓取和搜索用于渲染的 Chrome 进程。"
            ),
            locale(
                "zh-TW",
                summary = "關閉網頁渲染器",
                help = "停止網頁擷取和搜尋用於渲染的 Chrome 程序。"
            ),
            locale(
                "fr-FR",
                summary = "Arrêter le moteur de rendu web",
                help = "Arrête le processus Chrome qui rend les pages récupérées et les résultats de recherche."
            ),
            locale(
                "es-ES",
                summary = "Detener el renderizador web",
                help = "Detiene el proceso de Chrome que renderiza las páginas recuperadas y las de búsqueda."
            ),
            locale(
                "pt-BR",
                summary = "Encerrar a renderização web",
                help = "Encerra o processo do Chrome usado para renderizar páginas coletadas e de pesquisa."
            ),
            locale(
                "de-DE",
                summary = "Web-Rendering beenden",
                help = "Beendet den Chrome-Prozess, der abgerufene Seiten und Suchergebnisse rendert."
            ),
            locale(
                "ja-JP",
                summary = "Web レンダラーを終了",
                help = "ページ取得や検索時のレンダリングに使用する Chrome プロセスを終了します。"
            ),
            locale(
                "ko-KR",
                summary = "웹 렌더러 종료",
                help = "페이지 수집과 검색의 렌더링에 사용하는 Chrome 프로세스를 종료합니다."
            ),
            locale(
                "hi-IN",
                summary = "वेब रेंडरर बंद करें",
                help = "पेज प्राप्ति और खोज में रेंडरिंग के लिए इस्तेमाल होने वाली Chrome प्रक्रिया बंद करता है।"
            ),
            locale(
                "ar-SA",
                summary = "إيقاف عارض صفحات الويب",
                help = "يوقف عملية Chrome التي تُستخدم لعرض الصفحات المسترجعة وصفحات البحث."
            )
        )
    )]
    async fn shutdown(&self) -> SdkResult<()> {
        self.renderer.shutdown().await
    }

    fn state(&self) -> SdkResult<&WebPluginState> {
        self.state
            .get()
            .ok_or_else(|| PluginError::internal("web plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&WebConfig> {
        Ok(&self.state()?.config)
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("web plugin invoked before init"))
    }

    fn store(&self, context: &ToolInvokeContext<'_>) -> SdkResult<CrawlStore> {
        let project = if context.is_global() {
            Path::new("")
        } else {
            self.workspace_root()?
        };
        let legacy = CrawlStore::for_workspace(&web_storage_root(context, project));
        if context.is_global() {
            let root = agena_storage_layout::StorageLayout::current()
                .session_web_cache_dir(context.session_id);
            if root.is_dir() || !legacy.dir().is_dir() {
                return Ok(CrawlStore::at(&root));
            }
        }
        Ok(legacy)
    }

    fn web_fetch_options(&self) -> SdkResult<WebFetchOptions> {
        let config = self.config()?;
        let delay = (config.browser.wait.delay_ms > 0)
            .then(|| Duration::from_millis(config.browser.wait.delay_ms));
        Ok(WebFetchOptions {
            extractor: config.fetch.extractor,
            max_body_bytes: config.fetch.request.max_body_bytes as usize,
            timeout: Duration::from_secs(config.fetch.request.timeout_secs),
            user_agent: self.user_agent.clone(),
            respect_robots_txt: config.fetch.request.respect_robots_txt,
            browser: BrowserRenderOptions {
                wait_for_network_idle: config.browser.wait.for_network_idle,
                wait_for_selector: config.browser.wait.for_selector.clone(),
                wait_timeout: Duration::from_secs(config.browser.wait.timeout_secs),
                delay,
            },
        })
    }

    async fn validate_network_target(&self, url: &url::Url) -> SdkResult<()> {
        let state = self.state()?;
        network_policy::require_browser_target(
            url,
            &state.config.network.fake_ip_ranges,
            state.host.as_ref(),
        )
        .await
    }

    async fn fetch_page_with_extractor(
        &self,
        url: &url::Url,
        use_cache: bool,
        render_js: bool,
        extractor: agena_web::ExtractionBackend,
    ) -> SdkResult<FetchedPage> {
        if render_js {
            rendered_fetch::require_runtime()?;
        }
        let state = self.state()?;
        if !state.config.fetch.enabled {
            return Err(PluginError::internal(
                "web fetching is disabled by plugin config `fetch.enabled`",
            ));
        }
        let require_page_permission = |target: String| async move {
            if render_js {
                state.host.require_network_permission(target).await
            } else {
                state.host.require_pinned_http_permission(target).await
            }
        };
        require_page_permission(url.to_string()).await?;
        // Alternate per-call extractors must not pollute the configured cache.
        let use_cache = use_cache && extractor == state.config.fetch.extractor;
        let page = state
            .fetch_coordinator
            .fetch_or_cached(url, render_js, use_cache, || async {
                let mut options = self.web_fetch_options()?;
                options.extractor = extractor;
                if render_js {
                    Box::pin(rendered_fetch::fetch(self, url, &options)).await
                } else {
                    let _slot = HTTP_PAGE_FETCHES
                        .acquire()
                        .await
                        .expect("page admission is never closed");
                    let source = state
                        .http_transport
                        .fetch(
                            url,
                            &options,
                            &state.fetch_coordinator,
                            |target| async move {
                                state
                                    .host
                                    .require_pinned_http_permission(target.to_string())
                                    .await?;
                                resolve_public_http_route(
                                    &target,
                                    &state.config.network,
                                    state.host.as_ref(),
                                )
                                .await
                            },
                        )
                        .await?;
                    fetch_transport::extract(source, url.clone(), extractor).await
                }
            })
            .await?;
        if !page.final_url.is_empty() {
            require_page_permission(page.final_url.clone()).await?;
        }
        Ok(page)
    }

    #[tool(
        summary = "Fetch one web page and inspect its actual content.",
        help = "Read actual page evidence after search. Markdown preserves main content, sibling articles, code, tables and resolved links. Returns content_status, extraction_strategy, final_url, warnings, page_id and next_offset. max_chars defaults to 8000 (maximum 24000); continue long pages with web.read without refetching. prompt adds relevant excerpts from the full snapshot. Use fetch_many for independent known URLs. Omit render_js for HTTP first and one JavaScript-shell browser retry when browser.enabled; false forces HTTP, true forces an isolated browser. One shared fetch deadline includes admission, HTTP, rendering and extraction. Blocked/empty/partial pages are not reusable cache entries. Read statuses before using the text as evidence.",
        tags(network, read_only),
        translations(
            locale("ar-SA", summary = "اجلب صفحة ويب واحدة وافحص محتواها الفعلي."),
            locale(
                "de-DE",
                summary = "Ruft eine Webseite ab und prüft ihren tatsächlichen Inhalt."
            ),
            locale(
                "es-ES",
                summary = "Obtiene una página web e inspecciona su contenido real."
            ),
            locale(
                "fr-FR",
                summary = "Récupère une page web et inspecte son contenu réel."
            ),
            locale("hi-IN", summary = "एक वेब पेज प्राप्त करके उसकी वास्तविक सामग्री देखता है।"),
            locale("ja-JP", summary = "ウェブページを1件取得し、実際の内容を確認します。"),
            locale("ko-KR", summary = "웹 페이지 하나를 가져와 실제 내용을 확인합니다."),
            locale(
                "pt-BR",
                summary = "Obtém uma página web e inspeciona seu conteúdo real."
            ),
            locale("zh-CN", summary = "获取一个网页并检查其实际内容。"),
            locale("zh-TW", summary = "取得一個網頁並檢查其實際內容。")
        )
    )]
    async fn invoke_fetch(&self, input: &CrawlFetchInput) -> SdkResult<ToolInvokeOutput> {
        let max_chars = content::bounded(input.max_chars, 8000, 24000, "max_chars")?;
        let url = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?;
        let page = self
            .fetch_selected(&url, input.use_cache, input.render_js, input.extractor)
            .await?;
        self.page_output(page, input.prompt.as_deref(), max_chars)
            .await
    }

    #[tool(
        summary = "Read several independent web pages concurrently.",
        help = "Fetch 1–16 known URLs concurrently (default concurrency 8, maximum 16), deduplicating normalized URLs in input order. Use after search to gather independent sources in one call. Each result includes its own status/error, content_status, bounded Markdown, page_id and next_offset for web.read. Failures do not discard other pages; partial also flags unreadable or truncated sources. Per-host pacing and global HTTP/browser limits still apply. Omit render_js for HTTP first and one conditional JavaScript-shell browser retry if enabled; true forces browser, false forces HTTP. No CAPTCHA bypass. max_chars is per page (default 4000, maximum 8000).",
        stream = invoke_fetch_many_stream,
        tags(network, read_only),
    translations(
        locale("ar-SA", summary = "اقرأ عدة صفحات ويب مستقلة في وقت واحد."),
        locale("de-DE", summary = "Liest mehrere unabhängige Webseiten gleichzeitig."),
        locale("es-ES", summary = "Lee varias páginas web independientes a la vez."),
        locale("fr-FR", summary = "Lit plusieurs pages web indépendantes en parallèle."),
        locale("hi-IN", summary = "कई स्वतंत्र वेब पेज एक साथ पढ़ता है।"),
        locale("ja-JP", summary = "複数の独立したウェブページを同時に読み取ります。"),
        locale("ko-KR", summary = "여러 독립 웹 페이지를 동시에 읽습니다."),
        locale("pt-BR", summary = "Lê várias páginas web independentes ao mesmo tempo."),
        locale("zh-CN", summary = "并发阅读多个独立网页。"),
        locale("zh-TW", summary = "同時讀取多個獨立網頁。")
)
)]
    async fn invoke_fetch_many(&self, input: &FetchManyInput) -> SdkResult<ToolInvokeOutput> {
        self.run_fetch_many(input, None).await
    }

    /// Streaming variant: report every URL as it settles, so a reader watches
    /// the pages arrive instead of waiting for the slowest one.
    async fn invoke_fetch_many_stream(
        &self,
        sink: ToolStreamSink,
        input: &FetchManyInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.run_fetch_many(input, Some(&sink)).await
    }

    async fn run_fetch_many(
        &self,
        input: &FetchManyInput,
        progress: Option<&ToolStreamSink>,
    ) -> SdkResult<ToolInvokeOutput> {
        let urls = content::unique_urls(&input.urls)?;
        let concurrency = content::bounded(
            input.concurrency,
            DEFAULT_FETCH_BATCH_CONCURRENCY,
            MAX_FETCH_BATCH,
            "concurrency",
        )?;
        let max_chars = content::bounded(input.max_chars, 4000, 8000, "max_chars")?;
        let total = urls.len();
        let done = std::sync::atomic::AtomicUsize::new(0);
        let done = &done;
        if let Some(sink) = progress {
            sink.text(format!(
                "Fetching {total} URL(s), concurrency {concurrency}\n"
            ))
            .await;
        }
        let results = content::batch(urls, concurrency, |raw| async move {
            let result = async {
                let url = prepare_fetch_url(&raw).map_err(crawl_error_to_plugin)?;
                let page = self.fetch_selected(&url, input.use_cache, input.render_js, input.extractor).await?;
                let usable = (200..300).contains(&page.status) && !page.truncated && page.content_status == agena_web::PageContentStatus::Readable;
                let output = self.page_output(page, None, max_chars).await?;
                Ok::<_, PluginError>((output, usable))
            }.await;
            if let Some(sink) = progress {
                let position = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let state = if result.is_ok() { "ok" } else { "failed" };
                sink.text(format!("[{position}/{total}] {state} {raw}\n")).await;
            }
            match result {
                Ok((output, usable)) => (serde_json::json!({"url":raw,"ok":true,"usable":usable,"page":output.payload}), output.output_text, usable, None),
                Err(error) => {
                    tracing::warn!(url = %raw, diagnostic = %error.diagnostic_message(), "batch page failed");
                    let message = &error.failure.user.fallback;
                    (serde_json::json!({"url":raw,"ok":false,"usable":false,"error_kind":error.kind,"error":message,"failure_id":error.failure.id}), format!("URL: {raw}\n{message}"), false, Some(error))
                }
            }
        }).await;
        let mut results = results;
        if results.iter().all(|entry| entry.3.is_some())
            && let Some(error) = results.iter_mut().find_map(|entry| {
                entry.3.as_ref().filter(|error| {
                    matches!(
                        error.kind,
                        agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
                            | agena_plugin_host::sdk::PluginErrorKind::UserDeclined
                    )
                })?;
                entry.3.take()
            })
        {
            return Err(error);
        }
        let usable = results.iter().filter(|entry| entry.2).count();
        let text = results
            .iter()
            .map(|entry| entry.1.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        let payload = serde_json::json!({"input_count":input.urls.len(),"unique_url_count":results.len(),"concurrency":concurrency,"usable_count":usable,"partial":usable < results.len(),"results":results.into_iter().map(|entry| entry.0).collect::<Vec<_>>()});
        Ok(ToolInvokeOutput::from_parts(
            "web fetch_many",
            format!("{usable} readable page(s)"),
            text,
            Some(payload),
            BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Continue reading an immutable fetched-page snapshot.",
        help = "Use page_id and next_offset returned by fetch, fetch_many or query. Returns a contiguous slice of the same extracted Markdown without refetching; offset counts Unicode characters, not bytes. max_chars defaults to 8000, maximum 24000. next_offset=null means the end of available extracted text, not necessarily a complete source: inspect truncated and content_status. Snapshots expire after 15 minutes and share a 32 MiB memory budget; if evicted, fetch the URL or query the crawl index again. Revalidates requested and final URL permissions.",
        tags(network, read_only),
        translations(
            locale("ar-SA", summary = "تابع قراءة لقطة ثابتة لصفحة تم جلبها."),
            locale(
                "de-DE",
                summary = "Liest einen unveränderlichen Schnappschuss einer abgerufenen Seite weiter."
            ),
            locale(
                "es-ES",
                summary = "Continúa leyendo una instantánea inmutable de una página obtenida."
            ),
            locale(
                "fr-FR",
                summary = "Poursuit la lecture d’un instantané immuable d’une page récupérée."
            ),
            locale("hi-IN", summary = "प्राप्त पेज के अपरिवर्तनीय स्नैपशॉट को पढ़ना जारी रखता है।"),
            locale(
                "ja-JP",
                summary = "取得済みページの不変スナップショットの読み取りを続けます。"
            ),
            locale("ko-KR", summary = "가져온 페이지의 변경 불가 스냅샷을 계속 읽습니다."),
            locale(
                "pt-BR",
                summary = "Continua a leitura de um instantâneo imutável de uma página obtida."
            ),
            locale("zh-CN", summary = "继续阅读已获取页面的不可变快照。"),
            locale("zh-TW", summary = "繼續閱讀已取得頁面的不可變快照。")
        )
    )]
    async fn invoke_read(&self, input: &ReadPageInput) -> SdkResult<ToolInvokeOutput> {
        let max_chars = content::bounded(input.max_chars, 8000, 24000, "max_chars")?;
        let page = self
            .state()?
            .snapshots
            .get(&input.page_id)
            .await
            .map_err(crawl_error_to_plugin)?;
        self.authorize_page(&page).await?;
        self.slice_output(&input.page_id, &page, input.offset, max_chars, None)
    }

    #[tool(
        summary = "Find evidence in locally crawled pages and obtain readable snapshots.",
        help = "Search the index populated by web.crawl, without new network requests. max_results defaults to 5, maximum 20. Hits include source URL, stored fetch time, chunk preview, page_id, and read_offset for web.read. Stored content may be stale; refetch its URL when freshness matters. Permissions are revalidated before exposing each hit; denied hits are omitted and counted. This searches stored documents only; use web.search to discover public pages.",
        tags(network, read_only),
        translations(
            locale(
                "ar-SA",
                summary = "ابحث عن الأدلة في الصفحات المزحوفة محليًا واحصل على لقطات قابلة للقراءة."
            ),
            locale(
                "de-DE",
                summary = "Findet Belege in lokal gecrawlten Seiten und liefert lesbare Schnappschüsse."
            ),
            locale(
                "es-ES",
                summary = "Busca pruebas en páginas rastreadas localmente y obtiene instantáneas legibles."
            ),
            locale(
                "fr-FR",
                summary = "Trouve des preuves dans les pages explorées localement et fournit des instantanés lisibles."
            ),
            locale(
                "hi-IN",
                summary = "स्थानीय रूप से क्रॉल किए गए पेजों में साक्ष्य खोजता है और पठनीय स्नैपशॉट देता है।"
            ),
            locale(
                "ja-JP",
                summary = "ローカルでクロール済みのページから根拠を探し、読めるスナップショットを返します。"
            ),
            locale(
                "ko-KR",
                summary = "로컬에서 크롤링한 페이지에서 근거를 찾아 읽을 수 있는 스냅샷을 제공합니다."
            ),
            locale(
                "pt-BR",
                summary = "Encontra evidências em páginas rastreadas localmente e fornece instantâneos legíveis."
            ),
            locale("zh-CN", summary = "在本地已抓取的页面中查找证据并获取可读快照。"),
            locale("zh-TW", summary = "在本機已檢索的頁面中尋找證據並取得可讀快照。")
        )
    )]
    async fn invoke_query(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &QueryCrawlInput,
    ) -> SdkResult<ToolInvokeOutput> {
        if input.query.trim().is_empty() || input.query.chars().count() > 4096 {
            return Err(PluginError::invalid_params(
                "query must contain 1–4096 characters",
            ));
        }
        let limit = content::bounded(input.max_results, 5, 20, "max_results")?;
        let hits = self
            .store(context)?
            .search_async(input.query.clone(), limit)
            .await
            .map_err(crawl_error_to_plugin)?;
        let mut results = Vec::new();
        let mut omitted_count = 0usize;
        for (hit, document) in hits {
            if document.truncated
                || !(200..300).contains(&document.status)
                || document.content_status != agena_web::PageContentStatus::Readable
            {
                omitted_count += 1;
                continue;
            }
            let fetched_at = document.fetched_at;
            let read_offset = document
                .chunks
                .get(hit.chunk_index as usize)
                .and_then(|chunk| document.markdown.find(chunk))
                .map(|byte| document.markdown[..byte].chars().count())
                .unwrap_or(0);
            let page = document.into_fetched_page();
            if self.authorize_page(&page).await.is_err() {
                omitted_count += 1;
                continue;
            }
            let page_id = self
                .state()?
                .snapshots
                .insert(page)
                .await
                .map_err(crawl_error_to_plugin)?;
            results.push(serde_json::json!({"hit":hit,"fetched_at":fetched_at,"page_id":page_id,"read_offset":read_offset}));
        }
        let payload = serde_json::json!({"results":results,"omitted_count":omitted_count,"source":"local_crawl_index"});
        let text = serde_json::to_string_pretty(&payload)
            .map_err(|error| PluginError::internal_error(&error))?;
        Ok(ToolInvokeOutput::from_parts(
            "web query",
            format!("{} local hit(s)", results.len()),
            text,
            Some(payload),
            BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Crawl a site and cache indexed pages locally.",
        help = "Traverse links breadth first with bounded concurrency (default 4, maximum 32, capped by crawl.limits.concurrency). max_pages counts attempts and cache hits, including failures. same_host_only defaults true. Per-host pacing, robots, depth and URL-discovery budgets still apply. Omit render_js for HTTP first with conditional rendering when enabled; true forces browser and false forces HTTP. Report includes per-URL page_errors and effective concurrency. Only complete, readable 2xx documents enter storage. Use web.query to locate evidence in the resulting local index and web.read to read it.",
        stream = invoke_crawl_stream,
        tags(network, discovery, mutate),
    translations(
        locale("ar-SA", summary = "ازحف موقعًا واخزّن الصفحات المفهرسة محليًا."),
        locale("de-DE", summary = "Crawlt eine Website und speichert indizierte Seiten lokal zwischen."),
        locale("es-ES", summary = "Rastrea un sitio y almacena en caché local las páginas indexadas."),
        locale("fr-FR", summary = "Explore un site et met en cache local les pages indexées."),
        locale("hi-IN", summary = "किसी साइट को क्रॉल करता है और अनुक्रमित पेज स्थानीय रूप से कैश करता है।"),
        locale("ja-JP", summary = "サイトをクロールし、索引付けしたページをローカルにキャッシュします。"),
        locale("ko-KR", summary = "사이트를 크롤링하고 색인된 페이지를 로컬에 캐시합니다."),
        locale("pt-BR", summary = "Rastreia um site e armazena em cache local as páginas indexadas."),
        locale("zh-CN", summary = "抓取站点并在本地缓存已索引页面。"),
        locale("zh-TW", summary = "檢索網站並在本機快取已索引頁面。")
)
)]
    async fn invoke_crawl(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &CrawlRunInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.run_crawl(context, input, None).await
    }

    /// Streaming variant: a crawl runs for a while, so report where it starts
    /// before the final report arrives.
    async fn invoke_crawl_stream(
        &self,
        sink: ToolStreamSink,
        context: &ToolInvokeContext<'_>,
        input: &CrawlRunInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.run_crawl(context, input, Some(&sink)).await
    }

    async fn run_crawl(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &CrawlRunInput,
        progress: Option<&ToolStreamSink>,
    ) -> SdkResult<ToolInvokeOutput> {
        if let Some(sink) = progress {
            sink.text(format!("Crawling {}\n", input.start_url)).await;
        }
        let start_url =
            prepare_fetch_url(input.start_url.as_str()).map_err(crawl_error_to_plugin)?;
        let store = self.store(context)?;
        let config = self.config()?;
        let options = CrawlRunOptions {
            extraction_backend: config.fetch.extractor,
            concurrency: content::bounded(
                input.concurrency,
                config.crawl.defaults.concurrency,
                agena_web::MAX_CRAWL_CONCURRENCY,
                "concurrency",
            )?
            .min(config.crawl.limits.concurrency as usize),
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
            render_js: input.render_js == Some(true),
            allow_rendered_cache: input.render_js.is_none() && config.browser.enabled,
            document_cache_ttl: Duration::from_secs(config.crawl.indexing.document_cache_ttl_secs),
            max_chunk_chars: config.crawl.indexing.chunk_chars as usize,
            near_duplicate_hamming_distance: config.crawl.indexing.near_duplicate_hamming_distance,
            store_retention: Some(CrawlStoreRetention {
                max_documents: config.store.retention.max_documents as usize,
                max_total_bytes: config.store.retention.max_bytes,
            }),
        };
        let fetcher = PluginPageFetcher {
            plugin: self,
            render_js: input.render_js,
        };
        let report = crawl_site(&start_url, &store, &options, &fetcher)
            .await
            .map_err(crawl_error_to_plugin)?;
        let text = format_crawl_run(&report);
        let summary = format!(
            "{} indexed · {} cached · {} failures",
            report.stored_count, report.cached_count, report.failure_count
        );
        let payload =
            serde_json::to_value(report).map_err(|err| PluginError::internal_error(&err))?;
        Ok(ToolInvokeOutput::from_parts(
            format!("web crawl {}", start_url),
            summary,
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Find candidate public-web pages to fetch.",
        help = "Discover candidate pages; fetch 1-3 relevant URLs for factual answers. Omit engine or use auto for the configured provider (HTML, Brave, Tavily, Exa or SearXNG). HTML auto searches eight free public websites (DuckDuckGo, Bing, Baidu, Yandex, Google, Yahoo, Brave and Naver) concurrently, deduplicates URLs and combines rankings. Set engine to one name or a comma-separated list such as baidu,google to search only those HTML websites, overriding API settings. Case and surrounding spaces are ignored; duplicates run once in first-occurrence order. auto must stand alone; empty and unknown names are errors. max_results caps the final total; max_results_per_engine and max_pages_per_engine independently cap each selected HTML source (defaults: 10 candidates, 1 page including the first). Increase both source budgets for deeper retrieval; filtering or a larger final limit never triggers extra pages. No global offset or continuation cursor. Effective limits appear in the HTML response. Each result's engines lists contributing engines. The source field identifies the publisher. Multi-engine HTML searches preserve successful sources with partial and engine_errors; all selected engines failing is an error. A single engine's failure is returned directly. Inspect engine_reports for per-source result counts, page counts, cache hits and structured issues. Automatic one-page HTML searches have an 8-second active budget and wait 750 ms for extra engines after reaching max_results with distinct, domain-matching URLs. Outstanding engines are marked skipped and the response is partial. Both timings are configurable. Explicit engine selections and multi-page searches wait within the configured fetch timeout. Request starts are paced per engine. Up to four requests to the same engine can overlap, and successful identical queries are reused for two minutes. Rate limits honor Retry-After and verification failures enter cooldown: respect retry_after_secs, use other working sources and avoid bursts of overlapping queries. Verification and unknown pages are failures, not empty results. A confirmed JavaScript shell gets one browser attempt per page when browser.enabled and the current tool session supports rendering. Later-page failures preserve earlier results with partial diagnostics. Browser rendering does not solve human verification or consent. API providers return at most 20 results without switching providers and reject HTML-only budget arguments. Domain filters accept bare hostnames; exclusions win. Snippets are previews, not fetched-page evidence.",
        tags(network, discovery, read_only),
        translations(
            locale("ar-SA", summary = "اعثر على صفحات ويب عامة مرشحة لجلبها."),
            locale(
                "de-DE",
                summary = "Findet Kandidatenseiten im offenen Web zum Abrufen."
            ),
            locale(
                "es-ES",
                summary = "Encuentra páginas web públicas candidatas para obtener."
            ),
            locale(
                "fr-FR",
                summary = "Trouve des pages web publiques candidates à récupérer."
            ),
            locale("hi-IN", summary = "प्राप्त करने योग्य सार्वजनिक वेब पेजों के उम्मीदवार ढूँढता है।"),
            locale("ja-JP", summary = "取得候補となる公開ウェブページを探します。"),
            locale("ko-KR", summary = "가져올 공개 웹 페이지 후보를 찾습니다."),
            locale(
                "pt-BR",
                summary = "Encontra páginas web públicas candidatas para obter."
            ),
            locale("zh-CN", summary = "查找可获取的候选公开网页。"),
            locale("zh-TW", summary = "尋找可取得的候選公開網頁。")
        )
    )]
    async fn invoke_search(&self, input: &CrawlWebSearchInput) -> SdkResult<ToolInvokeOutput> {
        search_provider::validate_input(input)?;
        let query = input.query.as_str();
        let config = self.config()?;
        let limits = search_aggregate::HtmlSearchLimits::from_input(input, &config.search);
        if input
            .engine
            .as_ref()
            .is_none_or(WebSearchEngineSelection::is_auto)
            && config.search.provider != WebSearchBackend::Html
        {
            if input.max_results_per_engine.is_some() || input.max_pages_per_engine.is_some() {
                return Err(PluginError::invalid_params(
                    "max_results_per_engine and max_pages_per_engine require HTML search; select an explicit engine or configure search.provider=html",
                ));
            }
            return self.search_with_provider(input, limits.max_results).await;
        }
        let fast_auto = limits.max_pages_per_engine == 1
            && input
                .engine
                .as_ref()
                .is_none_or(WebSearchEngineSelection::is_auto);
        let timeout = if fast_auto {
            config
                .search
                .auto_timeout_secs
                .min(config.fetch.request.timeout_secs)
        } else {
            config.fetch.request.timeout_secs
        };
        let timeout = Duration::from_secs(timeout);
        let grace = (fast_auto && config.search.auto_grace_ms > 0)
            .then(|| Duration::from_millis(config.search.auto_grace_ms));
        search_aggregate::search(
            input,
            limits,
            timeout,
            grace,
            |engine, source_limit, max_pages| {
                Box::pin(self.search_with_engine(query, source_limit, max_pages, engine, timeout))
            },
        )
        .await?
        .into_tool_output()
    }

    async fn search_with_engine(
        &self,
        query: &str,
        limit: usize,
        max_pages: usize,
        engine: WebSearchEngine,
        timeout: Duration,
    ) -> SdkResult<WebSearchResponse> {
        // Let a later page return already collected results before the aggregate ends.
        let deadline = agena_plugin_host::sdk::permission_wait::Deadline::new(
            timeout.saturating_sub(Duration::from_millis(50)),
        );
        let engine_url = url::Url::parse(engine.permission_url())
            .map_err(|err| PluginError::internal_error(&err))?;
        let state = self.state()?;
        state
            .host
            .require_pinned_http_permission(engine_url.to_string())
            .await?;
        let route =
            resolve_public_http_route(&engine_url, &state.config.network, state.host.as_ref())
                .await?;
        let config = &state.config;
        let options = WebSearchOptions {
            engine,
            route: Some(route),
            limit,
            max_pages,
            timeout: deadline.remaining(),
            user_agent: self.user_agent.clone(),
        };
        state
            .search_coordinator
            .search_with_renderer(
                query,
                &options,
                config.browser.enabled && rendered_fetch::runtime_available(),
                |url, remaining| {
                    Box::pin(async move {
                        rendered_fetch::search_source(self, &url, remaining)
                            .await
                            .map(Some)
                            .map_err(plugin_error_to_crawl)
                    })
                },
            )
            .await
            .map_err(search_error_to_plugin)
    }
}

async fn resolve_public_http_route(
    url: &url::Url,
    network: &network_policy::WebNetworkConfig,
    host: &dyn HostClient,
) -> SdkResult<agena_web::WebHttpRoute> {
    let route = agena_web::WebHttpRoute::for_url(url, network.proxy.as_deref())
        .map_err(crawl_error_to_plugin)?;
    if route.remote_dns() {
        return Err(PluginError::from_kind(
            agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
            "the web proxy uses remote DNS, so approved destination IPs cannot be pinned; configure a direct route or a SOCKS proxy with local DNS",
        ));
    }
    let addresses = resolve_public_network_target(url, network).await?;
    let addresses = authorized_network_addresses(url, addresses, host, false).await?;
    crate::sandbox_operations::http_route(host, url, route.with_addresses(addresses)).await
}

fn ensure_search_available(successes: usize, failures: &[String]) -> SdkResult<()> {
    if successes == 0 && !failures.is_empty() {
        let mut error = PluginError::internal(format!(
            "all search engines failed; this is not an empty search result: {}",
            failures.join("; ")
        ));
        error.failure.recovery = agena_failure::RecoveryDirective::ChooseAlternative;
        error.failure.retry = agena_failure::RetryDirective::UseAlternative;
        return Err(error);
    }
    Ok(())
}

fn plugin_error_to_crawl(error: PluginError) -> agena_web::CrawlError {
    if matches!(
        error.kind,
        agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
            | agena_plugin_host::sdk::PluginErrorKind::UserDeclined
    ) {
        agena_web::CrawlError::AccessRejected(Box::new(error))
    } else {
        agena_web::CrawlError::InvalidInput(error.diagnostic_message().to_owned())
    }
}

fn crawl_error_to_plugin(err: agena_web::CrawlError) -> PluginError {
    if let agena_web::CrawlError::AccessRejected(source) = err {
        return match source.downcast::<PluginError>() {
            Ok(error) => *error,
            Err(error) => PluginError::internal(error.to_string()),
        };
    }
    PluginError::internal_error(&err)
}

fn search_error_to_plugin(err: agena_web::CrawlError) -> PluginError {
    if matches!(err, agena_web::CrawlError::AccessRejected(_)) {
        return crawl_error_to_plugin(err);
    }
    let mut error = PluginError::internal_error(&err);
    if let agena_web::CrawlError::SearchUnavailable { issue, .. } = &err {
        error.diagnostic.data = Some(serde_json::json!({"search_issue": issue}));
        error.failure.recovery = agena_failure::RecoveryDirective::ChooseAlternative;
        error.failure.retry = agena_failure::RetryDirective::UseAlternative;
    }
    error
}

struct PluginPageFetcher<'a> {
    plugin: &'a WebPlugin,
    render_js: Option<bool>,
}

impl CrawlPageFetcher for PluginPageFetcher<'_> {
    fn authorize_cached<'a>(
        &'a self,
        requested_url: &'a url::Url,
        document: &'a agena_web::StoredDocument,
    ) -> Pin<Box<dyn Future<Output = Result<(), agena_web::CrawlError>> + Send + 'a>> {
        Box::pin(async move {
            let state = self.plugin.state().map_err(plugin_error_to_crawl)?;
            // Legacy documents have no final URL; do not infer it from an HTML
            // canonical hint, which need not be the transport destination.
            if document.final_url.is_empty() {
                return Err(agena_web::CrawlError::InvalidInput(
                    "cached document predates transport URL tracking; crawl with use_cache=false"
                        .into(),
                ));
            }
            let mut checked = BTreeSet::new();
            for url in [requested_url.as_str(), &document.url, &document.final_url] {
                if checked.insert(url) {
                    state
                        .host
                        .require_network_permission(url.to_owned())
                        .await
                        .map_err(plugin_error_to_crawl)?;
                }
            }
            Ok(())
        })
    }

    fn fetch_page<'a>(
        &'a self,
        url: &'a url::Url,
        use_cache: bool,
        _render_js: bool,
    ) -> Pin<Box<dyn Future<Output = Result<FetchedPage, agena_web::CrawlError>> + Send + 'a>> {
        Box::pin(async move {
            self.plugin
                .fetch_selected(url, use_cache, self.render_js, None)
                .await
                .map_err(plugin_error_to_crawl)
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
        serde_json::from_value(value).map_err(|error| {
            plugin_internal_error_with_context("invalid web plugin config", &error)
        })?
    };
    validate_web_config(&config)?;
    Ok(config)
}

fn validate_web_config(web: &WebConfig) -> SdkResult<()> {
    network_policy::validate_config(&web.network)?;
    if web.fetch.request.max_body_bytes > 32 * 1024 * 1024 || web.fetch.request.timeout_secs > 120 {
        return Err(PluginError::invalid_params(
            "fetch.request allows at most 32 MiB and 120 seconds",
        ));
    }
    if !(1..=agena_web::MAX_CRAWL_CONCURRENCY).contains(&web.crawl.limits.concurrency)
        || web.crawl.defaults.concurrency == 0
        || web.crawl.defaults.concurrency > web.crawl.limits.concurrency
    {
        return Err(PluginError::invalid_params(format!(
            "crawl concurrency requires 1 <= defaults.concurrency <= limits.concurrency <= {}",
            agena_web::MAX_CRAWL_CONCURRENCY
        )));
    }
    if web.crawl.limits.max_pages > 1000 || web.crawl.limits.max_depth > 16 {
        return Err(PluginError::invalid_params(
            "crawl limits allow at most 1000 pages and depth 16",
        ));
    }
    search_provider::validate_config(&web.search)?;
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
            return Err(PluginError::internal(format!(
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
            return Err(PluginError::internal(format!(
                "web plugin config `{label}` must be greater than 0"
            )));
        }
    }
    if web.store.retention.max_documents == 0 {
        return Err(PluginError::internal(
            "web plugin config `store.retention.max_documents` must be greater than 0",
        ));
    }
    if web.crawl.defaults.max_pages > web.crawl.limits.max_pages {
        return Err(PluginError::internal(
            "web plugin config `crawl.defaults.max_pages` must be less than or equal to `crawl.limits.max_pages`",
        ));
    }
    if web.crawl.defaults.max_depth > web.crawl.limits.max_depth {
        return Err(PluginError::internal(
            "web plugin config `crawl.defaults.max_depth` must be less than or equal to `crawl.limits.max_depth`",
        ));
    }
    if web.search.default_limit > web.search.max_limit {
        return Err(PluginError::internal(
            "web plugin config `search.default_limit` must be less than or equal to `search.max_limit`",
        ));
    }
    web.browser.chrome_options().validate()?;
    if web
        .browser
        .wait
        .for_selector
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(PluginError::internal(
            "web plugin config `browser.wait.for_selector` must not be empty when set",
        ));
    }
    Ok(())
}

fn format_fetched_page(page: &FetchedPage, focus: Option<&str>) -> String {
    let mut lines = vec![format!("Title: {}", page.title)];
    lines.push(format!("URL: {}", page.canonical_url));
    lines.push(format!(
        "Status: {} · Content: {:?} · Strategy: {}",
        page.status, page.content_status, page.extraction_strategy
    ));
    lines.push(format!(
        "Rendered: {}",
        if page.rendered { "yes" } else { "no" }
    ));
    lines.push(format!("Final URL: {}", page.final_url));
    lines.push(format!(
        "Extractor: {}",
        match page.extraction_backend {
            agena_web::ExtractionBackend::Readability => "readability",
            agena_web::ExtractionBackend::Trafilatura => "trafilatura",
        }
    ));
    for warning in &page.warnings {
        lines.push(format!("Warning: {warning}"));
    }
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
        lines.push(page.markdown.clone());
    }
    lines.join("\n")
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
    lines.push(format!(
        "Fetch attempts: {}. Discovered URLs: {}. Partial/truncated: {}.",
        output.attempted_count, output.discovered_count, output.truncated
    ));
    for document in &output.documents {
        lines.push(format!(
            "- {} [{} chunk(s), depth {}] {}",
            document.title, document.chunk_count, document.depth, document.url
        ));
    }
    lines.push(format!(
        "Concurrency: {}. Search these documents with web.query, then continue with web.read.",
        output.concurrency
    ));
    for error in &output.page_errors {
        lines.push(format!(
            "- Failed {}: {} (HTTP {:?}, content {:?})",
            error.url, error.reason, error.status, error.content_status
        ));
    }
    if !output.failures.is_empty() {
        lines.push("Failures:".to_string());
        lines.extend(
            output
                .failures
                .iter()
                .take(5)
                .map(|failure| format!("- {}", failure.user.fallback)),
        );
    }
    lines.join("\n")
}

fn domain_allowed(url: &str, allow: &[String], block: &[String]) -> bool {
    agena_web::search_domain_allowed(url, allow, block)
}

#[cfg(test)]
mod permission_tests;
#[cfg(test)]
mod sandbox_network_tests;
