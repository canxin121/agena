use super::*;
use WebSearchEngine::{Brave, Google, Naver, Yahoo, Yandex};

/// Optional diagnostic against public websites, never an Agena tool invocation.
#[tokio::test]
#[ignore = "contacts five public search websites; availability depends on the network"]
async fn live_public_html_smoke() {
    let mut available = 0;
    for engine in [Yandex, Google, Yahoo, Brave, Naver] {
        let options = WebSearchOptions {
            engine,
            limit: 3,
            max_pages: 1,
            timeout: std::time::Duration::from_secs(12),
            user_agent: "agena-web".into(),
        };
        match search_web("Rust ownership official documentation", &options).await {
            Ok(rows) => {
                assert!(rows.len() <= 3);
                for row in &rows {
                    assert_eq!(row.engine, engine.label());
                    assert!(Url::parse(&row.url).is_ok());
                    assert!(!row.title.is_empty());
                }
                available += usize::from(!rows.is_empty());
                eprintln!(
                    "PUBLIC_SEARCH_SAMPLE {}",
                    serde_json::json!({"engine":engine.label(),"results":rows.len(),"first_url":rows.first().map(|row| &row.url)})
                );
            }
            Err(error) => eprintln!(
                "PUBLIC_SEARCH_SAMPLE {}",
                serde_json::json!({"engine":engine.label(),"error":error.to_string()})
            ),
        }
    }
    assert!(
        available > 0,
        "no source returned results from this network"
    );
}

fn parse(engine: WebSearchEngine, html: &str) -> Result<Vec<WebSearchResult>, CrawlError> {
    parse_page(
        engine,
        html,
        &Url::parse(engine.permission_url()).unwrap(),
        10,
    )
}

#[test]
fn real_brave_and_naver_cards_keep_titles_snippets_and_destination_links() {
    for (engine, html, first) in [
        (
            Brave,
            include_str!("fixtures/brave.html"),
            "https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html",
        ),
        (
            Naver,
            include_str!("fixtures/naver.html"),
            "https://rust-lang.org/learn/",
        ),
    ] {
        let rows = parse(engine, html).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].url, first);
        for row in rows {
            assert!(!row.title.is_empty());
            assert!(!row.description.is_empty());
            assert_eq!(row.engine, engine.label());
            assert_eq!(
                row.source,
                Url::parse(&row.url).unwrap().host_str().unwrap()
            );
        }
    }
}

#[test]
fn google_yandex_and_yahoo_extract_card_metadata_and_unwrap_destinations() {
    for (engine, html) in [
        (
            Google,
            r#"<div class="MjjYud"><div class="g"><a href="/url?q=https%3A%2F%2Fexample.com%2FArticle%3Fid%3D1%26q%3Da%2526b"><h3>Guide</h3></a><div class="VwiC3b">Correct preview</div></div></div>"#,
        ),
        (
            Yandex,
            r#"<li class="serp-item"><h2 class="OrganicTitle"><a class="OrganicTitle-Link" href="https://example.com/Article?id=1&amp;q=a%26b">Guide</a></h2><div class="OrganicTextContentSpan">Correct preview</div></li>"#,
        ),
        (
            Yahoo,
            r#"<div class="algo-sr"><div class="compTitle"><h3><a href="https://r.search.yahoo.com/_ylt=x/RU=https%3A%2F%2Fexample.com%2FArticle%3Fid%3D1%26q%3Da%2526b/RK=2/RS=token">Guide</a></h3></div><div class="compText"><p>Correct preview</p></div></div>"#,
        ),
    ] {
        let rows = parse(engine, html).unwrap();
        assert_eq!(rows.len(), 1, "{engine}");
        assert_eq!(rows[0].url, "https://example.com/Article?id=1&q=a%26b");
        assert_eq!(rows[0].title, "Guide");
        assert_eq!(rows[0].description, "Correct preview");
    }
}

#[test]
fn cards_do_not_cross_assign_snippets_and_ads_do_not_become_results() {
    let html = r#"
      <div id="tads"><div class="g"><a href="https://ads.test"><h3>Ad</h3></a></div></div>
      <div class="g"><a href="https://example.com/1"><h3>One</h3></a></div>
      <div class="g"><a href="https://example.com/2"><h3>Two</h3></a><div class="VwiC3b">Second only</div></div>
      <div class="g"><a href="javascript:alert(1)"><h3>Bad</h3></a></div>
    "#;
    let rows = parse(Google, html).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].description, "");
    assert_eq!(rows[1].description, "Second only");
    let mut brave = include_str!("fixtures/brave.html").to_owned();
    brave.push_str(r#"<div class="snippet" data-type="ad"><a href="https://ads.test"><div class="title">Ad</div></a></div>"#);
    assert_eq!(parse(Brave, &brave).unwrap().len(), 2);
}

#[test]
fn challenges_consent_javascript_and_unknown_pages_are_errors_not_empty_successes() {
    for (engine, html, expected) in [
        (
            Yandex,
            r#"<form action="/checkcaptcha"><div class="CheckboxCaptcha">Verify</div></form>"#,
            "captcha_required",
        ),
        (
            Google,
            r#"<form id="captcha-form" action="/sorry/index">Verify</form>"#,
            "captcha_required",
        ),
        (
            Google,
            r#"<a href="/httpservice/retry/enablejs">Enable JavaScript</a>"#,
            "javascript_required",
        ),
        (
            Google,
            r#"<html><head><title>Google Search</title></head><body><noscript><a href="/httpservice/retry/enablejs?sei=fixture">here</a></noscript></body></html>"#,
            "javascript_required",
        ),
        (
            Yahoo,
            r#"<form action="https://consent.yahoo.com/v2/collectConsent">Continue</form>"#,
            "consent_required",
        ),
        (
            Brave,
            "<html><body>New layout</body></html>",
            "unrecognized_results_page",
        ),
        (Naver, "", "unrecognized_results_page"),
    ] {
        assert!(
            parse(engine, html)
                .unwrap_err()
                .to_string()
                .contains(expected)
        );
    }
    let mut normal = include_str!("fixtures/brave.html").to_owned();
    normal.push_str(
        r#"<script>const translations = { captcha: "Switch to traditional captcha" };</script>"#,
    );
    assert_eq!(parse(Brave, &normal).unwrap().len(), 2);
    let blocked = Url::parse("https://yandex.com/showcaptcha").unwrap();
    assert!(
        parse_page(Yandex, "<html></html>", &blocked, 10)
            .unwrap_err()
            .to_string()
            .contains("captcha_required")
    );
}

#[test]
fn explicit_empty_states_and_result_limits_are_respected() {
    for engine in [Yandex, Google, Yahoo, Brave, Naver] {
        assert!(
            parse(engine, "<div class='no-results'>No results found</div>")
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        parse_page(
            Brave,
            include_str!("fixtures/brave.html"),
            &Url::parse(Brave.permission_url()).unwrap(),
            1
        )
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn request_parameters_round_trip_unicode_and_query_operators_and_advance_pages() {
    let query = "Rust 所有权 site:example.com a&b + #";
    for (engine, key, page_key, page_value) in [
        (Yandex, "text", "p", "1"),
        (Google, "q", "start", "10"),
        (Yahoo, "p", "b", "11"),
        (Brave, "q", "offset", "1"),
        (Naver, "query", "start", "16"),
    ] {
        let url = page_url(engine, query, 1).unwrap();
        let pairs: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(pairs[key], query);
        assert_eq!(pairs[page_key], page_value);
        if engine == Google {
            assert_eq!(pairs["gws_rd"], "cr");
        }
        assert_eq!(
            url.host_str(),
            Url::parse(engine.permission_url()).unwrap().host_str()
        );
        assert!(url.fragment().is_none());
        assert_eq!(normalize_web_search_engine(engine.label()), Some(engine));
    }
}

#[test]
fn redirect_decoding_keeps_target_identity_and_rejects_navigation_or_non_http_targets() {
    let target = "https://example.com:8443/Article?a=1&a=2&q=x%2By#section";
    let expected = "https://example.com:8443/Article?a=1&a=2&q=x%2By";
    let encoded = urlencoding::encode(target);
    for (engine, wrapper) in [
        (Google, format!("/url?url={encoded}")),
        (
            Yahoo,
            format!("https://r.search.yahoo.com/path/RU={encoded}/RK=2/RS=x"),
        ),
        (
            Yandex,
            format!("https://yandex.com/clck/jsredir?url={encoded}"),
        ),
        (
            Naver,
            format!("https://search.naver.com/p/crd/rd?u={encoded}"),
        ),
    ] {
        assert_eq!(result_url(engine, &wrapper).as_deref(), Some(expected));
    }
    for (engine, url) in [
        (Google, "/search?q=next"),
        (Google, "/url?q=javascript%3Aalert(1)"),
        (Yahoo, "https://r.search.yahoo.com/bad"),
        (Naver, "https://keep.naver.com/"),
        (Brave, "/search?q=next"),
    ] {
        assert!(result_url(engine, url).is_none());
    }
}
