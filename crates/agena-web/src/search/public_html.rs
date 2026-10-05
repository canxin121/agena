//! Direct, keyless requests to the engines' public search websites.

use std::collections::HashSet;

use scraper::ElementRef;
use url::Url;

use super::*;

pub(super) fn page_url(
    engine: WebSearchEngine,
    query: &str,
    page: usize,
) -> Result<Url, CrawlError> {
    let mut url = Url::parse(engine.permission_url())?;
    let mut params = url.query_pairs_mut();
    match engine {
        WebSearchEngine::Bing => {
            params
                .append_pair("q", query)
                .append_pair("first", &(1 + page * 10).to_string());
        }
        WebSearchEngine::DuckDuckGo => {
            if page > 0 {
                return Err(CrawlError::InvalidInput(
                    "DuckDuckGo pagination requires the preceding page's Next form".into(),
                ));
            }
            params.append_pair("q", query);
        }
        WebSearchEngine::Baidu => {
            params
                .append_pair("wd", query)
                .append_pair("ie", "utf-8")
                .append_pair("pn", &(page * 10).to_string());
        }
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

pub(super) fn parse_results(
    engine: WebSearchEngine,
    doc: &Html,
    limit: usize,
) -> Vec<WebSearchResult> {
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
        _ => return Vec::new(),
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
    results
}

#[cfg(test)]
fn parse_page(
    engine: WebSearchEngine,
    html: &str,
    final_url: &Url,
    _limit: usize,
) -> Result<Vec<WebSearchResult>, CrawlError> {
    status::parse_page(
        engine,
        &WebSearchPage {
            final_url: final_url.clone(),
            status: 200,
            html: html.into(),
            retry_after_secs: None,
        },
    )
}

#[cfg(test)]
mod tests;
