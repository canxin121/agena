use super::*;

fn page(body: &str, mime: &str) -> FetchedPage {
    let url = Url::parse("https://example.test/docs/page").unwrap();
    extract_page_from_body(&url, &url, mime, 200, false, false, body, None, None)
}

#[test]
fn authored_content_fixtures_retain_facts_without_navigation_noise() {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../tools/fixtures/web-content.json")).unwrap();
    for fixture in fixtures {
        let extracted = page(fixture["html"].as_str().unwrap(), "text/html");
        let text = format!(
            "{}\n{}",
            extracted.title,
            extracted.markdown.replace("\\_", "_")
        );
        for marker in fixture["facts"].as_array().unwrap() {
            assert!(
                text.contains(marker.as_str().unwrap()),
                "{} missing {marker}: {text}",
                fixture["name"]
            );
        }
        for marker in fixture["noise"].as_array().unwrap() {
            assert!(
                !text.contains(marker.as_str().unwrap()),
                "{} retained noise {marker}: {text}",
                fixture["name"]
            );
        }
        assert_eq!(extracted.content_status, PageContentStatus::Readable);
    }
}

#[test]
fn structured_code_tables_math_and_absolute_links_survive() {
    let extracted = page(
        r#"<base href='/v2/'><link rel='canonical' href='https://other.test/'>
        <main><h1>Reference</h1><pre><code class='language-rust'>fn main() {
    println!("hello");
}</code></pre><table><tr><th>Name</th><th>Value</th></tr><tr><td>size</td><td>42</td></tr></table>
        <a href='child'>Child</a><img src='image.png' alt='Diagram'>
        <math><semantics><mi>x</mi><annotation encoding='application/x-tex'>x^2</annotation></semantics></math>
        <details><summary>Explanation</summary>Hidden until opened</details></main>"#,
        "application/xhtml+xml",
    );
    for marker in [
        "```rust",
        "    println!",
        "| Name | Value |",
        "https://example.test/v2/child",
        "https://example.test/v2/image.png",
        "x^2",
        "Hidden until opened",
    ] {
        assert!(
            extracted.markdown.contains(marker),
            "missing {marker}: {}",
            extracted.markdown
        );
    }
    assert_eq!(extracted.canonical_url, "https://other.test/");
}

#[test]
fn mime_and_document_level_statuses_do_not_confuse_real_articles() {
    for mime in ["text/plain", "application/json", "text/markdown"] {
        assert_eq!(
            page("<main>literal</main>", mime).markdown,
            "<main>literal</main>"
        );
    }
    assert_eq!(
        page(
            "<html><body><script src='/app.js'></script><div id='root'></div></body></html>",
            "text/html"
        )
        .content_status,
        PageContentStatus::RequiresJavascript
    );
    assert_eq!(
        page(
            "<title>Just a moment...</title><form id='challenge-form'>Verify</form>",
            "text/html"
        )
        .content_status,
        PageContentStatus::Blocked
    );
    assert_eq!(page("<main><h1>How CAPTCHA works</h1><p>Enable JavaScript to try this demonstration.</p></main>", "text/html").content_status, PageContentStatus::Readable);
    assert_eq!(page("<div id='root'><main><h1>Status</h1><p>All systems operational.</p></main></div><script src='/app.js'></script>", "text/html").content_status, PageContentStatus::Readable);
    assert_eq!(
        page("<html><body></body></html>", "text/html").content_status,
        PageContentStatus::Empty
    );
}

#[test]
fn complexity_and_unicode_output_are_bounded() {
    let nested = format!("{}text{}", "<div>".repeat(260), "</div>".repeat(260));
    assert_eq!(
        page(&nested, "text/html").content_status,
        PageContentStatus::TooComplex
    );
    let many = "<p>x</p>".repeat(50_001);
    assert_eq!(
        page(&many, "text/html").content_status,
        PageContentStatus::TooComplex
    );
    let extracted = page(&"中文".repeat(400_000), "text/plain");
    assert!(extracted.truncated);
    assert!(extracted.markdown.len() <= 2 * 1024 * 1024);
    assert!(!extracted.markdown.contains('�'));
}

#[test]
fn sibling_articles_nested_main_and_fragment_links_remain_complete() {
    let articles = page(
        "<article><h2>问题</h2><p>怎么办？</p></article><article><h2>回答</h2><p>第一步打开文件。</p></article><article><p>第二步保存。</p></article>",
        "text/html",
    );
    assert_eq!(articles.extraction_strategy, "multiple_articles");
    for text in ["问题", "第一步打开文件", "第二步保存"] {
        assert!(articles.markdown.contains(text));
    }
    let nested = page(
        "<main><div role='main'><h1>Once only</h1><a href='child#section'>Child</a><a href='#here'>Here</a></div></main>",
        "text/html",
    );
    assert_eq!(nested.markdown.matches("Once only").count(), 1);
    assert!(
        nested
            .markdown
            .contains("https://example.test/docs/child#section")
    );
    assert!(
        nested
            .markdown
            .contains("https://example.test/docs/page#here")
    );
    assert!(
        nested
            .links
            .contains(&"https://example.test/docs/child".into())
    );
}

#[test]
fn javascript_shell_detection_covers_inline_scripts_without_misclassifying_short_help() {
    assert_eq!(
        page(
            "<h1>Loading...</h1><script>setTimeout(render, 1)</script>",
            "text/html"
        )
        .content_status,
        PageContentStatus::RequiresJavascript
    );
    assert_eq!(page("<main><h1>JavaScript help</h1><p>Enable JavaScript to try this demonstration.</p></main><script src='/app.js'></script>", "text/html").content_status, PageContentStatus::Readable);
}

#[test]
fn explicit_single_article_keeps_heading_even_with_an_unrelated_site_title() {
    let extracted = page(
        "<title>Site name</title><article><h1>Important heading</h1><p>A short answer.</p></article>",
        "text/html",
    );
    assert_eq!(extracted.extraction_strategy, "semantic_article");
    assert!(extracted.markdown.contains("Important heading"));
    assert!(extracted.markdown.contains("A short answer"));
}
