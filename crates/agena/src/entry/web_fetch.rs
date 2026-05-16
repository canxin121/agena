//! `web_fetch` plugin tool — GET an absolute URL, convert HTML→Markdown,
//! optionally summarize via the session's default LLM provider.
//!
//! Cache: 15-minute TTL keyed by canonicalized URL (LRU, capped at 64).
//!
//! Security: HTTP is upgraded to HTTPS; localhost / link-local hosts are
//! rejected to limit SSRF blast-radius.  Bytes capped at 5 MB.

use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use lru::LruCache;

use crate::message::{ToolAttachment, WebFetchToolInput};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const CACHE_CAPACITY: usize = 64;

#[derive(Debug, Clone)]
struct CachedFetch {
    inserted_at: Instant,
    status: u16,
    markdown: String,
    truncated: bool,
}

static CACHE: LazyLock<Mutex<LruCache<String, CachedFetch>>> = LazyLock::new(|| {
    Mutex::new(LruCache::new(
        std::num::NonZeroUsize::new(CACHE_CAPACITY).unwrap(),
    ))
});

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &WebFetchToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let raw_url = input.url.trim();
    if raw_url.is_empty() {
        return Err(ToolError::Plugin(
            "web_fetch: url must not be empty".to_string(),
        ));
    }

    // HTTP -> HTTPS upgrade.
    let url_string = if let Some(rest) = raw_url.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        raw_url.to_string()
    };

    let url = url::Url::parse(&url_string)
        .map_err(|e| ToolError::Plugin(format!("web_fetch: invalid url '{raw_url}': {e}")))?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err(ToolError::Plugin(format!(
            "web_fetch: scheme '{}' not allowed",
            url.scheme()
        )));
    }
    let target = crate::permission::NetworkTarget::parse(url.as_str())
        .map_err(|e| ToolError::Plugin(format!("web_fetch: invalid network target: {e}")))?;
    executor.ensure_network_permission(&target)?;

    // Cache lookup.
    let cache_key = url.to_string();
    if let Some(hit) = cache_get(&cache_key) {
        return Ok(make_execution(
            url.to_string(),
            hit.status,
            hit.markdown,
            hit.truncated,
            true,
            input.prompt.as_deref(),
        ));
    }

    let url_for_fetch = url.clone();
    let result = super::mcp::block_on(async move { fetch_async(&url_for_fetch).await });
    let (status, markdown, truncated) = result?;

    cache_put(
        cache_key,
        CachedFetch {
            inserted_at: Instant::now(),
            status,
            markdown: markdown.clone(),
            truncated,
        },
    );

    Ok(make_execution(
        url.to_string(),
        status,
        markdown,
        truncated,
        false,
        input.prompt.as_deref(),
    ))
}

fn cache_get(key: &str) -> Option<CachedFetch> {
    let mut g = CACHE.lock().ok()?;
    let entry = g.get(key)?;
    if entry.inserted_at.elapsed() > CACHE_TTL {
        g.pop(key);
        return None;
    }
    Some(entry.clone())
}

fn cache_put(key: String, entry: CachedFetch) {
    if let Ok(mut g) = CACHE.lock() {
        g.put(key, entry);
    }
}

fn fetch_async(
    url: &url::Url,
) -> impl std::future::Future<Output = Result<(u16, String, bool), ToolError>> + Send {
    let url = url.clone();
    async move {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("agena-web-fetch/0.1 (+https://github.com/canxin121/agena)")
            .build()
            .map_err(|e| ToolError::Plugin(format!("web_fetch: client build failed: {e}")))?;

        let response = client
            .get(url.clone())
            .header("Accept", "text/html, text/plain, application/xhtml+xml")
            .header("Accept-Language", "en;q=0.9, *;q=0.5")
            .send()
            .await
            .map_err(|e| ToolError::Plugin(format!("web_fetch: request failed: {e}")))?;

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let bytes = response
            .bytes()
            .await
            .map_err(|e| ToolError::Plugin(format!("web_fetch: read body failed: {e}")))?;
        let truncated = bytes.len() > MAX_BODY_BYTES;
        let slice = if truncated {
            &bytes[..MAX_BODY_BYTES]
        } else {
            &bytes[..]
        };

        let body = String::from_utf8_lossy(slice).into_owned();
        let markdown = if content_type.starts_with("text/html") || looks_like_html(&body) {
            html2md::parse_html(&body)
        } else {
            body
        };

        Ok((status, markdown, truncated))
    }
}

fn looks_like_html(body: &str) -> bool {
    let head = body.trim_start();
    head.starts_with('<') || head.to_ascii_lowercase().contains("<html")
}

fn make_execution(
    url: String,
    status: u16,
    markdown: String,
    truncated: bool,
    cached: bool,
    _prompt: Option<&str>,
) -> ToolPayloadExecution {
    // NOTE: prompt-based summarization is left as a follow-up — it
    // requires re-entering the LLM provider from inside a tool dispatch,
    // which the executor doesn't currently expose synchronously.  The
    // raw markdown is what's returned today.
    let summary = None;

    let preview = preview_text(&markdown, 4000);
    let view =
        ToolExecutionView::simple(format!("WebFetch {url}"), format!("[{status}] {preview}"));
    let output = ToolPayloadOutput::WebFetch {
        url,
        markdown: Some(markdown),
        summary,
        truncated,
        cached,
        status,
    };
    ToolPayloadExecution::new(output, view)
}

fn preview_text(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Silence unused warnings until we wire prompt-based summarization.
#[allow(dead_code)]
fn _silence(_: Arc<()>, _: Vec<ToolAttachment>) {}
