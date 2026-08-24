//! Local browser OAuth callback listener shared by process entrypoints.

use agena_provider::OAuthCallback;
use axum::{
    Router,
    extract::State,
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Response},
    routing::any,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
/// Error handling a runtime OAuth callback.
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
    let issuer = parsed
        .query_pairs()
        .find(|(key, _)| key == "iss")
        .map(|(_, value)| value.to_string())
        .filter(|value| !value.trim().is_empty());
    Ok(OAuthCallback {
        code,
        state,
        issuer,
    })
}

pub fn wait_for_oauth_callback(
    port: u16,
    expected_state: &str,
    timeout: Duration,
) -> Result<OAuthCallback, RuntimeOAuthCallbackError> {
    let expected_state = expected_state.to_owned();
    std::thread::Builder::new()
        .name("agena-oauth-callback".to_owned())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(RuntimeOAuthCallbackError::Io)?
                .block_on(wait_for_oauth_callback_async(
                    port,
                    expected_state.as_str(),
                    timeout,
                ))
        })
        .map_err(RuntimeOAuthCallbackError::Io)?
        .join()
        .map_err(|_| {
            RuntimeOAuthCallbackError::Io(std::io::Error::other("oauth callback worker panicked"))
        })?
}

/// Wait for one loopback OAuth redirect using Tokio and Axum's bounded HTTP
/// parser. This is the preferred API for async entrypoints.
pub async fn wait_for_oauth_callback_async(
    port: u16,
    expected_state: &str,
    timeout: Duration,
) -> Result<OAuthCallback, RuntimeOAuthCallbackError> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| {
            RuntimeOAuthCallbackError::Configuration(format!(
                "failed to bind oauth callback port {port}: {error}"
            ))
        })?;
    let (result_tx, result_rx) = oneshot::channel();
    let state = OAuthCallbackState {
        port,
        expected_state: Arc::from(expected_state),
        result: Arc::new(Mutex::new(Some(result_tx))),
    };
    let app = Router::new()
        .fallback(any(oauth_callback_handler))
        .with_state(state);
    let shutdown = CancellationToken::new();
    let server_shutdown = shutdown.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(server_shutdown.cancelled_owned())
            .await
    });

    let result = match tokio::time::timeout(timeout, result_rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(RuntimeOAuthCallbackError::Io(std::io::Error::other(
            agena_failure::diagnostic::format_error_chain_with_context(
                "oauth callback server stopped before returning a result",
                &error,
            ),
        ))),
        Err(error) => Err(RuntimeOAuthCallbackError::Provider(
            agena_failure::diagnostic::format_error_chain_with_context(
                format!(
                    "oauth callback timed out after {} seconds",
                    timeout.as_secs_f64()
                ),
                &error,
            ),
        )),
    };
    shutdown.cancel();
    match tokio::time::timeout(Duration::from_secs(1), &mut server).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => return Err(RuntimeOAuthCallbackError::Io(error)),
        Ok(Err(error)) => {
            return Err(RuntimeOAuthCallbackError::Io(std::io::Error::other(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "oauth callback server task failed",
                    &error,
                ),
            )));
        }
        Err(error) => tracing::warn!(
            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                "oauth callback server did not stop within the 1-second graceful shutdown window",
                &error,
            ),
            "aborting OAuth callback server after graceful shutdown timeout"
        ),
    }
    if !server.is_finished() {
        server.abort();
        if let Err(error) = server.await
            && !error.is_cancelled()
        {
            return Err(RuntimeOAuthCallbackError::Io(std::io::Error::other(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "oauth callback server failed while being aborted after its shutdown timeout",
                    &error,
                ),
            )));
        }
    }
    result
}

#[derive(Clone)]
struct OAuthCallbackState {
    port: u16,
    expected_state: Arc<str>,
    result: Arc<Mutex<Option<oneshot::Sender<Result<OAuthCallback, RuntimeOAuthCallbackError>>>>>,
}

async fn oauth_callback_handler(State(state): State<OAuthCallbackState>, uri: Uri) -> Response {
    let url = format!("http://localhost:{}{}", state.port, uri);
    let result = parse_oauth_callback_url(url.as_str(), Some(state.expected_state.as_ref()));
    let response = match &result {
        Ok(_) => (StatusCode::OK, Html(oauth_html_success().to_owned())).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Html(oauth_html_error(error.to_string().as_str())),
        )
            .into_response(),
    };
    if let Some(sender) = state
        .result
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        && sender.send(result).is_err()
    {
        tracing::warn!(
            "OAuth callback result receiver was dropped before the browser callback completed"
        );
    }
    response
}

fn oauth_html_success() -> &'static str {
    "<!doctype html><html><body><h1>Authorization Successful</h1><p>You can close this window.</p><script>setTimeout(() => window.close(), 1500)</script></body></html>"
}

fn oauth_html_error(error: &str) -> String {
    format!(
        "<!doctype html><html><body><h1>Authorization Failed</h1><p>{}</p></body></html>",
        html_escape::encode_text(error)
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

#[cfg(test)]
mod tests {
    use super::{
        RuntimeOAuthCallbackError, parse_oauth_callback_url, wait_for_oauth_callback_async,
    };
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn parses_authorization_code_and_state() {
        let callback = parse_oauth_callback_url(
            "http://localhost:8765/callback?code=authorization-code&state=expected-state",
            Some("expected-state"),
        )
        .expect("valid OAuth callback");

        assert_eq!(callback.code, "authorization-code");
        assert_eq!(callback.state, "expected-state");
        assert_eq!(callback.issuer, None);
    }

    #[test]
    fn preserves_rfc_9207_issuer_for_callback_validation() {
        let callback = parse_oauth_callback_url(
            "http://localhost:8765/callback?code=authorization-code&state=expected-state&iss=https%3A%2F%2Fissuer.example",
            Some("expected-state"),
        )
        .expect("valid callback issuer");
        assert_eq!(callback.issuer.as_deref(), Some("https://issuer.example"));
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
            html_escape::encode_text("<script>alert('x') & \"y\"</script>"),
            "&lt;script&gt;alert('x') &amp; \"y\"&lt;/script&gt;"
        );
    }

    #[tokio::test]
    async fn async_server_accepts_a_loopback_callback_and_stops() {
        let reservation = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("reserve loopback port");
        let port = reservation.local_addr().expect("local address").port();
        drop(reservation);

        let callback_task = tokio::spawn(async move {
            wait_for_oauth_callback_async(port, "expected", Duration::from_secs(2)).await
        });
        let mut stream = loop {
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        };
        stream
            .write_all(
                b"GET /callback?code=authorization-code&state=expected HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("send callback request");

        let callback = callback_task
            .await
            .expect("callback task")
            .expect("valid callback");
        assert_eq!(callback.code, "authorization-code");
        assert_eq!(callback.state, "expected");
    }
}
