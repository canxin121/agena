use std::time::Duration;

use backoff::ExponentialBackoffBuilder;
use backoff::backoff::Backoff;
use reqwest::header::{CONTENT_TYPE, ETAG, LAST_MODIFIED};
use url::Url;

use crate::extract::extract_page_from_body;
use crate::{CrawlError, FetchedPage};

pub const DEFAULT_MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
pub const DEFAULT_FETCH_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct FetchOptions {
    pub max_body_bytes: usize,
    pub timeout: Duration,
    pub user_agent: String,
    pub max_retries: usize,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            timeout: Duration::from_secs(DEFAULT_FETCH_TIMEOUT_SECS),
            user_agent: format!("agena-web/{}", env!("CARGO_PKG_VERSION")),
            max_retries: 2,
        }
    }
}

pub fn build_client(options: &FetchOptions) -> Result<reqwest::Client, CrawlError> {
    reqwest::Client::builder()
        .timeout(options.timeout)
        .user_agent(options.user_agent.clone())
        .build()
        .map_err(CrawlError::from)
}

pub async fn fetch_page(url: &Url, options: &FetchOptions) -> Result<FetchedPage, CrawlError> {
    let client = build_client(options)?;
    fetch_page_with_client(&client, url, options).await
}

pub async fn fetch_page_with_client(
    client: &reqwest::Client,
    url: &Url,
    options: &FetchOptions,
) -> Result<FetchedPage, CrawlError> {
    let mut backoff = ExponentialBackoffBuilder::new()
        .with_initial_interval(Duration::from_millis(250))
        .with_max_interval(Duration::from_secs(4))
        .with_max_elapsed_time(Some(Duration::from_secs(
            2 + (options.max_retries as u64 * 4),
        )))
        .build();
    let mut attempt = 0usize;

    loop {
        let response = client
            .get(url.clone())
            .header("Accept", "text/html, text/plain, application/xhtml+xml")
            .header("Accept-Language", "en;q=0.9, *;q=0.5")
            .send()
            .await?;

        if should_retry_status(response.status().as_u16()) && attempt < options.max_retries {
            if let Some(delay) = backoff.next_backoff() {
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }
        }

        return response_to_page(response, url, options).await;
    }
}

pub fn prepare_fetch_url(raw: &str) -> Result<Url, CrawlError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(CrawlError::InvalidInput(
            "url must not be empty".to_string(),
        ));
    }
    let url = if let Some(rest) = raw.strip_prefix("http://")
        && should_upgrade_http(raw)
    {
        format!("https://{rest}")
    } else {
        raw.to_string()
    };
    canonicalize_url(url.as_str())
}

pub fn canonicalize_url(raw: &str) -> Result<Url, CrawlError> {
    let url = Url::parse(raw)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(CrawlError::InvalidInput(format!(
            "unsupported url scheme '{}'",
            url.scheme()
        )));
    }
    Ok(normalize_url(url))
}

pub fn resolve_link_url(base: &Url, href: &str) -> Option<Url> {
    let href = href.trim();
    if href.is_empty()
        || href.starts_with('#')
        || href.starts_with("mailto:")
        || href.starts_with("javascript:")
    {
        return None;
    }
    let joined = base.join(href).ok()?;
    matches!(joined.scheme(), "http" | "https").then(|| normalize_url(joined))
}

async fn response_to_page(
    response: reqwest::Response,
    requested_url: &Url,
    options: &FetchOptions,
) -> Result<FetchedPage, CrawlError> {
    let status = response.status().as_u16();
    let final_url = canonicalize_url(response.url().as_str())?;
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let last_modified = response
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response.bytes().await?;
    let truncated = bytes.len() > options.max_body_bytes;
    let body = if truncated {
        String::from_utf8_lossy(&bytes[..options.max_body_bytes]).into_owned()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(extract_page_from_body(
        requested_url,
        &final_url,
        content_type.as_str(),
        status,
        truncated,
        false,
        body.as_str(),
        etag,
        last_modified,
    ))
}

fn normalize_url(mut url: Url) -> Url {
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    if url.path().is_empty() {
        url.set_path("/");
    }
    if matches!(
        (url.scheme(), url.port()),
        ("http", Some(80)) | ("https", Some(443))
    ) {
        let _ = url.set_port(None);
    }

    let mut filtered_pairs = url
        .query_pairs()
        .filter(|(key, _)| !is_tracking_query_param(key.as_ref()))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if filtered_pairs.is_empty() {
        url.set_query(None);
        return url;
    }
    filtered_pairs.sort();
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in filtered_pairs {
        serializer.append_pair(key.as_str(), value.as_str());
    }
    url.set_query(Some(serializer.finish().as_str()));
    url
}

fn is_tracking_query_param(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.starts_with("utm_")
        || matches!(
            lower.as_str(),
            "fbclid" | "gclid" | "igshid" | "mc_cid" | "mc_eid" | "ref"
        )
}

fn should_retry_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

fn should_upgrade_http(raw_url: &str) -> bool {
    let Ok(url) = Url::parse(raw_url) else {
        return true;
    };
    let Some(host) = url.host_str() else {
        return true;
    };
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return false;
    }
    match url.host() {
        Some(url::Host::Ipv4(addr)) => !addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => !addr.is_loopback(),
        Some(url::Host::Domain(_)) | None => true,
    }
}
