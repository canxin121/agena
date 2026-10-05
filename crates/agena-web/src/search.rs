//! Embedded ferris-style web search functions and result types.
//!
//! The upstream `ferris-search` project is an MCP server rather than a
//! crates.io library, so Agena keeps the no-API-key engine functions in-process
//! here instead of spawning or connecting to a separate search service.

use std::time::Duration;
use std::{collections::HashSet, future::Future};
use std::{fmt, str::FromStr};

use base64::{Engine as _, engine::general_purpose};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use crate::{CrawlError, canonicalize_url};

mod coordinator;
mod public_html;
mod status;
mod transport;

pub use coordinator::{WebSearchCoordinator, WebSearchResponse};
pub use status::{SearchIssue, SearchIssueKind, WebSearchPage};

const DEFAULT_SEARCH_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_SEARCH_RESULTS: usize = 50;
const MAX_SEARCH_PAGES: usize = 5;
const MAX_SEARCH_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const BING_BASE: &str = "https://www.bing.com/search";
const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";
const BAIDU_BASE: &str = "https://www.baidu.com/s";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Search engine used for web search.
pub enum WebSearchEngine {
    #[default]
    Bing,
    DuckDuckGo,
    Baidu,
    Yandex,
    Google,
    Yahoo,
    Brave,
    Naver,
}

impl AsRef<str> for WebSearchEngine {
    fn as_ref(&self) -> &str {
        self.label()
    }
}

impl fmt::Display for WebSearchEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl WebSearchEngine {
    /// Stable aggregation order, independent of network completion timing.
    pub const ALL: [Self; 8] = [
        Self::DuckDuckGo,
        Self::Bing,
        Self::Baidu,
        Self::Yandex,
        Self::Google,
        Self::Yahoo,
        Self::Brave,
        Self::Naver,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Bing => "bing",
            Self::DuckDuckGo => "duckduckgo",
            Self::Baidu => "baidu",
            Self::Yandex => "yandex",
            Self::Google => "google",
            Self::Yahoo => "yahoo",
            Self::Brave => "brave",
            Self::Naver => "naver",
        }
    }

    pub fn permission_url(self) -> &'static str {
        match self {
            Self::Bing => BING_BASE,
            Self::DuckDuckGo => DDG_HTML_URL,
            Self::Baidu => BAIDU_BASE,
            Self::Yandex => "https://yandex.com/search/",
            Self::Google => "https://www.google.com/search",
            Self::Yahoo => "https://search.yahoo.com/search",
            Self::Brave => "https://search.brave.com/search",
            Self::Naver => "https://search.naver.com/search.naver",
        }
    }
}

impl FromStr for WebSearchEngine {
    type Err = CrawlError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        normalize_web_search_engine(value).ok_or_else(|| {
            CrawlError::InvalidInput(format!(
                "unsupported search engine '{value}', expected one of: bing, duckduckgo, baidu, yandex, google, yahoo, brave, naver"
            ))
        })
    }
}

#[derive(Debug, Clone)]
/// Options of a web search.
pub struct WebSearchOptions {
    pub engine: WebSearchEngine,
    /// Maximum distinct candidates from this engine, before caller filtering (1–50).
    pub limit: usize,
    /// Maximum result pages to fetch, including the first page (1–5).
    /// This is an independent ceiling, not a page number or an output offset.
    pub max_pages: usize,
    pub timeout: Duration,
    pub user_agent: String,
}

impl Default for WebSearchOptions {
    fn default() -> Self {
        Self {
            engine: WebSearchEngine::default(),
            limit: 10,
            max_pages: 1,
            timeout: DEFAULT_SEARCH_TIMEOUT,
            user_agent: format!("agena-web/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Result of a web search.
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub source: String,
    pub engine: String,
}

pub fn normalize_web_search_engine(value: &str) -> Option<WebSearchEngine> {
    match value.trim().to_ascii_lowercase().as_str() {
        "bing" | "microsoft bing" => Some(WebSearchEngine::Bing),
        "duckduckgo" | "duck duck go" | "ddg" => Some(WebSearchEngine::DuckDuckGo),
        "baidu" | "百度" => Some(WebSearchEngine::Baidu),
        "yandex" => Some(WebSearchEngine::Yandex),
        "google" => Some(WebSearchEngine::Google),
        "yahoo" => Some(WebSearchEngine::Yahoo),
        "brave" => Some(WebSearchEngine::Brave),
        "naver" => Some(WebSearchEngine::Naver),
        _ => None,
    }
}

pub async fn search_web(
    query: &str,
    options: &WebSearchOptions,
) -> Result<Vec<WebSearchResult>, CrawlError> {
    coordinator::search(query, options)
        .await
        .map(|response| response.results)
}

pub fn results_to_text(results: &[WebSearchResult]) -> String {
    results
        .iter()
        .enumerate()
        .map(|(idx, result)| {
            format!(
                "{}. {}\nURL: {}\nSource: {}\nDescription: {}",
                idx + 1,
                result.title,
                result.url,
                result.source,
                result.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn parse_bing_results(html: &str, limit: usize) -> Vec<WebSearchResult> {
    let doc = Html::parse_document(html);
    let sel_algo = selector("#b_results li.b_algo");
    let sel_h2a = selector("h2 a");
    let sel_cap = selector(".b_caption p, .b_dList li, p");
    let sel_src = selector(".b_attribution cite, cite");
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();

    for el in doc.select(&sel_algo) {
        if results.len() >= limit {
            break;
        }
        let Some(link) = el.select(&sel_h2a).next() else {
            continue;
        };
        let Some(url) = link.value().attr("href").and_then(normalize_bing_url) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = normalize_whitespace(&link.text().collect::<String>());
        let description = el
            .select(&sel_cap)
            .next()
            .map(|item| normalize_whitespace(&item.text().collect::<String>()))
            .unwrap_or_default();
        let source = el
            .select(&sel_src)
            .next()
            .map(|item| normalize_whitespace(&item.text().collect::<String>()))
            .unwrap_or_default();
        if title.is_empty() && description.is_empty() {
            continue;
        }
        results.push(WebSearchResult {
            title,
            url,
            description,
            source,
            engine: WebSearchEngine::Bing.to_string(),
        });
    }

    results
}

fn parse_duckduckgo_results(html: &str, limit: usize) -> Vec<WebSearchResult> {
    let doc = Html::parse_document(html);
    let sel_result = selector(".result:not(.result--ad)");
    let sel_title = selector(".result__a");
    let sel_snip = selector(".result__snippet");
    let sel_url = selector(".result__url");
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();

    for el in doc.select(&sel_result) {
        if results.len() >= limit {
            break;
        }
        let Some(link) = el.select(&sel_title).next() else {
            continue;
        };
        let raw_href = link.value().attr("href").unwrap_or_default();
        let Some(url) = normalize_duckduckgo_url(raw_href) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = normalize_whitespace(&link.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let description = el
            .select(&sel_snip)
            .next()
            .map(|item| normalize_whitespace(&item.text().collect::<String>()))
            .unwrap_or_default();
        let source = el
            .select(&sel_url)
            .next()
            .map(|item| normalize_whitespace(&item.text().collect::<String>()))
            .unwrap_or_default();
        results.push(WebSearchResult {
            title,
            url,
            description,
            source,
            engine: WebSearchEngine::DuckDuckGo.to_string(),
        });
    }

    results
}

fn parse_baidu_results(html: &str, limit: usize) -> Vec<WebSearchResult> {
    let doc = Html::parse_document(html);
    let sel_item = selector("div.c-container, div[tpl]");
    let sel_title = selector("h3.t a, h3 a");
    let sel_desc = selector(".c-font-normal.c-color-text, .c-abstract");
    let sel_src = selector(".cosc-source, .c-showurl");
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();

    for el in doc.select(&sel_item) {
        if results.len() >= limit {
            break;
        }
        let Some(link) = el.select(&sel_title).next() else {
            continue;
        };
        let Some(url) = link.value().attr("href").and_then(normalize_result_url) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = normalize_whitespace(&link.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let description = el
            .select(&sel_desc)
            .next()
            .map(|item| normalize_whitespace(&item.text().collect::<String>()))
            .unwrap_or_default();
        let source = el
            .select(&sel_src)
            .next()
            .map(|item| normalize_whitespace(&item.text().collect::<String>()))
            .unwrap_or_default();
        results.push(WebSearchResult {
            title,
            url,
            description,
            source,
            engine: WebSearchEngine::Baidu.to_string(),
        });
    }

    results
}

fn normalize_duckduckgo_url(raw_href: &str) -> Option<String> {
    let raw = raw_href.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(encoded) = raw
        .split("uddg=")
        .nth(1)
        .and_then(|tail| tail.split('&').next())
    {
        return urlencoding::decode(encoded)
            .ok()
            .and_then(|decoded| normalize_result_url(decoded.as_ref()));
    }
    normalize_result_url(raw)
}

fn normalize_bing_url(raw_href: &str) -> Option<String> {
    let raw = raw_href.trim();
    if raw.is_empty() {
        return None;
    }

    let parsed = url::Url::parse(raw).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host == "bing.com" || host.ends_with(".bing.com") {
        if let Some((_, target)) = parsed.query_pairs().find(|(name, _)| name == "u")
            && let Some(decoded) = decode_bing_target(target.as_ref())
        {
            return normalize_result_url(decoded.as_str());
        }
        return None;
    }

    normalize_result_url(raw)
}

fn decode_bing_target(value: &str) -> Option<String> {
    // Bing's result links encode the destination in the `u` parameter. Current
    // SERPs prefix a URL-safe base64 payload with `a1` (for example,
    // `u=a1aHR0cHM6Ly9leGFtcGxlLmNvbQ`). Returning the wrapper itself is not
    // useful: it is an internal Bing URL and is filtered from results.
    let encoded = value.strip_prefix("a1").unwrap_or(value);
    [
        &general_purpose::URL_SAFE_NO_PAD,
        &general_purpose::URL_SAFE,
        &general_purpose::STANDARD_NO_PAD,
        &general_purpose::STANDARD,
    ]
    .into_iter()
    .find_map(|engine| {
        engine
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    })
    .filter(|target| target.starts_with("http://") || target.starts_with("https://"))
}

fn normalize_result_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    canonicalize_url(raw)
        .ok()
        .map(|url| url.to_string())
        .filter(|url| url.len() <= 8192)
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static CSS selector parses")
}

#[cfg(test)]
mod tests {
    use super::{parse_bing_results, parse_duckduckgo_results};

    #[test]
    fn parse_bing_results_decodes_redirect_urls() {
        let results = parse_bing_results(
            r#"
                <ol id="b_results">
                  <li class="b_algo">
                    <h2><a href="https://www.bing.com/ck/a?u=a1aHR0cHM6Ly9vcGVuYWkuY29tL2luZGV4L2dwdC01LTYv">GPT-5.6</a></h2>
                    <div class="b_caption"><p>OpenAI's next-generation model.</p></div>
                    <cite>https://openai.com</cite>
                  </li>
                </ol>
            "#,
            8,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://openai.com/index/gpt-5-6/");
        assert_eq!(results[0].title, "GPT-5.6");
    }

    #[test]
    fn parse_duckduckgo_results_keeps_direct_result_urls() {
        let results = parse_duckduckgo_results(
            r#"
                <div class="result results_links results_links_deep web-result">
                  <a class="result__a" href="https://openai.com/index/gpt-5-6/">GPT-5.6</a>
                  <a class="result__snippet">OpenAI's next-generation model.</a>
                  <a class="result__url">openai.com</a>
                </div>
            "#,
            8,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://openai.com/index/gpt-5-6/");
    }
}
