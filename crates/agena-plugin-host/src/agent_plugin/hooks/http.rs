//! HTTP handlers use the same authorized sandbox as command handlers.

use std::{collections::BTreeMap, process::Output, sync::LazyLock};

use regex::Regex;
use url::Url;

use crate::agent_plugin::mcp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HttpHook {
    url: Url,
    headers: BTreeMap<String, String>,
    allowed_env: Vec<String>,
}

impl HttpHook {
    pub(super) fn new(
        url: Option<&str>,
        headers: BTreeMap<String, String>,
        allowed_env: Vec<String>,
    ) -> Result<Self, String> {
        let url = Url::parse(url.ok_or("HTTP hook requires a URL")?)
            .map_err(|_| "invalid HTTP hook URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err("HTTP hook requires an HTTP(S) URL without user info or fragment".into());
        }
        if headers.iter().any(|(key, value)| {
            key.is_empty()
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
                || value.contains(['\r', '\n', '\0'])
                || key.eq_ignore_ascii_case("content-type")
        }) {
            return Err("invalid HTTP hook header".into());
        }
        Ok(Self {
            url,
            headers,
            allowed_env,
        })
    }

    pub(super) fn request(&self, event: &str, enabled_env: &[String]) -> Result<Vec<u8>, String> {
        static VARIABLE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)\}|\$([a-zA-Z_][a-zA-Z0-9_]*)").unwrap()
        });
        // curl reads the request and credentials on stdin. The config writer
        // escapes every value as one option, without shell parsing.
        let mut config = String::new();
        config_option(&mut config, "url", self.url.as_str());
        config_option(&mut config, "header", "Content-Type: application/json");
        config_option(&mut config, "data-binary", event);
        for (name, template) in &self.headers {
            let value = VARIABLE.replace_all(template, |matched: &regex::Captures<'_>| {
                let name = matched
                    .get(1)
                    .or_else(|| matched.get(2))
                    .expect("matched variable")
                    .as_str();
                if self.allowed_env.iter().any(|allowed| allowed == name)
                    && enabled_env.iter().any(|allowed| allowed == name)
                {
                    mcp::read_var(name).unwrap_or_default()
                } else {
                    String::new()
                }
            });
            if value.contains(['\r', '\n', '\0']) || value.len() > 16 * 1024 {
                return Err("invalid expanded HTTP hook header value".into());
            }
            config_option(&mut config, "header", &format!("{name}: {value}"));
        }
        Ok(config.into_bytes())
    }

    pub(super) fn response(mut output: Output) -> Result<Output, String> {
        if !output.status.success() {
            return Err(format!("HTTP hook request failed with {}", output.status));
        }
        let end = output
            .stdout
            .iter()
            .rposition(|byte| *byte == b'\n')
            .ok_or("HTTP hook response is missing its status")?;
        let status = std::str::from_utf8(&output.stdout[end + 1..])
            .ok()
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or("invalid HTTP hook response status")?;
        if !(200..300).contains(&status) {
            return Err(format!("HTTP hook returned status {status}"));
        }
        let body = std::str::from_utf8(&output.stdout[..end])
            .map_err(|_| "HTTP hook body is not UTF-8")?
            .trim();
        if !body.is_empty() && !body.starts_with('{') {
            return Err("HTTP hook must return an empty body or a JSON object".into());
        }
        output.stdout.truncate(end);
        Ok(output)
    }
}

fn config_option(config: &mut String, option: &str, value: &str) {
    config.push_str(option);
    config.push_str(" = \"");
    for ch in value.chars() {
        match ch {
            '\\' => config.push_str("\\\\"),
            '"' => config.push_str("\\\""),
            '\n' => config.push_str("\\n"),
            '\r' => config.push_str("\\r"),
            '\t' => config.push_str("\\t"),
            _ => config.push(ch),
        }
    }
    config.push_str("\"\n");
}
