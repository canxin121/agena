use std::time::Duration;

use anyhow::{Result, anyhow};
use reqwest::Client;
use unicode_width::UnicodeWidthStr;
use url::Url;

const SHORT_LINK_TRIGGER_WIDTH: usize = 56;
const SHORT_LINK_TIMEOUT: Duration = Duration::from_millis(2500);
const SHORT_LINK_CONNECT_TIMEOUT: Duration = Duration::from_millis(1200);
const SHORT_LINK_USER_AGENT: &str = "agena-tui-auth-shortener";

#[derive(Debug, Clone, Copy)]
enum ShortLinkBackend {
    IsGd,
    VGd,
    CleanUri,
}

impl ShortLinkBackend {
    fn expected_host(self) -> &'static str {
        match self {
            Self::IsGd => "is.gd",
            Self::VGd => "v.gd",
            Self::CleanUri => "cleanuri.com",
        }
    }

    async fn shorten(self, client: &Client, url: &str) -> Result<String> {
        match self {
            Self::IsGd => shorten_simple(client, "https://is.gd/create.php", url).await,
            Self::VGd => shorten_simple(client, "https://v.gd/create.php", url).await,
            Self::CleanUri => shorten_cleanuri(client, url).await,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct CleanUriResponse {
    result_url: Option<String>,
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
        ShortLinkBackend::IsGd,
        ShortLinkBackend::VGd,
        ShortLinkBackend::CleanUri,
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
    let short_url = response.text().await?.trim().to_owned();
    if short_url.is_empty() {
        return Err(anyhow!("shortener returned empty response"));
    }
    Ok(short_url)
}

async fn shorten_cleanuri(client: &Client, url: &str) -> Result<String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("url", url)
        .finish();
    let response = client
        .post("https://cleanuri.com/api/v1/shorten")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    let payload: CleanUriResponse = response.json().await?;
    payload
        .result_url
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("cleanuri returned no result_url"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_shorten_wide_urls() {
        assert!(!should_shorten_url("https://is.gd/demo"));
        assert!(should_shorten_url(
            "https://auth.openai.com/oauth/authorize?response_type=code&client_id=demo&redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fcallback&scope=openid%20profile%20email&state=demo"
        ));
    }

    #[test]
    fn accepts_only_valid_short_urls_from_expected_hosts() {
        let original = "https://auth.openai.com/oauth/authorize?response_type=code&client_id=demo&redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fcallback&scope=openid%20profile%20email&state=demo";
        assert!(should_use_short_url(
            original,
            "https://is.gd/AbCdEf",
            "is.gd"
        ));
        assert!(!should_use_short_url(
            original,
            "https://example.com/not-short",
            "is.gd"
        ));
        assert!(!should_use_short_url(original, original, "auth.openai.com"));
    }
}
