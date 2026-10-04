use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn fixture(
    status: &str,
    headers: &str,
    body: String,
) -> (Url, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/search", listener.local_addr().unwrap())).unwrap();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            assert!(request.len() < 32 * 1024);
            if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length = header
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map(|value| value.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        // Oversized-response tests intentionally close before all bytes arrive.
        let _ = socket.write_all(response.as_bytes()).await;
        String::from_utf8(request).unwrap()
    });
    (url, server)
}

fn options(provider: SearchApiProvider, url: &Url) -> SearchApiOptions<'_> {
    SearchApiOptions {
        provider,
        endpoint: url,
        resolved_addrs: &[],
        api_key: Some("fixture-secret"),
        limit: 2,
        timeout: Duration::from_secs(2),
        user_agent: "agena-fixture",
        allowed_domains: &[],
        blocked_domains: &[],
    }
}

#[tokio::test]
async fn all_backends_send_documented_requests_and_return_normalized_rows() {
    for (provider, body, description) in [
        (
            SearchApiProvider::Brave,
            json!({"type":"search", "web":{"results":[{"title":"Guide","url":"https://example.com/doc#section", "description":"<b>Brave</b> snippet"}]}}),
            "Brave snippet",
        ),
        (
            SearchApiProvider::Tavily,
            json!({"results":[{"title":"Guide","url":"https://example.com/doc", "content":"Tavily snippet"}],"usage":{"credits":1}}),
            "Tavily snippet",
        ),
        (
            SearchApiProvider::Exa,
            json!({"results":[{"title":"Guide","url":"https://example.com/doc", "highlights":["Exa snippet"]}],"costDollars":{"total":0.005}}),
            "Exa snippet",
        ),
        (
            SearchApiProvider::Searxng,
            json!({"results":[{"title":"Guide","url":"https://example.com/doc", "content":"SearXNG snippet"}]}),
            "SearXNG snippet",
        ),
    ] {
        let (url, server) = fixture("200 OK", "", body.to_string()).await;
        let result = search_api("中文 & rust", &options(provider, &url))
            .await
            .unwrap();
        assert_eq!(result.results.len(), 1);
        let row = &result.results[0];
        assert_eq!(row.title, "Guide");
        assert_eq!(row.url, "https://example.com/doc");
        assert_eq!(row.description, description);
        assert_eq!(row.source, "example.com");
        assert_eq!(row.engine, provider.label());
        assert!(!result.partial);
        let request = server.await.unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        let headers = headers.to_ascii_lowercase();
        let first = headers.lines().next().unwrap();
        assert!(!first.contains("fixture-secret"));
        match provider {
            SearchApiProvider::Brave => {
                assert!(headers.contains("x-subscription-token: fixture-secret"));
                let query = Url::parse(&format!(
                    "http://fixture{}",
                    first.split_whitespace().nth(1).unwrap()
                ))
                .unwrap();
                let pairs = query
                    .query_pairs()
                    .collect::<std::collections::BTreeMap<_, _>>();
                assert_eq!(pairs["q"], "中文 & rust");
                assert_eq!(pairs["count"], "2");
            }
            SearchApiProvider::Tavily => {
                assert!(headers.contains("authorization: bearer fixture-secret"));
                let body: Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["query"], "中文 & rust");
                assert_eq!(body["max_results"], 2);
                assert_eq!(body["search_depth"], "basic");
                assert_eq!(body["include_raw_content"], false);
                assert_eq!(result.usage, Some(json!({"credits":1})));
            }
            SearchApiProvider::Exa => {
                assert!(headers.contains("x-api-key: fixture-secret"));
                let body: Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["query"], "中文 & rust");
                assert_eq!(body["numResults"], 2);
                assert_eq!(body["contents"]["highlights"], true);
                assert_eq!(result.usage, Some(json!({"cost_usd":0.005})));
            }
            SearchApiProvider::Searxng => {
                assert!(first.contains("format=json"));
                assert!(!request.contains("fixture-secret"));
            }
        }
    }
}

#[test]
fn result_filters_deduplication_partial_status_and_unicode_budgets_are_explicit() {
    let body = json!({"results":[
        {"url":"https://docs.example.com/a", "content":"汉".repeat(2200)},
        {"url":"https://docs.example.com/a#duplicate"},
        {"url":"https://badexample.com/a"},
        {"url":"https://blocked.example.com/a"},
        {"url":"javascript:alert(1)"},
        {"url":"https://secret:credential@example.com/private"},
        {"url":"https://example.com/b"},
        {"url":"https://example.com/c"}
    ], "unresponsive_engines":[["engine-a", "timeout"]]});
    let result = parse_response(
        SearchApiProvider::Searxng,
        &body,
        2,
        &["example.com".into()],
        &["blocked.example.com".into()],
    )
    .unwrap();
    assert_eq!(result.results.len(), 2);
    assert_eq!(result.filtered_result_count, 3);
    assert_eq!(result.results[0].description.chars().count(), 2000);
    assert!(result.partial && result.truncated);
    assert_eq!(result.warnings.len(), 2);
    assert!(search_domain_allowed(
        "https://EXAMPLE.com./a",
        &["example.com".into()],
        &[]
    ));
    assert!(!search_domain_allowed("invalid", &[], &[]));
}

#[tokio::test]
async fn redirects_rate_limits_and_invalid_responses_do_not_leak_bodies_or_keys() {
    for (status, headers, body, expected) in [
        (
            "302 Found",
            "Location: http://127.0.0.1:1/leak\r\n",
            "fixture-secret",
            "HTTP 302",
        ),
        (
            "429 Too Many Requests",
            "Retry-After: 7\r\n",
            "fixture-secret",
            "retry after 7 seconds",
        ),
        ("200 OK", "", "fixture-secret", "invalid JSON"),
        (
            "200 OK",
            "",
            "{\"error\":\"fixture-secret\"}",
            "results array",
        ),
    ] {
        let (url, server) = fixture(status, headers, body.into()).await;
        let error = search_api("private query", &options(SearchApiProvider::Tavily, &url))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("fixture-secret") && !error.contains("private query"));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn response_and_result_budgets_are_enforced() {
    let (url, server) = fixture("200 OK", "", "x".repeat(MAX_RESPONSE_BYTES + 1)).await;
    let error = search_api("query", &options(SearchApiProvider::Brave, &url))
        .await
        .unwrap_err();
    assert!(matches!(error, CrawlError::ResponseTooLarge { .. }));
    server.await.unwrap();
    let rows = (0..30)
        .map(|i| json!({"url":format!("https://example.com/{i}")}))
        .collect::<Vec<_>>();
    let (url, server) = fixture("200 OK", "", json!({"results":rows}).to_string()).await;
    let mut options = options(SearchApiProvider::Exa, &url);
    options.limit = 50;
    let result = search_api("query", &options).await.unwrap();
    assert_eq!(result.results.len(), 20);
    assert_eq!(result.effective_limit, 20);
    assert!(result.truncated);
    server.await.unwrap();
}

#[tokio::test]
async fn authorized_dns_addresses_are_used_for_the_actual_connection() {
    let (url, server) = fixture("200 OK", "", "{\"results\":[]}".into()).await;
    let address = std::net::SocketAddr::new("127.0.0.1".parse().unwrap(), url.port().unwrap());
    let mut named_url = url.clone();
    named_url.set_host(Some("search.invalid")).unwrap();
    let addresses = [address];
    let mut options = options(SearchApiProvider::Searxng, &named_url);
    options.resolved_addrs = &addresses;
    assert!(
        search_api("query", &options)
            .await
            .unwrap()
            .results
            .is_empty()
    );
    let request = server.await.unwrap();
    assert!(request.contains("host: search.invalid:"));
}

#[tokio::test]
async fn chunked_responses_without_content_length_cannot_escape_the_body_budget() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/search", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        let chunk = format!("10000\r\n{}\r\n", "x".repeat(65536));
        for _ in 0..65 {
            if socket.write_all(chunk.as_bytes()).await.is_err() {
                break;
            }
        }
        let _ = socket.write_all(b"0\r\n\r\n").await;
    });
    let error = search_api("query", &options(SearchApiProvider::Searxng, &url))
        .await
        .unwrap_err();
    assert!(matches!(error, CrawlError::ResponseTooLarge { .. }));
    server.await.unwrap();
}

#[tokio::test]
async fn timeout_applies_while_waiting_for_the_response_body() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/search", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    let mut options = options(SearchApiProvider::Searxng, &url);
    options.timeout = Duration::from_millis(50);
    let error = search_api("query", &options).await.unwrap_err();
    assert!(matches!(error, CrawlError::Http(ref error) if error.is_timeout()));
    server.abort();
    let _ = server.await;
}

#[test]
fn empty_successes_are_distinct_from_malformed_successes() {
    assert!(
        parse_response(
            SearchApiProvider::Brave,
            &json!({"type":"search"}),
            5,
            &[],
            &[]
        )
        .unwrap()
        .results
        .is_empty()
    );
    assert!(
        parse_response(
            SearchApiProvider::Tavily,
            &json!({"results":[]}),
            5,
            &[],
            &[]
        )
        .unwrap()
        .results
        .is_empty()
    );
    assert!(parse_response(SearchApiProvider::Brave, &json!({}), 5, &[], &[]).is_err());
}
