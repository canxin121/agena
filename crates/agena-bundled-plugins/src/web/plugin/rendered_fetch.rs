//! Short-lived rendered fetches share native CDP policy and context ownership.
//! Chromium still resolves hostnames itself; this is not DNS connection pinning
//! or a sandbox for arbitrary browser networking (e.g. WebRTC/WebSockets).
use super::*;

static RENDERS: Semaphore = Semaphore::const_new(2);

pub(super) async fn fetch(
    plugin: &WebPlugin,
    url: &url::Url,
    options: &SpiderFetchOptions,
) -> SdkResult<FetchedPage> {
    let operation = async {
        let _slot = RENDERS
            .acquire()
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        plugin.validate_network_target(url).await?;
        let _lease = agena_web::local_browser_lease().map_err(crawl_error_to_plugin)?;
        let host = plugin.state()?.host.clone();
        let pacing = plugin.state()?.fetch_coordinator.clone();
        let config = options.clone();
        let robots = Arc::new(Mutex::new(BTreeMap::<String, Option<String>>::new()));
        let policy: BrowserRequestPolicy = Arc::new(move |raw, document| {
            let host = host.clone();
            let pacing = pacing.clone();
            let options = config.clone();
            let robots = robots.clone();
            Box::pin(async move {
                let checked = async {
                    if agena_plugin_host::sdk::host_api::current_host_callback_context().is_none() {
                        return Err(PluginError::internal(
                            "rendered network authorization requires an active tool call",
                        ));
                    }
                    let target = prepare_fetch_url(&raw).map_err(crawl_error_to_plugin)?;
                    host.require_network_permission(target.to_string()).await?;
                    validate_public_network_target(&target).await?;
                    if document && options.respect_robots_txt {
                        let origin = target.origin().ascii_serialization();
                        let mut policies = robots.lock().await;
                        if !policies.contains_key(&origin) {
                            let rules = fetch_transport::robots(&target, &options, &|target| {
                                let host = host.clone();
                                let pacing = pacing.clone();
                                async move {
                                    host.require_network_permission(target.to_string()).await?;
                                    pacing.wait_for_url_host(&target).await;
                                    resolve_public_network_target(&target).await
                                }
                            })
                            .await?;
                            policies.insert(origin.clone(), rules);
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
                .await;
                checked.map_err(|error| error.diagnostic_message().to_owned())
            })
        });
        let endpoint = plugin.browser_endpoint().await?;
        let source = render(&endpoint, url, options, policy).await?;
        let mut page = fetch_transport::extract(source, url.clone(), options.extractor).await?;
        page.rendered = true;
        Ok(page)
    };
    tokio::time::timeout(options.timeout, operation).await.map_err(|_| {
        PluginError::internal("rendered fetch deadline exceeded, including admission, launch, requests and extraction")
    })?
}

async fn render(
    endpoint: &str,
    url: &url::Url,
    options: &SpiderFetchOptions,
    policy: BrowserRequestPolicy,
) -> SdkResult<fetch_transport::Source> {
    // This root socket owns only this fetch's disposable context. Its last
    // sender disappearing closes the socket even if the caller is cancelled
    // during context/target creation. No interactive context is shared.
    let root = CdpClient::connect(endpoint, None, None).await?;
    let context = root
        .command(
            "Target.createBrowserContext",
            serde_json::json!({"disposeOnDetach":true}),
        )
        .await?;
    let context_id = context["browserContextId"]
        .as_str()
        .ok_or_else(|| PluginError::internal("render context identity missing"))?;
    let operation = async {
        root.command("Browser.setDownloadBehavior", serde_json::json!({"behavior":"deny","browserContextId":context_id})).await?;
        let target = root.command("Target.createTarget", serde_json::json!({"url":"about:blank","browserContextId":context_id})).await?;
        let id = target["targetId"].as_str().ok_or_else(|| PluginError::internal("render target identity missing"))?;
        let (events, mut rx) = mpsc::channel(1024);
        let page = CdpClient::connect_with_policy(endpoint, Some(id), Some(events), policy.clone()).await?;
        page.command("Page.enable", serde_json::json!({})).await?;
        page.command("Network.enable", serde_json::json!({"maxTotalBufferSize":65536,"maxResourceBufferSize":16384})).await?;
        page.command("Network.setBypassServiceWorker", serde_json::json!({"bypass":true})).await?;
        page.command("Network.setCacheDisabled", serde_json::json!({"cacheDisabled":true})).await?;
        page.command("Network.setUserAgentOverride", serde_json::json!({"userAgent":options.user_agent})).await?;
        page.command("Fetch.enable", serde_json::json!({"patterns":[{"urlPattern":"*","requestStage":"Request"}]})).await?;
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
        tokio::time::timeout(options.browser.wait_timeout, readiness).await
            .map_err(|_| PluginError::internal("rendered page readiness timed out"))??;
        if let Some(delay) = options.browser.delay { tokio::time::sleep(delay).await; }
        let limit = options.max_body_bytes;
        let captured = page.evaluate(&format!(r#"(() => {{
            const html = document.documentElement?.outerHTML || '';
            let end = Math.min(html.length, {limit} + 1);
            if (end > 0 && /[\uD800-\uDBFF]/.test(html[end - 1])) end--;
            return {{url:location.href,html:html.slice(0,end),truncated:html.length>end}};
        }})()"#)).await?;
        let final_url = prepare_fetch_url(captured["url"].as_str().unwrap_or_default()).map_err(crawl_error_to_plugin)?;
        // Script navigation can race capture; revalidate the returned location.
        policy(final_url.to_string(), true).await.map_err(PluginError::internal)?;
        let html = captured["html"].as_str().ok_or_else(|| PluginError::internal("rendered HTML missing"))?;
        let mut end = html.len().min(limit);
        while !html.is_char_boundary(end) { end -= 1; }
        Ok(fetch_transport::Source {
            final_url,
            status: status.ok_or_else(|| PluginError::internal("rendered document HTTP status missing"))?,
            content_type: "text/html; charset=utf-8".into(),
            body: html[..end].to_owned(),
            truncated: end < html.len() || captured["truncated"] == true,
            etag: None,
            last_modified: None,
            decoding_errors: false,
        })
    }.await;
    let cleanup = root
        .command(
            "Target.disposeBrowserContext",
            serde_json::json!({"browserContextId":context_id}),
        )
        .await;
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

#[cfg(test)]
mod tests;
