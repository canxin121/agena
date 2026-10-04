//! Direct, keyless requests to the engines' public search websites.

use std::collections::HashSet;

use scraper::ElementRef;
use url::Url;

use super::*;

fn failure(engine: WebSearchEngine, message: &str) -> CrawlError {
    CrawlError::SearchProvider {
        provider: engine.label(),
        message: message.into(),
    }
}

pub(super) async fn search(
    query: &str,
    limit: usize,
    options: &WebSearchOptions,
) -> Result<Vec<WebSearchResult>, CrawlError> {
    let engine = options.engine;
    let origin = Url::parse(engine.permission_url())?;
    let host = origin.host_str().unwrap_or_default().to_owned();
    // Keep session cookies only for this call. Redirects may not escape the
    // exact search host whose network permission the caller checked.
    let client = reqwest::Client::builder()
        .timeout(options.timeout)
        .user_agent(&options.user_agent)
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("search redirect limit exceeded")
            } else if attempt.url().scheme() == "https"
                && attempt.url().host_str() == Some(host.as_str())
                && attempt.url().port_or_known_default() == Some(443)
                && attempt.url().username().is_empty()
                && attempt.url().password().is_none()
            {
                attempt.follow()
            } else {
                attempt.error("search redirect left the permitted search origin")
            }
        }))
        .build()?;
    // Use a predictable interface language without changing the query's
    // language. The existing Chinese engines retain their request settings.
    let mut headers = browser_headers(&options.user_agent);
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    collect_search_pages(limit, options.max_pages, |page| {
        let client = &client;
        let headers = headers.clone();
        async move {
            let url = page_url(engine, query, page)?;
            let response = client.get(url).headers(headers).send().await?;
            let final_url = response.url().clone();
            let html = response_text_bounded(response).await?;
            parse_page(engine, &html, &final_url, MAX_SEARCH_RESULTS)
        }
    })
    .await
}

fn page_url(engine: WebSearchEngine, query: &str, page: usize) -> Result<Url, CrawlError> {
    let mut url = Url::parse(engine.permission_url())?;
    let mut params = url.query_pairs_mut();
    match engine {
        WebSearchEngine::Yandex => {
            params
                .append_pair("text", query)
                .append_pair("p", &page.to_string());
        }
        WebSearchEngine::Google => {
            params
                .append_pair("q", query)
                .append_pair("num", "10")
                .append_pair("hl", "en")
                // Stay on the authorized global origin instead of a regional
                // redirect. This does not bypass verification or JS rendering.
                .append_pair("gws_rd", "cr")
                .append_pair("start", &(page * 10).to_string());
        }
        WebSearchEngine::Yahoo => {
            params
                .append_pair("p", query)
                .append_pair("b", &(page * 10 + 1).to_string());
        }
        WebSearchEngine::Brave => {
            params
                .append_pair("q", query)
                .append_pair("source", "web")
                .append_pair("offset", &page.to_string());
        }
        WebSearchEngine::Naver => {
            params
                .append_pair("query", query)
                .append_pair("where", "web")
                .append_pair("start", &(page * 15 + 1).to_string());
        }
        _ => return Err(failure(engine, "unsupported public HTML adapter")),
    }
    drop(params);
    Ok(url)
}

fn is_ad(element: ElementRef<'_>) -> bool {
    std::iter::once(element)
        .chain(element.ancestors().filter_map(ElementRef::wrap))
        .any(|el| {
            let value = el.value();
            value.attr("data-text-ad").is_some()
                || value.attr("data-ad-client").is_some()
                || matches!(value.attr("data-type"), Some("ad" | "ads" | "sponsored"))
                || matches!(value.id(), Some("tads" | "bottomads"))
                || value.classes().any(|class| {
                    matches!(
                        class,
                        "ad" | "ads"
                            | "ad-result"
                            | "serp-item_type_ad"
                            | "uEierd"
                            | "commercial-unit-desktop-top"
                    )
                })
        })
}

fn text_of(element: ElementRef<'_>) -> String {
    normalize_whitespace(&element.text().collect::<Vec<_>>().join(" "))
}

fn link_for_title(title: ElementRef<'_>) -> Option<ElementRef<'_>> {
    if title.value().name() == "a" {
        return Some(title);
    }
    title.select(&selector("a[href]")).next().or_else(|| {
        title
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|el| el.value().name() == "a")
    })
}

fn result_url(engine: WebSearchEngine, raw: &str) -> Option<String> {
    let base = Url::parse(engine.permission_url()).ok()?;
    let url = base.join(raw.trim()).ok()?;
    let host = url.host_str()?;
    let target = match engine {
        WebSearchEngine::Google if matches!(host, "www.google.com" | "google.com") => {
            if url.path() != "/url" {
                return None;
            }
            url.query_pairs()
                .find(|(key, _)| matches!(key.as_ref(), "q" | "url"))?
                .1
                .into_owned()
        }
        WebSearchEngine::Yahoo if host == "r.search.yahoo.com" => {
            let encoded = url.path().split_once("/RU=")?.1;
            let encoded = encoded.split("/RK=").next()?.split("/RS=").next()?;
            urlencoding::decode(encoded).ok()?.into_owned()
        }
        WebSearchEngine::Yandex if matches!(host, "yandex.com" | "yandex.ru") => {
            if !url.path().starts_with("/clck/") {
                return None;
            }
            url.query_pairs()
                .find(|(key, _)| key == "url")?
                .1
                .into_owned()
        }
        WebSearchEngine::Naver if host == "search.naver.com" => {
            if !url.path().starts_with("/p/crd/") {
                return None;
            }
            url.query_pairs()
                .find(|(key, _)| key == "u")?
                .1
                .into_owned()
        }
        _ if Some(host) == base.host_str() || host == "keep.naver.com" => return None,
        _ => url.to_string(),
    };
    // Decode wrapper parameters once, preserving escapes within the target.
    normalize_result_url(&target)
}

fn parse_page(
    engine: WebSearchEngine,
    html: &str,
    final_url: &Url,
    limit: usize,
) -> Result<Vec<WebSearchResult>, CrawlError> {
    let doc = Html::parse_document(html);
    let (cards, titles, snippets) = match engine {
        WebSearchEngine::Yandex => (
            ".serp-item",
            "a.OrganicTitle-Link, .OrganicTitle a, a.b-serp-item__title-link",
            ".OrganicTextContentSpan, .OrganicText, .TextContainer, .text-container, .b-serp-item__text",
        ),
        WebSearchEngine::Google => (
            ".MjjYud, .g, .Gx5Zad",
            "h3, .vvjwJb",
            ".VwiC3b, .IsZvec, .aCOpRe, .BNeawe.s3v9rd",
        ),
        WebSearchEngine::Yahoo => (".algo-sr, #web .algo", "h3", ".compText"),
        WebSearchEngine::Brave => (
            ".snippet[data-type=web]",
            ".search-snippet-title, .title",
            ".generic-snippet .content, .snippet-description, .snippet-content",
        ),
        WebSearchEngine::Naver => (
            ".fds-web-normal-doc-root, .web-doc",
            ".sds-comps-text-type-headline1, .link_tit",
            ".sds-comps-text-type-body1, .link_desc",
        ),
        _ => return Err(failure(engine, "unsupported public HTML parser")),
    };
    let title_selector = selector(titles);
    let snippet_selector = selector(snippets);
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    for card in doc.select(&selector(cards)).filter(|card| !is_ad(*card)) {
        let Some(title_el) = card.select(&title_selector).next() else {
            continue;
        };
        let Some(link) = link_for_title(title_el) else {
            continue;
        };
        if is_ad(link) {
            continue;
        }
        let Some(url) = link
            .value()
            .attr("href")
            .and_then(|raw| result_url(engine, raw))
        else {
            continue;
        };
        let title = text_of(title_el);
        if title.is_empty() || !seen.insert(url.clone()) {
            continue;
        }
        let description = card
            .select(&snippet_selector)
            .next()
            .map(text_of)
            .unwrap_or_default();
        let source = Url::parse(&url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default();
        results.push(WebSearchResult {
            title,
            url,
            description,
            source,
            engine: engine.to_string(),
        });
        if results.len() >= limit {
            break;
        }
    }
    if !results.is_empty() {
        return Ok(results);
    }

    // Inspect actual page structure / navigation targets, not arbitrary script
    // strings (normal Brave scripts also contain translated CAPTCHA messages).
    let path = final_url.path();
    if path.starts_with("/showcaptcha") || path.starts_with("/sorry") || path.contains("/captcha")
        || doc.select(&selector(".CheckboxCaptcha, #captcha-form, form[action*='captcha'], .g-recaptcha, .anomaly-modal")).next().is_some()
    {
        return Err(failure(engine, "captcha_required: search website returned a verification page"));
    }
    if final_url
        .host_str()
        .is_some_and(|host| host.starts_with("consent."))
        || doc
            .select(&selector(
                "form[action*='consent.google'], form[action*='consent.yahoo']",
            ))
            .next()
            .is_some()
    {
        return Err(failure(
            engine,
            "consent_required: search website returned a consent page",
        ));
    }
    if doc
        .select(&selector("a[href*='/httpservice/retry/enablejs']"))
        .next()
        .is_some()
        // html5ever treats noscript contents as raw text with scripting
        // enabled, so the real Google interstitial has no selectable anchor.
        || doc.select(&selector("noscript")).any(|el| {
            el.text().any(|text| text.contains("/httpservice/retry/enablejs"))
        })
    {
        return Err(failure(
            engine,
            "javascript_required: search website requires browser rendering",
        ));
    }
    let empty_selector = selector(
        ".no-results, .no-results-message, .NoResults, .serp-error, #topstuff, .api_noresult_wrap, .not_found",
    );
    if doc.select(&empty_selector).any(|el| {
        let text = text_of(el).to_lowercase();
        [
            "no results",
            "did not match any documents",
            "nothing found",
            "ничего не нашлось",
            "没有找到",
            "未找到",
            "검색결과가 없습니다",
            "검색 결과가 없습니다",
        ]
        .iter()
        .any(|message| text.contains(message))
    }) {
        return Ok(Vec::new());
    }
    Err(failure(
        engine,
        "unrecognized_results_page: no usable result cards or explicit empty state; website markup may have changed",
    ))
}

#[cfg(test)]
mod tests;
