//! Bounded content reads, with HTTP-first rendering and immutable continuation.
use super::*;

pub(super) async fn select_fetch<F, Fut>(
    render_js: Option<bool>,
    browser_enabled: bool,
    timeout: Duration,
    fetch: F,
) -> SdkResult<FetchedPage>
where
    F: Fn(bool) -> Fut,
    Fut: Future<Output = SdkResult<FetchedPage>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let mut page = tokio::time::timeout_at(deadline, fetch(render_js == Some(true)))
        .await
        .map_err(|_| {
            PluginError::timeout_with_public_detail(
                "web page overall deadline elapsed",
                "Page fetch exceeded its overall time budget, including admission and rendering.",
            )
        })??;
    if render_js.is_none()
        && browser_enabled
        && !page.rendered
        && (200..300).contains(&page.status)
        && page.content_status == agena_web::PageContentStatus::RequiresJavascript
    {
        match tokio::time::timeout_at(deadline, fetch(true)).await {
            Ok(Ok(mut rendered)) => {
                rendered.warnings.push("HTTP returned a JavaScript shell; retried once in an isolated browser context.".into());
                return Ok(rendered);
            }
            Ok(Err(error)) => {
                tracing::warn!(diagnostic = %error.diagnostic_message(), "automatic rendered read failed");
                page.warnings.push("Automatic browser retry failed; the returned content is the original HTTP shell.".into());
            }
            Err(_) => page.warnings.push("Automatic browser retry exhausted the shared fetch time budget; the returned content is the original HTTP shell.".into()),
        }
    }
    Ok(page)
}

pub(super) async fn batch<F, Fut, T>(urls: Vec<String>, concurrency: usize, fetch: F) -> Vec<T>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = T>,
{
    // No detached tasks: cancellation drops every future, and task-local host
    // bindings remain the invoking tool's bindings. buffered preserves order.
    futures_util::stream::iter(urls)
        .map(fetch)
        .buffered(concurrency)
        .collect()
        .await
}

pub(super) fn unique_urls(urls: &[String]) -> SdkResult<Vec<String>> {
    if !(1..=8).contains(&urls.len()) {
        return Err(PluginError::invalid_params(
            "urls must contain between 1 and 8 entries",
        ));
    }
    let mut seen = BTreeSet::new();
    Ok(urls
        .iter()
        .map(|raw| {
            prepare_fetch_url(raw.trim())
                .map(|url| url.to_string())
                .unwrap_or_else(|_| raw.trim().to_owned())
        })
        .filter(|url| seen.insert(url.clone()))
        .collect())
}

pub(super) fn bounded(
    value: Option<u32>,
    default: u32,
    maximum: u32,
    label: &str,
) -> SdkResult<usize> {
    let value = value.unwrap_or(default);
    if !(1..=maximum).contains(&value) {
        return Err(PluginError::invalid_params(format!(
            "{label} must be between 1 and {maximum}"
        )));
    }
    Ok(value as usize)
}

impl WebPlugin {
    pub(super) async fn fetch_selected(
        &self,
        url: &url::Url,
        use_cache: bool,
        render_js: Option<bool>,
        extractor: Option<agena_web::ExtractionBackend>,
    ) -> SdkResult<FetchedPage> {
        let config = self.config()?;
        select_fetch(
            render_js,
            config.browser.enabled,
            Duration::from_secs(config.fetch.request.timeout_secs),
            |render| {
                self.fetch_page_with_extractor(
                    url,
                    use_cache,
                    render,
                    extractor.unwrap_or(config.fetch.extractor),
                )
            },
        )
        .await
    }

    pub(super) async fn page_output(
        &self,
        page: FetchedPage,
        focus: Option<&str>,
        max_chars: usize,
    ) -> SdkResult<ToolInvokeOutput> {
        let id = self
            .state()?
            .snapshots
            .insert(page.clone())
            .await
            .map_err(crawl_error_to_plugin)?;
        self.slice_output(&id, &page, 0, max_chars, focus)
    }

    pub(super) fn slice_output(
        &self,
        id: &str,
        page: &FetchedPage,
        offset: usize,
        max_chars: usize,
        focus: Option<&str>,
    ) -> SdkResult<ToolInvokeOutput> {
        let slice = agena_web::page_slice(&page.markdown, offset, max_chars)
            .map_err(|error| PluginError::invalid_params(error.to_string()))?;
        let mut returned = page.clone();
        returned.markdown = slice.markdown.clone();
        returned.links.truncate(32);
        for link in &mut returned.links {
            *link = preview_text(link, 2048);
        }
        let mut payload =
            serde_json::to_value(&returned).map_err(|error| PluginError::internal_error(&error))?;
        payload["page_id"] = serde_json::json!(id);
        payload["offset"] = serde_json::json!(slice.offset);
        payload["returned_chars"] = serde_json::json!(slice.returned_chars);
        payload["total_chars"] = serde_json::json!(slice.total_chars);
        payload["next_offset"] = serde_json::json!(slice.next_offset);
        payload["available_markdown_bytes"] = serde_json::json!(page.markdown.len());
        payload["available_link_count"] = serde_json::json!(page.links.len());
        payload["output_truncated"] = serde_json::json!(
            offset > 0 || slice.next_offset.is_some() || returned.links != page.links
        );
        let mut text = format_fetched_page(&returned, None);
        if let Some(focus) = focus {
            let excerpts = focused_page_excerpts(&page.markdown, focus);
            payload["focus_excerpts"] = serde_json::json!(excerpts);
            if !excerpts.is_empty() {
                text.push_str(&format!(
                    "\n\nFocused excerpts from the full snapshot:\n{}",
                    excerpts.join("\n\n")
                ));
            }
        }
        let continuation = slice.next_offset.map_or_else(
            || "End of available extracted text.".into(),
            |next| format!("Continue with web.read page_id={id}, offset={next}."),
        );
        text = format!(
            "Page ID: {id}\nCharacters: {}–{} of {} (Unicode character offsets). {continuation}\nSnapshot: 15-minute TTL, subject to memory eviction. Source truncated: {}.\n\n{text}",
            offset,
            offset + slice.returned_chars,
            slice.total_chars,
            page.truncated
        );
        Ok(ToolInvokeOutput::from_parts(
            format!("web read {}", page.url),
            format!(
                "{} · HTTP {} · {:?}",
                page.title, page.status, page.content_status
            ),
            text,
            Some(payload),
            BTreeMap::new(),
            Vec::new(),
        ))
    }

    pub(super) async fn authorize_page(&self, page: &FetchedPage) -> SdkResult<()> {
        if page.final_url.is_empty() {
            return Err(PluginError::invalid_params(
                "This legacy document has no transport provenance; fetch it again.",
            ));
        }
        for url in [&page.url, &page.final_url] {
            self.state()?
                .host
                .require_network_permission(url.clone())
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
