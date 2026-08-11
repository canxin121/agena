//! Link handling and URL detection in the TUI.

use std::time::Duration;

use anyhow::{Result, anyhow};
use reqwest::Client;
use unicode_width::UnicodeWidthStr;
use url::Url;

const SHORT_LINK_TRIGGER_WIDTH: usize = 56;
const SHORT_LINK_TIMEOUT: Duration = Duration::from_millis(2500);
const SHORT_LINK_CONNECT_TIMEOUT: Duration = Duration::from_millis(1200);
const SHORT_LINK_USER_AGENT: &str = "agena-tui-auth-shortener";
const SHORT_LINK_MAX_RESPONSE_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy)]
enum ShortLinkBackend {
    TinyUrl,
    ClckRu,
    VGd,
    IsGd,
}

impl ShortLinkBackend {
    fn expected_host(self) -> &'static str {
        match self {
            Self::TinyUrl => "tinyurl.com",
            Self::ClckRu => "clck.ru",
            Self::VGd => "v.gd",
            Self::IsGd => "is.gd",
        }
    }

    async fn shorten(self, client: &Client, url: &str) -> Result<String> {
        match self {
            Self::TinyUrl => shorten_tinyurl(client, url).await,
            Self::ClckRu => shorten_clckru(client, url).await,
            Self::VGd => shorten_simple(client, "https://v.gd/create.php", url).await,
            Self::IsGd => shorten_simple(client, "https://is.gd/create.php", url).await,
        }
    }
}

pub async fn shorten_url_for_display(url: &str) -> Option<String> {
    if !should_shorten_url(url) {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let client = build_client().ok()?;
    for backend in [
        ShortLinkBackend::TinyUrl,
        ShortLinkBackend::ClckRu,
        ShortLinkBackend::VGd,
        ShortLinkBackend::IsGd,
    ] {
        if let Ok(short_url) = backend.shorten(&client, url).await
            && should_use_short_url(url, short_url.as_str(), backend.expected_host())
        {
            return Some(short_url);
        }
    }
    None
}

pub fn should_shorten_url(url: &str) -> bool {
    UnicodeWidthStr::width(url) > SHORT_LINK_TRIGGER_WIDTH
}

fn should_use_short_url(original: &str, short_url: &str, expected_host: &str) -> bool {
    let parsed = match Url::parse(short_url) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    let host_matches = parsed
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(expected_host));
    host_matches
        && matches!(parsed.scheme(), "http" | "https")
        && UnicodeWidthStr::width(short_url) <= SHORT_LINK_TRIGGER_WIDTH
        && UnicodeWidthStr::width(short_url) < UnicodeWidthStr::width(original)
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(SHORT_LINK_USER_AGENT)
        .timeout(SHORT_LINK_TIMEOUT)
        .connect_timeout(SHORT_LINK_CONNECT_TIMEOUT)
        .build()
        .map_err(Into::into)
}

async fn shorten_simple(client: &Client, endpoint: &str, url: &str) -> Result<String> {
    let response = client
        .get(endpoint)
        .query(&[("format", "simple"), ("url", url)])
        .send()
        .await?
        .error_for_status()?;
    let short_url = read_short_link_response(response).await?;
    if short_url.is_empty() {
        return Err(anyhow!("shortener returned empty response"));
    }
    Ok(short_url)
}

async fn shorten_tinyurl(client: &Client, url: &str) -> Result<String> {
    let response = client
        .get("https://tinyurl.com/api-create.php")
        .query(&[("url", url)])
        .send()
        .await?
        .error_for_status()?;
    let short_url = read_short_link_response(response).await?;
    if short_url.is_empty() {
        return Err(anyhow!("tinyurl returned empty response"));
    }
    Ok(short_url)
}

async fn shorten_clckru(client: &Client, url: &str) -> Result<String> {
    let response = client
        .get("https://clck.ru/--")
        .query(&[("url", url)])
        .send()
        .await?
        .error_for_status()?;
    let short_url = read_short_link_response(response).await?;
    if short_url.is_empty() {
        return Err(anyhow!("clck.ru returned empty response"));
    }
    Ok(short_url)
}

async fn read_short_link_response(mut response: reqwest::Response) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|length| length > SHORT_LINK_MAX_RESPONSE_BYTES as u64)
    {
        return Err(anyhow!("shortener response exceeds 8 KiB"));
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > SHORT_LINK_MAX_RESPONSE_BYTES {
            return Err(anyhow!("shortener response exceeds 8 KiB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(std::str::from_utf8(&bytes)?.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::{SHORT_LINK_TRIGGER_WIDTH, should_shorten_url, should_use_short_url};

    #[test]
    fn only_visually_wide_urls_trigger_shortening() {
        assert!(!should_shorten_url("https://example.test/short"));
        assert!(should_shorten_url(&format!(
            "https://example.test/{}",
            "x".repeat(SHORT_LINK_TRIGGER_WIDTH)
        )));
    }

    #[test]
    fn accepted_short_urls_must_use_the_expected_safe_host() {
        let original = format!("https://example.test/{}", "x".repeat(120));
        assert!(should_use_short_url(
            &original,
            "https://tinyurl.com/agena",
            "tinyurl.com"
        ));
        assert!(!should_use_short_url(
            &original,
            "https://attacker.example/agena",
            "tinyurl.com"
        ));
        assert!(!should_use_short_url(
            &original,
            "javascript:alert(1)",
            "tinyurl.com"
        ));
    }
}
