use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_web::{
    BrowserRenderOptions, CrawlPageFetcher, CrawlRunOptions, CrawlRunReport, CrawlStore,
    CrawlStoreRetention, FetchedPage, LocalBrowserOptions, SpiderFetchOptions, WebFetchCoordinator,
    WebFetchCoordinatorConfig, WebSearchEngine, WebSearchOptions, WebSearchResult, crawl_site,
    fetch_page_with_spider, local_browser_endpoint, local_browser_running, local_browser_touch,
    prepare_fetch_url, preview_text, results_to_text, search_web, shutdown_local_browser,
};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};

use agena_domain::{
    BackgroundActivity, BackgroundActivityKind, BackgroundActivityLogLine,
    BackgroundActivityLogRead, BackgroundActivityStatus,
};
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::attachment::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    ActivitySourceAdapter, Result as SdkResult, ToolInvokeOutput, async_trait,
};

fn json_schema_for_default_with_metadata<T>(
    default: T,
    metadata: &[(&str, &str, &str)],
) -> serde_json::Value
where
    T: schemars::JsonSchema + serde::Serialize,
{
    let mut schema = agena_plugin_sdk::macro_support::json_schema_for_default(default);
    for (pointer, title, description) in metadata {
        agena_plugin_sdk::macro_support::set_schema_metadata(
            &mut schema,
            pointer,
            Some(title),
            Some(description),
        );
    }
    schema
}

pub(crate) const WEB_PLUGIN_ID: &str = "agena.web";

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
/// Crawl limits of the web plugin.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    pub wait: WebBrowserWaitConfig,
    /// Seconds of inactivity after which the managed browser process is shut
    /// down automatically. `0` disables idle auto-close.
    #[serde(default = "default_web_browser_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
}

impl Default for WebBrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            executable_path: None,
            wait: WebBrowserWaitConfig::default(),
            idle_timeout_secs: default_web_browser_idle_timeout_secs(),
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

fn web_config_schema() -> serde_json::Value {
    json_schema_for_default_with_metadata(
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
                "/properties/browser/properties/idle_timeout_secs",
                "Idle Timeout (sec)",
                "Seconds of inactivity before the managed browser process is closed automatically. 0 disables auto-close.",
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

pub(crate) struct WebPlugin {
    state: OnceLock<WebPluginState>,
    workspace_root: OnceLock<PathBuf>,
    crawl_lock: Mutex<()>,
    browser_download_lock: Mutex<()>,
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
        let line = BackgroundActivityLogLine {
            seq: self.next_seq,
            stream: stream.to_string(),
            ts_ms: chrono::Utc::now().timestamp_millis(),
            text: text.into(),
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
}

impl BrowserActivityState {
    fn new() -> Self {
        Self {
            clients: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            connecting: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            meta: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            logs: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            root: Arc::new(tokio::sync::Mutex::new(None)),
        }
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
        let closed = match root {
            Some(root) => root
                .command(
                    "Target.closeTarget",
                    serde_json::json!({ "targetId": target_id }),
                )
                .await
                .map(|value| {
                    value
                        .get("success")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                })
                .unwrap_or(false),
            None => false,
        };
        self.clients.lock().await.remove(target_id);
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
        let _ = host.publish_activity(activity).await;
        self.logs.lock().await.remove(target_id);
        Ok(closed)
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
        let _ = self
            .state
            .close_session(
                &target_id,
                "Stopped from the activities panel.",
                self.host.as_ref(),
            )
            .await?;
        Ok(())
    }
}

struct WebPluginState {
    config: WebConfig,
    fetch_coordinator: WebFetchCoordinator,
    host: Arc<dyn HostClient>,
}

impl WebPluginState {
    fn new(config: WebConfig, host: Arc<dyn HostClient>) -> Self {
        Self {
            fetch_coordinator: WebFetchCoordinator::new(WebFetchCoordinatorConfig {
                cache_ttl: Duration::from_secs(config.fetch.cache.ttl_secs),
                cache_capacity: config.fetch.cache.capacity,
                per_host_delay: Duration::from_millis(config.fetch.request.delay_ms),
            }),
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
        created_at_ms: started_at_ms.unwrap_or(now),
        started_at_ms: started_at_ms.unwrap_or(now),
        finished_at_ms,
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
    render_js: Option<bool>,
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
    render_js: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, agena_plugin_sdk::ToolInput)]
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
    selector: Option<String>,
    /// Snapshot-local index returned by `browser_snapshot.elements[].ref`.
    /// It is valid only while the page DOM has not materially changed.
    #[serde(
        default,
        rename = "ref",
        alias = "element_ref",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(range(min = 0, max = 199))]
    element_ref: Option<u16>,
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
    selector: Option<String>,
    #[serde(
        default,
        rename = "ref",
        alias = "element_ref",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(range(min = 0, max = 199))]
    element_ref: Option<u16>,
    text: String,
    #[serde(default)]
    press_enter: bool,
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
    selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "web",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Local web search/fetch/crawl plugin with an embedded crawl cache, deduplication, and optional browser rendering.",
    config_schema = web_config_schema(),
)]
impl WebPlugin {
    pub(crate) fn new() -> Self {
        Self {
            state: OnceLock::new(),
            workspace_root: OnceLock::new(),
            crawl_lock: Mutex::new(()),
            browser_download_lock: Mutex::new(()),
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
                parse_web_config(ctx.config)?,
                host.clone(),
            ))
            .map_err(|_| PluginError::internal("web plugin initialized more than once"))?;
        self.workspace_root.set(ctx.workspace_root).map_err(|_| {
            PluginError::internal("web plugin workspace root initialized more than once")
        })?;
        // First-class activity source: the host dispatches browser log reads
        // and stop requests back to this plugin, so the activities panel can
        // tail and stop sessions like any other background work. Registration
        // is best-effort: older hosts simply keep the empty/not-stoppable
        // fallbacks for the Browser kind.
        let source = BrowserActivitySource {
            state: Arc::clone(&self.browser_state),
            host: host.clone(),
        };
        let _ = host
            .register_activity_source(BackgroundActivityKind::Browser, Arc::new(source))
            .await;
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
            .map_err(|_| PluginError::internal("browser worker pool is unavailable"))?;
        tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            shutdown_local_browser()
        })
        .await
        .map_err(|error| PluginError::internal(format!("browser shutdown task failed: {error}")))?
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

    /// Publish a browser session as a unified background activity so the TUI
    /// `/activities` panel and web `/activities` page can list and follow it.
    /// Records go straight into the host's activity registry through the
    /// first-class `HostClient::publish_activity` capability.
    async fn publish_browser_activity(&self, activity: BackgroundActivity) -> SdkResult<()> {
        let host = self.state()?.host.clone();
        host.publish_activity(activity)
            .await
            .map_err(|error| PluginError::internal(format!("publish browser activity: {error}")))
    }

    async fn publish_browser_running(&self, session_id: &str, meta: &BrowserSessionMeta) {
        let _ = self
            .publish_browser_activity(browser_activity(
                format!("browser_{session_id}"),
                meta.title.clone(),
                meta.url.clone(),
                BackgroundActivityStatus::Running,
                Some(meta.started_at_ms),
                None,
                None,
            ))
            .await;
    }

    async fn publish_browser_terminal(
        &self,
        session_id: &str,
        meta: Option<&BrowserSessionMeta>,
        finished_at_ms: i64,
        message: String,
    ) {
        let now = chrono::Utc::now().timestamp_millis();
        let _ = self
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
            .await;
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
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()
            .map_err(|error| {
                PluginError::internal(format!("browser redirect preflight setup failed: {error}"))
            })?;
        let mut current = initial.clone();
        let mut checked = Vec::new();
        for _ in 0..=MAX_REDIRECTS {
            self.validate_network_target(&current).await?;
            checked.push(current.to_string());
            let response = client.head(current.clone()).send().await.map_err(|error| {
                PluginError::internal(format!(
                    "browser redirect preflight failed for {current}: {error}"
                ))
            })?;
            if !response.status().is_redirection() {
                return Ok(checked);
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    PluginError::internal(format!(
                        "browser redirect from {current} had no valid Location header"
                    ))
                })?;
            current = resolve_browser_redirect(&current, location)?;
        }
        Err(PluginError::internal(format!(
            "browser redirect preflight exceeded {MAX_REDIRECTS} hops"
        )))
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

    async fn browser_endpoint(&self) -> SdkResult<String> {
        let options = self.local_browser_options()?;
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|_| PluginError::internal("browser worker pool is unavailable"))?;
        tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            local_browser_endpoint(&options)
        })
        .await
        .map_err(|error| PluginError::internal(format!("browser launcher failed: {error}")))?
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
            let endpoint = self.browser_endpoint().await?;
            return CdpClient::connect(endpoint.as_str(), None, events).await;
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
        let client = CdpClient::connect(endpoint.as_str(), Some(target_id), events).await?;
        client.enable_navigation_interception().await?;
        self.browser_state
            .clients
            .lock()
            .await
            .insert(target_id.to_string(), client.clone());
        Ok(client)
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
                        let _ = host
                            .publish_activity(browser_activity(
                                format!("browser_{session_id}"),
                                title,
                                url_text,
                                BackgroundActivityStatus::Running,
                                Some(started_at_ms),
                                None,
                                None,
                            ))
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
        let expression = r#"(() => {
            const clean = value => String(value || '').replace(/\s+/g, ' ').trim();
            const elements = Array.from(document.querySelectorAll('a,button,input,textarea,select,[role],[contenteditable="true"]'))
                .slice(0, 200)
                .map((el, index) => ({
                    ref: index,
                    tag: el.tagName.toLowerCase(),
                    role: el.getAttribute('role'),
                    type: el.getAttribute('type'),
                    name: el.getAttribute('name'),
                    id: el.id || null,
                    text: clean(el.innerText || el.value || el.getAttribute('aria-label') || el.getAttribute('title')).slice(0, 300),
                    disabled: Boolean(el.disabled),
                }));
            return {
                url: location.href,
                title: document.title,
                ready_state: document.readyState,
                text: clean(document.body ? document.body.innerText : '').slice(0, 50000),
                elements,
            };
        })()"#;
        client.evaluate(expression).await
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

    async fn fetch_page(
        &self,
        url: &url::Url,
        use_cache: bool,
        render_js: bool,
    ) -> SdkResult<FetchedPage> {
        let state = self.state()?;
        if !state.config.fetch.enabled {
            return Err(PluginError::internal(
                "web fetching is disabled by plugin config `fetch.enabled`",
            ));
        }
        state
            .fetch_coordinator
            .fetch_or_cached(url, render_js, use_cache, || async {
                self.validate_network_target(url).await?;
                let options = self.spider_fetch_options(render_js)?;
                let page = fetch_page_with_spider(url, &options)
                    .await
                    .map_err(crawl_error_to_plugin)?;
                let final_url = url::Url::parse(page.canonical_url.as_str()).map_err(|error| {
                    PluginError::internal(format!("invalid final fetch URL: {error}"))
                })?;
                // Spider follows HTTP redirects internally. Do not return a response
                // whose final destination would fail the same network policy.
                self.validate_network_target(&final_url).await?;
                Ok(page)
            })
            .await
    }

    #[tool(
        summary = "Fetch one web page and inspect its actual content.",
        help = "Use this tool after search when you need evidence from the actual page rather than search snippets. If you already know what facts you need, set `prompt` so Agena prioritizes the most relevant excerpts from the page in the returned text output.",
        read_only,


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
            serde_json::to_value(&page).map_err(|err| PluginError::internal(err.to_string()))?;
        let text = format_fetched_page(&page, input.prompt.as_deref());
        Ok(ToolInvokeOutput::from_parts(
            format!("web fetch {}", url),
            format!(
                "{} · HTTP {}{}",
                page.title,
                page.status,
                if page.truncated { " · truncated" } else { "" }
            ),
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Crawl a site and cache indexed pages locally.",
        mutating,


        discovery,
        path(write = self.store_write_permission_path()?),
        network(connect = prepare_fetch_url(input.start_url.as_str()).map_err(crawl_error_to_plugin)?.to_string())
    )]
    async fn invoke_crawl(&self, input: &CrawlRunInput) -> SdkResult<ToolInvokeOutput> {
        let start_url =
            prepare_fetch_url(input.start_url.as_str()).map_err(crawl_error_to_plugin)?;
        let store = self.store()?;
        let _guard = self.crawl_lock.lock().await;
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
        let summary = format!(
            "{} indexed · {} cached · {} failures",
            report.stored_count, report.cached_count, report.failure_count
        );
        let payload =
            serde_json::to_value(report).map_err(|err| PluginError::internal(err.to_string()))?;
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
        help = "Use this tool to discover candidate pages, not to answer from result snippets alone. After searching, fetch 1-3 relevant result URLs before answering when the user needs facts, summaries, comparisons, or latest information. Use allowed_domains and blocked_domains to steer source quality.",
        read_only,


        discovery,
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
        let summary = format!("{} results · {}", output.results.len(), output.engine);
        let payload =
            serde_json::to_value(output).map_err(|err| PluginError::internal(err.to_string()))?;
        Ok(ToolInvokeOutput::from_parts(
            format!("web search {query}"),
            summary,
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "browser_open",
        summary = "Open a page in a managed interactive browser session.",
        read_only
    )]
    async fn browser_open(&self, input: &BrowserOpenInput) -> SdkResult<ToolInvokeOutput> {
        let url = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?;
        self.validate_network_target(&url).await?;
        let preflight_redirects = self.browser_preflight_redirects(&url).await?;
        let browser = self.browser_client(None).await?;
        *self.browser_state.root.lock().await = Some(browser.clone());
        let created = browser
            .command(
                "Target.createTarget",
                serde_json::json!({ "url": "about:blank" }),
            )
            .await?;
        let target_id = created
            .get("targetId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PluginError::internal("browser did not return a target id"))?
            .to_string();
        let (event_tx, event_rx) = mpsc::channel::<CdpEvent>(512);
        let page = match self
            .browser_client_with_events(Some(target_id.as_str()), Some(event_tx))
            .await
        {
            Ok(page) => page,
            Err(error) => {
                let _ = browser
                    .command(
                        "Target.closeTarget",
                        serde_json::json!({ "targetId": target_id }),
                    )
                    .await;
                return Err(error);
            }
        };
        page.command("Page.enable", serde_json::json!({})).await?;
        page.command("Runtime.enable", serde_json::json!({}))
            .await?;
        page.command("Log.enable", serde_json::json!({})).await?;
        let title_target = browser_title_target(&url);
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let meta = BrowserSessionMeta {
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
            let _ = self
                .publish_browser_activity(browser_activity(
                    format!("browser_{target_id}"),
                    meta.title.clone(),
                    meta.url.clone(),
                    BackgroundActivityStatus::Failed,
                    Some(started_at_ms),
                    Some(finished_at_ms),
                    Some(format!("Navigation to {url} failed.")),
                ))
                .await;
            let _ = browser
                .command(
                    "Target.closeTarget",
                    serde_json::json!({ "targetId": target_id }),
                )
                .await;
            return Err(error);
        }
        self.wait_for_browser_condition(target_id.as_str(), None, None, input.timeout_ms)
            .await?;
        let snapshot = self.ensure_browser_final_url(target_id.as_str()).await?;
        Ok(ToolInvokeOutput::from_parts(
            format!("Open browser · {title_target}"),
            browser_snapshot_summary(&snapshot),
            format!("Opened {} in browser session {}.", url, target_id),
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
        tags(network, query, discovery),
        name = "browser_list",
        summary = "List open page targets in the managed interactive browser.",
        read_only,
        concurrency_safe
    )]
    async fn browser_list(&self, _input: &BrowserListInput) -> SdkResult<ToolInvokeOutput> {
        // Listing must never start the managed browser. Report the current
        // process state first and only connect when it is already running.
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|_| PluginError::internal("browser worker pool is unavailable"))?;
        let running = tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            local_browser_running()
        })
        .await
        .map_err(|error| PluginError::internal(format!("browser status task failed: {error}")))?;
        if !running {
            return Ok(ToolInvokeOutput::from_parts(
                "browser list",
                "0 open pages",
                "The managed browser is not running. Use browser_open to start it.",
                Some(serde_json::json!({
                    "browser_running": false,
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
                "sessions": sessions,
            })),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(network, mutate),
        name = "browser_close",
        summary = "Close one page target in the managed interactive browser.",
        mutating
    )]
    async fn browser_close(&self, input: &BrowserSessionInput) -> SdkResult<ToolInvokeOutput> {
        let host = self.state()?.host.clone();
        let closed = self
            .browser_state
            .close_session(
                &input.session_id,
                &format!("Closed browser session {}.", input.session_id),
                host.as_ref(),
            )
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
        tags(network, mutate),
        name = "browser_shutdown",
        summary = "Shut down the managed browser process and all its sessions.",
        help = "Closes the underlying Chrome/Chromium process used for rendered fetches and interactive browsing, and removes its temporary profile. All browser sessions are discarded; the next browser_open starts a fresh browser. Use this to release memory without exiting Agena.",
        mutating
    )]
    async fn browser_shutdown(&self, _input: &BrowserListInput) -> SdkResult<ToolInvokeOutput> {
        let finished_at_ms = chrono::Utc::now().timestamp_millis();
        let sessions = std::mem::take(&mut *self.browser_state.meta.lock().await)
            .into_iter()
            .collect::<Vec<_>>();
        for (session_id, meta) in sessions {
            self.browser_state
                .append_log(&session_id, "event", "Managed browser shut down.")
                .await;
            self.publish_browser_terminal(
                &session_id,
                Some(&meta),
                finished_at_ms,
                "Managed browser shut down.".to_string(),
            )
            .await;
        }
        self.browser_state.clients.lock().await.clear();
        *self.browser_state.root.lock().await = None;
        self.browser_state.logs.lock().await.clear();
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|_| PluginError::internal("browser worker pool is unavailable"))?;
        let closed = tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            shutdown_local_browser()
        })
        .await
        .map_err(|error| PluginError::internal(format!("browser shutdown task failed: {error}")))?
        .map_err(crawl_error_to_plugin)?;
        Ok(ToolInvokeOutput::from_parts(
            "browser shutdown",
            if closed { "Closed" } else { "Not running" },
            if closed {
                "Closed the managed browser process and removed its profile."
            } else {
                "No managed browser process was running."
            },
            Some(serde_json::json!({ "closed": closed })),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(network, query),
        name = "browser_snapshot",
        summary = "Inspect visible text and interactive elements in a browser session.",
        read_only,
        concurrency_safe
    )]
    async fn browser_snapshot(&self, input: &BrowserSessionInput) -> SdkResult<ToolInvokeOutput> {
        let snapshot = self
            .browser_snapshot_value(input.session_id.as_str())
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
        summary = "Click a browser element selected by CSS or the latest snapshot ref.",
        mutating
    )]
    async fn browser_click(&self, input: &BrowserClickInput) -> SdkResult<ToolInvokeOutput> {
        let target = browser_element_expression(input.selector.as_deref(), input.element_ref)?;
        let expression = format!(
            "(() => {{ const el = {target}; if (!el) return {{ok:false,error:'browser element not found'}}; if (el.disabled) return {{ok:false,error:'element is disabled'}}; el.scrollIntoView({{block:'center'}}); el.focus(); el.click(); return {{ok:true}}; }})()"
        );
        let client = self.browser_client(Some(input.session_id.as_str())).await?;
        let result = client.evaluate(expression.as_str()).await?;
        ensure_browser_action(&result)?;
        let snapshot = self
            .ensure_browser_final_url(input.session_id.as_str())
            .await?;
        Ok(browser_action_output(
            "browser click",
            input.session_id.as_str(),
            serde_json::json!({ "action": result, "snapshot": snapshot }),
        ))
    }

    #[tool(
        tags(network, interactive, mutate),
        name = "browser_type",
        summary = "Fill a browser input selected by CSS or the latest snapshot ref, optionally pressing Enter.",
        mutating
    )]
    async fn browser_type(&self, input: &BrowserTypeInput) -> SdkResult<ToolInvokeOutput> {
        let expression = browser_type_expression(
            input.selector.as_deref(),
            input.element_ref,
            input.text.as_str(),
            input.press_enter,
        )?;
        let client = self.browser_client(Some(input.session_id.as_str())).await?;
        let result = client.evaluate(expression.as_str()).await?;
        ensure_browser_action(&result)?;
        let snapshot = self
            .ensure_browser_final_url(input.session_id.as_str())
            .await?;
        Ok(browser_action_output(
            "browser type",
            input.session_id.as_str(),
            serde_json::json!({ "action": result, "snapshot": snapshot }),
        ))
    }

    #[tool(
        tags(network, query),
        name = "browser_wait",
        summary = "Wait for page readiness, a CSS selector, or visible text.",
        read_only,
        concurrency_safe
    )]
    async fn browser_wait(&self, input: &BrowserWaitInput) -> SdkResult<ToolInvokeOutput> {
        if input.selector.is_some() && input.text.is_some() {
            return Err(PluginError::invalid_params(
                "browser_wait accepts selector or text, not both",
            ));
        }
        self.wait_for_browser_condition(
            input.session_id.as_str(),
            input.selector.as_deref(),
            input.text.as_deref(),
            input.timeout_ms,
        )
        .await?;
        let snapshot = self
            .ensure_browser_final_url(input.session_id.as_str())
            .await?;
        Ok(browser_action_output(
            "browser wait",
            input.session_id.as_str(),
            serde_json::json!({ "ok": true, "snapshot": snapshot }),
        ))
    }

    #[tool(
        tags(network, mutate, filesystem),
        name = "browser_screenshot",
        summary = "Capture a browser screenshot and return it as an image attachment.",
        mutating
    )]
    async fn browser_screenshot(
        &self,
        input: &BrowserScreenshotInput,
    ) -> SdkResult<ToolInvokeOutput> {
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
        let encoded = result
            .get("data")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PluginError::internal("browser screenshot returned no image data"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| PluginError::internal(format!("invalid screenshot data: {error}")))?;
        let relative = input
            .path
            .clone()
            .unwrap_or_else(|| format!(".agena/artifacts/browser/{}.png", input.session_id));
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
        tags(network, mutate, filesystem),
        name = "browser_download",
        summary = "Download one HTTP(S) URL through a managed browser session and return a local artifact.",
        mutating
    )]
    async fn browser_download(&self, input: &BrowserDownloadInput) -> SdkResult<ToolInvokeOutput> {
        const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;
        let url = prepare_fetch_url(input.url.as_str()).map_err(crawl_error_to_plugin)?;
        self.validate_network_target(&url).await?;
        let preflight_redirects = self.browser_preflight_redirects(&url).await?;
        let download_dir = self
            .workspace_root()?
            .join(".agena/artifacts/browser/downloads")
            .join(uuid::Uuid::new_v4().simple().to_string());
        tokio::fs::create_dir_all(&download_dir)
            .await
            .map_err(|error| {
                PluginError::internal(format!("cannot create browser download directory: {error}"))
            })?;

        // Chromium's download behavior is browser-global. Serialize this
        // short setup/navigation/polling sequence so two model calls cannot
        // redirect each other's artifacts into the wrong managed directory.
        let _guard = self.browser_download_lock.lock().await;
        let root = self.browser_client(None).await?;
        root.command(
            "Browser.setDownloadBehavior",
            serde_json::json!({
                "behavior": "allow",
                "downloadPath": download_dir,
                "eventsEnabled": true,
            }),
        )
        .await?;
        drop(root);
        let page = self.browser_client(Some(input.session_id.as_str())).await?;
        page.command("Page.enable", serde_json::json!({})).await?;
        page.command("Page.navigate", serde_json::json!({ "url": url.as_str() }))
            .await?;
        drop(page);

        let deadline = tokio::time::Instant::now() + Duration::from_millis(input.timeout_ms);
        let mut last_candidate: Option<(PathBuf, u64)> = None;
        loop {
            let mut entries = tokio::fs::read_dir(&download_dir).await.map_err(|error| {
                PluginError::internal(format!(
                    "cannot inspect browser download directory: {error}"
                ))
            })?;
            while let Some(entry) = entries.next_entry().await.map_err(|error| {
                PluginError::internal(format!("cannot inspect browser download artifact: {error}"))
            })? {
                let path = entry.path();
                let partial = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("crdownload"));
                if partial
                    || !entry
                        .file_type()
                        .await
                        .map_err(|error| PluginError::internal(error.to_string()))?
                        .is_file()
                {
                    continue;
                }
                let size = entry
                    .metadata()
                    .await
                    .map_err(|error| PluginError::internal(error.to_string()))?
                    .len();
                if size > MAX_DOWNLOAD_BYTES {
                    return Err(PluginError::internal(format!(
                        "browser download exceeds the {} MiB artifact limit",
                        MAX_DOWNLOAD_BYTES / (1024 * 1024)
                    )));
                }
                if last_candidate
                    .as_ref()
                    .is_some_and(|(previous_path, previous_size)| {
                        previous_path == &path && *previous_size == size
                    })
                {
                    let filename = path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .map(ToOwned::to_owned);
                    let kind = AttachmentKind::detect("", filename.as_deref());
                    let attachment = AttachmentItem {
                        kind,
                        mime: "application/octet-stream".to_string(),
                        source: AttachmentSource::LocalPath {
                            path: path.to_string_lossy().to_string(),
                        },
                        filename,
                        title: Some(format!("Browser download from {url}")),
                        size_bytes: Some(size),
                        sha256: None,
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: None,
                    };
                    return Ok(ToolInvokeOutput::from_parts(
                        "browser download",
                        format!("{} bytes", size),
                        format!("Saved browser download to '{}'.", path.display()),
                        Some(serde_json::json!({
                            "session_id": input.session_id,
                            "url": url,
                            "path": path,
                            "size_bytes": size,
                            "preflight_redirects": preflight_redirects,
                        })),
                        std::collections::BTreeMap::from([
                            ("agena.effect".to_string(), "file_changes".to_string()),
                            ("path".to_string(), path.to_string_lossy().to_string()),
                        ]),
                        vec![attachment],
                    ));
                }
                last_candidate = Some((path, size));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(PluginError::internal(format!(
                    "browser download timed out after {} ms",
                    input.timeout_ms
                )));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
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
            .map_err(|error| PluginError::invalid_params(error.to_string()))?;
        let text = text
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| PluginError::invalid_params(error.to_string()))?;
        loop {
            let expression = if let Some(selector) = selector.as_deref() {
                format!("Boolean(document.querySelector({selector}))")
            } else if let Some(text) = text.as_deref() {
                format!("Boolean(document.body && document.body.innerText.includes({text}))")
            } else {
                "document.readyState === 'complete' || document.readyState === 'interactive'"
                    .to_string()
            };
            if let Ok(client) = self.browser_client(Some(target_id)).await
                && client.evaluate(expression.as_str()).await?.as_bool() == Some(true)
            {
                return Ok(());
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
        engine: WebSearchEngine,
        input: &CrawlWebSearchInput,
    ) -> SdkResult<Vec<WebSearchResult>> {
        let engine_url = url::Url::parse(engine.permission_url())
            .map_err(|err| PluginError::internal(err.to_string()))?;
        let state = self.state()?;
        state.fetch_coordinator.wait_for_url_host(&engine_url).await;
        self.validate_network_target(&engine_url).await?;
        let config = &state.config;
        let options = WebSearchOptions {
            engine,
            limit,
            timeout: Duration::from_secs(config.fetch.request.timeout_secs),
            user_agent: self.user_agent.clone(),
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

async fn validate_public_network_target(url: &url::Url) -> SdkResult<()> {
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::invalid_params("web URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PluginError::invalid_params("web URL has no known port"))?;

    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| PluginError::internal(format!("failed to resolve {host}: {error}")))?
        .map(|address| address.ip())
        .collect::<BTreeSet<_>>();
    if addresses.is_empty() {
        return Err(PluginError::internal(format!(
            "DNS resolution returned no addresses for {host}"
        )));
    }
    for address in addresses {
        if !is_public_address(address) {
            return Err(PluginError::invalid_params(format!(
                "web URL host `{host}` resolves to non-public address `{address}`"
            )));
        }
    }
    Ok(())
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

#[derive(Clone)]
struct CdpClient {
    commands: mpsc::Sender<CdpCommandRequest>,
    navigation_interception_enabled: Arc<OnceLock<()>>,
    navigation_errors: Arc<std::sync::Mutex<VecDeque<String>>>,
}

impl CdpClient {
    async fn connect(
        endpoint: &str,
        target_id: Option<&str>,
        events: Option<mpsc::Sender<CdpEvent>>,
    ) -> SdkResult<Self> {
        let (mut socket, _) = tokio::time::timeout(
            CDP_CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async(endpoint),
        )
        .await
        .map_err(|_| PluginError::internal("timed out connecting to browser CDP"))?
        .map_err(|error| {
            PluginError::internal(format!("cannot connect to browser CDP: {error}"))
        })?;

        let (session_id, next_id) = if let Some(target_id) = target_id {
            let result = tokio::time::timeout(
                CDP_CONNECT_TIMEOUT,
                cdp_attach_target(&mut socket, target_id),
            )
            .await
            .map_err(|_| PluginError::internal("timed out attaching browser CDP target"))??;
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
        tokio::spawn(run_cdp_connection(
            socket,
            session_id,
            next_id,
            command_receiver,
            Arc::clone(&navigation_errors),
            events,
        ));

        Ok(Self {
            commands,
            navigation_interception_enabled,
            navigation_errors,
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
        let _ = self.navigation_interception_enabled.set(());
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
        local_browser_touch();
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
                .map_err(|_| PluginError::internal("browser CDP connection ended"))?;
            receiver
                .await
                .map_err(|_| PluginError::internal("browser CDP connection ended"))
        })
        .await
        .map_err(|_| {
            PluginError::internal(format!(
                "browser CDP command `{method}` timed out after {}s",
                command_timeout.as_secs_f64()
            ))
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
        self.navigation_errors
            .lock()
            .ok()
            .and_then(|mut errors| errors.pop_front())
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
        .map_err(|error| PluginError::internal(format!("browser CDP send failed: {error}")))?;

    while let Some(message) = socket.next().await {
        let message = message
            .map_err(|error| PluginError::internal(format!("browser CDP read failed: {error}")))?;
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
                PluginError::internal(format!("browser CDP returned non-UTF-8 data: {error}"))
            })?,
        tokio_tungstenite::tungstenite::Message::Ping(payload) => {
            socket
                .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                .await
                .map_err(|error| {
                    PluginError::internal(format!("browser CDP pong failed: {error}"))
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
        .map_err(|error| PluginError::internal(format!("invalid browser CDP response: {error}")))
}

async fn run_cdp_connection(
    socket: CdpSocket,
    session_id: Option<String>,
    mut next_id: u64,
    mut commands: mpsc::Receiver<CdpCommandRequest>,
    navigation_errors: Arc<std::sync::Mutex<VecDeque<String>>>,
    events: Option<mpsc::Sender<CdpEvent>>,
) {
    let (mut sink, mut source) = socket.split();
    // At most sixteen authorization checks can be active, so one result slot
    // per check is sufficient and gives the CDP loop deterministic memory use.
    let (decisions, mut decision_receiver) = mpsc::channel::<NavigationDecision>(16);
    let authorization_slots = Arc::new(Semaphore::new(16));
    let mut pending = BTreeMap::<u64, PendingCdpCommand>::new();

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    let _ = sink.close().await;
                    break;
                };
                let id = next_id;
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
                        let _ = command.response.send(Err(error));
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
                    let _ = events.try_send(CdpEvent {
                        method: method.to_string(),
                        params: value
                            .get("params")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    });
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
                tokio::spawn(async move {
                    let _authorization_slot = authorization_slot;
                    let result = if !is_document {
                        Ok(())
                    } else {
                        authorize_browser_document_request(url.as_str()).await
                    };
                    let _ = decisions
                        .send(NavigationDecision {
                            request_id,
                            url,
                            result,
                        })
                        .await;
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
        .map_err(|error| error.to_string())
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
            let _ = tx.send(result);
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
    for (_, command) in std::mem::take(pending) {
        if let PendingCdpCommand::Caller { response, .. } = command {
            let _ = response.send(Err(error.to_string()));
        }
    }
}

fn push_navigation_error(errors: &Arc<std::sync::Mutex<VecDeque<String>>>, error: String) {
    if let Ok(mut errors) = errors.lock() {
        if errors.len() >= 32 {
            errors.pop_front();
        }
        errors.push_back(error);
    }
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
fn browser_element_expression(
    selector: Option<&str>,
    element_ref: Option<u16>,
) -> SdkResult<String> {
    match (
        selector.map(str::trim).filter(|value| !value.is_empty()),
        element_ref,
    ) {
        (Some(selector), None) => serde_json::to_string(selector)
            .map(|selector| format!("document.querySelector({selector})"))
            .map_err(|error| PluginError::invalid_params(error.to_string())),
        (None, Some(element_ref)) => Ok(format!(
            "Array.from(document.querySelectorAll('a,button,input,textarea,select,[role],[contenteditable=\"true\"]'))[{element_ref}] || null"
        )),
        (Some(_), Some(_)) => Err(PluginError::invalid_params(
            "browser action accepts exactly one of selector or ref",
        )),
        (None, None) => Err(PluginError::invalid_params(
            "browser action requires selector or ref from browser_snapshot",
        )),
    }
}

fn browser_type_expression(
    selector: Option<&str>,
    element_ref: Option<u16>,
    text: &str,
    press_enter: bool,
) -> SdkResult<String> {
    let target = browser_element_expression(selector, element_ref)?;
    let text = serde_json::to_string(text)
        .map_err(|error| PluginError::invalid_params(error.to_string()))?;
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
            return {{ok:true,value,method}};
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
        format!("Completed {title} in browser session {session_id}."),
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

fn format_browser_snapshot(snapshot: &serde_json::Value) -> String {
    let title = snapshot
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let url = snapshot
        .get("url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let text = snapshot
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let elements = snapshot
        .get("elements")
        .and_then(serde_json::Value::as_array)
        .map(|elements| elements.len())
        .unwrap_or_default();
    format!(
        "Title: {title}\nURL: {url}\nInteractive elements: {elements}\n\n{}",
        preview_text(text, 12_000)
    )
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
    PluginError::internal(err.to_string())
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
            .map_err(|err| PluginError::internal(format!("invalid web plugin config: {err}")))?
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
                .map(|failure| format!("- {}", failure.user.fallback)),
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
    use std::{
        collections::VecDeque,
        net::{Ipv4Addr, Ipv6Addr},
        sync::{Arc, OnceLock},
    };

    use agena_domain::{BackgroundActivityKind, BackgroundActivityStatus};
    use agena_plugin_host::sdk::Plugin;

    use super::{
        BrowserSessionLog, CdpClient, WebPlugin, browser_activity, browser_element_expression,
        browser_type_expression, is_public_address, resolve_browser_redirect,
    };

    #[tokio::test]
    async fn cdp_commands_fail_instead_of_waiting_forever_for_a_response() {
        let (commands, _requests) = tokio::sync::mpsc::channel(1);
        let client = CdpClient {
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
        }
    }

    #[test]
    fn browser_type_expression_uses_native_setter_and_react_tracker_fallback() {
        let expression = browser_type_expression(Some("#search"), None, "hello", true)
            .expect("browser type expression");
        assert!(expression.contains("Object.getOwnPropertyDescriptor"));
        assert!(expression.contains("_valueTracker"));
        assert!(expression.contains("native_setter"));
        assert!(expression.contains("KeyboardEvent('keypress'"));
        assert!(expression.contains("\"#search\""));
        assert!(expression.contains("\"hello\""));
        assert!(
            browser_element_expression(None, Some(7))
                .expect("snapshot ref expression")
                .contains("[7] || null")
        );
        assert!(browser_element_expression(Some("#search"), Some(7)).is_err());
        assert!(browser_element_expression(None, None).is_err());
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
                browser_type_expression(Some("#controlled"), None, "updated", false)
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
        // Never leak the managed browser from tests: shut it down explicitly
        // (Rust statics are not dropped at process exit).
        let _ = tokio::task::spawn_blocking(agena_web::shutdown_local_browser)
            .await
            .expect("browser shutdown task");
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
