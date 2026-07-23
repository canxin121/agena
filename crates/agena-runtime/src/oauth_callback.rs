//! Local browser OAuth callback listener shared by process entrypoints.

use agena_provider::OAuthCallback;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeOAuthCallbackError {
    #[error("oauth callback configuration error: {0}")]
    Configuration(String),
    #[error("oauth callback provider error: {0}")]
    Provider(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn parse_oauth_callback_url(
    url: &str,
    expected_state: Option<&str>,
) -> Result<OAuthCallback, RuntimeOAuthCallbackError> {
    let parsed = url::Url::parse(url.trim()).map_err(|error| {
        RuntimeOAuthCallbackError::Configuration(format!("invalid oauth callback url: {error}"))
    })?;
    if let Some(error_code) = parsed
        .query_pairs()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.to_string())
    {
        let error_description = parsed
            .query_pairs()
            .find(|(key, _)| key == "error_description")
            .map(|(_, value)| value.to_string());
        let request_id = parsed
            .query_pairs()
            .find(|(key, _)| key == "request_id")
            .map(|(_, value)| value.to_string());
        return Err(RuntimeOAuthCallbackError::Provider(
            oauth_callback_error_message(
                error_code.as_str(),
                error_description.as_deref(),
                request_id.as_deref(),
            ),
        ));
    }
    let code = parsed
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| {
            RuntimeOAuthCallbackError::Provider("oauth callback missing code".to_owned())
        })?;
    let state = parsed
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| {
            RuntimeOAuthCallbackError::Provider("oauth callback missing state".to_owned())
        })?;
    if let Some(expected_state) = expected_state
        && state != expected_state
    {
        return Err(RuntimeOAuthCallbackError::Provider(
            "oauth callback state mismatch (potential csrf)".to_owned(),
        ));
    }
    Ok(OAuthCallback { code, state })
}

pub fn wait_for_oauth_callback(
    port: u16,
    expected_state: &str,
    timeout: Duration,
) -> Result<OAuthCallback, RuntimeOAuthCallbackError> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|error| {
        RuntimeOAuthCallbackError::Configuration(format!(
            "failed to bind oauth callback port {port}: {error}"
        ))
    })?;
    listener.set_nonblocking(true).map_err(|error| {
        RuntimeOAuthCallbackError::Configuration(format!(
            "failed to set oauth callback nonblocking: {error}"
        ))
    })?;
    let started = Instant::now();
    while started.elapsed() < timeout {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut bytes = [0; 4096];
                let count = stream.read(&mut bytes)?;
                let request = String::from_utf8_lossy(&bytes[..count]);
                let path = request
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_owned();
                let url = format!("http://localhost:{port}{path}");
                match parse_oauth_callback_url(url.as_str(), Some(expected_state)) {
                    Ok(callback) => {
                        write_http_html(&mut stream, 200, oauth_html_success())?;
                        return Ok(callback);
                    }
                    Err(error) => {
                        write_http_html(
                            &mut stream,
                            400,
                            oauth_html_error(error.to_string().as_str()).as_str(),
                        )?;
                        return Err(error);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(RuntimeOAuthCallbackError::Provider(
        "oauth callback timeout".to_owned(),
    ))
}

fn write_http_html(
    stream: &mut impl Write,
    status: u16,
    html: &str,
) -> Result<(), RuntimeOAuthCallbackError> {
    let status_line = if status == 200 {
        "HTTP/1.1 200 OK"
    } else {
        "HTTP/1.1 400 Bad Request"
    };
    let body = html.as_bytes();
    let response = format!(
        "{status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

fn oauth_html_success() -> &'static str {
    "<!doctype html><html><body><h1>Authorization Successful</h1><p>You can close this window.</p><script>setTimeout(() => window.close(), 1500)</script></body></html>"
}

fn oauth_html_error(error: &str) -> String {
    format!(
        "<!doctype html><html><body><h1>Authorization Failed</h1><p>{}</p></body></html>",
        escape_html(error)
    )
}

fn oauth_callback_error_message(
    error_code: &str,
    error_description: Option<&str>,
    request_id: Option<&str>,
) -> String {
    let description = error_description
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let request_id = request_id.map(str::trim).filter(|value| !value.is_empty());

    let mut message = match description {
        Some(description) => format!("oauth callback failed: {description}"),
        None => format!("oauth callback failed: {error_code}"),
    };

    let mut details = Vec::new();
    if !error_code.trim().is_empty() {
        details.push(format!("error_code: {}", error_code.trim()));
    }
    if let Some(request_id) = request_id {
        details.push(format!("request_id: {request_id}"));
    }
    if !details.is_empty() {
        message.push_str(" (");
        message.push_str(details.join(", ").as_str());
        message.push(')');
    }
    message
}

fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{RuntimeOAuthCallbackError, escape_html, parse_oauth_callback_url};

    #[test]
    fn parses_authorization_code_and_state() {
        let callback = parse_oauth_callback_url(
            "http://localhost:8765/callback?code=authorization-code&state=expected-state",
            Some("expected-state"),
        )
        .expect("valid OAuth callback");

        assert_eq!(callback.code, "authorization-code");
        assert_eq!(callback.state, "expected-state");
    }

    #[test]
    fn includes_provider_error_description_and_request_id() {
        let error = parse_oauth_callback_url(
            "http://localhost/callback?error=access_denied&error_description=User%20cancelled&request_id=req-123",
            None,
        )
        .expect_err("provider error must fail the callback");

        assert!(matches!(
            error,
            RuntimeOAuthCallbackError::Provider(message)
                if message == "oauth callback failed: User cancelled (error_code: access_denied, request_id: req-123)"
        ));
    }

    #[test]
    fn rejects_a_mismatched_state() {
        let error = parse_oauth_callback_url(
            "http://localhost/callback?code=authorization-code&state=unexpected-state",
            Some("expected-state"),
        )
        .expect_err("mismatched state must fail the callback");

        assert!(matches!(
            error,
            RuntimeOAuthCallbackError::Provider(message)
                if message == "oauth callback state mismatch (potential csrf)"
        ));
    }

    #[test]
    fn escapes_error_html() {
        assert_eq!(
            escape_html("<script>alert('x') & \"y\"</script>"),
            "&lt;script&gt;alert(&#39;x&#39;) &amp; &quot;y&quot;&lt;/script&gt;"
        );
    }
}
