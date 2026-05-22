use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

pub(super) const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(super) const OPENAI_ISSUER: &str = "https://auth.openai.com";
pub(super) const COPILOT_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";

pub(super) fn normalize_domain(url_or_domain: &str) -> String {
    url_or_domain
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

pub(super) fn parse_device_auth_interval(
    raw: Option<serde_json::Value>,
    default_seconds: u64,
) -> u64 {
    let Some(raw) = raw else {
        return default_seconds;
    };

    if let Some(interval) = raw.as_u64() {
        return interval.max(1);
    }

    if let Some(interval) = raw
        .as_str()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        return interval.max(1);
    }

    default_seconds
}

pub(super) fn extract_openai_account_id(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    json.get("chatgpt_account_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            json.get("https://api.openai.com/auth")
                .and_then(|value| value.get("chatgpt_account_id"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            json.get("organizations")
                .and_then(|value| value.as_array())
                .and_then(|items| items.first())
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
}
