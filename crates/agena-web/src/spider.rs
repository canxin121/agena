use std::time::Duration;

#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
use spider::features::chrome_common::{WaitForDelay, WaitForIdleNetwork, WaitForSelector};
use spider::website::Website;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinSet;
use url::Url;

pub use crate::browser::LocalBrowserOptions;
use crate::browser::local_browser_endpoint;
use crate::extract::{extract_page_from_body, looks_like_html, truncate_utf8};
use crate::{CrawlError, FetchedPage, canonicalize_url};

#[derive(Debug, Clone)]
/// Options for browser rendering.
pub struct BrowserRenderOptions {
    pub enabled: bool,
    pub local_browser: LocalBrowserOptions,
    pub wait_for_network_idle: bool,
    pub wait_for_selector: Option<String>,
    pub wait_timeout: Duration,
    pub delay: Option<Duration>,
}

impl Default for BrowserRenderOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            local_browser: LocalBrowserOptions::default(),
            wait_for_network_idle: true,
            wait_for_selector: None,
            wait_timeout: Duration::from_secs(10),
            delay: None,
        }
    }
}

#[derive(Debug, Clone)]
/// Options for spider-based fetching.
pub struct SpiderFetchOptions {
    pub extractor: crate::ExtractionBackend,
    pub max_body_bytes: usize,
    pub timeout: Duration,
    pub delay_ms: u64,
    pub user_agent: String,
    pub respect_robots_txt: bool,
    pub browser: BrowserRenderOptions,
}

impl Default for SpiderFetchOptions {
    fn default() -> Self {
        Self {
            extractor: crate::ExtractionBackend::default(),
            max_body_bytes: crate::DEFAULT_MAX_BODY_BYTES,
            timeout: Duration::from_secs(crate::DEFAULT_FETCH_TIMEOUT_SECS),
            delay_ms: 0,
            user_agent: format!("agena-web/{}", env!("CARGO_PKG_VERSION")),
            respect_robots_txt: true,
            browser: BrowserRenderOptions::default(),
        }
    }
}

pub async fn fetch_page_with_spider(
    url: &Url,
    options: &SpiderFetchOptions,
) -> Result<FetchedPage, CrawlError> {
    let _lease = options
        .browser
        .enabled
        .then(crate::local_browser_lease)
        .transpose()?;
    tracing::debug!(
        target: "agena::web",
        url = %url,
        rendered = options.browser.enabled,
        "fetching page via spider"
    );

    let mut website = Website::new(url.as_str());
    website
        .with_limit(1)
        .with_depth(0)
        .with_delay(options.delay_ms)
        .with_request_timeout(Some(options.timeout))
        .with_respect_robots_txt(options.respect_robots_txt)
        .with_user_agent(Some(options.user_agent.as_str()))
        .with_max_page_bytes(Some(options.max_body_bytes.saturating_add(1) as f64))
        .with_max_bytes_allowed(Some(options.max_body_bytes.saturating_mul(4) as u64))
        .with_redirect_limit(10)
        .with_ignore_sitemap(true);
    let browser_connection = browser_connection(&options.browser).await?;
    configure_browser(
        &mut website,
        &options.browser,
        browser_connection.as_deref(),
    );
    let mut website = website
        .build()
        .map_err(|_| CrawlError::InvalidInput(format!("invalid crawl url '{}'", url)))?;

    let mut rx = website.subscribe(8);
    let mut collectors = JoinSet::new();
    collectors.spawn(async move {
        let mut first = None;
        loop {
            match rx.recv().await {
                Ok(page) if first.is_none() => first = Some(page),
                Ok(_) => {}
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        target: "agena::web",
                        skipped,
                        "spider page receiver lagged while fetching a single page"
                    );
                }
            }
        }
        first
    });

    let crawl = async {
        if options.browser.enabled {
            website.crawl().await;
        } else {
            website.crawl_raw().await;
        }
    };
    let result = tokio::time::timeout(options.timeout, crawl).await;
    website.unsubscribe();
    if result.is_err() {
        collectors.shutdown().await;
        return Err(CrawlError::InvalidInput(
            "Spider fetch deadline exceeded".into(),
        ));
    }
    let page = collectors
        .join_next()
        .await
        .ok_or_else(|| CrawlError::NotFound("Spider page collector".into()))?
        .map_err(|error| CrawlError::InvalidInput(format!("Spider collector failed: {error}")))?
        .ok_or_else(|| CrawlError::NotFound(format!("crawl page '{url}'")))?;
    let requested_url = url.clone();
    let options = options.clone();
    let backend = options.extractor;
    let permit = PAGE_EXTRACTORS.acquire().await.map_err(|error| {
        CrawlError::InvalidInput(format!("page extraction admission failed: {error}"))
    })?;
    let (page, body) = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        page_from_spider_page(&requested_url, page, &options)
    })
    .await
    .map_err(|error| CrawlError::InvalidInput(format!("page extraction failed: {error}")))??;
    crate::extract_with_backend(page, &body, backend).await
}

static PAGE_EXTRACTORS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

fn page_from_spider_page(
    requested_url: &Url,
    page: spider::page::Page,
    options: &SpiderFetchOptions,
) -> Result<(FetchedPage, String), CrawlError> {
    let final_url = canonicalize_url(page.get_url_final())?;
    let bytes = page.get_html_bytes_u8();
    let truncated = page.content_truncated || bytes.len() > options.max_body_bytes;
    let html = String::from_utf8_lossy(&bytes[..bytes.len().min(options.max_body_bytes)]);
    let (body, unicode_clipped) = truncate_utf8(&html, options.max_body_bytes);
    let truncated = truncated || unicode_clipped;
    let content_type = if looks_like_html(body.as_str()) {
        "text/html"
    } else {
        "text/plain"
    };
    let extracted = extract_page_from_body(
        requested_url,
        &final_url,
        content_type,
        page.status_code.as_u16(),
        truncated,
        options.browser.enabled,
        body.as_str(),
        None,
        None,
    );
    Ok((extracted, body))
}

#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
async fn browser_connection(options: &BrowserRenderOptions) -> Result<Option<String>, CrawlError> {
    if !options.enabled {
        return Ok(None);
    }
    let local_browser = options.local_browser.clone();
    let endpoint = tokio::task::spawn_blocking(move || local_browser_endpoint(&local_browser))
        .await
        .map_err(|error| {
            CrawlError::InvalidInput(format!("browser launcher worker failed: {error}"))
        })??;
    Ok(Some(endpoint))
}

#[cfg(not(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    any(target_arch = "aarch64", target_arch = "x86_64")
)))]
async fn browser_connection(options: &BrowserRenderOptions) -> Result<Option<String>, CrawlError> {
    if options.enabled {
        return Err(CrawlError::InvalidInput(format!(
            "browser rendering is unsupported on target {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )));
    }
    Ok(None)
}

#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
fn configure_browser(
    website: &mut Website,
    options: &BrowserRenderOptions,
    connection_url: Option<&str>,
) {
    if !options.enabled {
        return;
    }
    // The endpoint is launched on a blocking worker before this synchronous
    // website builder is configured.
    website.with_chrome_connection(connection_url.map(str::to_owned));
    if options.wait_for_network_idle {
        website
            .with_wait_for_idle_network0(Some(WaitForIdleNetwork::new(Some(options.wait_timeout))));
    }
    if let Some(selector) = &options.wait_for_selector {
        website.with_wait_for_selector(Some(WaitForSelector::new(
            Some(options.wait_timeout),
            selector.clone(),
        )));
    }
    if let Some(delay) = options.delay {
        website.with_wait_for_delay(Some(WaitForDelay::new(Some(delay))));
    }
}

#[cfg(not(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    any(target_arch = "aarch64", target_arch = "x86_64")
)))]
fn configure_browser(
    _website: &mut Website,
    options: &BrowserRenderOptions,
    connection_url: Option<&str>,
) {
    debug_assert!(!options.enabled);
    debug_assert!(connection_url.is_none());
}
