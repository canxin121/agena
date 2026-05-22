use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallback {
    pub code: String,
    pub state: String,
}

pub fn parse_oauth_callback_url(
    callback_url: &str,
    expected_state: Option<&str>,
) -> Result<OAuthCallback, AppError> {
    let parsed = url::Url::parse(callback_url.trim())
        .map_err(|error| AppError::Config(format!("invalid oauth callback url: {error}")))?;

    if let Some(error) = parsed.query_pairs().find(|(key, _)| key == "error") {
        return Err(AppError::Provider(format!(
            "oauth callback failed: {}",
            error.1
        )));
    }

    let code = parsed
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| AppError::Provider("oauth callback missing code".to_owned()))?;
    let state = parsed
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| AppError::Provider("oauth callback missing state".to_owned()))?;

    if let Some(expected_state) = expected_state
        && state != expected_state
    {
        return Err(AppError::Provider(
            "oauth callback state mismatch (potential csrf)".to_owned(),
        ));
    }

    Ok(OAuthCallback { code, state })
}

pub fn wait_for_oauth_callback(
    port: u16,
    expected_state: &str,
    timeout: Duration,
) -> Result<OAuthCallback, AppError> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|error| {
        AppError::Config(format!(
            "failed to bind oauth callback port {port}: {error}"
        ))
    })?;
    listener.set_nonblocking(true).map_err(|error| {
        AppError::Config(format!("failed to set oauth callback nonblocking: {error}"))
    })?;

    let started = Instant::now();
    while started.elapsed() < timeout {
        match listener.accept() {
            Ok((mut stream, _addr)) => {
                let mut buf = [0u8; 4096];
                let bytes_read = stream.read(&mut buf)?;
                let request = String::from_utf8_lossy(&buf[..bytes_read]);

                let first_line = request.lines().next().unwrap_or_default();
                let path = first_line
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
                    Err(AppError::Provider(message)) => {
                        write_http_html(
                            &mut stream,
                            400,
                            oauth_html_error(message.as_str()).as_str(),
                        )?;
                        return Err(AppError::Provider(message));
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
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(AppError::Io(error)),
        }
    }

    Err(AppError::Provider("oauth callback timeout".to_owned()))
}

fn write_http_html(stream: &mut impl Write, status: u16, html: &str) -> Result<(), AppError> {
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
        error
    )
}
