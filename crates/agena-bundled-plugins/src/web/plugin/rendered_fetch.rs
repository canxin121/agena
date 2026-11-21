//! Short-lived rendered fetches share native CDP policy and context ownership.
//! Chromium still resolves hostnames itself; this is not DNS connection pinning
//! or a sandbox for arbitrary browser networking (e.g. WebRTC/WebSockets).
use super::*;
use crate::browser::driver::{BrowserDriver, BrowserRequestPolicy, DomFormat, PageEvents};

static RENDERS: Semaphore = Semaphore::const_new(2);

// Chrome resolves destinations itself and cannot apply the invocation's
// MXC policy to the actual connection.
pub(super) fn runtime_available() -> bool {
    !agena_process::sandbox::enabled()
        || agena_plugin_host::sdk::host_api::current_host_callback_context()
            .is_some_and(|context| context.sandbox_enabled == Some(false))
}

pub(super) fn require_runtime() -> SdkResult<()> {
    if runtime_available() {
        return Ok(());
    }
    Err(PluginError::from_kind(
        agena_plugin_host::sdk::PluginErrorKind::HostUnavailable,
        "JavaScript rendering in agena.web requires direct execution mode. Use render_js: false for HTTP fetch or browser.open for an interactive page.",
    ))
}

pub(super) async fn fetch(
    plugin: &WebPlugin,
    url: &url::Url,
    options: &WebFetchOptions,
) -> SdkResult<FetchedPage> {
    agena_plugin_host::sdk::permission_wait::timeout(options.timeout, async {
        let source = fetch_source(plugin, url, options, None).await?;
        let mut page = fetch_transport::extract(source, url.clone(), options.extractor).await?;
        page.rendered = true;
        Ok(page)
    })
    .await
    .map_err(|_| PluginError::internal("rendered fetch deadline exceeded, including extraction"))?
}

pub(super) async fn search_source(
    plugin: &WebPlugin,
    url: &url::Url,
    remaining: Duration,
) -> SdkResult<agena_web::WebSearchPage> {
    require_runtime()?;
    let mut options = plugin.web_fetch_options()?;
    options.timeout = remaining;
    options.max_body_bytes = 4 * 1024 * 1024;
    // Search adapters use public SERPs, not the crawl/robots policy. Every
    // document and subresource still goes through native permission checks.
    options.respect_robots_txt = false;
    // Keep Chrome's own user agent when it actually executes JavaScript.
    options.user_agent.clear();
    options.browser.wait_timeout = options.browser.wait_timeout.min(remaining);
    options.browser.wait_for_selector = None;
    options.browser.delay = Some(Duration::from_millis(500));
    let source = fetch_source(plugin, url, &options, Some(url.origin())).await?;
    if source.truncated {
        return Err(PluginError::internal(
            "rendered search page exceeded its body budget",
        ));
    }
    Ok(agena_web::WebSearchPage {
        final_url: source.final_url,
        status: source.status,
        html: source.body,
        retry_after_secs: None,
    })
}

async fn fetch_source(
    plugin: &WebPlugin,
    url: &url::Url,
    options: &WebFetchOptions,
    search_origin: Option<url::Origin>,
) -> SdkResult<fetch_transport::Source> {
    require_runtime()?;
    let operation = async {
        let _slot = RENDERS
            .acquire()
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        plugin.validate_network_target(url).await?;
        let _lease = plugin.renderer.lease()?;
        let host = plugin.state()?.host.clone();
        let network = plugin.config()?.network.clone();
        let pacing = plugin.state()?.fetch_coordinator.clone();
        let config = options.clone();
        let transport = plugin.state()?.http_transport.clone();
        let robots = Arc::new(Mutex::new(BTreeMap::new()));
        let policy: BrowserRequestPolicy = Arc::new(move |raw, document| {
            let host = host.clone();
            let network = network.clone();
            let pacing = pacing.clone();
            let options = config.clone();
            let robots = robots.clone();
            let transport = transport.clone();
            let search_origin = search_origin.clone();
            Box::pin(async move {
                async {
                    if agena_plugin_host::sdk::host_api::current_host_callback_context().is_none() {
                        return Err(PluginError::internal(
                            "rendered network authorization requires an active tool call",
                        ));
                    }
                    let target = prepare_fetch_url(&raw).map_err(crawl_error_to_plugin)?;
                    if document
                        && search_origin
                            .as_ref()
                            .is_some_and(|origin| *origin != target.origin())
                    {
                        return Err(PluginError::invalid_params(
                            "rendered search redirect left the selected engine origin",
                        ));
                    }
                    network_policy::require_browser_target(
                        &target,
                        &network.fake_ip_ranges,
                        host.as_ref(),
                    )
                    .await?;
                    if document && options.respect_robots_txt {
                        let origin = target.origin().ascii_serialization();
                        let mut policies = robots.lock().await;
                        if let std::collections::btree_map::Entry::Vacant(entry) =
                            policies.entry(origin.clone())
                        {
                            let rules = transport
                                .robots(&target, &options, &pacing, &|target| {
                                    let host = host.clone();
                                    let network = network.clone();
                                    async move {
                                        host.require_pinned_http_permission(target.to_string())
                                            .await?;
                                        resolve_public_http_route(&target, &network, host.as_ref())
                                            .await
                                    }
                                })
                                .await?;
                            entry.insert(rules);
                        }
                        if let Some(Some(rules)) = policies.get(&origin)
                            && !agena_web::robots_allows(
                                rules,
                                &options.user_agent,
                                target.as_str(),
                            )
                        {
                            return Err(PluginError::invalid_params(
                                "robots.txt disallows this rendered document",
                            ));
                        }
                    }
                    pacing.wait_for_url_host(&target).await;
                    Ok::<(), PluginError>(())
                }
                .await
            })
        });
        let endpoint = plugin
            .renderer
            .endpoint(plugin.config()?.browser.chrome_options())
            .await?;
        render(&endpoint, url, options, policy).await
    };
    agena_plugin_host::sdk::permission_wait::timeout(options.timeout, operation)
        .await
        .map_err(|_| {
            PluginError::internal(
                "rendered fetch deadline exceeded, including admission, launch and requests",
            )
        })?
}

async fn render(
    endpoint: &str,
    url: &url::Url,
    options: &WebFetchOptions,
    policy: BrowserRequestPolicy,
) -> SdkResult<fetch_transport::Source> {
    // This root socket owns only this fetch's disposable context. Its last
    // sender disappearing closes the socket even if the caller is cancelled
    // during context/target creation. No interactive context is shared.
    let root = BrowserDriver::connect_root(endpoint, None).await?;
    let context_id = root.create_context().await?;
    let operation = async {
        let id = root.create_blank_target(&context_id).await?;
        let (events, mut rx) = mpsc::channel(1024);
        let page = BrowserDriver::connect_page(endpoint, &id, PageEvents::Network(events)).await?;
        let _authorization = page.authorize(policy.clone())?;
        if !options.user_agent.is_empty() {
            page.command("Network.setUserAgentOverride", serde_json::json!({"userAgent":options.user_agent})).await?;
        }
        let navigation = page.command("Page.navigate", serde_json::json!({"url":url.as_str()})).await?;
        if navigation.get("errorText").is_some() {
            return Err(PluginError::internal("rendered page navigation failed"));
        }
        let frame = navigation["frameId"].as_str().unwrap_or_default();
        let mut outstanding = BTreeSet::new();
        let mut last_network = tokio::time::Instant::now();
        let mut transferred = 0u64;
        let mut status = None;
        let mut redirects = 0usize;
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        let selector = serde_json::to_string(&options.browser.wait_for_selector).map_err(|error| PluginError::internal_error(&error))?;
        let ready_expression = format!("document.readyState === 'complete' && ({selector} === null || document.querySelector({selector}) !== null)");
        let readiness = async {
            loop {
                tokio::select! {
                    biased;
                    event = rx.recv() => {
                        let event = event.ok_or_else(|| PluginError::internal("render event stream closed"))?;
                        let request = event.params["requestId"].as_str().unwrap_or_default();
                        match event.method.as_str() {
                            "Network.requestWillBeSent" => {
                                outstanding.insert(request.to_owned());
                                last_network = tokio::time::Instant::now();
                                if event.params["type"] == "Document" && event.params.get("redirectResponse").is_some() {
                                    redirects += 1;
                                    if redirects > 10 { return Err(PluginError::internal("rendered redirect budget exceeded")); }
                                }
                            }
                            "Network.loadingFinished" | "Network.loadingFailed" => {
                                outstanding.remove(request);
                                last_network = tokio::time::Instant::now();
                            }
                            "Network.dataReceived" => {
                                transferred = transferred.saturating_add(event.params["dataLength"].as_u64().unwrap_or_default());
                                if transferred > options.max_body_bytes.saturating_mul(4) as u64 {
                                    return Err(PluginError::internal("rendered resource data budget exceeded"));
                                }
                                last_network = tokio::time::Instant::now();
                            }
                            "Network.responseReceived" if event.params["type"] == "Document" && event.params["frameId"] == frame => {
                                status = event.params["response"]["status"].as_f64().map(|code| code as u16);
                            }
                            _ => {}
                        }
                    }
                    _ = tick.tick() => {
                        if page.evaluate(&ready_expression).await?.as_bool() == Some(true)
                            && status.is_some()
                            && (!options.browser.wait_for_network_idle || (outstanding.is_empty() && last_network.elapsed() >= Duration::from_millis(500)))
                        { break; }
                    }
                }
            }
            Ok::<(), PluginError>(())
        };
        agena_plugin_host::sdk::permission_wait::timeout(options.browser.wait_timeout, readiness).await
            .map_err(|_| PluginError::internal("rendered page readiness timed out"))??;
        if let Some(delay) = options.browser.delay { tokio::time::sleep(delay).await; }
        let (captured_url, document) = page.capture_dom(
            DomFormat::Html, None, None, options.max_body_bytes, options.browser.wait_timeout,
        ).await?;
        let final_url = prepare_fetch_url(&captured_url).map_err(crawl_error_to_plugin)?;
        // Script navigation can race capture; revalidate the returned location.
        policy(final_url.to_string(), true).await?;
        Ok(fetch_transport::Source {
            final_url,
            status: status.ok_or_else(|| PluginError::internal("rendered document HTTP status missing"))?,
            content_type: "text/html; charset=utf-8".into(),
            body: document.text,
            truncated: document.truncated,
            etag: None,
            last_modified: None,
            decoding_errors: false,
        })
    }.await;
    let cleanup = root.dispose_context(&context_id).await;
    match (operation, cleanup) {
        (Ok(source), Ok(_)) => Ok(source),
        (Err(error), Ok(_)) => Err(error),
        (result, Err(error)) => Err(PluginError::internal(format!(
            "{}; render context cleanup failed: {}",
            result
                .err()
                .map(|e| e.diagnostic_message().to_owned())
                .unwrap_or_else(|| "page retrieved".into()),
            error.diagnostic_message()
        ))),
    }
}
