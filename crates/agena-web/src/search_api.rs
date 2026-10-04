//! Structured search transports. The caller authorizes and resolves the endpoint;
//! connections use those addresses, disable proxies and never follow redirects.
//! Credentials are request headers, excluded from Debug, URLs and error bodies.

use std::{collections::BTreeSet, net::SocketAddr, time::Duration};

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use serde::Serialize;
use serde_json::{Value, json};
use url::Url;

use crate::{CrawlError, WebSearchResult};

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESULT_CANDIDATES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchApiProvider {
    Brave,
    Tavily,
    Exa,
    Searxng,
}

impl SearchApiProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Brave => "brave",
            Self::Tavily => "tavily",
            Self::Exa => "exa",
            Self::Searxng => "searxng",
        }
    }

    pub fn default_endpoint(self) -> Option<&'static str> {
        match self {
            Self::Brave => Some("https://api.search.brave.com/res/v1/web/search"),
            Self::Tavily => Some("https://api.tavily.com/search"),
            Self::Exa => Some("https://api.exa.ai/search"),
            Self::Searxng => None,
        }
    }

    pub fn default_key_env(self) -> Option<&'static str> {
        match self {
            Self::Brave => Some("BRAVE_SEARCH_API_KEY"),
            Self::Tavily => Some("TAVILY_API_KEY"),
            Self::Exa => Some("EXA_API_KEY"),
            Self::Searxng => None,
        }
    }
}

// Deliberately does not implement Debug: api_key is a credential.
pub struct SearchApiOptions<'a> {
    pub provider: SearchApiProvider,
    pub endpoint: &'a Url,
    pub resolved_addrs: &'a [SocketAddr],
    pub api_key: Option<&'a str>,
    pub limit: usize,
    pub timeout: Duration,
    pub user_agent: &'a str,
    pub allowed_domains: &'a [String],
    pub blocked_domains: &'a [String],
}

#[derive(Debug, Serialize)]
pub struct SearchApiResponse {
    pub results: Vec<WebSearchResult>,
    /// The provider returned incomplete results or malformed individual rows.
    pub partial: bool,
    /// Some text or additional candidate rows were omitted by local budgets.
    pub truncated: bool,
    pub warnings: Vec<String>,
    pub effective_limit: usize,
    pub provider_result_count: usize,
    pub filtered_result_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
}

fn response_error(provider: SearchApiProvider, message: impl Into<String>) -> CrawlError {
    CrawlError::SearchProvider {
        provider: provider.label(),
        message: message.into(),
    }
}

fn credential(options: &SearchApiOptions<'_>, bearer: bool) -> Result<HeaderValue, CrawlError> {
    let key = options
        .api_key
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| {
            CrawlError::InvalidInput(format!(
                "{} API credential is missing",
                options.provider.label()
            ))
        })?;
    let mut value = HeaderValue::from_str(&if bearer {
        format!("Bearer {key}")
    } else {
        key.to_owned()
    })
    .map_err(|_| {
        CrawlError::InvalidInput("search API credential is not a valid HTTP header".into())
    })?;
    value.set_sensitive(true);
    Ok(value)
}

pub async fn search_api(
    query: &str,
    options: &SearchApiOptions<'_>,
) -> Result<SearchApiResponse, CrawlError> {
    let query = query.trim();
    if query.is_empty() || query.len() > 8192 || options.limit == 0 {
        return Err(CrawlError::InvalidInput(
            "search needs a 1–8192 byte query and a positive limit".into(),
        ));
    }
    let host = options
        .endpoint
        .host_str()
        .ok_or_else(|| CrawlError::InvalidInput("search endpoint has no host".into()))?;
    let mut builder = reqwest::Client::builder()
        .timeout(options.timeout)
        .user_agent(options.user_agent)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if !options.resolved_addrs.is_empty() {
        builder = builder.resolve_to_addrs(host, options.resolved_addrs);
    }
    let client = builder.build()?;
    // Brave and Tavily support at most 20. Use a predictable common cap.
    let limit = options.limit.min(20);
    let request = match options.provider {
        SearchApiProvider::Brave => client
            .get(options.endpoint.clone())
            .header("X-Subscription-Token", credential(options, false)?)
            .query(&[("q", query.to_owned()), ("count", limit.to_string())]),
        SearchApiProvider::Tavily => client
            .post(options.endpoint.clone())
            .header(AUTHORIZATION, credential(options, true)?)
            .json(
                &json!({"query":query, "max_results":limit, "search_depth":"basic",
                "auto_parameters":false, "include_answer":false, "include_raw_content":false,
                "include_usage":true, "include_domains":options.allowed_domains,
                "exclude_domains":options.blocked_domains}),
            ),
        SearchApiProvider::Exa => client
            .post(options.endpoint.clone())
            .header("x-api-key", credential(options, false)?)
            .json(&json!({"query":query, "numResults":limit, "type":"auto",
                "contents":{"highlights":true}, "includeDomains":options.allowed_domains,
                "excludeDomains":options.blocked_domains})),
        SearchApiProvider::Searxng => client.get(options.endpoint.clone()).query(&[
            ("q", query),
            ("format", "json"),
            ("categories", "general"),
        ]),
    };
    let mut response = request
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| CrawlError::Http(error.without_url()))?;
    if !response.status().is_success() {
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|seconds| format!("; retry after {seconds} seconds"))
            .unwrap_or_default();
        // Error bodies can echo credentials or queries. Keep only status facts.
        return Err(response_error(
            options.provider,
            format!(
                "HTTP {}{retry}; redirects and automatic retries are disabled",
                response.status().as_u16()
            ),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(CrawlError::ResponseTooLarge {
            maximum_bytes: MAX_RESPONSE_BYTES,
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| CrawlError::Http(error.without_url()))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(CrawlError::ResponseTooLarge {
                maximum_bytes: MAX_RESPONSE_BYTES,
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|_| response_error(options.provider, "invalid JSON response"))?;
    parse_response(
        options.provider,
        &body,
        limit,
        options.allowed_domains,
        options.blocked_domains,
    )
}

/// Exact hostname or subdomain matching; never matches `badexample.com`.
pub fn search_domain_allowed(url: &str, allowed: &[String], blocked: &[String]) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let matches = |domain: &String| {
        let Ok(domain) = url::Host::parse(domain.trim().trim_end_matches('.')) else {
            return false;
        };
        let domain = domain.to_string().to_ascii_lowercase();
        !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
    };
    (allowed.is_empty() || allowed.iter().any(matches)) && !blocked.iter().any(matches)
}

fn bounded_text(text: &str, limit: usize, truncated: &mut bool) -> String {
    let text = text.trim();
    if let Some((end, _)) = text.char_indices().nth(limit) {
        *truncated = true;
        text[..end].to_owned()
    } else {
        text.to_owned()
    }
}

fn parse_response(
    provider: SearchApiProvider,
    body: &Value,
    limit: usize,
    allowed: &[String],
    blocked: &[String],
) -> Result<SearchApiResponse, CrawlError> {
    let records = if provider == SearchApiProvider::Brave {
        body.pointer("/web/results")
    } else {
        body.get("results")
    };
    // Brave omits `web` for a successful search with no web matches.
    let empty = Vec::new();
    let rows = if provider == SearchApiProvider::Brave
        && body.get("type").and_then(Value::as_str) == Some("search")
        && body.get("web").is_none()
    {
        &empty
    } else {
        records
            .and_then(Value::as_array)
            .ok_or_else(|| response_error(provider, "response is missing its results array"))?
    };
    let mut response = SearchApiResponse {
        results: Vec::new(),
        partial: false,
        truncated: rows.len() > MAX_RESULT_CANDIDATES,
        warnings: Vec::new(),
        effective_limit: limit,
        provider_result_count: rows.len(),
        filtered_result_count: 0,
        usage: None,
    };
    let mut seen = BTreeSet::new();
    let mut invalid = 0;
    for row in rows.iter().take(MAX_RESULT_CANDIDATES) {
        let Some(raw_url) = row
            .get("url")
            .and_then(Value::as_str)
            .filter(|value| value.len() <= 4096)
        else {
            invalid += 1;
            continue;
        };
        let Ok(mut url) = Url::parse(raw_url) else {
            invalid += 1;
            continue;
        };
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            invalid += 1;
            continue;
        }
        url.set_fragment(None);
        if !search_domain_allowed(url.as_str(), allowed, blocked) || !seen.insert(url.to_string()) {
            response.filtered_result_count += 1;
            continue;
        }
        if response.results.len() >= limit {
            response.truncated = true;
            break;
        }
        let description = match provider {
            SearchApiProvider::Brave => row
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            SearchApiProvider::Tavily | SearchApiProvider::Searxng => row
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            SearchApiProvider::Exa => row
                .get("highlights")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .take(3)
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
        };
        let description = if matches!(
            provider,
            SearchApiProvider::Brave | SearchApiProvider::Searxng
        ) {
            scraper::Html::parse_fragment(&description)
                .root_element()
                .text()
                .collect::<String>()
        } else {
            description
        };
        response.results.push(WebSearchResult {
            title: bounded_text(
                row.get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(url.as_str()),
                512,
                &mut response.truncated,
            ),
            url: url.to_string(),
            description: bounded_text(&description, 2000, &mut response.truncated),
            source: url.host_str().unwrap_or_default().to_owned(),
            engine: provider.label().into(),
        });
    }
    if invalid > 0 {
        response.partial = true;
        response
            .warnings
            .push(format!("{invalid} malformed result(s) omitted"));
    }
    if provider == SearchApiProvider::Searxng {
        if let Some(failures) = body
            .get("unresponsive_engines")
            .and_then(Value::as_array)
            .filter(|rows| !rows.is_empty())
        {
            response.partial = true;
            response.warnings.push(format!(
                "{} SearXNG upstream engine(s) did not respond",
                failures.len()
            ));
        }
    } else if provider == SearchApiProvider::Tavily {
        response.usage = body
            .pointer("/usage/credits")
            .filter(|value| value.is_number())
            .map(|credits| json!({"credits":credits}));
    } else if provider == SearchApiProvider::Exa {
        response.usage = body
            .pointer("/costDollars/total")
            .filter(|value| value.is_number())
            .map(|total| json!({"cost_usd":total}));
    }
    Ok(response)
}

#[cfg(test)]
mod tests;
