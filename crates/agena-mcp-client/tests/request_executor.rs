//! Public request-executor contracts; all responses are in-memory fixtures.

use futures::StreamExt;
use http::{HeaderName, HeaderValue};
use rmcp::transport::streamable_http_client::{
    HttpRequestExecutor, RequestExecutingHttpClient, StreamableHttpClient, StreamableHttpError,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct FakeIo {
    status: u16,
    body: &'static str,
    requests: Arc<Mutex<Vec<reqwest::Request>>>,
}
impl HttpRequestExecutor for FakeIo {
    async fn execute(
        &self,
        request: reqwest::Request,
    ) -> Result<reqwest::Response, StreamableHttpError<reqwest::Error>> {
        self.requests.lock().unwrap().push(request);
        let response = http::Response::builder()
            .status(self.status)
            .header("content-type", "text/event-stream")
            .header("www-authenticate", "Bearer realm=\"worker\"")
            .body(reqwest::Body::from(self.body))
            .unwrap();
        Ok(response.into())
    }
}

#[tokio::test]
async fn delegated_io_keeps_session_headers_and_bounded_sse_parsing() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let client = RequestExecutingHttpClient::new(
        reqwest::Client::new(),
        FakeIo {
            status: 200,
            body: "data: larger than the configured event budget\n\n",
            requests: calls.clone(),
        },
    );
    let mut stream = client
        .get_stream_with_max_sse_event_size(
            "https://no-host-connection.invalid/mcp".into(),
            Some("session-one".into()),
            Some("event-two".into()),
            Some("credential".into()),
            HashMap::new(),
            16,
        )
        .await
        .unwrap();
    assert!(
        stream
            .next()
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("maximum size of 16 bytes")
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method(), reqwest::Method::GET);
    assert_eq!(calls[0].headers()["mcp-session-id"], "session-one");
    assert_eq!(calls[0].headers()["last-event-id"], "event-two");
    assert_eq!(calls[0].headers()["authorization"], "Bearer credential");
}

#[tokio::test]
async fn delegated_io_keeps_auth_challenges_and_rejects_reserved_header_override() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let client = RequestExecutingHttpClient::new(
        reqwest::Client::new(),
        FakeIo {
            status: 401,
            body: "",
            requests: calls.clone(),
        },
    );
    assert!(matches!(
        client
            .get_stream(
                "https://no-host-connection.invalid/mcp".into(),
                None,
                None,
                None,
                HashMap::new()
            )
            .await,
        Err(StreamableHttpError::AuthRequired(_))
    ));
    let headers = HashMap::from([(
        HeaderName::from_static("mcp-session-id"),
        HeaderValue::from_static("injected"),
    )]);
    assert!(matches!(
        client
            .get_stream(
                "https://no-host-connection.invalid/mcp".into(),
                None,
                None,
                None,
                headers
            )
            .await,
        Err(StreamableHttpError::ReservedHeaderConflict(_))
    ));
    assert_eq!(calls.lock().unwrap().len(), 1);
}
