//! `web_search` plugin tool backed by Brave Search.

use serde::Deserialize;

use crate::message::WebSearchToolInput;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    WebSearchHit,
};

const DEFAULT_MAX_RESULTS: u32 = 8;
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const BRAVE_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/web/search";

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
    let backend_name = backend.name().to_string();
    let api_key = backend.api_key.clone();
    let request_url = executor
        .web_search_url_override()
        .unwrap_or(BRAVE_SEARCH_URL)
        .to_string();
    let target = crate::permission::NetworkTarget::parse(request_url.as_str())
        .map_err(|e| ToolError::Plugin(format!("web_search: invalid network target: {e}")))?;
    executor.ensure_network_permission(&target)?;

    let raw_hits: Vec<WebSearchHit> = super::mcp::block_on(async move {
        brave_search(&query, max, &api_key, request_url.as_str()).await
    })?;

    let hits: Vec<WebSearchHit> = raw_hits
        .into_iter()
        .filter(|hit| domain_allowed(&hit.url, &allow, &block))
        .take(max as usize)
        .collect();

    let summary = if hits.is_empty() {
        format!("[brave] no results for {q:?}")
    } else {
        let mut buf = format!("[brave] {} result(s):\n", hits.len());
        for (i, h) in hits.iter().enumerate() {
            use std::fmt::Write as _;
            let _ = writeln!(&mut buf, "  {}. {} — {}", i + 1, h.title, h.url);
        }
        buf
    };

    let view = ToolExecutionView::simple(format!("WebSearch {q:?}"), summary);
    let output = ToolPayloadOutput::WebSearch {
        query: q.to_string(),
        backend: backend_name,
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

async fn brave_search(
    query: &str,
    max: u32,
    api_key: &str,
    request_url: &str,
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
        "{request_url}?q={}&count={}",
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
    struct Response {
        web: Option<Web>,
    }
    #[derive(Deserialize)]
    struct Web {
        results: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        title: Option<String>,
        url: String,
        description: Option<String>,
    }
    let parsed: Response = resp
        .json()
        .await
        .map_err(|e| ToolError::Plugin(e.to_string()))?;
    Ok(parsed
        .web
        .map(|web| web.results)
        .unwrap_or_default()
        .into_iter()
        .map(|item| WebSearchHit {
            title: item.title.unwrap_or_default(),
            url: item.url,
            snippet: item.description,
        })
        .collect())
}
