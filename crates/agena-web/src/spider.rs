use std::time::Duration;

use spider::features::chrome_common::{WaitForDelay, WaitForIdleNetwork, WaitForSelector};
use spider::website::Website;
use tokio::sync::broadcast::error::RecvError;
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
        .with_user_agent(Some(options.user_agent.as_str()));
    let browser_connection = if options.browser.enabled {
        let local_browser = options.browser.local_browser.clone();
        Some(
            tokio::task::spawn_blocking(move || local_browser_endpoint(&local_browser))
                .await
                .map_err(|error| {
                    CrawlError::InvalidInput(format!("browser launcher worker failed: {error}"))
                })??,
        )
    } else {
        None
    };
    configure_browser(
        &mut website,
        &options.browser,
        browser_connection.as_deref(),
    );
    let mut website = website
        .build()
        .map_err(|_| CrawlError::InvalidInput(format!("invalid crawl url '{}'", url)))?;

    let mut rx = website.subscribe(8);
    let collector = tokio::spawn(async move {
        let mut pages = Vec::new();
        loop {
            match rx.recv().await {
                Ok(page) => pages.push(page),
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
        pages
    });

    website.crawl().await;
    website.unsubscribe();
    let pages = collector
        .await
        .map_err(|err| CrawlError::InvalidInput(err.to_string()))?;
    let page = pages
        .into_iter()
        .next()
        .ok_or_else(|| CrawlError::NotFound(format!("crawl page '{}'", url)))?;
    page_from_spider_page(url, page, options)
}

fn page_from_spider_page(
    requested_url: &Url,
    page: spider::page::Page,
    options: &SpiderFetchOptions,
) -> Result<FetchedPage, CrawlError> {
    let final_url = canonicalize_url(page.get_url_final())?;
    let html = page.get_html();
    let (body, truncated) = truncate_utf8(html.as_str(), options.max_body_bytes);
    let content_type = if looks_like_html(body.as_str()) {
        "text/html"
    } else {
        "text/plain"
    };
    Ok(extract_page_from_body(
        requested_url,
        &final_url,
        content_type,
        page.status_code.as_u16(),
        truncated,
        options.browser.enabled,
        body.as_str(),
        None,
        None,
    ))
}

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
