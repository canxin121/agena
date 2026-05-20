//! `web_search` plugin tool.
//!
//! Backend selection: configured per-runtime via `[web.search] backend = "..."`.
//! Supported backends:
//!
//! * `tavily`            — POST <https://api.tavily.com/search> (needs TAVILY_API_KEY)
//! * `exa`               — POST <https://api.exa.ai/search>     (needs EXA_API_KEY)
//! * `brave`             — GET  <https://api.search.brave.com/res/v1/web/search> (needs BRAVE_API_KEY)
//! * `duckduckgo_html`   — scrape <https://html.duckduckgo.com/html/> (no key required, default)
//!
//! The backend is picked from config, falling back to `duckduckgo_html` so
//! the tool works out of the box with zero credentials.

use serde::Deserialize;

use crate::config::WebSearchBackend;
use crate::message::WebSearchToolInput;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    WebSearchHit,
};

const DEFAULT_MAX_RESULTS: u32 = 8;
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &WebSearchToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let q = input.query.trim();
    if q.is_empty() {
        return Err(ToolError::Plugin(
            "web_search: query must not be empty".to_string(),
        ));
    }

    let backend = executor.web_search_backend();
    let max = input
        .max_results
        .unwrap_or(DEFAULT_MAX_RESULTS)
        .clamp(1, 20);
    let allow = input.allowed_domains.clone();
    let block = input.blocked_domains.clone();

    let query = q.to_string();
    let backend_name = backend.name();
    let target = crate::permission::NetworkTarget::parse(web_search_backend_target(&backend))
        .map_err(|e| ToolError::Plugin(format!("web_search: invalid network target: {e}")))?;
    executor.ensure_network_permission(&target)?;

    let raw_hits: Vec<WebSearchHit> = super::mcp::block_on(async move {
        match backend {
            WebSearchBackend::Tavily { api_key } => tavily_search(&query, max, &api_key).await,
            WebSearchBackend::Exa { api_key } => exa_search(&query, max, &api_key).await,
            WebSearchBackend::Brave { api_key } => brave_search(&query, max, &api_key).await,
            WebSearchBackend::DuckDuckGoHtml => duckduckgo_html_search(&query, max).await,
        }
    })?;

    let hits: Vec<WebSearchHit> = raw_hits
        .into_iter()
        .filter(|hit| domain_allowed(&hit.url, &allow, &block))
        .take(max as usize)
        .collect();

    let summary = if hits.is_empty() {
        format!("[{backend_name}] no results for {q:?}")
    } else {
        let mut buf = format!("[{backend_name}] {} result(s):\n", hits.len());
        for (i, h) in hits.iter().enumerate() {
            use std::fmt::Write as _;
            let _ = writeln!(&mut buf, "  {}. {} — {}", i + 1, h.title, h.url);
        }
        buf
    };

    let view = ToolExecutionView::simple(format!("WebSearch {q:?}"), summary);
    let output = ToolPayloadOutput::WebSearch {
        query: q.to_string(),
        backend: backend_name.to_string(),
        results: hits,
    };
    Ok(ToolPayloadExecution::new(output, view))
}

fn domain_allowed(url: &str, allow: &[String], block: &[String]) -> bool {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default();
    if !allow.is_empty() && !allow.iter().any(|d| host_matches(&host, d)) {
        return false;
    }
    if block.iter().any(|d| host_matches(&host, d)) {
        return false;
    }
    true
}

fn host_matches(host: &str, pattern: &str) -> bool {
    let h = host.to_ascii_lowercase();
    let p = pattern.to_ascii_lowercase();
    h == p || h.ends_with(&format!(".{p}"))
}

pub(crate) fn web_search_backend_target(backend: &WebSearchBackend) -> &'static str {
    match backend {
        WebSearchBackend::Tavily { .. } => "https://api.tavily.com/search",
        WebSearchBackend::Exa { .. } => "https://api.exa.ai/search",
        WebSearchBackend::Brave { .. } => "https://api.search.brave.com/res/v1/web/search",
        WebSearchBackend::DuckDuckGoHtml => "https://html.duckduckgo.com/html/",
    }
}

// ─── Backends ──────────────────────────────────────────────────────────

async fn tavily_search(
    query: &str,
    max: u32,
    api_key: &str,
) -> Result<Vec<WebSearchHit>, ToolError> {
    if api_key.is_empty() {
        return Err(ToolError::Plugin(
            "web_search[tavily]: TAVILY_API_KEY missing".to_string(),
        ));
    }
    let body = serde_json::json!({
        "api_key": api_key,
        "query": query,
        "max_results": max,
        "search_depth": "basic",
    });
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ToolError::Plugin(e.to_string()))?;
    let resp = client
        .post("https://api.tavily.com/search")
        .json(&body)
        .send()
        .await
        .map_err(|e| ToolError::Plugin(format!("tavily request failed: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(ToolError::Plugin(format!("tavily {status}")));
    }
    #[derive(Deserialize)]
    struct R {
        results: Vec<TItem>,
    }
    #[derive(Deserialize)]
    struct TItem {
        title: Option<String>,
        url: String,
        content: Option<String>,
    }
    let parsed: R = resp
        .json()
        .await
        .map_err(|e| ToolError::Plugin(e.to_string()))?;
    Ok(parsed
        .results
        .into_iter()
        .map(|i| WebSearchHit {
            title: i.title.unwrap_or_default(),
            url: i.url,
            snippet: i.content,
        })
        .collect())
}

async fn exa_search(query: &str, max: u32, api_key: &str) -> Result<Vec<WebSearchHit>, ToolError> {
    if api_key.is_empty() {
        return Err(ToolError::Plugin(
            "web_search[exa]: EXA_API_KEY missing".to_string(),
        ));
    }
    let body = serde_json::json!({"query": query, "numResults": max});
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ToolError::Plugin(e.to_string()))?;
    let resp = client
        .post("https://api.exa.ai/search")
        .header("x-api-key", api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| ToolError::Plugin(format!("exa request failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(ToolError::Plugin(format!("exa {}", resp.status())));
    }
    #[derive(Deserialize)]
    struct R {
        results: Vec<EItem>,
    }
    #[derive(Deserialize)]
    struct EItem {
        title: Option<String>,
        url: String,
        text: Option<String>,
    }
    let parsed: R = resp
        .json()
        .await
        .map_err(|e| ToolError::Plugin(e.to_string()))?;
    Ok(parsed
        .results
        .into_iter()
        .map(|i| WebSearchHit {
            title: i.title.unwrap_or_default(),
            url: i.url,
            snippet: i.text,
        })
        .collect())
}

async fn brave_search(
    query: &str,
    max: u32,
    api_key: &str,
) -> Result<Vec<WebSearchHit>, ToolError> {
    if api_key.is_empty() {
        return Err(ToolError::Plugin(
            "web_search[brave]: BRAVE_API_KEY missing".to_string(),
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ToolError::Plugin(e.to_string()))?;
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        max
    );
    let resp = client
        .get(&url)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| ToolError::Plugin(format!("brave request failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(ToolError::Plugin(format!("brave {}", resp.status())));
    }
    #[derive(Deserialize)]
    struct R {
        web: Option<W>,
    }
    #[derive(Deserialize)]
    struct W {
        results: Vec<BItem>,
    }
    #[derive(Deserialize)]
    struct BItem {
        title: Option<String>,
        url: String,
        description: Option<String>,
    }
    let parsed: R = resp
        .json()
        .await
        .map_err(|e| ToolError::Plugin(e.to_string()))?;
    Ok(parsed
        .web
        .map(|w| w.results)
        .unwrap_or_default()
        .into_iter()
        .map(|i| WebSearchHit {
            title: i.title.unwrap_or_default(),
            url: i.url,
            snippet: i.description,
        })
        .collect())
}

async fn duckduckgo_html_search(query: &str, max: u32) -> Result<Vec<WebSearchHit>, ToolError> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(crate::provider::CLAUDE_USER_WEB_FETCH_USER_AGENT)
        .build()
        .map_err(|e| ToolError::Plugin(e.to_string()))?;
    let body = format!("q={}", urlencoding::encode(query));
    let resp = client
        .post("https://html.duckduckgo.com/html/")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| ToolError::Plugin(format!("duckduckgo request failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(ToolError::Plugin(format!("duckduckgo {}", resp.status())));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| ToolError::Plugin(e.to_string()))?;
    Ok(parse_ddg_html(&body, max as usize))
}

/// Best-effort parse of DuckDuckGo's HTML page.  We avoid pulling in a
/// full HTML parser; the markup is stable enough for a regex pass.
fn parse_ddg_html(body: &str, max: usize) -> Vec<WebSearchHit> {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r#"(?s)<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>(.*?)</a>.*?<a[^>]*class="[^"]*result__snippet[^"]*"[^>]*>(.*?)</a>"#,
        )
        .expect("ddg regex compiles")
    });

    let mut out = Vec::new();
    for caps in re.captures_iter(body).take(max) {
        let raw_url = &caps[1];
        let title = strip_html(&caps[2]);
        let snippet = strip_html(&caps[3]);
        let url = decode_ddg_redirect(raw_url);
        out.push(WebSearchHit {
            title,
            url,
            snippet: Some(snippet),
        });
    }
    out
}

fn strip_html(s: &str) -> String {
    let re = regex::Regex::new("<[^>]+>").unwrap();
    let stripped = re.replace_all(s, "");
    html_escape::decode_html_entities(&stripped).into_owned()
}

/// DuckDuckGo wraps result links in a `/l/?uddg=<encoded-url>` redirect;
/// pull out the underlying URL when present.
fn decode_ddg_redirect(url: &str) -> String {
    if let Some(encoded) = url.split("uddg=").nth(1) {
        let raw = encoded.split('&').next().unwrap_or(encoded);
        if let Ok(decoded) = urlencoding::decode(raw) {
            return decoded.into_owned();
        }
    }
    if let Some(stripped) = url.strip_prefix("//") {
        format!("https://{stripped}")
    } else {
        url.to_string()
    }
}
