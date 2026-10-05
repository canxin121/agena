use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, HeaderValue, RETRY_AFTER};

use super::*;

#[derive(Debug, Clone)]
pub(super) struct SearchRequest {
    pub(super) url: url::Url,
    pub(super) form: Option<Vec<(String, String)>>,
}

/// DDG supplies opaque continuation fields; guessing `s = page * N` can
/// repeat or skip results. Follow only its actual same-origin Next form.
pub(super) fn duckduckgo_next(page: &WebSearchPage) -> Option<SearchRequest> {
    let doc = Html::parse_document(&page.html);
    for form in doc.select(&selector("form")) {
        let next = form
            .select(&selector("input[type=submit], button"))
            .any(|el| {
                let text = el
                    .value()
                    .attr("value")
                    .map(str::to_owned)
                    .unwrap_or_else(|| el.text().collect::<String>());
                text.to_ascii_lowercase().contains("next")
            });
        if !next {
            continue;
        }
        let mut url = page
            .final_url
            .join(form.value().attr("action").unwrap_or("/html/"))
            .ok()?;
        if url.origin() != url::Url::parse(DDG_HTML_URL).ok()?.origin()
            || !matches!(url.path(), "/html/" | "/html")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        let fields: Vec<_> = form
            .select(&selector("input[name]"))
            .filter_map(|el| {
                if matches!(el.value().attr("type"), Some("submit" | "button")) {
                    return None;
                }
                Some((
                    el.value().attr("name")?.to_owned(),
                    el.value().attr("value").unwrap_or_default().to_owned(),
                ))
            })
            .take(65)
            .collect();
        if fields.len() > 64
            || fields.iter().any(|(k, v)| k.len() > 64 || v.len() > 8192)
            || fields.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() > 16384
        {
            return None;
        }
        if form
            .value()
            .attr("method")
            .is_some_and(|method| method.eq_ignore_ascii_case("post"))
        {
            return Some(SearchRequest {
                url,
                form: Some(fields),
            });
        }
        url.set_query(None);
        url.query_pairs_mut().extend_pairs(fields);
        return Some(SearchRequest { url, form: None });
    }
    None
}

pub(super) fn client(
    engine: WebSearchEngine,
    user_agent: &str,
) -> Result<reqwest::Client, CrawlError> {
    let origin = url::Url::parse(engine.permission_url())?.origin();
    Ok(reqwest::Client::builder()
        .user_agent(user_agent)
        .cookie_store(true)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("search redirect limit exceeded");
            }
            if attempt.url().origin() == origin
                && attempt.url().username().is_empty()
                && attempt.url().password().is_none()
            {
                attempt.follow()
            } else {
                // Keep the redirect response to classify consent/verification
                // without connecting to an unauthorized different origin.
                attempt.stop()
            }
        }))
        .build()?)
}

pub(super) async fn fetch(
    client: reqwest::Client,
    engine: WebSearchEngine,
    query: String,
    page: usize,
    continuation: Option<SearchRequest>,
) -> Result<WebSearchPage, CrawlError> {
    let request = match continuation {
        Some(SearchRequest {
            url,
            form: Some(fields),
        }) => client.post(url).form(&fields),
        Some(SearchRequest { url, form: None }) => client.get(url),
        None => client.get(public_html::page_url(engine, &query, page)?),
    };
    let response = request
        .header(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        )
        .header(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("en-US,en;q=0.9,zh-CN;q=0.8"),
        )
        .send()
        .await?;
    response_page(response).await
}

async fn response_page(mut response: reqwest::Response) -> Result<WebSearchPage, CrawlError> {
    let status = response.status().as_u16();
    let mut final_url = response.url().clone();
    if response.status().is_redirection()
        && let Some(location) = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
        && let Ok(target) = final_url.join(location)
    {
        final_url = target;
    }
    let retry_after_secs = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(retry_after);
    // A challenge body may be huge or stall. The status and Retry-After are
    // enough to enter cooldown; do not lose them to a body/size/parse error.
    if status == 429 {
        return Ok(WebSearchPage {
            final_url,
            status,
            html: String::new(),
            retry_after_secs,
        });
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if response
        .content_length()
        .is_some_and(|n| n > MAX_SEARCH_RESPONSE_BYTES as u64)
    {
        return Err(CrawlError::ResponseTooLarge {
            maximum_bytes: MAX_SEARCH_RESPONSE_BYTES,
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_SEARCH_RESPONSE_BYTES {
            return Err(CrawlError::ResponseTooLarge {
                maximum_bytes: MAX_SEARCH_RESPONSE_BYTES,
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    let (html, _) = crate::decode_response_body(&bytes, &content_type);
    Ok(WebSearchPage {
        final_url,
        status,
        html,
        retry_after_secs,
    })
}

fn retry_after(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(seconds.min(86_400));
    }
    let when = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some((when.timestamp() - chrono::Utc::now().timestamp()).clamp(0, 86_400) as u64)
}

#[cfg(test)]
mod tests;
