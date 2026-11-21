use std::time::Duration;

#[derive(Debug, Clone)]
/// Options for browser rendering.
pub struct BrowserRenderOptions {
    pub wait_for_network_idle: bool,
    pub wait_for_selector: Option<String>,
    pub wait_timeout: Duration,
    pub delay: Option<Duration>,
}

impl Default for BrowserRenderOptions {
    fn default() -> Self {
        Self {
            wait_for_network_idle: true,
            wait_for_selector: None,
            wait_timeout: Duration::from_secs(10),
            delay: None,
        }
    }
}

#[derive(Debug, Clone)]
/// Options for HTTP and browser-backed page fetches.
pub struct WebFetchOptions {
    pub extractor: crate::ExtractionBackend,
    pub max_body_bytes: usize,
    pub timeout: Duration,
    pub user_agent: String,
    pub respect_robots_txt: bool,
    pub browser: BrowserRenderOptions,
}

impl Default for WebFetchOptions {
    fn default() -> Self {
        Self {
            extractor: crate::ExtractionBackend::default(),
            max_body_bytes: crate::DEFAULT_MAX_BODY_BYTES,
            timeout: Duration::from_secs(crate::DEFAULT_FETCH_TIMEOUT_SECS),
            user_agent: format!("agena-web/{}", env!("CARGO_PKG_VERSION")),
            respect_robots_txt: true,
            browser: BrowserRenderOptions::default(),
        }
    }
}
