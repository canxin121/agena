mod content;
mod downloads;
mod fetch_transport;
mod playwright;
mod rendered_fetch;
use playwright::BrowserInteractionBackend;
mod search_aggregate;
mod search_provider;
mod search_selection;
use search_provider::WebSearchBackend;
use search_selection::WebSearchEngineSelection;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::future::Future;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_web::{
    BrowserRenderOptions, CrawlPageFetcher, CrawlRunOptions, CrawlRunReport, CrawlStore,
    CrawlStoreRetention, FetchedPage, LocalBrowserOptions, SpiderFetchOptions, WebFetchCoordinator,
    WebFetchCoordinatorConfig, WebSearchCoordinator, WebSearchEngine, WebSearchOptions,
    WebSearchResponse, crawl_site, local_browser_endpoint, local_browser_running,
    local_browser_touch, prepare_fetch_url, preview_text, results_to_text, shutdown_local_browser,
};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OnceCell, Semaphore, mpsc, oneshot};

use agena_domain::{
    BackgroundActivity, BackgroundActivityKind, BackgroundActivityLogLine,
    BackgroundActivityLogRead, BackgroundActivityStatus,
};
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::attachment::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    ActivitySourceAdapter, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
    ToolStreamSink, async_trait,
};

pub(crate) const WEB_PLUGIN_ID: &str = "agena.web";

fn browser_owner(context: &ToolInvokeContext<'_>) -> SdkResult<agena_runtime_tools::TerminalOwner> {
    Ok(agena_runtime_tools::TerminalOwner {
        workspace: Path::new(context.workspace_root)
            .canonicalize()
            .map_err(|error| PluginError::internal_error(&error))?,
        session_id: Some(context.session_id),
    })
}

fn plugin_internal_error_with_context(
    context: &str,
    error: &(dyn std::error::Error + 'static),
) -> PluginError {
    PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
        context, error,
    ))
}

fn plugin_internal_failure_with_context(context: &str, error: &PluginError) -> PluginError {
    PluginError::internal(format!("{context}: {}", error.diagnostic_message()))
}

fn log_secondary_plugin_failure(operation: &str, error: &PluginError) {
    tracing::warn!(
        operation,
        diagnostic = %error.diagnostic_message(),
        "secondary browser plugin operation failed"
    );
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WebConfig {
    pub fetch: WebFetchConfig,
    pub crawl: WebCrawlConfig,
    pub search: WebSearchConfig,
    pub store: WebStoreConfig,
    pub browser: WebBrowserConfig,
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
            concurrency: 8,
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
/// Browser settings of the web plugin.
pub struct WebBrowserConfig {
    pub enabled: bool,
    /// Optional mature click/fill/wait adapter; browser ownership stays native.
    pub interaction_backend: BrowserInteractionBackend,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    pub wait: WebBrowserWaitConfig,
    /// Seconds of inactivity after which the managed browser process is shut
    /// down automatically. `0` disables idle auto-close.
    #[serde(default = "default_web_browser_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    /// Opt in to unredacted page console text. May contain sensitive page data.
    #[serde(default)]
    pub capture_console_text: bool,
}

impl Default for WebBrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interaction_backend: BrowserInteractionBackend::default(),
            executable_path: None,
            wait: WebBrowserWaitConfig::default(),
            idle_timeout_secs: default_web_browser_idle_timeout_secs(),
            capture_console_text: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Wait settings of the web plugin browser.
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
            "Default simultaneous page reads per crawl (1–8).",
        ),
        (
            "/crawl/limits/concurrency",
            "Maximum Concurrent Pages",
            "Upper bound on crawl concurrency, at most 8.",
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
            "Browser",
            "Optional browser rendering support for JavaScript-heavy pages.",
        ),
        (
            "/browser/enabled",
            "Enabled",
            "Allows rendered fetches and crawls to use a local browser.",
        ),
        (
            "/browser/interaction_backend",
            "Interaction Backend",
            "Native CDP or optional Playwright click/fill/wait; Playwright requires AGENA_BROWSER_PYTHON with the playwright package installed.",
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
        fetch: WebFetchConfig::default(),
        crawl: WebCrawlConfig::default(),
        search: WebSearchConfig::default(),
        store: WebStoreConfig::default(),
        browser: WebBrowserConfig::default(),
    }
}

pub(crate) struct WebPlugin {
    state: OnceLock<WebPluginState>,
    workspace_root: OnceLock<PathBuf>,
    crawl_lock: Mutex<()>,
    browser_download_lock: Arc<Mutex<()>>,
    /// Activity-source registration must happen after the runtime installs
    /// its live host client. Static plugins are initialized while the host is
    /// still being assembled, so registering from `init` would hit the
    /// temporary no-op client and lose the adapter permanently.
    browser_activity_source: OnceCell<()>,
    /// Shared interactive-browser session state. Owned by the plugin and by
    /// the registered [`BrowserActivitySource`] so log reads and stop requests
    /// can reach live sessions without going through a tool invocation.
    browser_state: Arc<BrowserActivityState>,
    user_agent: String,
}

impl Default for WebPlugin {
    fn default() -> Self {
        Self::new()
    }
}

/// Human-facing details for an interactive browser session, retained so the
/// terminal activity record reuses the same title/URL/start time.
#[derive(Debug, Clone)]
struct BrowserSessionMeta {
    owner: agena_runtime_tools::TerminalOwner,
    browser_context_id: Option<String>,
    title: String,
    url: String,
    started_at_ms: i64,
}

/// A CDP notification forwarded from a target session's connection.
#[derive(Debug, Clone)]
struct CdpEvent {
    method: String,
    params: serde_json::Value,
}

/// Bounded per-session log buffer implementing the unified `since_seq` cursor
/// protocol so the activities panel can tail browser output like any other
/// activity.
#[derive(Debug, Clone)]
struct BrowserSessionLog {
    lines: VecDeque<BackgroundActivityLogLine>,
    next_seq: u64,
    dropped: u64,
}

const BROWSER_LOG_CAPACITY: usize = 500;

impl BrowserSessionLog {
    fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            // Seq numbering follows the unified cursor protocol used by the
            // shell monitor and plugin host logs: 0 means "no events yet" and
            // the first line gets seq 1, so a fresh read with `since_seq = 0`
            // (`seq > since_seq`) returns every line including the first.
            next_seq: 1,
            dropped: 0,
        }
    }

    fn append(&mut self, stream: &str, text: impl Into<String>) {
        const MAX_LOG_BYTES: usize = 16 * 1024;
        let mut text = text.into();
        if text.len() > MAX_LOG_BYTES {
            let mut end = MAX_LOG_BYTES - " [truncated]".len();
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str(" [truncated]");
        }
        let line = BackgroundActivityLogLine {
            chunk: false,
            seq: self.next_seq,
            stream: stream.to_string(),
            ts_ms: chrono::Utc::now().timestamp_millis(),
            text,
        };
        self.next_seq = self.next_seq.saturating_add(1);
        if self.lines.len() >= BROWSER_LOG_CAPACITY {
            self.lines.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.lines.push_back(line);
    }

    fn read(
        &self,
        activity_id: &str,
        since_seq: u64,
        limit: Option<u32>,
    ) -> BackgroundActivityLogRead {
        let mut lines = self
            .lines
            .iter()
            .filter(|line| line.seq > since_seq)
            .cloned()
            .collect::<Vec<_>>();
        let has_more = limit.is_some_and(|limit| lines.len() as u32 > limit);
        if let Some(limit) = limit {
            lines.truncate(limit as usize);
        }
        let last_seq = lines.last().map(|line| line.seq).unwrap_or(since_seq);
        BackgroundActivityLogRead {
            activity_id: activity_id.to_string(),
            status: BackgroundActivityStatus::Running,
            lines,
            last_seq,
            has_more,
            dropped_lines: self.dropped,
            exit_code: None,
            completion_reason: None,
        }
    }
}

/// Shared interactive-browser session state: live CDP clients, activity
/// metadata, per-session log buffers, and the browser-level (root) client used
/// to close targets. Both [`WebPlugin`] and [`BrowserActivitySource`] hold an
/// `Arc` to this so control requests can reach the live sessions.
#[derive(Clone)]
struct BrowserActivityState {
    clients: Arc<tokio::sync::Mutex<BTreeMap<String, CdpClient>>>,
    connecting: Arc<tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    meta: Arc<tokio::sync::Mutex<BTreeMap<String, BrowserSessionMeta>>>,
    logs: Arc<tokio::sync::Mutex<BTreeMap<String, BrowserSessionLog>>>,
    root: Arc<tokio::sync::Mutex<Option<CdpClient>>>,
    contexts: Arc<tokio::sync::Mutex<BTreeMap<(PathBuf, Option<i64>), String>>>,
    /// One gate per live page target. The host fans every tool in a batch out
    /// at once, so two `browser_*` calls on the *same* page would otherwise
    /// interleave their CDP commands and land in whatever order the socket
    /// happened to serialize them — `click` racing `type` racing `screenshot`.
    /// Holding the page's gate for the whole action keeps one session's
    /// actions ordered while different sessions keep running in parallel.
    actions: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Serializes "a shared Chrome process exists and this caller's browser
    /// context exists" — the read-then-create pair in
    /// [`WebPlugin::browser_context_for_owner`]. Different callers still run
    /// their actions concurrently; only context creation is exclusive.
    context_creation: Arc<tokio::sync::Mutex<()>>,
}

impl BrowserActivityState {
    fn new() -> Self {
        Self {
            clients: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            connecting: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            meta: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            logs: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            root: Arc::new(tokio::sync::Mutex::new(None)),
            contexts: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            actions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            context_creation: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Clone the action gate for one page target, creating it on first use.
    async fn action_gate(&self, target_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.actions
                .lock()
                .await
                .entry(target_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    }

    async fn append_log(&self, target_id: &str, stream: &str, text: impl Into<String>) {
        self.logs
            .lock()
            .await
            .entry(target_id.to_string())
            .or_insert_with(BrowserSessionLog::new)
            .append(stream, text);
    }

    /// Close one target: ask the browser to close it over CDP, drop the local
    /// client and metadata, append a log line, and publish the terminal
    /// activity record. Returns whether the CDP close was acknowledged.
    async fn close_session(
        &self,
        target_id: &str,
        message: &str,
        host: &dyn HostClient,
    ) -> SdkResult<bool> {
        let connect_lock = {
            let mut connecting = self.connecting.lock().await;
            Arc::clone(
                connecting
                    .entry(target_id.to_string())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        let _connect_guard = connect_lock.lock().await;
        // Clone the client under the state lock, then perform CDP I/O after
        // releasing it. The command can wait on the browser's reader task,
        // which must remain free to update browser state.
        let root = self.root.lock().await.clone();
        let close_result = match root {
            Some(root) => root
                .command(
                    "Target.closeTarget",
                    serde_json::json!({ "targetId": target_id }),
                )
                .await
                .and_then(|value| {
                    value
                        .get("success")
                        .and_then(serde_json::Value::as_bool)
                        .ok_or_else(|| {
                            PluginError::internal(
                                "browser Target.closeTarget response had no boolean success field",
                            )
                        })
                }),
            None => Ok(false),
        };
        if !close_result? {
            return Ok(false);
        }
        self.clients.lock().await.remove(target_id);
        self.actions.lock().await.remove(target_id);
        self.connecting.lock().await.remove(target_id);
        let finished_at_ms = chrono::Utc::now().timestamp_millis();
        let meta = self.meta.lock().await.remove(target_id);
        self.append_log(
            target_id,
            "event",
            format!("Closed browser session {target_id}."),
        )
        .await;
        let now = chrono::Utc::now().timestamp_millis();
        let activity = browser_activity(
            format!("browser_{target_id}"),
            meta.as_ref()
                .map(|meta| meta.title.clone())
                .unwrap_or_else(|| "Browser session".to_string()),
            meta.as_ref()
                .map(|meta| meta.url.clone())
                .unwrap_or_else(|| format!("session {target_id}")),
            BackgroundActivityStatus::Stopped,
            Some(meta.as_ref().map(|meta| meta.started_at_ms).unwrap_or(now)),
            Some(finished_at_ms),
            Some(message.to_string()),
        );
        if let Err(error) = host.publish_activity(activity).await {
            log_secondary_plugin_failure("publish stopped browser activity", &error);
        }
        self.logs.lock().await.remove(target_id);
        Ok(true)
    }
}

/// `ActivitySourceAdapter` the web plugin registers for the `Browser` kind so
/// the host can route log reads and stop requests to the live sessions.
struct BrowserActivitySource {
    state: Arc<BrowserActivityState>,
    host: Arc<dyn HostClient>,
}

#[async_trait]
impl ActivitySourceAdapter for BrowserActivitySource {
    async fn read_logs(
        &self,
        activity_id: &str,
        since_seq: u64,
        limit: Option<u32>,
        _wait_ms: u64,
    ) -> SdkResult<BackgroundActivityLogRead> {
        let target_id = activity_id.strip_prefix("browser_").unwrap_or(activity_id);
        let running = self.state.meta.lock().await.contains_key(target_id);
        let mut read = self
            .state
            .logs
            .lock()
            .await
            .get(target_id)
            .map(|log| log.read(activity_id, since_seq, limit))
            .unwrap_or_else(|| agena_plugin_host::sdk::activity::empty_log_read(activity_id));
        read.status = if running {
            BackgroundActivityStatus::Running
        } else {
            BackgroundActivityStatus::Stopped
        };
        Ok(read)
    }

    async fn stop(&self, activity_id: &str) -> SdkResult<()> {
        let target_id = activity_id
            .strip_prefix("browser_")
            .unwrap_or(activity_id)
            .to_string();
        let gate = self.state.action_gate(&target_id).await;
        let _guard = gate.lock().await;
        let closed = self
            .state
            .close_session(
                &target_id,
                "Stopped from the activities panel.",
                self.host.as_ref(),
            )
            .await?;
        if !closed {
            return Err(PluginError::invalid_params(format!(
                "browser session '{target_id}' could not be closed"
            )));
        }
        Ok(())
    }
}

struct WebPluginState {
    config: WebConfig,
    fetch_coordinator: Arc<WebFetchCoordinator>,
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
            snapshots: agena_web::PageSnapshots::default(),
            search_coordinator: WebSearchCoordinator::new(Duration::from_millis(
                config.fetch.request.delay_ms,
            )),
            config,
            host,
        }
    }
}

fn browser_activity(
    id: String,
    title: String,
    description: String,
    status: BackgroundActivityStatus,
    started_at_ms: Option<i64>,
    finished_at_ms: Option<i64>,
    message: Option<String>,
) -> BackgroundActivity {
    let now = chrono::Utc::now().timestamp_millis();
    BackgroundActivity {
        id,
        kind: BackgroundActivityKind::Browser,
        status,
        title,
        description,
        command: None,
        workdir: None,
        session_id: None,
        parent_session_id: None,
        operation_id: None,
        source_part_id: None,
        created_at_ms: started_at_ms.unwrap_or(now),
        started_at_ms: started_at_ms.unwrap_or(now),
        finished_at_ms,
        next_event_at_ms: None,
        exit_code: None,
        message,
        failure: None,
        last_seq: 0,
        has_more: false,
        dropped_lines: 0,
        cancellable: status.is_active(),
        dismissible: status.is_terminal(),
    }
}

/// Human-facing title segment for a URL: host plus the non-root path.
fn browser_title_target(url: &url::Url) -> String {
    let mut title = url.host_str().unwrap_or(url.as_str()).to_owned();
    let path = url.path().trim_end_matches('/');
    if !path.is_empty() {
        title.push_str(path);
    }
    title
}

/// Human-readable text for a CDP `RemoteObject` in `Runtime.consoleAPICalled`
/// args: prefer the structured value, fall back to the inspector description.
fn cdp_remote_object_text(value: &serde_json::Value) -> String {
    if let Some(description) = value.get("description").and_then(serde_json::Value::as_str) {
        return description.to_string();
    }
    match value.get("value") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
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
    /// Concurrent page reads; default 4, hard maximum 8, capped by crawl.limits.concurrency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 8))]
    concurrency: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[serde(deny_unknown_fields)]
struct FetchManyInput {
    /// One to eight known HTTP(S) URLs; normalized duplicates are fetched once, in first-occurrence order.
    #[schemars(length(min = 1, max = 8))]
    urls: Vec<String>,
    /// In-flight page reads, default 4, maximum 8. Per-host pacing still applies.
    #[serde(default)]
    #[schemars(range(min = 1, max = 8))]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("url"), non_empty("url"))]
#[serde(deny_unknown_fields)]
struct BrowserOpenInput {
    url: String,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("session_id"), non_empty("session_id"))]
#[serde(deny_unknown_fields)]
struct BrowserSessionInput {
    session_id: String,
}

#[derive(
    Debug, Clone, Default, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput,
)]
#[serde(deny_unknown_fields)]
struct BrowserListInput {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector"),
    non_empty("session_id"),
    non_empty_if_present("selector")
)]
#[serde(deny_unknown_fields)]
struct BrowserClickInput {
    session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    selector: Option<String>,
    /// Optional CSS iframe selector (Playwright backend only; use CSS selectors, not snapshot refs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    frame_selector: Option<String>,
    /// Snapshot-local index returned by `browser_snapshot.elements[].ref`.
    /// It is valid only while the page DOM has not materially changed.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 199))]
    element_ref: Option<u16>,
    /// ID of the snapshot that supplied ref. Required with ref; stale refs are rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    snapshot_id: Option<String>,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector"),
    non_empty("session_id"),
    non_empty_if_present("selector")
)]
#[serde(deny_unknown_fields)]
struct BrowserTypeInput {
    session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    selector: Option<String>,
    /// Optional CSS iframe selector (Playwright backend only; use CSS selectors, not snapshot refs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    frame_selector: Option<String>,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 199))]
    element_ref: Option<u16>,
    /// ID of the snapshot that supplied ref. Required with ref; stale refs are rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 128)]
    snapshot_id: Option<String>,
    #[arg(max_chars = 65536)]
    text: String,
    #[serde(default)]
    press_enter: bool,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(
    trim("session_id", "selector", "text"),
    non_empty("session_id"),
    non_empty_if_present("selector", "text")
)]
#[serde(deny_unknown_fields)]
struct BrowserWaitInput {
    session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    selector: Option<String>,
    /// Optional CSS iframe selector (Playwright backend only; use CSS selectors, not snapshot refs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    frame_selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 4096)]
    text: Option<String>,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("session_id", "path"), non_empty("session_id"))]
#[serde(deny_unknown_fields)]
struct BrowserScreenshotInput {
    session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default)]
    full_page: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
#[input(trim("session_id", "url"), non_empty("session_id", "url"))]
#[serde(deny_unknown_fields)]
struct BrowserDownloadInput {
    /// Existing managed browser page to use for the navigation. Its browser
    /// profile (for example, authenticated cookies) remains intact.
    session_id: String,
    /// HTTP(S) download URL. The artifact is always written under the
    /// managed workspace artifact directory; callers cannot choose an
    /// arbitrary destination path.
    url: String,
    #[serde(default = "default_browser_action_timeout_ms")]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: u64,
}

const fn default_browser_action_timeout_ms() -> u64 {
    30_000
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "web",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Local web search/fetch/crawl plugin with an embedded crawl cache, deduplication, and optional browser rendering.",
    settings = WebConfig,
    settings_default = default_web_config(),
    settings_metadata = web_settings_metadata(),
)]
impl WebPlugin {
    pub(crate) fn new() -> Self {
        Self {
            state: OnceLock::new(),
            workspace_root: OnceLock::new(),
            crawl_lock: Mutex::new(()),
            browser_download_lock: Arc::new(Mutex::new(())),
            browser_activity_source: OnceCell::new(),
            browser_state: Arc::new(BrowserActivityState::new()),
            user_agent: "agena-web".to_owned(),
        }
    }

    #[hook(init)]
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
        Ok(agena_plugin_host::sdk::InitOutcome::ack(
            agena_plugin_host::sdk::Plugin::manifest(self),
        ))
    }

    #[hook(shutdown)]
    async fn shutdown(&self) -> SdkResult<()> {
        // Drop CDP sockets before killing the underlying browser process.
        self.browser_state.clients.lock().await.clear();
        *self.browser_state.root.lock().await = None;
        let finished_at_ms = chrono::Utc::now().timestamp_millis();
        let sessions = std::mem::take(&mut *self.browser_state.meta.lock().await)
            .into_iter()
            .collect::<Vec<_>>();
        for (session_id, meta) in sessions {
            self.browser_state
                .append_log(
                    &session_id,
                    "event",
                    "Plugin shutdown closed the managed browser.",
                )
                .await;
            self.publish_browser_terminal(
                &session_id,
                Some(&meta),
                finished_at_ms,
                "Plugin shutdown closed the managed browser.".to_string(),
            )
            .await;
        }
        self.browser_state.logs.lock().await.clear();
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|error| {
                plugin_internal_error_with_context("acquire browser shutdown worker", &error)
            })?;
        tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            shutdown_local_browser()
        })
        .await
        .map_err(|error| {
            plugin_internal_error_with_context("browser shutdown task failed", &error)
        })?
        .map_err(crawl_error_to_plugin)?;
        Ok(())
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

    /// Register the browser activity adapter on first real browser use. The
    /// runtime installs its concrete callback host only after static plugin
    /// initialization has completed; doing this lazily avoids calling the
    /// temporary `NoopHostClient` during startup.
    async fn ensure_browser_activity_source(&self) -> SdkResult<()> {
        let host = self.state()?.host.clone();
        let state = Arc::clone(&self.browser_state);
        self.browser_activity_source
            .get_or_try_init(|| async move {
                let source = BrowserActivitySource {
                    state,
                    host: host.clone(),
                };
                host.register_activity_source(BackgroundActivityKind::Browser, Arc::new(source))
                    .await
            })
            .await
            .map(|_| ())
    }

    /// Publish a browser session as a unified background activity so the TUI
    /// `/activities` panel and web `/activities` page can list and follow it.
    /// Records go straight into the host's activity registry through the
    /// first-class `HostClient::publish_activity` capability.
    async fn publish_browser_activity(&self, activity: BackgroundActivity) -> SdkResult<()> {
        let host = self.state()?.host.clone();
        host.publish_activity(activity).await.map_err(|error| {
            plugin_internal_failure_with_context("publish browser activity", &error)
        })
    }

    async fn publish_browser_running(&self, session_id: &str, meta: &BrowserSessionMeta) {
        if let Err(error) = self
            .publish_browser_activity(browser_activity(
                format!("browser_{session_id}"),
                meta.title.clone(),
                meta.url.clone(),
                BackgroundActivityStatus::Running,
                Some(meta.started_at_ms),
                None,
                None,
            ))
            .await
        {
            log_secondary_plugin_failure("publish running browser activity", &error);
        }
    }

    async fn publish_browser_terminal(
        &self,
        session_id: &str,
        meta: Option<&BrowserSessionMeta>,
        finished_at_ms: i64,
        message: String,
    ) {
        let now = chrono::Utc::now().timestamp_millis();
        if let Err(error) = self
            .publish_browser_activity(browser_activity(
                format!("browser_{session_id}"),
                meta.map(|meta| meta.title.clone())
                    .unwrap_or_else(|| "Browser session".to_string()),
                meta.map(|meta| meta.url.clone())
                    .unwrap_or_else(|| format!("session {session_id}")),
                BackgroundActivityStatus::Stopped,
                Some(meta.map(|meta| meta.started_at_ms).unwrap_or(now)),
                Some(finished_at_ms),
                Some(message),
            ))
            .await
        {
            log_secondary_plugin_failure("publish terminal browser activity", &error);
        }
    }

    fn store(&self) -> SdkResult<CrawlStore> {
        Ok(CrawlStore::for_workspace(self.workspace_root()?))
    }

    fn spider_fetch_options(&self, rendered: bool) -> SdkResult<SpiderFetchOptions> {
        let config = self.config()?;
        let delay = (config.browser.wait.delay_ms > 0)
            .then(|| Duration::from_millis(config.browser.wait.delay_ms));
        Ok(SpiderFetchOptions {
            extractor: config.fetch.extractor,
            max_body_bytes: config.fetch.request.max_body_bytes as usize,
            timeout: Duration::from_secs(config.fetch.request.timeout_secs),
            delay_ms: config.fetch.request.delay_ms,
            user_agent: self.user_agent.clone(),
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
                    idle_timeout: (config.browser.idle_timeout_secs > 0)
                        .then(|| Duration::from_secs(config.browser.idle_timeout_secs)),
                },
                wait_for_network_idle: config.browser.wait.for_network_idle,
                wait_for_selector: config.browser.wait.for_selector.clone(),
                wait_timeout: Duration::from_secs(config.browser.wait.timeout_secs),
                delay,
            },
        })
    }

    async fn validate_network_target(&self, url: &url::Url) -> SdkResult<()> {
        self.state()?
            .host
            .require_network_permission(url.to_string())
            .await?;
        validate_public_network_target(url).await
    }

    /// Resolve ordinary HTTP redirect hops before a managed browser target is
    /// allowed to navigate. Each hop is independently subjected to the same
    /// hostname/DNS permission policy as the originally requested URL. HEAD
    /// is deliberately used with redirects disabled: this preflight never
    /// follows a target behind the browser's back or replay a form request.
    ///
    /// The persistent CDP Fetch interceptor remains authoritative for the
    /// browser's real document requests, including cookie-, JavaScript-,
    /// form-, and method-dependent navigations. This preflight is retained as
    /// an early error and redirect diagnostic for ordinary HTTP URLs.
    async fn browser_preflight_redirects(&self, initial: &url::Url) -> SdkResult<Vec<String>> {
        const MAX_REDIRECTS: usize = 10;
        let timeout = Duration::from_secs(self.config()?.fetch.request.timeout_secs);
        let preflight = async {
            let mut current = initial.clone();
            let mut checked = Vec::new();
            for _ in 0..=MAX_REDIRECTS {
                self.validate_network_target(&current).await?;
                let addresses = resolve_public_network_target(&current).await?;
                let client = reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(timeout)
                    .no_proxy()
                    .resolve_to_addrs(current.host_str().expect("validated host"), &addresses)
                    .build()
                    .map_err(|error| PluginError::internal_error(&error))?;
                checked.push(current.to_string());
                let response = client.head(current.clone()).send().await.map_err(|error| {
                    plugin_internal_error_with_context(
                        format!("browser redirect preflight failed for {current}").as_str(),
                        &error,
                    )
                })?;
                if !response.status().is_redirection() {
                    return Ok(checked);
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .ok_or_else(|| {
                        PluginError::internal(format!(
                            "browser redirect from {current} had no Location header"
                        ))
                    })?
                    .to_str()
                    .map_err(|error| {
                        plugin_internal_error_with_context(
                            format!(
                                "browser redirect from {current} had an invalid Location header"
                            )
                            .as_str(),
                            &error,
                        )
                    })?;
                current = resolve_browser_redirect(&current, location)?;
            }
            Err(PluginError::internal(format!(
                "browser redirect preflight exceeded {MAX_REDIRECTS} hops"
            )))
        };
        tokio::time::timeout(timeout, preflight)
            .await
            .map_err(|_| PluginError::internal("browser redirect preflight deadline exceeded"))?
    }

    fn local_browser_options(&self) -> SdkResult<LocalBrowserOptions> {
        let config = self.config()?;
        Ok(LocalBrowserOptions {
            executable_path: config
                .browser
                .executable_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            startup_timeout: Duration::from_secs(config.browser.wait.timeout_secs),
            idle_timeout: (config.browser.idle_timeout_secs > 0)
                .then(|| Duration::from_secs(config.browser.idle_timeout_secs)),
        })
    }

    /// Run one action against a single page target under that page's gate.
    ///
    /// Ownership is re-checked inside the gate: the check and the CDP traffic
    /// must be one critical section, otherwise a concurrent `browser_close`
    /// can retire the target between them.
    async fn with_browser_action<T, F>(
        &self,
        context: &ToolInvokeContext<'_>,
        target: &str,
        action: F,
    ) -> SdkResult<T>
    where
        F: std::future::Future<Output = SdkResult<T>>,
    {
        self.require_browser_owner(context, target).await?;
        let gate = self.browser_state.action_gate(target).await;
        let _guard = gate.lock().await;
        self.require_browser_owner(context, target).await?;
        let _lease = agena_web::local_browser_lease().map_err(crawl_error_to_plugin)?;
        action.await
    }

    /// `browser_open` creates a target this caller does not own yet, so it
    /// cannot use [`WebPlugin::with_browser_action`]; it holds the freshly
    /// created target's gate directly instead.
    async fn lock_new_browser_action(&self, target: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.browser_state
            .action_gate(target)
            .await
            .lock_owned()
            .await
    }

    async fn require_browser_owner(
        &self,
        context: &ToolInvokeContext<'_>,
        target: &str,
    ) -> SdkResult<()> {
        let owner = browser_owner(context)?;
        if self
            .browser_state
            .meta
            .lock()
            .await
            .get(target)
            .is_none_or(|meta| meta.owner != owner)
        {
            return Err(PluginError::invalid_params(
                "browser page not found or not owned by this Agena session; call browser_open",
            ));
        }
        Ok(())
    }

    async fn browser_context_for_owner(
        &self,
        browser: &CdpClient,
        owner: &agena_runtime_tools::TerminalOwner,
    ) -> SdkResult<String> {
        let key = (owner.workspace.clone(), owner.session_id);
        // Context creation is a read-then-create over shared browser state, so
        // two concurrent opens for the same or different owners would both
        // miss the cache and each create a context, orphaning one of them.
        let _create_guard = self.browser_state.context_creation.lock().await;
        let mut contexts = self.browser_state.contexts.lock().await;
        let active = browser
            .command("Target.getBrowserContexts", serde_json::json!({}))
            .await?;
        let live = active
            .get("browserContextIds")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| PluginError::internal("browser did not return context identities"))?;
        contexts.retain(|_, id| live.iter().any(|value| value.as_str() == Some(id.as_str())));
        if let Some(id) = contexts.get(&key).filter(|id| {
            active
                .get("browserContextIds")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|ids| ids.iter().any(|value| value.as_str() == Some(id.as_str())))
        }) {
            return Ok(id.clone());
        }
        if contexts.len() >= 64 && !contexts.contains_key(&key) {
            return Err(PluginError::invalid_params(
                "browser context quota reached; close owned contexts before opening more",
            ));
        }
        let result = browser
            .command(
                "Target.createBrowserContext",
                serde_json::json!({"disposeOnDetach":true}),
            )
            .await?;
        let id = result
            .get("browserContextId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PluginError::internal("browser did not return a context id"))?
            .to_owned();
        contexts.insert(key, id.clone());
        Ok(id)
    }

    async fn browser_endpoint(&self) -> SdkResult<String> {
        let options = self.local_browser_options()?;
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|error| {
                plugin_internal_error_with_context("acquire browser launcher worker", &error)
            })?;
        tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            local_browser_endpoint(&options)
        })
        .await
        .map_err(|error| plugin_internal_error_with_context("browser launcher failed", &error))?
        .map_err(crawl_error_to_plugin)
    }

    async fn browser_client(&self, target_id: Option<&str>) -> SdkResult<CdpClient> {
        self.browser_client_with_events(target_id, None).await
    }

    /// Like [`WebPlugin::browser_client`] but attaches the target session with
    /// an optional CDP notification sink used for live activity logging.
    async fn browser_client_with_events(
        &self,
        target_id: Option<&str>,
        events: Option<mpsc::Sender<CdpEvent>>,
    ) -> SdkResult<CdpClient> {
        let Some(target_id) = target_id else {
            if events.is_some() {
                // Dedicated notification streams (such as downloads) must
                // not replace the connection that owns browser contexts.
                let endpoint = self.browser_endpoint().await?;
                return CdpClient::connect(endpoint.as_str(), None, events).await;
            }
            let mut root = self.browser_state.root.lock().await;
            if let Some(client) = root.as_ref().filter(|client| !client.is_closed()) {
                return Ok(client.clone());
            }
            // Contexts use disposeOnDetach. Retain one root connection and
            // coalesce initial connections, otherwise opening another page
            // can tear down the previous caller's contexts and cookies.
            let endpoint = self.browser_endpoint().await?;
            let client = CdpClient::connect(endpoint.as_str(), None, None).await?;
            *root = Some(client.clone());
            return Ok(client);
        };

        let existing = {
            let mut clients = self.browser_state.clients.lock().await;
            if let Some(client) = clients.get(target_id)
                && !client.is_closed()
            {
                Some(client.clone())
            } else {
                clients.remove(target_id);
                None
            }
        };
        if let Some(client) = existing {
            return Ok(client);
        }

        // Coalesce concurrent first use of one target. Without the second
        // check below, two commands can establish separate CDP sockets and
        // whichever inserts last silently orphans the other's reader task.
        let connect_lock = {
            let mut connecting = self.browser_state.connecting.lock().await;
            Arc::clone(
                connecting
                    .entry(target_id.to_string())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        let _connect_guard = connect_lock.lock().await;
        let existing = {
            let mut clients = self.browser_state.clients.lock().await;
            if let Some(client) = clients.get(target_id)
                && !client.is_closed()
            {
                Some(client.clone())
            } else {
                clients.remove(target_id);
                None
            }
        };
        if let Some(client) = existing {
            return Ok(client);
        }

        let endpoint = self.browser_endpoint().await?;
        let host = self.state()?.host.clone();
        let policy: BrowserRequestPolicy = Arc::new(move |url, _| {
            let host = host.clone();
            Box::pin(async move {
                if agena_plugin_host::sdk::host_api::current_host_callback_context().is_none() {
                    return Err("browser network authorization requires an active tool call".into());
                }
                host.require_network_permission(url.clone())
                    .await
                    .map_err(|error| error.diagnostic_message().to_owned())?;
                authorize_browser_document_request(&url).await
            })
        });
        let client =
            CdpClient::connect_with_policy(endpoint.as_str(), Some(target_id), events, policy)
                .await?;
        client.enable_navigation_interception().await?;
        self.browser_state
            .clients
            .lock()
            .await
            .insert(target_id.to_string(), client.clone());
        Ok(client)
    }

    fn validate_browser_frame(
        &self,
        frame: Option<&str>,
        element_ref: Option<u16>,
    ) -> SdkResult<()> {
        if let Some(frame) = frame {
            if frame.trim().is_empty() || element_ref.is_some() {
                return Err(PluginError::invalid_params(
                    "frame_selector requires a nonempty CSS iframe selector and cannot be combined with snapshot refs",
                ));
            }
            if self.config()?.browser.interaction_backend != BrowserInteractionBackend::Playwright {
                return Err(PluginError::invalid_params(
                    "frame_selector requires browser.interaction_backend=playwright",
                ));
            }
        }
        Ok(())
    }

    async fn playwright_action(
        &self,
        request: playwright::Request<'_>,
    ) -> SdkResult<serde_json::Value> {
        // Downloads change browser-level CDP behavior; prevent bridge attach
        // while a native download holds that lease.
        let _download_guard = self.browser_download_lock.lock().await;
        let _native = self.browser_client(Some(request.target_id)).await?;
        _native.refresh_callback_context()?;
        let endpoint = self.browser_endpoint().await?;
        let context_id = self
            .browser_state
            .meta
            .lock()
            .await
            .get(request.target_id)
            .and_then(|meta| meta.browser_context_id.clone())
            .ok_or_else(|| {
                PluginError::invalid_params("owned browser context is unavailable; reopen the page")
            })?;
        let scoped = playwright::Request {
            endpoint: &endpoint,
            context_id: &context_id,
            ..request
        };
        playwright::run(&scoped).await
    }

    async fn forget_browser_client(&self, target_id: &str) {
        self.browser_state.clients.lock().await.remove(target_id);
    }

    /// Consume CDP notifications for one interactive session and project
    /// main-frame navigations, console output, and browser log entries into
    /// the shared activity log buffer and the live activity record. Runs
    /// detached for the session's lifetime; exits when the target connection
    /// closes and the event channel is dropped.
    fn spawn_browser_event_task(
        &self,
        session_id: &str,
        mut rx: mpsc::Receiver<CdpEvent>,
    ) -> SdkResult<()> {
        let capture_console_text = self.config()?.browser.capture_console_text;
        let state = Arc::clone(&self.browser_state);
        let host = self.state()?.host.clone();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event.method.as_str() {
                    "Page.frameNavigated" => {
                        // Only the main frame drives the activity title/URL.
                        if event.params.pointer("/frame/parentId").is_some() {
                            continue;
                        }
                        let Some(raw_url) = event
                            .params
                            .pointer("/frame/url")
                            .and_then(serde_json::Value::as_str)
                        else {
                            continue;
                        };
                        let Ok(parsed) = url::Url::parse(raw_url) else {
                            continue;
                        };
                        if !matches!(parsed.scheme(), "http" | "https") {
                            continue;
                        }
                        let title_target = browser_title_target(&parsed);
                        let title = format!("Browser · {title_target}");
                        let url_text = parsed.to_string();
                        let (previous_url, started_at_ms) = {
                            let mut meta = state.meta.lock().await;
                            let Some(entry) = meta.get_mut(&session_id) else {
                                continue;
                            };
                            let previous_url = entry.url.clone();
                            entry.title = title.clone();
                            entry.url = url_text.clone();
                            (previous_url, entry.started_at_ms)
                        };
                        if previous_url == url_text {
                            continue;
                        }
                        state
                            .append_log(&session_id, "event", format!("Navigated to {url_text}."))
                            .await;
                        if let Err(error) = host
                            .publish_activity(browser_activity(
                                format!("browser_{session_id}"),
                                title,
                                url_text,
                                BackgroundActivityStatus::Running,
                                Some(started_at_ms),
                                None,
                                None,
                            ))
                            .await
                        {
                            log_secondary_plugin_failure(
                                "publish browser navigation activity",
                                &error,
                            );
                        }
                    }
                    "Runtime.consoleAPICalled" if !capture_console_text => {
                        state
                            .append_log(
                                &session_id,
                                "console",
                                "Console event (page payload omitted)",
                            )
                            .await;
                    }
                    "Log.entryAdded" if !capture_console_text => {
                        state
                            .append_log(
                                &session_id,
                                "log",
                                "Browser log event (page payload omitted)",
                            )
                            .await;
                    }
                    "Runtime.consoleAPICalled" => {
                        let level = event
                            .params
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("log");
                        let text = event
                            .params
                            .pointer("/args")
                            .and_then(serde_json::Value::as_array)
                            .map(|args| {
                                args.iter()
                                    .map(cdp_remote_object_text)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .unwrap_or_default();
                        if !text.trim().is_empty() {
                            state
                                .append_log(&session_id, "console", format!("[{level}] {text}"))
                                .await;
                        }
                    }
                    "Log.entryAdded" => {
                        let level = event
                            .params
                            .pointer("/entry/level")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("info");
                        let text = event
                            .params
                            .pointer("/entry/text")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default();
                        if !text.trim().is_empty() {
                            state
                                .append_log(&session_id, "log", format!("[{level}] {text}"))
                                .await;
                        }
                    }
                    _ => {}
                }
            }
        });
        Ok(())
    }

    async fn browser_snapshot_value(&self, target_id: &str) -> SdkResult<serde_json::Value> {
        let client = self.browser_client(Some(target_id)).await?;
        client
            .command("Runtime.enable", serde_json::json!({}))
            .await?;
        client.evaluate(&browser_snapshot_expression()).await
    }

    /// Re-check the managed page's committed URL after an interaction as a
    /// defense-in-depth consistency check. The persistent Fetch-domain
    /// interceptor independently authorizes every document request before it
    /// is continued, while this check verifies the state exposed back to the
    /// model against the same host and DNS policy.
    async fn ensure_browser_final_url(&self, target_id: &str) -> SdkResult<serde_json::Value> {
        let snapshot = self.browser_snapshot_value(target_id).await?;
        if let Some(final_url) = snapshot.get("url").and_then(serde_json::Value::as_str)
            && let Ok(final_url) = url::Url::parse(final_url)
        {
            self.validate_network_target(&final_url).await?;
        }
        Ok(snapshot)
    }

    async fn fetch_page_with_extractor(
        &self,
        url: &url::Url,
        use_cache: bool,
        render_js: bool,
        extractor: agena_web::ExtractionBackend,
    ) -> SdkResult<FetchedPage> {
        let state = self.state()?;
        if !state.config.fetch.enabled {
            return Err(PluginError::internal(
                "web fetching is disabled by plugin config `fetch.enabled`",
            ));
        }
        state
            .host
            .require_network_permission(url.to_string())
            .await?;
        // Alternate per-call extractors must not pollute the configured cache.
        let use_cache = use_cache && extractor == state.config.fetch.extractor;
        let page = state
            .fetch_coordinator
            .fetch_or_cached(url, render_js, use_cache, || async {
                static FETCHES: Semaphore = Semaphore::const_new(8);
                let _permit = FETCHES
                    .acquire()
                    .await
                    .map_err(|error| PluginError::internal_error(&error))?;
                let mut options = self.spider_fetch_options(render_js)?;
                options.extractor = extractor;
                if render_js {
                    rendered_fetch::fetch(self, url, &options).await
                } else {
                    let source = fetch_transport::fetch(url, &options, |target| async move {
                        state
                            .host
                            .require_network_permission(target.to_string())
                            .await?;
                        state.fetch_coordinator.wait_for_url_host(&target).await;
                        resolve_public_network_target(&target).await
                    })
                    .await?;
                    fetch_transport::extract(source, url.clone(), extractor).await
                }
            })
            .await?;
        if !page.final_url.is_empty() {
            state
                .host
                .require_network_permission(page.final_url.clone())
                .await?;
        }
        Ok(page)
    }

    #[tool(
        summary = "Fetch one web page and inspect its actual content.",
        help = "Read actual page evidence after search. Markdown preserves main content, sibling articles, code, tables and resolved links. Returns content_status, extraction_strategy, final_url, warnings, page_id and next_offset. max_chars defaults to 8000 (maximum 24000); continue long pages with web.read without refetching. prompt adds relevant excerpts from the full snapshot. Use fetch_many for independent known URLs. Omit render_js for HTTP first and one JavaScript-shell browser retry when browser.enabled; false forces HTTP, true forces an isolated browser. One shared fetch deadline includes admission, HTTP, rendering and extraction. Blocked/empty/partial pages are not reusable cache entries. Read statuses before using the text as evidence.",
        tags(network, read_only)
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
        help = "Fetch 1–8 known URLs concurrently (default concurrency 4, maximum 8), deduplicating normalized URLs in input order. Use after search to gather independent sources in one call. Each result includes its own status/error, content_status, bounded Markdown, page_id and next_offset for web.read. Failures do not discard other pages; partial also flags unreadable or truncated sources. Per-host pacing and global HTTP/browser limits still apply. Omit render_js for HTTP first and one conditional JavaScript-shell browser retry if enabled; true forces browser, false forces HTTP. No CAPTCHA bypass. max_chars is per page (default 4000, maximum 8000).",
        stream = invoke_fetch_many_stream,
        tags(network, read_only)
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
        let concurrency = content::bounded(input.concurrency, 4, 8, "concurrency")?;
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
                Ok((output, usable)) => (serde_json::json!({"url":raw,"ok":true,"usable":usable,"page":output.payload}), output.output_text, usable),
                Err(error) => {
                    tracing::warn!(url = %raw, diagnostic = %error.diagnostic_message(), "batch page failed");
                    let message = &error.failure.user.fallback;
                    (serde_json::json!({"url":raw,"ok":false,"usable":false,"error_kind":error.kind,"error":message,"failure_id":error.failure.id}), format!("URL: {raw}\n{message}"), false)
                }
            }
        }).await;
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
        tags(network, read_only)
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
        tags(network, read_only)
    )]
    async fn invoke_query(&self, input: &QueryCrawlInput) -> SdkResult<ToolInvokeOutput> {
        if input.query.trim().is_empty() || input.query.chars().count() > 4096 {
            return Err(PluginError::invalid_params(
                "query must contain 1–4096 characters",
            ));
        }
        let limit = content::bounded(input.max_results, 5, 20, "max_results")?;
        let hits = self
            .store()?
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
        help = "Traverse links breadth first with bounded concurrency (default 4, maximum 8; also capped by config). max_pages counts attempts and cache hits, including failures. same_host_only defaults true. Per-host pacing, robots, depth and URL-discovery budgets still apply. Omit render_js for HTTP first with conditional rendering when enabled; true forces browser and false forces HTTP. Report includes per-URL page_errors and effective concurrency. Only complete, readable 2xx documents enter storage. Use web.query to locate evidence in the resulting local index and web.read to read it.",
        stream = invoke_crawl_stream,
        tags(network, discovery, mutate)
    )]
    async fn invoke_crawl(&self, input: &CrawlRunInput) -> SdkResult<ToolInvokeOutput> {
        self.run_crawl(input, None).await
    }

    /// Streaming variant: a crawl runs for a while, so report where it starts
    /// before the final report arrives.
    async fn invoke_crawl_stream(
        &self,
        sink: ToolStreamSink,
        input: &CrawlRunInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.run_crawl(input, Some(&sink)).await
    }

    async fn run_crawl(
        &self,
        input: &CrawlRunInput,
        progress: Option<&ToolStreamSink>,
    ) -> SdkResult<ToolInvokeOutput> {
        if let Some(sink) = progress {
            sink.text(format!("Crawling {}\n", input.start_url)).await;
        }
        let start_url =
            prepare_fetch_url(input.start_url.as_str()).map_err(crawl_error_to_plugin)?;
        let store = self.store()?;
        let _guard = self.crawl_lock.lock().await;
        let config = self.config()?;
        let options = CrawlRunOptions {
            extraction_backend: config.fetch.extractor,
            concurrency: content::bounded(
                input.concurrency,
                config.crawl.defaults.concurrency,
                8,
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
        help = "Discover candidate pages; fetch 1-3 relevant URLs for factual answers. Omit engine or use auto for the configured provider (HTML, Brave, Tavily, Exa or SearXNG). HTML auto searches eight free public websites (DuckDuckGo, Bing, Baidu, Yandex, Google, Yahoo, Brave and Naver) concurrently, deduplicates URLs and combines rankings. Set engine to one name or a comma-separated list such as baidu,google to search only those HTML websites, overriding API settings. Case and surrounding spaces are ignored; duplicates run once in first-occurrence order. auto must stand alone; empty and unknown names are errors. max_results caps the final total; max_results_per_engine and max_pages_per_engine independently cap each selected HTML source (defaults: 10 candidates, 1 page including the first). Increase both source budgets for deeper retrieval; filtering or a larger final limit never triggers extra pages. No global offset or continuation cursor. Effective limits appear in the HTML response. Each result's engines lists contributing engines; source is the publisher. Multi-engine HTML searches preserve successful sources with partial and engine_errors; all selected engines failing is an error. A single engine's failure is returned directly. Inspect engine_reports for per-source result counts, page counts, cache hits and structured issues. Searches to the same engine are serialized and paced; successful identical queries are reused for two minutes. Rate limits honor Retry-After and verification failures enter cooldown: respect retry_after_secs, use other working sources and avoid bursts of overlapping queries. Verification and unknown pages are failures, not empty results. A confirmed JavaScript shell gets one browser attempt per page when browser.enabled. Later-page failures preserve earlier results with partial diagnostics. Browser rendering does not solve human verification or consent. API providers return at most 20 results without switching providers and reject HTML-only budget arguments. Domain filters accept bare hostnames; exclusions win. Snippets are previews, not fetched-page evidence.",
        tags(network, discovery, read_only)
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
        search_aggregate::search(
            input,
            limits,
            Duration::from_secs(config.fetch.request.timeout_secs),
            |engine, source_limit, max_pages| {
                self.search_with_engine(query, source_limit, max_pages, engine)
            },
        )
        .await?
        .into_tool_output()
    }

    #[tool(
        tags(network, interactive, mutate, read_only),
        name = "browser_open",
        summary = "Open a page in a managed interactive browser session."
    )]
    async fn browser_open(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserOpenInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let owner = browser_owner(context)?;
        let url = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?;
        let _lease = agena_web::local_browser_lease().map_err(crawl_error_to_plugin)?;
        self.validate_network_target(&url).await?;
        self.ensure_browser_activity_source().await?;
        let preflight_redirects = self.browser_preflight_redirects(&url).await?;
        let browser = self.browser_client(None).await?;
        let browser_context_id = self.browser_context_for_owner(&browser, &owner).await?;
        let created = browser
            .command(
                "Target.createTarget",
                serde_json::json!({ "url": "about:blank", "browserContextId": browser_context_id }),
            )
            .await?;
        let target_id = created
            .get("targetId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PluginError::internal("browser did not return a target id"))?
            .to_string();
        // The target is not this caller's yet, so ownership cannot be checked
        // before attaching; take the target's action gate to keep every later
        // action on this page behind the initialization below.
        let _action_guard = self.lock_new_browser_action(&target_id).await;
        let (event_tx, event_rx) = mpsc::channel::<CdpEvent>(512);
        let page = match self
            .browser_client_with_events(Some(target_id.as_str()), Some(event_tx))
            .await
        {
            Ok(page) => page,
            Err(error) => {
                if let Err(cleanup_error) = browser
                    .command(
                        "Target.closeTarget",
                        serde_json::json!({ "targetId": &target_id }),
                    )
                    .await
                {
                    log_secondary_plugin_failure(
                        "close browser target after target attachment failed",
                        &cleanup_error,
                    );
                }
                return Err(error);
            }
        };
        if let Err(error) = async {
            page.command("Page.enable", serde_json::json!({})).await?;
            page.command("Runtime.enable", serde_json::json!({}))
                .await?;
            page.command("Log.enable", serde_json::json!({})).await?;
            SdkResult::Ok(())
        }
        .await
        {
            if let Err(cleanup_error) = browser
                .command(
                    "Target.closeTarget",
                    serde_json::json!({ "targetId": &target_id }),
                )
                .await
            {
                log_secondary_plugin_failure(
                    "close browser target after page initialization failed",
                    &cleanup_error,
                );
            }
            return Err(error);
        }
        let title_target = browser_title_target(&url);
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let meta = BrowserSessionMeta {
            owner,
            browser_context_id: Some(browser_context_id),
            title: format!("Browser · {title_target}"),
            url: url.to_string(),
            started_at_ms,
        };
        // Register the session before navigation so CDP notifications for the
        // initial load and any redirects land in the activity log and update
        // the live record's title/URL.
        self.browser_state
            .meta
            .lock()
            .await
            .insert(target_id.clone(), meta.clone());
        self.browser_state
            .append_log(&target_id, "event", format!("Opened {url}."))
            .await;
        self.publish_browser_running(&target_id, &meta).await;
        self.spawn_browser_event_task(&target_id, event_rx)?;
        if let Err(error) = page
            .command("Page.navigate", serde_json::json!({ "url": url.as_str() }))
            .await
        {
            self.forget_browser_client(target_id.as_str()).await;
            self.browser_state.meta.lock().await.remove(&target_id);
            self.browser_state.logs.lock().await.remove(&target_id);
            let finished_at_ms = chrono::Utc::now().timestamp_millis();
            if let Err(activity_error) = self
                .publish_browser_activity(browser_activity(
                    format!("browser_{target_id}"),
                    meta.title.clone(),
                    meta.url.clone(),
                    BackgroundActivityStatus::Failed,
                    Some(started_at_ms),
                    Some(finished_at_ms),
                    Some(format!("Navigation to {url} failed.")),
                ))
                .await
            {
                log_secondary_plugin_failure(
                    "publish failed browser navigation activity",
                    &activity_error,
                );
            }
            if let Err(cleanup_error) = browser
                .command(
                    "Target.closeTarget",
                    serde_json::json!({ "targetId": &target_id }),
                )
                .await
            {
                log_secondary_plugin_failure(
                    "close browser target after navigation failed",
                    &cleanup_error,
                );
            }
            return Err(error);
        }
        let ready = async {
            self.wait_for_browser_condition(target_id.as_str(), None, None, input.timeout_ms)
                .await?;
            self.ensure_browser_final_url(target_id.as_str()).await
        }
        .await;
        let snapshot = match ready {
            Ok(snapshot) => snapshot,
            Err(error) => {
                match self
                    .browser_state
                    .close_session(
                        &target_id,
                        "Browser open failed.",
                        self.state()?.host.as_ref(),
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        return Err(PluginError::internal(format!(
                            "{}; additionally, failed to close the incomplete browser target",
                            error.diagnostic_message()
                        )));
                    }
                    Err(cleanup) => {
                        return Err(PluginError::internal(format!(
                            "{}; additionally, browser target cleanup failed: {}",
                            error.diagnostic_message(),
                            cleanup.diagnostic_message()
                        )));
                    }
                }
                return Err(error);
            }
        };
        Ok(ToolInvokeOutput::from_parts(
            format!("Open browser · {title_target}"),
            browser_snapshot_summary(&snapshot),
            format!(
                "Opened {} in browser session {}.\n\n{}",
                url,
                target_id,
                format_browser_snapshot(&snapshot)
            ),
            Some(serde_json::json!({
                "session_id": target_id,
                "snapshot": snapshot,
                "preflight_redirects": preflight_redirects,
                "document_requests_intercepted": true,
            })),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(network, interactive, query, discovery, read_only),
        name = "browser_list",
        summary = "List open page targets in the managed interactive browser."
    )]
    async fn browser_list(
        &self,
        context: &ToolInvokeContext<'_>,
        _input: &BrowserListInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let owner = browser_owner(context)?;
        let owned: BTreeSet<String> = self
            .browser_state
            .meta
            .lock()
            .await
            .iter()
            .filter(|(_, meta)| meta.owner == owner)
            .map(|(id, _)| id.clone())
            .collect();
        // Listing must never start the managed browser. Report the current
        // process state first and only connect when it is already running.
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|error| {
                plugin_internal_error_with_context("acquire browser shutdown worker", &error)
            })?;
        let running = tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            local_browser_running()
        })
        .await
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "browser status task failed",
                &error,
            ))
        })?
        .map_err(crawl_error_to_plugin)?;
        if !running {
            return Ok(ToolInvokeOutput::from_parts(
                "browser list",
                "0 open pages",
                "The managed browser is not running. Use browser_open to start it.",
                Some(serde_json::json!({
                    "browser_running": false,
                    "interaction_backend": self.config()?.browser.interaction_backend,
                    "sessions": [],
                })),
                std::collections::BTreeMap::new(),
                Vec::new(),
            ));
        }
        let browser = self.browser_client(None).await?;
        let result = browser
            .command("Target.getTargets", serde_json::json!({}))
            .await?;
        let sessions = result
            .get("targetInfos")
            .and_then(serde_json::Value::as_array)
            .map(|targets| {
                targets
                    .iter()
                    .filter(|target| {
                        target.get("type").and_then(serde_json::Value::as_str) == Some("page")
                            && target.get("targetId").and_then(serde_json::Value::as_str).is_some_and(|id|owned.contains(id))
                    })
                    .map(|target| {
                        serde_json::json!({
                            "session_id": target.get("targetId").and_then(serde_json::Value::as_str),
                            "title": target.get("title").and_then(serde_json::Value::as_str),
                            "url": target.get("url").and_then(serde_json::Value::as_str),
                            "attached": target.get("attached").and_then(serde_json::Value::as_bool),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(ToolInvokeOutput::from_parts(
            "browser list",
            format!("{} open pages", sessions.len()),
            format!("{} managed browser page target(s).", sessions.len()),
            Some(serde_json::json!({
                "browser_running": true,
                "interaction_backend": self.config()?.browser.interaction_backend,
                "sessions": sessions,
            })),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "browser_close",
        summary = "Close one page target in the managed interactive browser."
    )]
    async fn browser_close(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserSessionInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let host = self.state()?.host.clone();
        let closed = self
            .with_browser_action(context, input.session_id.as_str(), async {
                self.browser_state
                    .close_session(
                        &input.session_id,
                        &format!("Closed browser session {}.", input.session_id),
                        host.as_ref(),
                    )
                    .await
            })
            .await?;
        if !closed {
            return Err(PluginError::invalid_params(format!(
                "browser session '{}' could not be closed",
                input.session_id
            )));
        }
        Ok(ToolInvokeOutput::from_parts(
            "Close browser",
            "Closed",
            format!("Closed browser session {}.", input.session_id),
            Some(serde_json::json!({ "session_id": input.session_id, "closed": true })),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }
    #[tool(
        tags(network, interactive, mutate),
        name = "browser_shutdown",
        summary = "Close browser pages owned by the current Agena session without affecting other callers.",
        help = "Caller-scoped shutdown closes owned pages only. The shared Chrome process and other sessions are not stopped. Global browser shutdown is reserved for trusted host lifecycle control."
    )]
    async fn browser_shutdown(
        &self,
        context: &ToolInvokeContext<'_>,
        _input: &BrowserListInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let owner = browser_owner(context)?;
        let targets = self
            .browser_state
            .meta
            .lock()
            .await
            .iter()
            .filter(|(_, meta)| meta.owner == owner)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let key = (owner.workspace.clone(), owner.session_id);
        let mut closed = Vec::new();
        let mut failures = Vec::new();
        for target in targets {
            // Nothing to close means nothing to publish, so a caller-scoped
            // shutdown with no owned pages stays valid before init.
            let host = self.state()?.host.clone();
            let gate = self.browser_state.action_gate(&target).await;
            let _guard = gate.lock().await;
            match self
                .browser_state
                .close_session(&target, "Closed by owning session", host.as_ref())
                .await
            {
                Ok(true) => closed.push(target),
                Ok(false) => failures.push(format!("{target}: not acknowledged")),
                Err(error) => failures.push(format!("{target}: {}", error.failure.user.fallback)),
            }
        }
        if failures.is_empty() {
            let context_id = self.browser_state.contexts.lock().await.get(&key).cloned();
            if let Some(context_id) = context_id {
                let root = self.browser_state.root.lock().await.clone();
                match root {
                    Some(root) => match root
                        .command(
                            "Target.disposeBrowserContext",
                            serde_json::json!({"browserContextId":context_id}),
                        )
                        .await
                    {
                        Ok(_) => {
                            self.browser_state.contexts.lock().await.remove(&key);
                        }
                        Err(error) => failures.push(format!(
                            "owned context cleanup failed: {}",
                            error.failure.user.fallback
                        )),
                    },
                    None => failures.push(
                        "owned context cleanup could not reach the browser; state retained".into(),
                    ),
                }
            }
        }
        Ok(ToolInvokeOutput::from_parts(
            "Close owned browser pages",
            format!("{} closed · {} failures", closed.len(), failures.len()),
            format!(
                "Closed {} page(s) owned by this Agena session. Shared browser process was not shut down.\n{}",
                closed.len(),
                failures.join("\n")
            ),
            Some(
                serde_json::json!({"closed_sessions":closed,"failures":failures,"process_shutdown":false}),
            ),
            Default::default(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(network, interactive, query, read_only),
        name = "browser_snapshot",
        summary = "Inspect visible text and interactive elements in a browser session."
    )]
    async fn browser_snapshot(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserSessionInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let snapshot = self
            .with_browser_action(
                context,
                &input.session_id,
                self.browser_snapshot_value(&input.session_id),
            )
            .await?;
        let text = format_browser_snapshot(&snapshot);
        Ok(ToolInvokeOutput::from_parts(
            "Browser snapshot",
            browser_snapshot_summary(&snapshot),
            text,
            Some(serde_json::json!({
                "session_id": input.session_id,
                "snapshot": snapshot,
            })),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "browser_click",
        summary = "Click a browser element selected by CSS or the latest snapshot ref."
    )]
    async fn browser_click(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserClickInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.validate_browser_frame(input.frame_selector.as_deref(), input.element_ref)?;
        let target = browser_element_expression(
            input.selector.as_deref(),
            input.element_ref,
            input.snapshot_id.as_deref(),
        )?;
        let expression = format!(
            "(() => {{ const el = {target}; if (!el) return {{ok:false,error:'browser element not found'}}; if (el.disabled) return {{ok:false,error:'element is disabled'}}; el.scrollIntoView({{block:'center'}}); el.focus(); el.click(); return {{ok:true}}; }})()"
        );
        self.with_browser_action(context, input.session_id.as_str(), async {
            let result = if self.config()?.browser.interaction_backend
                == BrowserInteractionBackend::Playwright
            {
                self.playwright_action(playwright::Request {
                    endpoint: "",
                    context_id: "",
                    target_id: &input.session_id,
                    action: "click",
                    selector: input.selector.as_deref(),
                    frame_selector: input.frame_selector.as_deref(),
                    element_expression: input.element_ref.map(|_| target.as_str()),
                    text: None,
                    press_enter: false,
                    timeout_ms: input.timeout_ms,
                })
                .await?
            } else {
                let client = self.browser_client(Some(input.session_id.as_str())).await?;
                tokio::time::timeout(
                    Duration::from_millis(input.timeout_ms),
                    client.evaluate(expression.as_str()),
                )
                .await
                .map_err(|_| PluginError::internal("browser click timed out"))??
            };
            ensure_browser_action(&result)?;
            let snapshot = self
                .ensure_browser_final_url(input.session_id.as_str())
                .await?;
            Ok(browser_action_output(
                "browser click",
                input.session_id.as_str(),
                serde_json::json!({ "action": result, "snapshot": snapshot }),
            ))
        })
        .await
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "browser_type",
        summary = "Fill a browser input selected by CSS or the latest snapshot ref, optionally pressing Enter."
    )]
    async fn browser_type(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserTypeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.validate_browser_frame(input.frame_selector.as_deref(), input.element_ref)?;
        let expression = browser_type_expression(
            input.selector.as_deref(),
            input.element_ref,
            input.snapshot_id.as_deref(),
            input.text.as_str(),
            input.press_enter,
        )?;
        self.with_browser_action(context, input.session_id.as_str(), async {
            let result = if self.config()?.browser.interaction_backend
                == BrowserInteractionBackend::Playwright
            {
                let target = browser_element_expression(
                    input.selector.as_deref(),
                    input.element_ref,
                    input.snapshot_id.as_deref(),
                )?;
                self.playwright_action(playwright::Request {
                    endpoint: "",
                    context_id: "",
                    target_id: &input.session_id,
                    action: "fill",
                    selector: input.selector.as_deref(),
                    frame_selector: input.frame_selector.as_deref(),
                    element_expression: input.element_ref.map(|_| target.as_str()),
                    text: Some(&input.text),
                    press_enter: input.press_enter,
                    timeout_ms: input.timeout_ms,
                })
                .await?
            } else {
                let client = self.browser_client(Some(input.session_id.as_str())).await?;
                tokio::time::timeout(
                    Duration::from_millis(input.timeout_ms),
                    client.evaluate(expression.as_str()),
                )
                .await
                .map_err(|_| PluginError::internal("browser type timed out"))??
            };
            ensure_browser_action(&result)?;
            let snapshot = self
                .ensure_browser_final_url(input.session_id.as_str())
                .await?;
            Ok(browser_action_output(
                "browser type",
                input.session_id.as_str(),
                serde_json::json!({ "action": result, "snapshot": snapshot }),
            ))
        })
        .await
    }

    #[tool(
        tags(network, interactive, query, read_only),
        name = "browser_wait",
        summary = "Wait for page readiness, a CSS selector, or visible text."
    )]
    async fn browser_wait(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserWaitInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.validate_browser_frame(input.frame_selector.as_deref(), None)?;
        if input.selector.is_some() && input.text.is_some() {
            return Err(PluginError::invalid_params(
                "browser_wait accepts selector or text, not both",
            ));
        }
        self.with_browser_action(context, input.session_id.as_str(), async {
            if self.config()?.browser.interaction_backend == BrowserInteractionBackend::Playwright {
                self.playwright_action(playwright::Request {
                    endpoint: "",
                    context_id: "",
                    target_id: &input.session_id,
                    action: "wait",
                    selector: input.selector.as_deref(),
                    frame_selector: input.frame_selector.as_deref(),
                    element_expression: None,
                    text: input.text.as_deref(),
                    press_enter: false,
                    timeout_ms: input.timeout_ms,
                })
                .await?;
            } else {
                self.wait_for_browser_condition(
                    input.session_id.as_str(),
                    input.selector.as_deref(),
                    input.text.as_deref(),
                    input.timeout_ms,
                )
                .await?;
            }
            let snapshot = self
                .ensure_browser_final_url(input.session_id.as_str())
                .await?;
            Ok(browser_action_output(
                "browser wait",
                input.session_id.as_str(),
                serde_json::json!({ "ok": true, "snapshot": snapshot }),
            ))
        })
        .await
    }

    #[tool(
        tags(network, interactive, mutate, filesystem),
        name = "browser_screenshot",
        summary = "Capture a browser screenshot and return it as an image attachment."
    )]
    async fn browser_screenshot(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserScreenshotInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let bytes = self
            .with_browser_action(context, input.session_id.as_str(), async {
                let client = self.browser_client(Some(input.session_id.as_str())).await?;
                client.command("Page.enable", serde_json::json!({})).await?;
                let result = client
                    .command(
                        "Page.captureScreenshot",
                        serde_json::json!({
                            "format": "png",
                            "captureBeyondViewport": input.full_page,
                        }),
                    )
                    .await?;
                Ok(result)
            })
            .await
            .and_then(|result| {
                let encoded = result
                    .get("data")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        PluginError::internal("browser screenshot returned no image data")
                    })?;
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|error| {
                        plugin_internal_error_with_context("invalid screenshot data", &error)
                    })
            })?;
        let relative = input
            .path
            .clone()
            .unwrap_or_else(|| format!(".agena/artifacts/browser/{}.png", uuid::Uuid::new_v4()));
        let path = if Path::new(relative.as_str()).is_absolute() {
            PathBuf::from(relative.as_str())
        } else {
            self.workspace_root()?.join(relative.as_str())
        };
        let size_bytes = bytes.len() as u64;
        crate::artifact_file::persist_replace_or_create(path.clone(), bytes, "browser screenshot")
            .await?;
        let attachment = AttachmentItem {
            kind: AttachmentKind::Image,
            mime: "image/png".to_string(),
            source: AttachmentSource::LocalPath {
                path: path.to_string_lossy().to_string(),
            },
            filename: path
                .file_name()
                .and_then(|value| value.to_str())
                .map(ToOwned::to_owned),
            title: Some(format!("Browser screenshot {}", input.session_id)),
            size_bytes: Some(size_bytes),
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        };
        Ok(ToolInvokeOutput::from_parts(
            "browser screenshot",
            format!("image/png · {size_bytes} bytes"),
            format!("Saved browser screenshot to '{}'.", path.display()),
            Some(serde_json::json!({
                "session_id": input.session_id,
                "path": path,
                "size_bytes": size_bytes,
            })),
            std::collections::BTreeMap::from([
                ("agena.effect".to_string(), "file_changes".to_string()),
                ("path".to_string(), path.to_string_lossy().to_string()),
            ]),
            vec![attachment],
        ))
    }

    #[tool(
        tags(network, interactive, mutate, filesystem),
        name = "browser_download",
        summary = "Download one HTTP(S) URL through a managed browser session and return a local artifact."
    )]
    async fn browser_download(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &BrowserDownloadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let action_guard = self
            .browser_state
            .action_gate(&input.session_id)
            .await
            .lock_owned()
            .await;
        let _lease = agena_web::local_browser_lease().map_err(crawl_error_to_plugin)?;
        self.require_browser_owner(context, &input.session_id)
            .await?;
        const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;
        let url = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?;
        self.validate_network_target(&url).await?;
        let preflight_redirects = self.browser_preflight_redirects(&url).await?;
        let download_dir = self
            .workspace_root()?
            .join(".agena/artifacts/browser/downloads")
            .join(uuid::Uuid::new_v4().simple().to_string());
        let endpoint = self.browser_endpoint().await?;
        let browser_context_id = self
            .browser_state
            .meta
            .lock()
            .await
            .get(&input.session_id)
            .and_then(|meta| meta.browser_context_id.clone());
        let result = downloads::transfer(
            endpoint,
            browser_context_id,
            input.session_id.clone(),
            url.to_string(),
            download_dir,
            MAX_DOWNLOAD_BYTES,
            Duration::from_millis(input.timeout_ms),
            Arc::clone(&self.browser_download_lock),
            Some(action_guard),
        )
        .await?;
        let filename = result
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned);
        let attachment = AttachmentItem {
            kind: AttachmentKind::detect("", filename.as_deref()),
            mime: "application/octet-stream".into(),
            source: AttachmentSource::LocalPath {
                path: result.path.display().to_string(),
            },
            filename,
            title: Some(format!("Browser download from {url}")),
            size_bytes: Some(result.size),
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        };
        Ok(ToolInvokeOutput::from_parts(
            "browser download",
            format!("{} bytes · completed GUID {}", result.size, result.guid),
            format!(
                "Saved completed browser download at {}.\n{}",
                result.path.display(),
                result.warnings.join("\n")
            ),
            Some(
                serde_json::json!({"session_id":input.session_id,"url":url,"path":result.path,"size_bytes":result.size,"download_guid":result.guid,"state":"completed","cleanup_warnings":result.warnings,"preflight_redirects":preflight_redirects}),
            ),
            Default::default(),
            vec![attachment],
        ))
    }

    async fn wait_for_browser_condition(
        &self,
        target_id: &str,
        selector: Option<&str>,
        text: Option<&str>,
        timeout_ms: u64,
    ) -> SdkResult<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        let selector = selector
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let text = text
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        loop {
            let expression = if let Some(selector) = selector.as_deref() {
                format!("Boolean(document.querySelector({selector}))")
            } else if let Some(text) = text.as_deref() {
                format!("Boolean(document.body && document.body.innerText.includes({text}))")
            } else {
                "document.readyState === 'complete' || document.readyState === 'interactive'"
                    .to_string()
            };
            let probe = async {
                let client = self.browser_client(Some(target_id)).await?;
                client.evaluate(expression.as_str()).await
            };
            match tokio::time::timeout_at(deadline, probe).await {
                Ok(Ok(value)) if value.as_bool() == Some(true) => return Ok(()),
                Ok(Ok(_)) => {}
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    return Err(PluginError::internal(format!(
                        "browser wait exceeded {timeout_ms} ms"
                    )));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(PluginError::internal(format!(
                    "browser wait timed out after {timeout_ms} ms"
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn search_with_engine(
        &self,
        query: &str,
        limit: usize,
        max_pages: usize,
        engine: WebSearchEngine,
    ) -> SdkResult<WebSearchResponse> {
        let engine_url = url::Url::parse(engine.permission_url())
            .map_err(|err| PluginError::internal_error(&err))?;
        let state = self.state()?;
        // Reserve a small completion margin inside the aggregate deadline so
        // a timed-out later page can return its already collected results.
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(state.config.fetch.request.timeout_secs)
                .saturating_sub(Duration::from_millis(50));
        state
            .host
            .require_network_permission(engine_url.to_string())
            .await?;
        self.validate_network_target(&engine_url).await?;
        let config = &state.config;
        let options = WebSearchOptions {
            engine,
            limit,
            max_pages,
            timeout: deadline.saturating_duration_since(tokio::time::Instant::now()),
            user_agent: self.user_agent.clone(),
        };
        state
            .search_coordinator
            .search_with_renderer(
                query,
                &options,
                config.browser.enabled,
                |url, remaining| async move {
                    rendered_fetch::search_source(self, &url, remaining)
                        .await
                        .map(Some)
                        .map_err(|error| {
                            agena_web::CrawlError::InvalidInput(
                                error.diagnostic_message().to_owned(),
                            )
                        })
                },
            )
            .await
            .map_err(search_error_to_plugin)
    }
}

async fn validate_public_network_target(url: &url::Url) -> SdkResult<()> {
    resolve_public_network_target(url).await.map(|_| ())
}

async fn resolve_public_network_target(url: &url::Url) -> SdkResult<Vec<std::net::SocketAddr>> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(PluginError::invalid_params(
            "web requests require HTTP(S) without URL credentials",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::invalid_params("web URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PluginError::invalid_params("web URL has no known port"))?;

    let addresses: BTreeSet<std::net::SocketAddr> = match url.host() {
        Some(url::Host::Ipv4(ip)) => [std::net::SocketAddr::new(ip.into(), port)]
            .into_iter()
            .collect(),
        Some(url::Host::Ipv6(ip)) => [std::net::SocketAddr::new(ip.into(), port)]
            .into_iter()
            .collect(),
        _ => tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| {
                plugin_internal_error_with_context("failed to resolve fetch host", &error)
            })?
            .collect(),
    };
    if addresses.is_empty() {
        return Err(PluginError::internal(format!(
            "DNS resolution returned no addresses for {host}"
        )));
    }
    for address in &addresses {
        if !is_public_address(address.ip()) {
            return Err(PluginError::invalid_params(format!(
                "web URL host `{host}` resolves to non-public address `{address}`"
            )));
        }
    }
    Ok(addresses.into_iter().collect())
}

type CdpSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type CdpSink = futures_util::stream::SplitSink<CdpSocket, tokio_tungstenite::tungstenite::Message>;

struct CdpCommandRequest {
    method: String,
    params: serde_json::Value,
    response: oneshot::Sender<Result<serde_json::Value, String>>,
}

enum PendingCdpCommand {
    Caller {
        method: String,
        response: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    Interception {
        method: &'static str,
    },
}

struct NavigationDecision {
    request_id: String,
    url: String,
    result: Result<(), String>,
}

const CDP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CDP_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

type BrowserRequestPolicy = Arc<
    dyn Fn(String, bool) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync,
>;

type BrowserCallbackContext =
    Arc<std::sync::Mutex<Option<agena_plugin_host::sdk::host_api::HostCallbackContext>>>;
struct CdpAuthorization {
    policy: BrowserRequestPolicy,
    context: BrowserCallbackContext,
}

#[derive(Clone)]
struct CdpClient {
    commands: mpsc::Sender<CdpCommandRequest>,
    navigation_interception_enabled: Arc<OnceLock<()>>,
    navigation_errors: Arc<std::sync::Mutex<VecDeque<String>>>,
    callback_context: BrowserCallbackContext,
}

impl CdpClient {
    fn refresh_callback_context(&self) -> SdkResult<()> {
        if let Some(context) = agena_plugin_host::sdk::host_api::current_host_callback_context() {
            *self
                .callback_context
                .lock()
                .map_err(|_| PluginError::internal("browser callback context lock poisoned"))? =
                Some(context);
        }
        Ok(())
    }
    async fn connect(
        endpoint: &str,
        target_id: Option<&str>,
        events: Option<mpsc::Sender<CdpEvent>>,
    ) -> SdkResult<Self> {
        Self::connect_with_policy(
            endpoint,
            target_id,
            events,
            Arc::new(|url, _| {
                Box::pin(async move { authorize_browser_document_request(&url).await })
            }),
        )
        .await
    }

    async fn connect_with_policy(
        endpoint: &str,
        target_id: Option<&str>,
        events: Option<mpsc::Sender<CdpEvent>>,
        policy: BrowserRequestPolicy,
    ) -> SdkResult<Self> {
        let (mut socket, _) = tokio::time::timeout(
            CDP_CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async(endpoint),
        )
        .await
        .map_err(|error| {
            plugin_internal_error_with_context("timed out connecting to browser CDP", &error)
        })?
        .map_err(|error| {
            plugin_internal_error_with_context("cannot connect to browser CDP", &error)
        })?;

        let (session_id, next_id) = if let Some(target_id) = target_id {
            let result = tokio::time::timeout(
                CDP_CONNECT_TIMEOUT,
                cdp_attach_target(&mut socket, target_id),
            )
            .await
            .map_err(|error| {
                plugin_internal_error_with_context("timed out attaching browser CDP target", &error)
            })??;
            let session_id = result
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    PluginError::internal("browser target attach returned no session id")
                })?
                .to_string();
            (Some(session_id), 2)
        } else {
            (None, 1)
        };

        let (commands, command_receiver) = mpsc::channel(32);
        let navigation_interception_enabled = Arc::new(OnceLock::new());
        let navigation_errors = Arc::new(std::sync::Mutex::new(VecDeque::new()));
        let callback_context = Arc::new(std::sync::Mutex::new(
            agena_plugin_host::sdk::host_api::current_host_callback_context(),
        ));
        tokio::spawn(run_cdp_connection(
            socket,
            session_id,
            next_id,
            command_receiver,
            Arc::clone(&navigation_errors),
            events,
            CdpAuthorization {
                policy,
                context: callback_context.clone(),
            },
        ));

        Ok(Self {
            commands,
            navigation_interception_enabled,
            navigation_errors,
            callback_context,
        })
    }

    async fn enable_navigation_interception(&self) -> SdkResult<()> {
        if self.navigation_interception_enabled.get().is_some() {
            return Ok(());
        }
        self.command(
            "Fetch.enable",
            serde_json::json!({
                "patterns": [{
                    "urlPattern": "*",
                    "resourceType": "Document",
                    "requestStage": "Request",
                }],
            }),
        )
        .await?;
        if self.navigation_interception_enabled.set(()).is_err() {
            tracing::debug!(
                "browser navigation interception was concurrently enabled by another task"
            );
        }
        Ok(())
    }

    async fn command(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> SdkResult<serde_json::Value> {
        self.command_with_timeout(method, params, CDP_COMMAND_TIMEOUT)
            .await
    }

    async fn command_with_timeout(
        &self,
        method: &str,
        params: serde_json::Value,
        command_timeout: Duration,
    ) -> SdkResult<serde_json::Value> {
        // Every CDP exchange is browser activity: restart the idle auto-close
        // timer so a session mid-flight is never torn down underneath us.
        local_browser_touch().map_err(crawl_error_to_plugin)?;
        // Tokio tasks do not inherit task-local host authority. Refresh it on
        // each foreground command and echo it for intercepted requests. The
        // host rejects an expired authority; no background workspace fallback.
        self.refresh_callback_context()?;
        if let Some(error) = self.take_navigation_error() {
            return Err(PluginError::internal(error));
        }

        let (response, receiver) = oneshot::channel();
        let result = tokio::time::timeout(command_timeout, async {
            self.commands
                .send(CdpCommandRequest {
                    method: method.to_string(),
                    params,
                    response,
                })
                .await
                .map_err(|error| {
                    PluginError::internal(format!(
                        "browser CDP connection ended while sending `{method}`: {error}"
                    ))
                })?;
            receiver.await.map_err(|error| {
                plugin_internal_error_with_context(
                    format!("browser CDP connection ended while awaiting `{method}`").as_str(),
                    &error,
                )
            })
        })
        .await
        .map_err(|error| {
            plugin_internal_error_with_context(
                format!(
                    "browser CDP command `{method}` timed out after {}s",
                    command_timeout.as_secs_f64()
                )
                .as_str(),
                &error,
            )
        })??;

        if let Some(error) = self.take_navigation_error() {
            return Err(PluginError::internal(error));
        }
        result.map_err(PluginError::internal)
    }

    async fn evaluate(&self, expression: &str) -> SdkResult<serde_json::Value> {
        let result = self
            .command(
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(PluginError::internal(format!(
                "browser JavaScript evaluation failed: {exception}"
            )));
        }
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    fn take_navigation_error(&self) -> Option<String> {
        match self.navigation_errors.lock() {
            Ok(mut errors) => errors.pop_front(),
            Err(poisoned) => {
                tracing::error!(
                    "browser navigation error queue lock was poisoned; recovering queued safety diagnostics"
                );
                poisoned.into_inner().pop_front()
            }
        }
    }

    fn is_closed(&self) -> bool {
        self.commands.is_closed()
    }
}

async fn cdp_attach_target(
    socket: &mut CdpSocket,
    target_id: &str,
) -> SdkResult<serde_json::Value> {
    let request = serde_json::json!({
        "id": 1,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true,
        },
    });
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            request.to_string().into(),
        ))
        .await
        .map_err(|error| plugin_internal_error_with_context("browser CDP send failed", &error))?;

    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| {
            plugin_internal_error_with_context("browser CDP read failed", &error)
        })?;
        let Some(value) = cdp_message_value(socket, message).await? else {
            continue;
        };
        if value.get("id").and_then(serde_json::Value::as_u64) != Some(1) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(PluginError::internal(format!(
                "browser CDP Target.attachToTarget failed: {error}"
            )));
        }
        return Ok(value
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null));
    }
    Err(PluginError::internal("browser CDP connection ended"))
}

async fn cdp_message_value(
    socket: &mut CdpSocket,
    message: tokio_tungstenite::tungstenite::Message,
) -> SdkResult<Option<serde_json::Value>> {
    let text = match message {
        tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
        tokio_tungstenite::tungstenite::Message::Binary(bytes) => String::from_utf8(bytes.to_vec())
            .map_err(|error| {
                plugin_internal_error_with_context("browser CDP returned non-UTF-8 data", &error)
            })?,
        tokio_tungstenite::tungstenite::Message::Ping(payload) => {
            socket
                .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                .await
                .map_err(|error| {
                    plugin_internal_error_with_context("browser CDP pong failed", &error)
                })?;
            return Ok(None);
        }
        tokio_tungstenite::tungstenite::Message::Pong(_)
        | tokio_tungstenite::tungstenite::Message::Frame(_) => return Ok(None),
        tokio_tungstenite::tungstenite::Message::Close(_) => {
            return Err(PluginError::internal("browser CDP connection closed"));
        }
    };
    serde_json::from_str(text.as_str())
        .map(Some)
        .map_err(|error| plugin_internal_error_with_context("invalid browser CDP response", &error))
}

async fn run_cdp_connection(
    socket: CdpSocket,
    session_id: Option<String>,
    mut next_id: u64,
    mut commands: mpsc::Receiver<CdpCommandRequest>,
    navigation_errors: Arc<std::sync::Mutex<VecDeque<String>>>,
    events: Option<mpsc::Sender<CdpEvent>>,
    authorization: CdpAuthorization,
) {
    let (mut sink, mut source) = socket.split();
    // At most sixteen authorization checks can be active, so one result slot
    // per check is sufficient and gives the CDP loop deterministic memory use.
    let (decisions, mut decision_receiver) = mpsc::channel::<NavigationDecision>(16);
    let authorization_slots = Arc::new(Semaphore::new(16));
    let mut pending = BTreeMap::<u64, PendingCdpCommand>::new();
    let mut checks = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            result = checks.join_next(), if !checks.is_empty() => {
                if let Some(Err(error)) = result {
                    push_navigation_error(&navigation_errors, format!("browser request authorization worker failed: {error}"));
                }
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    if let Err(error) = sink.close().await {
                        tracing::warn!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to close the managed browser CDP connection after its command channel closed",
                                &error,
                            ),
                            "managed browser CDP connection did not close cleanly"
                        );
                    }
                    break;
                };
                let id = next_id;
                pending.retain(|_, command| match command {
                    PendingCdpCommand::Caller { response, .. } => !response.is_closed(),
                    PendingCdpCommand::Interception { .. } => true,
                });
                next_id = next_id.saturating_add(1);
                match send_cdp_request(
                    &mut sink,
                    id,
                    command.method.as_str(),
                    command.params,
                    session_id.as_deref(),
                )
                .await
                {
                    Ok(()) => {
                        pending.insert(
                            id,
                            PendingCdpCommand::Caller {
                                method: command.method,
                                response: command.response,
                            },
                        );
                    }
                    Err(error) => {
                        if command.response.send(Err(error)).is_err() {
                            tracing::debug!(
                                command_id = id,
                                "browser CDP command response receiver closed before send failure could be delivered"
                            );
                        }
                    }
                }
            }
            decision = decision_receiver.recv() => {
                let Some(decision) = decision else {
                    continue;
                };
                let (method, params) = match decision.result {
                    Ok(()) => (
                        "Fetch.continueRequest",
                        serde_json::json!({ "requestId": decision.request_id }),
                    ),
                    Err(error) => {
                        push_navigation_error(
                            &navigation_errors,
                            format!(
                                "browser document request to '{}' was blocked before dispatch: {error}",
                                decision.url
                            ),
                        );
                        (
                            "Fetch.failRequest",
                            serde_json::json!({
                                "requestId": decision.request_id,
                                "errorReason": "BlockedByClient",
                            }),
                        )
                    }
                };
                let id = next_id;
                next_id = next_id.saturating_add(1);
                match send_cdp_request(
                    &mut sink,
                    id,
                    method,
                    params,
                    session_id.as_deref(),
                )
                .await
                {
                    Ok(()) => {
                        pending.insert(id, PendingCdpCommand::Interception { method });
                    }
                    Err(error) => {
                        push_navigation_error(&navigation_errors, error);
                    }
                }
            }
            message = source.next() => {
                let Some(message) = message else {
                    fail_pending_commands(&mut pending, "browser CDP connection ended");
                    break;
                };
                let value = match message {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                        serde_json::from_str::<serde_json::Value>(text.as_ref())
                            .map_err(|error| format!("invalid browser CDP response: {error}"))
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)) => {
                        String::from_utf8(bytes.to_vec())
                            .map_err(|error| format!("browser CDP returned non-UTF-8 data: {error}"))
                            .and_then(|text| serde_json::from_str(text.as_str())
                                .map_err(|error| format!("invalid browser CDP response: {error}")))
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Ping(payload)) => {
                        if let Err(error) = sink
                            .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                            .await
                        {
                            fail_pending_commands(
                                &mut pending,
                                format!("browser CDP pong failed: {error}").as_str(),
                            );
                            break;
                        }
                        continue;
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Pong(_))
                    | Ok(tokio_tungstenite::tungstenite::Message::Frame(_)) => continue,
                    Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                        fail_pending_commands(&mut pending, "browser CDP connection closed");
                        break;
                    }
                    Err(error) => Err(format!("browser CDP read failed: {error}")),
                };
                let value = match value {
                    Ok(value) => value,
                    Err(error) => {
                        fail_pending_commands(&mut pending, error.as_str());
                        break;
                    }
                };

                if let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) {
                    complete_cdp_command(id, value, &mut pending, &navigation_errors);
                    continue;
                }

                // Forward CDP notifications to the live activity-logging sink
                // before the navigation-interception branch consumes them.
                if let Some(events) = &events
                    && let Some(method) =
                        value.get("method").and_then(serde_json::Value::as_str)
                {
                    let event = CdpEvent {
                        method: method.to_string(),
                        params: value
                            .get("params")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    };
                    if let Err(error) = events.try_send(event) {
                        let event = error.into_inner();
                        tracing::warn!(
                            event_method = %event.method,
                            queue_state = if events.is_closed() { "closed" } else { "full" },
                            "browser CDP activity event was dropped before projection"
                        );
                    }
                }

                if value.get("method").and_then(serde_json::Value::as_str)
                    != Some("Fetch.requestPaused")
                {
                    continue;
                }
                let Some(request_id) = value
                    .pointer("/params/requestId")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                else {
                    push_navigation_error(
                        &navigation_errors,
                        "browser Fetch.requestPaused notification had no request id".to_string(),
                    );
                    continue;
                };
                let url = value
                    .pointer("/params/request/url")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let is_document = value
                    .pointer("/params/resourceType")
                    .and_then(serde_json::Value::as_str)
                    == Some("Document");
                let decisions = decisions.clone();
                let authorization_slot = Arc::clone(&authorization_slots).try_acquire_owned();
                let Ok(authorization_slot) = authorization_slot else {
                    let error =
                        "browser document interception exceeded 16 concurrent network safety checks";
                    push_navigation_error(
                        &navigation_errors,
                        format!(
                            "browser document request to '{url}' was blocked before dispatch: {error}"
                        ),
                    );
                    let id = next_id;
                    next_id = next_id.saturating_add(1);
                    match send_cdp_request(
                        &mut sink,
                        id,
                        "Fetch.failRequest",
                        serde_json::json!({
                            "requestId": request_id,
                            "errorReason": "BlockedByClient",
                        }),
                        session_id.as_deref(),
                    )
                    .await
                    {
                        Ok(()) => {
                            pending.insert(
                                id,
                                PendingCdpCommand::Interception {
                                    method: "Fetch.failRequest",
                                },
                            );
                        }
                        Err(error) => push_navigation_error(&navigation_errors, error),
                    }
                    continue;
                };
                let policy = authorization.policy.clone();
                let context = match authorization.context.lock() {
                    Ok(context) => context.clone(),
                    Err(_) => { push_navigation_error(&navigation_errors, "browser callback context lock poisoned".into()); break; }
                };
                checks.spawn(async move {
                    let _authorization_slot = authorization_slot;
                    let authorized = async {
                        let operation = policy(url.clone(), is_document);
                        match context {
                            Some(context) => agena_plugin_host::sdk::host_api::run_in_isolated_host_callback_context(context, operation).await,
                            None => operation.await,
                        }
                    };
                    let result = tokio::time::timeout(CDP_COMMAND_TIMEOUT, authorized).await
                        .unwrap_or_else(|_| Err("browser request authorization timed out".into()));
                    if let Err(error) = decisions
                        .send(NavigationDecision {
                            request_id,
                            url,
                            result,
                        })
                        .await
                    {
                        let decision = error.0;
                        tracing::warn!(
                            request_id = %decision.request_id,
                            url = %decision.url,
                            "browser navigation authorization decision could not be delivered because the CDP connection ended"
                        );
                    }
                });
            }
        }
    }
}

async fn authorize_browser_document_request(raw_url: &str) -> Result<(), String> {
    let url = url::Url::parse(raw_url).map_err(|error| format!("invalid document URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "unsupported document URL scheme '{}'; only HTTP(S) navigation is allowed",
            url.scheme()
        ));
    }
    validate_public_network_target(&url)
        .await
        .map_err(|error| error.diagnostic_message().to_owned())
}

async fn send_cdp_request(
    sink: &mut CdpSink,
    id: u64,
    method: &str,
    params: serde_json::Value,
    session_id: Option<&str>,
) -> Result<(), String> {
    let mut request = serde_json::json!({
        "id": id,
        "method": method,
        "params": params,
    });
    if let Some(session_id) = session_id {
        request["sessionId"] = serde_json::Value::String(session_id.to_string());
    }
    sink.send(tokio_tungstenite::tungstenite::Message::Text(
        request.to_string().into(),
    ))
    .await
    .map_err(|error| format!("browser CDP {method} send failed: {error}"))
}

fn complete_cdp_command(
    id: u64,
    response: serde_json::Value,
    pending: &mut BTreeMap<u64, PendingCdpCommand>,
    navigation_errors: &Arc<std::sync::Mutex<VecDeque<String>>>,
) {
    let Some(command) = pending.remove(&id) else {
        return;
    };
    match command {
        PendingCdpCommand::Caller {
            method,
            response: tx,
        } => {
            let result = if let Some(error) = response.get("error") {
                Err(format!("browser CDP {method} failed: {error}"))
            } else {
                Ok(response
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null))
            };
            if tx.send(result).is_err() {
                tracing::debug!(
                    command_id = id,
                    command_method = %method,
                    "browser CDP command response receiver closed before delivery"
                );
            }
        }
        PendingCdpCommand::Interception { method } => {
            if let Some(error) = response.get("error") {
                push_navigation_error(
                    navigation_errors,
                    format!("browser CDP {method} failed: {error}"),
                );
            }
        }
    }
}

fn fail_pending_commands(pending: &mut BTreeMap<u64, PendingCdpCommand>, error: &str) {
    for (command_id, command) in std::mem::take(pending) {
        if let PendingCdpCommand::Caller { response, .. } = command
            && response.send(Err(error.to_string())).is_err()
        {
            tracing::debug!(
                command_id,
                "browser CDP failure response receiver closed before delivery"
            );
        }
    }
}

fn push_navigation_error(errors: &Arc<std::sync::Mutex<VecDeque<String>>>, error: String) {
    let mut errors = match errors.lock() {
        Ok(errors) => errors,
        Err(poisoned) => {
            tracing::error!(
                operation = "record browser navigation error",
                error = %poisoned,
                "recovering poisoned browser navigation-error queue"
            );
            poisoned.into_inner()
        }
    };
    if errors.len() >= 32 {
        errors.pop_front();
    }
    errors.push_back(error);
}

fn ensure_browser_action(result: &serde_json::Value) -> SdkResult<()> {
    if result.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(());
    }
    Err(PluginError::invalid_params(
        result
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("browser action failed"),
    ))
}

/// Build a browser-side input operation that cooperates with React-style
/// controlled inputs. Assigning `el.value` directly updates React's private
/// value tracker in many versions, so the subsequent synthetic `input` event
/// is ignored. Calling the native prototype setter and resetting the tracker
/// to the previous value lets the framework observe a real value transition.
///
/// This runs only in the already attached CDP target, and it does not
/// introduce a separate browser/plugin execution surface.
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

fn browser_snapshot_expression() -> String {
    include_str!("browser/snapshot.js").replace(
        "__AGENA_SNAPSHOT_ID__",
        &format!("\"{}\"", uuid::Uuid::new_v4()),
    )
}

fn browser_element_expression(
    selector: Option<&str>,
    element_ref: Option<u16>,
    snapshot_id: Option<&str>,
) -> SdkResult<String> {
    match (
        selector.filter(|value| !value.trim().is_empty()),
        element_ref,
    ) {
        (Some(selector), None) if snapshot_id.is_none() => serde_json::to_string(selector)
            .map(|selector| format!("document.querySelector({selector})"))
            .map_err(|error| PluginError::invalid_params_error(&error)),
        (None, Some(element_ref)) => {
            let id = snapshot_id.filter(|id| !id.is_empty()).ok_or_else(|| {
                PluginError::invalid_params(
                    "ref requires snapshot_id from browser_snapshot; refresh before retrying",
                )
            })?;
            let id = serde_json::to_string(id)
                .map_err(|error| PluginError::invalid_params_error(&error))?;
            Ok(format!(
                r#"(() => {{
                const state = globalThis[Symbol.for('agena.browser.snapshot')];
                if (!state || state.id !== {id} || state.document !== document || state.url !== location.href) throw new Error('stale browser snapshot; refresh before acting');
                const el = state.nodes[{element_ref}];
                if (!el || !el.isConnected || el.ownerDocument !== document || state.signature(el) !== state.signatures[{element_ref}]) throw new Error('stale browser element; refresh before acting');
                return el;
            }})()"#
            ))
        }
        _ => Err(PluginError::invalid_params(
            "provide selector alone, or ref together with snapshot_id",
        )),
    }
}

fn browser_type_expression(
    selector: Option<&str>,
    element_ref: Option<u16>,
    snapshot_id: Option<&str>,
    text: &str,
    press_enter: bool,
) -> SdkResult<String> {
    let target = browser_element_expression(selector, element_ref, snapshot_id)?;
    let text =
        serde_json::to_string(text).map_err(|error| PluginError::invalid_params_error(&error))?;
    let enter = if press_enter {
        "el.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',bubbles:true})); el.dispatchEvent(new KeyboardEvent('keypress',{key:'Enter',code:'Enter',bubbles:true})); el.dispatchEvent(new KeyboardEvent('keyup',{key:'Enter',code:'Enter',bubbles:true}));"
    } else {
        ""
    };
    Ok(format!(
        r#"(() => {{
            const el = {target};
            if (!el) return {{ok:false,error:'browser element not found'}};
            if (el.disabled || el.readOnly) return {{ok:false,error:'element is disabled or read-only'}};
            el.scrollIntoView({{block:'center'}});
            el.focus();
            const next = {text};
            let method = 'direct';
            let value = '';
            if (el.isContentEditable) {{
                el.textContent = next;
                value = el.textContent || '';
                method = 'contenteditable';
            }} else {{
                if (!('value' in el)) return {{ok:false,error:'selector does not target an editable element'}};
                if (String(el.type || '').toLowerCase() === 'file') return {{ok:false,error:'file inputs cannot be filled'}};
                const previous = String(el.value ?? '');
                const prototype = el instanceof HTMLTextAreaElement
                    ? HTMLTextAreaElement.prototype
                    : el instanceof HTMLSelectElement
                        ? HTMLSelectElement.prototype
                        : HTMLInputElement.prototype;
                const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
                if (typeof setter === 'function') {{
                    setter.call(el, next);
                    method = 'native_setter';
                }} else {{
                    el.value = next;
                }}
                // React's value tracker compares against this old value while
                // handling the event. Do not depend on it existing: it is an
                // implementation detail and other frameworks ignore it.
                const tracker = el._valueTracker;
                if (tracker && typeof tracker.setValue === 'function') tracker.setValue(previous);
                value = String(el.value ?? '');
            }}
            let inputEvent;
            try {{
                inputEvent = new InputEvent('input', {{bubbles:true,inputType:'insertText',data:next}});
            }} catch (_) {{
                inputEvent = new Event('input', {{bubbles:true}});
            }}
            el.dispatchEvent(inputEvent);
            el.dispatchEvent(new Event('change', {{bubbles:true}}));
            {enter}
            return {{ok:true,method,value_omitted:true}};
        }})()"#
    ))
}

fn browser_action_output(
    title: &str,
    session_id: &str,
    result: serde_json::Value,
) -> ToolInvokeOutput {
    let summary = result
        .get("snapshot")
        .map(browser_snapshot_summary)
        .unwrap_or_else(|| "Completed".to_string());
    ToolInvokeOutput::from_parts(
        title,
        summary,
        format!(
            "Browser action acknowledged: {title} in session {session_id}.\n\n{}",
            result
                .get("snapshot")
                .map(format_browser_snapshot)
                .unwrap_or_default()
        ),
        Some(serde_json::json!({
            "session_id": session_id,
            "result": result,
        })),
        std::collections::BTreeMap::new(),
        Vec::new(),
    )
}

fn browser_snapshot_summary(snapshot: &serde_json::Value) -> String {
    let title = snapshot
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Untitled page");
    let element_count = snapshot
        .get("elements")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    format!("{title} · {element_count} interactive elements")
}

fn browser_text_prefix(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn format_browser_snapshot(snapshot: &serde_json::Value) -> String {
    let value = |key| {
        snapshot
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    };
    let mut text = format!(
        "Snapshot ID: {}\nTitle: {}\nURL: {}\nReady state: {}\nUse ref together with this snapshot_id; form values are omitted.\n",
        value("snapshot_id"),
        browser_text_prefix(value("title"), 512),
        browser_text_prefix(value("url"), 1024),
        value("ready_state")
    );
    if let Some(elements) = snapshot
        .get("elements")
        .and_then(serde_json::Value::as_array)
    {
        text.push_str(&format!("Interactive elements: {}\n", elements.len()));
        for (index, element) in elements.iter().enumerate() {
            let label = element
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let tag = element
                .get("tag")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let row = format!(
                "ref {} [{}] {}{}\n",
                element
                    .get("ref")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(index as u64),
                browser_text_prefix(tag, 32),
                browser_text_prefix(label, 128),
                if element.get("disabled").and_then(serde_json::Value::as_bool) == Some(true) {
                    " (disabled)"
                } else {
                    ""
                }
            );
            if text.len() + row.len() > 20000 {
                text.push_str("Additional element rows omitted from the text preview.\n");
                break;
            }
            text.push_str(&row);
        }
    }
    text.push_str("\nVisible text:\n");
    let remaining = 32000usize.saturating_sub(text.len());
    text.push_str(browser_text_prefix(value("text"), remaining));
    text
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
            if let Some(mapped) = address.to_ipv4_mapped() {
                return is_public_address(mapped.into());
            }
            !address.is_loopback()
                && !address.is_unique_local()
                && !address.is_unicast_link_local()
                && !address.is_unspecified()
                && !address.is_multicast()
        }
    }
}

fn resolve_browser_redirect(base: &url::Url, location: &str) -> SdkResult<url::Url> {
    let redirect = base.join(location).map_err(|error| {
        PluginError::invalid_params(format!("invalid browser redirect Location: {error}"))
    })?;
    match redirect.scheme() {
        "http" | "https" => Ok(redirect),
        scheme => Err(PluginError::invalid_params(format!(
            "browser redirect uses unsupported scheme `{scheme}`"
        ))),
    }
}

fn crawl_error_to_plugin(err: agena_web::CrawlError) -> PluginError {
    PluginError::internal_error(&err)
}

fn search_error_to_plugin(err: agena_web::CrawlError) -> PluginError {
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
            let state = self.plugin.state().map_err(|error| {
                agena_web::CrawlError::InvalidInput(error.diagnostic_message().to_owned())
            })?;
            // Legacy documents have no final URL; do not infer it from an HTML
            // canonical hint, which need not be the transport destination.
            if document.final_url.is_empty() {
                return Err(agena_web::CrawlError::InvalidInput(
                    "cached document predates transport URL tracking; crawl with use_cache=false"
                        .into(),
                ));
            }
            for url in [requested_url.as_str(), &document.url, &document.final_url] {
                state
                    .host
                    .require_network_permission(url.to_owned())
                    .await
                    .map_err(|error| {
                        agena_web::CrawlError::InvalidInput(error.diagnostic_message().to_owned())
                    })?;
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
                .map_err(|error| {
                    agena_web::CrawlError::InvalidInput(error.diagnostic_message().to_owned())
                })
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
    if web.fetch.request.max_body_bytes > 32 * 1024 * 1024 || web.fetch.request.timeout_secs > 120 {
        return Err(PluginError::invalid_params(
            "fetch.request allows at most 32 MiB and 120 seconds",
        ));
    }
    if !(1..=8).contains(&web.crawl.limits.concurrency)
        || web.crawl.defaults.concurrency == 0
        || web.crawl.defaults.concurrency > web.crawl.limits.concurrency
    {
        return Err(PluginError::invalid_params(
            "crawl concurrency requires 1 <= defaults.concurrency <= limits.concurrency <= 8",
        ));
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
    if web
        .browser
        .executable_path
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(PluginError::internal(
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
mod tests {
    use std::{
        collections::VecDeque,
        net::{Ipv4Addr, Ipv6Addr},
        sync::{Arc, OnceLock},
    };

    use agena_domain::{BackgroundActivityKind, BackgroundActivityStatus};
    use agena_plugin_host::sdk::{Plugin, ToolTag};

    use super::{
        BrowserSessionLog, CdpClient, WebPlugin, browser_activity, browser_element_expression,
        browser_type_expression, is_public_address, resolve_browser_redirect,
    };

    #[tokio::test]
    async fn cdp_commands_fail_instead_of_waiting_forever_for_a_response() {
        let (commands, _requests) = tokio::sync::mpsc::channel(1);
        let client = CdpClient {
            callback_context: Default::default(),
            commands,
            navigation_interception_enabled: Arc::new(OnceLock::new()),
            navigation_errors: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        };

        let error = client
            .command_with_timeout(
                "Runtime.evaluate",
                serde_json::json!({}),
                std::time::Duration::from_millis(20),
            )
            .await
            .expect_err("a silent CDP peer must time out");
        assert!(error.to_string().contains("timed out"));
    }

    #[test]
    fn browser_session_log_implements_the_since_seq_cursor_protocol() {
        let mut log = BrowserSessionLog::new();
        log.append("event", "Opened https://example.com/.");
        log.append("console", "[log] hello");

        let first = log.read("browser_t1", 0, None);
        assert_eq!(first.lines.len(), 2);
        assert_eq!(first.lines[0].seq, 1);
        assert_eq!(first.lines[0].stream, "event");
        assert_eq!(first.lines[1].text, "[log] hello");
        assert_eq!(first.last_seq, 2);
        assert!(!first.has_more);

        let incremental = log.read("browser_t1", 1, None);
        assert_eq!(incremental.lines.len(), 1);
        assert_eq!(incremental.lines[0].seq, 2);

        let limited = log.read("browser_t1", 0, Some(1));
        assert_eq!(limited.lines.len(), 1);
        assert!(limited.has_more);
        assert_eq!(limited.lines[0].seq, 1);

        // Capacity is bounded; overflow is counted as dropped lines.
        for index in 0..600 {
            log.append("console", format!("line {index}"));
        }
        let tail = log.read("browser_t1", 0, None);
        assert_eq!(tail.lines.len(), 500);
        assert!(tail.dropped_lines > 0);
        assert_eq!(tail.lines.last().map(|line| line.seq), Some(602));
    }

    #[test]
    fn browser_activity_records_are_unified_and_terminal_state_is_dismissible() {
        let running = browser_activity(
            "browser_target-1".to_string(),
            "Browser · example.com".to_string(),
            "https://example.com/".to_string(),
            BackgroundActivityStatus::Running,
            Some(1000),
            None,
            None,
        );
        assert_eq!(running.id, "browser_target-1");
        assert_eq!(running.kind, BackgroundActivityKind::Browser);
        assert_eq!(running.status, BackgroundActivityStatus::Running);
        assert!(running.cancellable);
        assert!(!running.dismissible);

        let stopped = browser_activity(
            "browser_target-1".to_string(),
            "Browser · example.com".to_string(),
            "https://example.com/".to_string(),
            BackgroundActivityStatus::Stopped,
            Some(1000),
            Some(2000),
            Some("Closed by browser_close".to_string()),
        );
        assert_eq!(stopped.status, BackgroundActivityStatus::Stopped);
        assert_eq!(stopped.finished_at_ms, Some(2000));
        assert!(stopped.dismissible);
    }

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
        assert!(!is_public_address("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_public_address("::ffff:10.0.0.1".parse().unwrap()));
        assert!(!is_public_address("ff02::1".parse().unwrap()));
    }

    #[test]
    fn manifest_exposes_interactive_browser_lifecycle() {
        let manifest = WebPlugin::new().manifest();
        let names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        for name in [
            "browser_open",
            "browser_list",
            "browser_close",
            "browser_shutdown",
            "browser_snapshot",
            "browser_click",
            "browser_type",
            "browser_wait",
            "browser_screenshot",
            "browser_download",
        ] {
            assert!(names.contains(&name), "missing {name}");
            let tool = manifest
                .tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("tool name checked above");
            assert!(
                tool.has_tag(ToolTag::Interactive),
                "browser tool {name} must be marked interactive"
            );
        }
    }

    #[test]
    fn browser_type_expression_uses_native_setter_and_react_tracker_fallback() {
        let expression = browser_type_expression(Some("#search"), None, None, "hello", true)
            .expect("browser type expression");
        assert!(expression.contains("Object.getOwnPropertyDescriptor"));
        assert!(expression.contains("_valueTracker"));
        assert!(expression.contains("native_setter"));
        assert!(expression.contains("KeyboardEvent('keypress'"));
        assert!(expression.contains("\"#search\""));
        assert!(expression.contains("\"hello\""));
        assert!(
            browser_element_expression(None, Some(7), Some("snapshot"))
                .expect("snapshot ref expression")
                .contains("state.nodes[7]")
        );
        assert!(browser_element_expression(Some("#search"), Some(7), Some("snapshot")).is_err());
        assert!(browser_element_expression(None, None, None).is_err());
    }

    #[tokio::test]
    async fn cdp_client_can_create_attach_evaluate_and_capture() {
        let options = agena_web::LocalBrowserOptions::default();
        let Ok(endpoint) =
            tokio::task::spawn_blocking(move || agena_web::local_browser_endpoint(&options))
                .await
                .expect("browser launcher task")
        else {
            // Chrome is an optional runtime dependency; manifest coverage
            // still runs on build hosts without a browser binary.
            return;
        };
        struct BrowserCleanup;
        impl Drop for BrowserCleanup {
            fn drop(&mut self) {
                let _ = agena_web::shutdown_local_browser();
            }
        }
        let _cleanup = BrowserCleanup;
        let root = CdpClient::connect(endpoint.as_str(), None, None)
            .await
            .expect("connect browser");
        let created = root
            .command(
                "Target.createTarget",
                serde_json::json!({"url": "about:blank"}),
            )
            .await
            .expect("create target");
        let target = created["targetId"].as_str().expect("target id");
        let page = CdpClient::connect(endpoint.as_str(), Some(target), None)
            .await
            .expect("attach target");
        let value = page
            .evaluate("({ok:true, value: 42})")
            .await
            .expect("evaluate");
        assert_eq!(value["value"], 42);
        page
            .evaluate(
                r#"(() => {
                    document.body.innerHTML = '<input id="controlled">';
                    const input = document.querySelector('#controlled');
                    const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
                    let setterCalls = 0;
                    Object.defineProperty(HTMLInputElement.prototype, 'value', {
                        configurable: true,
                        enumerable: descriptor.enumerable,
                        get: descriptor.get,
                        set(value) { setterCalls += 1; return descriptor.set.call(this, value); },
                    });
                    const tracker = { value: 'unexpected', setValue(value) { this.value = value; } };
                    input._valueTracker = tracker;
                    let inputEvents = 0;
                    let changeEvents = 0;
                    input.addEventListener('input', () => { inputEvents += 1; });
                    input.addEventListener('change', () => { changeEvents += 1; });
                    window.__agenaTypeMetrics = () => ({
                        value: input.value,
                        setterCalls,
                        trackerValue: tracker.value,
                        inputEvents,
                        changeEvents,
                    });
                    return {ok:true};
                })()"#,
            )
            .await
            .expect("install controlled input fixture");
        let type_result = page
            .evaluate(
                browser_type_expression(Some("#controlled"), None, None, "updated", false)
                    .expect("browser type expression")
                    .as_str(),
            )
            .await
            .expect("type controlled input");
        assert_eq!(type_result["method"], "native_setter");
        let metrics = page
            .evaluate("window.__agenaTypeMetrics()")
            .await
            .expect("read controlled input metrics");
        assert_eq!(metrics["value"], "updated");
        assert!(metrics["setterCalls"].as_u64().unwrap_or_default() >= 1);
        assert_eq!(metrics["trackerValue"], "");
        assert_eq!(metrics["inputEvents"], 1);
        assert_eq!(metrics["changeEvents"], 1);
        page.evaluate(r#"(() => {
            document.body.innerHTML = '<div role="group"><input id="password" type="password" value="AUDIT_SECRET"><textarea id="token">AUDIT_TEXTAREA_SECRET</textarea><div contenteditable="true">AUDIT_EDITABLE_SECRET</div><button id="first">First</button><button id="second">Second</button></div>';
            const unicodeButton = document.createElement('button');
            unicodeButton.id = 'unicode'; unicodeButton.textContent = 'a' + '😀'.repeat(200);
            document.body.append(unicodeButton);
            window.auditClicks = [];
            document.querySelector('#second').onclick = () => window.auditClicks.push('second');
            return {ok:true};
        })()"#).await.expect("install sensitive DOM fixture");
        let snapshot = page
            .evaluate(&super::browser_snapshot_expression())
            .await
            .expect("capture production snapshot");
        let encoded = snapshot.to_string();
        assert!(!encoded.contains("AUDIT_SECRET"));
        assert!(!encoded.contains("AUDIT_TEXTAREA_SECRET"));
        assert!(!encoded.contains("AUDIT_EDITABLE_SECRET"));
        let unicode_text = snapshot["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|element| element["id"] == "unicode")
            .unwrap()["text"]
            .as_str()
            .unwrap();
        assert_eq!(
            unicode_text.chars().count(),
            150,
            "snapshot clipping must not split UTF-16 surrogate pairs"
        );
        let snapshot_id = snapshot["snapshot_id"].as_str().unwrap();
        let element_ref = snapshot["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|el| el["id"] == "second")
            .unwrap()["ref"]
            .as_u64()
            .unwrap() as u16;
        page.evaluate("(() => { const b=document.createElement('button'); b.textContent='Inserted'; document.body.prepend(b); return {ok:true}; })()").await.unwrap();
        let selected =
            browser_element_expression(None, Some(element_ref), Some(snapshot_id)).unwrap();
        page.evaluate(&format!(
            "(() => {{ ({selected}).click(); return {{ok:true}}; }})()"
        ))
        .await
        .expect("click captured node, not shifted array index");
        assert_eq!(
            page.evaluate("window.auditClicks").await.unwrap(),
            serde_json::json!(["second"])
        );
        let typed = page
            .evaluate(
                &browser_type_expression(Some("#password"), None, None, "AUDIT_NEW_SECRET", false)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(!typed.to_string().contains("AUDIT_NEW_SECRET"));
        assert!(typed.get("value").is_none());
        page.evaluate(&super::browser_snapshot_expression())
            .await
            .unwrap();
        assert!(
            page.evaluate(&selected).await.is_err(),
            "old snapshot must be rejected"
        );
        eprintln!(
            "AUDIT_REAL_BROWSER: secret omission, stable nodes, stale snapshot rejection passed"
        );
        page.command("Page.enable", serde_json::json!({}))
            .await
            .expect("enable page");
        let screenshot = page
            .command(
                "Page.captureScreenshot",
                serde_json::json!({"format": "png"}),
            )
            .await
            .expect("screenshot");
        assert!(
            screenshot["data"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        let isolated = WebPlugin::new();
        *isolated.browser_state.root.lock().await = Some(root.clone());
        let (first, second) =
            tokio::join!(isolated.browser_client(None), isolated.browser_client(None));
        let first = first.expect("reuse context-owning root without relaunching");
        let second = second.expect("coalesce root access for another call");
        assert!(first.commands.same_channel(&second.commands));
        assert!(root.commands.same_channel(&first.commands));

        let scope = tempfile::tempdir().unwrap();
        let owner_a = agena_runtime_tools::TerminalOwner {
            workspace: scope.path().canonicalize().unwrap(),
            session_id: Some(41),
        };
        let owner_b = agena_runtime_tools::TerminalOwner {
            workspace: owner_a.workspace.clone(),
            session_id: Some(42),
        };
        let context_a = isolated
            .browser_context_for_owner(&root, &owner_a)
            .await
            .unwrap();
        let context_b = isolated
            .browser_context_for_owner(&root, &owner_b)
            .await
            .unwrap();
        assert_ne!(context_a, context_b);
        let retained = isolated.browser_client(None).await.unwrap();
        assert!(retained.commands.same_channel(&root.commands));
        let contexts = retained
            .command("Target.getBrowserContexts", serde_json::json!({}))
            .await
            .unwrap();
        assert!(
            contexts["browserContextIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == &context_a)
        );
        assert!(
            contexts["browserContextIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == &context_b)
        );

        root.command("Storage.setCookies",serde_json::json!({"browserContextId":context_a,"cookies":[{"name":"audit_private","value":"synthetic_cookie","url":"https://example.invalid/"}]})).await.unwrap();
        let own = root
            .command(
                "Storage.getCookies",
                serde_json::json!({"browserContextId":context_a}),
            )
            .await
            .unwrap();
        let other = root
            .command(
                "Storage.getCookies",
                serde_json::json!({"browserContextId":context_b}),
            )
            .await
            .unwrap();
        assert!(
            own["cookies"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cookie| cookie["name"] == "audit_private")
        );
        assert!(
            !other["cookies"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cookie| cookie["name"] == "audit_private")
        );
        let download_target = root
            .command(
                "Target.createTarget",
                serde_json::json!({"url":"about:blank","browserContextId":context_a}),
            )
            .await
            .unwrap();
        super::downloads::real_browser_regressions(
            &endpoint,
            download_target["targetId"].as_str().unwrap(),
            Some(context_a.clone()),
        )
        .await;
        root.command(
            "Target.disposeBrowserContext",
            serde_json::json!({"browserContextId":context_a}),
        )
        .await
        .unwrap();
        root.command(
            "Target.disposeBrowserContext",
            serde_json::json!({"browserContextId":context_b}),
        )
        .await
        .unwrap();
        eprintln!(
            "AUDIT_BROWSER_CONTEXTS: separate caller cookies and context-scoped downloads passed"
        );
        // Never leak the managed browser from tests: shut it down explicitly
        // (Rust statics are not dropped at process exit).
        tokio::task::spawn_blocking(agena_web::shutdown_local_browser)
            .await
            .expect("browser shutdown task")
            .expect("shut down managed browser");
    }

    #[test]
    fn browser_redirect_resolution_stays_http_and_supports_relative_locations() {
        let base = url::Url::parse("https://example.test/docs/start").expect("base URL");
        assert_eq!(
            resolve_browser_redirect(&base, "../next")
                .expect("relative redirect")
                .as_str(),
            "https://example.test/next"
        );
        let error = resolve_browser_redirect(&base, "file:///private/data")
            .expect_err("file redirect must be rejected");
        assert!(error.diagnostic_message().contains("unsupported scheme"));
        assert!(error.to_string().contains("unsupported scheme"));
    }
}

#[cfg(test)]
mod audit_search_tests {
    #[test]
    fn all_sources_failing_is_not_a_successful_empty_search() {
        assert!(super::ensure_search_available(0, &["engine unavailable".into()]).is_err());
        assert!(super::ensure_search_available(1, &["secondary unavailable".into()]).is_ok());
        assert!(super::ensure_search_available(1, &[]).is_ok());
    }
    #[test]
    fn references_require_a_snapshot_and_plain_selectors_do_not() {
        assert!(super::browser_element_expression(None, Some(1), None).is_err());
        assert!(super::browser_element_expression(Some("#search"), None, None).is_ok());
        assert!(
            super::browser_element_expression(None, Some(1), Some("id"))
                .unwrap()
                .contains("isConnected")
        );
    }
}

#[cfg(test)]
mod audit_log_budget_tests {
    #[test]
    fn browser_log_budget_applies_to_bytes_and_preserves_utf8() {
        let mut log = super::BrowserSessionLog::new();
        log.append("console", format!("a{}", "中😀".repeat(10000)));
        let result = log.read("fixture", 0, None);
        assert!(result.lines[0].text.len() <= 16 * 1024);
        assert!(result.lines[0].text.ends_with(" [truncated]"));
    }
}

#[cfg(test)]
mod audit_model_output_tests {
    #[test]
    fn actionable_snapshot_identity_and_refs_reach_text_only_clients() {
        let snapshot = serde_json::json!({"snapshot_id":"revision-42","url":"https://example.invalid","title":"fixture","ready_state":"complete","text":"visible","elements":[{"ref":3,"tag":"button","text":"Continue"}]});
        let text = super::format_browser_snapshot(&snapshot);
        assert!(text.contains("revision-42"));
        assert!(text.contains("ref 3"));
        assert!(text.contains("Continue"));
        let output = super::browser_action_output(
            "click",
            "session",
            serde_json::json!({"snapshot":snapshot}),
        );
        assert!(output.output_text.contains("revision-42"));
        assert!(output.output_text.contains("ref 3"));
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    #[tokio::test]
    async fn foreign_page_access_and_shutdown_do_not_touch_browser_process() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = WebPlugin::new();
        let a = ToolInvokeContext {
            tool_name: "browser_snapshot",
            session_id: 41,
            call_id: 1,
            workspace_root: dir.path().to_str().unwrap(),
        };
        let b = ToolInvokeContext {
            session_id: 42,
            ..a
        };
        plugin.browser_state.meta.lock().await.insert(
            "page".into(),
            BrowserSessionMeta {
                owner: browser_owner(&a).unwrap(),
                browser_context_id: None,
                title: "private".into(),
                url: "https://example.invalid/private".into(),
                started_at_ms: 1,
            },
        );
        assert!(plugin.require_browser_owner(&a, "page").await.is_ok());
        assert!(
            plugin
                .browser_snapshot(
                    &b,
                    &BrowserSessionInput {
                        session_id: "page".into()
                    }
                )
                .await
                .is_err()
        );
        assert!(
            plugin
                .browser_close(
                    &b,
                    &BrowserSessionInput {
                        session_id: "page".into()
                    }
                )
                .await
                .is_err()
        );
        let output = plugin
            .browser_shutdown(&b, &BrowserListInput {})
            .await
            .unwrap()
            .payload
            .unwrap();
        assert_eq!(output["process_shutdown"], false);
        assert!(output["closed_sessions"].as_array().unwrap().is_empty());
        assert!(plugin.browser_state.meta.lock().await.contains_key("page"));
    }
}
