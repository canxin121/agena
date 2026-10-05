use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[ignore = "diagnoses HTTP protocol compatibility with the public DuckDuckGo HTML endpoint"]
async fn live_duckduckgo_protocol_diagnostic() {
    for http1 in [true, false] {
        let mut builder = reqwest::Client::builder()
            .user_agent("agena-web")
            .cookie_store(true)
            .timeout(Duration::from_secs(15));
        if http1 {
            builder = builder.http1_only();
        }
        let client = builder.build().unwrap();
        match fetch(
            client,
            WebSearchEngine::DuckDuckGo,
            "Rust ownership official documentation".into(),
            0,
            None,
        )
        .await
        {
            Ok(page) => eprintln!(
                "DDG_PROTOCOL_SAMPLE {}",
                serde_json::json!({"http1":http1,"status":page.status,"bytes":page.html.len(),"parsed":status::parse_page(WebSearchEngine::DuckDuckGo,&page).map(|rows|rows.len()).map_err(|e|e.to_string())})
            ),
            Err(error) => eprintln!(
                "DDG_PROTOCOL_SAMPLE http1={http1} {}",
                SearchIssue::from_error(error)
            ),
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[test]
fn duckduckgo_continuations_preserve_opaque_tokens_and_reject_foreign_actions() {
    let mut page=WebSearchPage {final_url:url::Url::parse(DDG_HTML_URL).unwrap(),status:200,html:r#"<form method="post" action="/html/"><input name="q" value="Rust 中文"><input name="s" value="30"><input name="vqd" value="opaque&amp;token"><input type="submit" value="Next"></form>"#.into(),retry_after_secs:None};
    let next = duckduckgo_next(&page).unwrap();
    assert_eq!(next.url.as_str(), DDG_HTML_URL);
    let fields = next.form.unwrap();
    assert!(fields.contains(&("s".into(), "30".into())));
    assert!(fields.contains(&("vqd".into(), "opaque&token".into())));
    page.html = page.html.replace("/html/", "https://other.invalid/html/");
    assert!(duckduckgo_next(&page).is_none());
    page.html = "<form><input type=submit value=Previous></form>".into();
    assert!(duckduckgo_next(&page).is_none());
}

#[test]
fn retry_after_accepts_seconds_and_http_dates_but_is_bounded() {
    assert_eq!(retry_after("90"), Some(90));
    assert_eq!(retry_after("99999999999"), Some(86_400));
    let date = (chrono::Utc::now() + chrono::Duration::seconds(120))
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    assert!(matches!(retry_after(&date), Some(119..=120)));
    assert_eq!(retry_after("nonsense"), None);
}

#[tokio::test]
async fn response_status_and_retry_after_survive_body_reading_instead_of_generic_http_errors() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 2048];
        let count = stream.read(&mut bytes).await.unwrap();
        assert!(count > 0);
        stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 75\r\nContent-Type: text/html\r\nContent-Length: 99999999\r\n\r\n").await.unwrap();
        std::future::pending::<()>().await;
    });
    let response = reqwest::Client::new()
        .get(format!("http://{address}/"))
        .send()
        .await
        .unwrap();
    let page = tokio::time::timeout(Duration::from_millis(200), response_page(response))
        .await
        .expect("429 must not wait for the oversized, stalled response body")
        .unwrap();
    assert_eq!(page.status, 429);
    assert_eq!(page.retry_after_secs, Some(75));
    let error = status::parse_page(WebSearchEngine::Bing, &page).unwrap_err();
    assert!(matches!(
        error,
        CrawlError::SearchUnavailable {
            issue: SearchIssue {
                kind: SearchIssueKind::RateLimited,
                ..
            },
            ..
        }
    ));
    server.abort();
    let _ = server.await;
}
