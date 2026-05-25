use std::collections::HashSet;

use crw_extract::clean::clean_html;
use crw_extract::markdown::html_to_markdown;
use crw_extract::readability::{extract_links, extract_main_content, extract_metadata};
use url::Url;

use crate::{FetchedPage, resolve_link_url};

pub fn extract_page_from_body(
    requested_url: &Url,
    final_url: &Url,
    content_type: &str,
    status: u16,
    truncated: bool,
    rendered: bool,
    body: &str,
    etag: Option<String>,
    last_modified: Option<String>,
) -> FetchedPage {
    let raw_html_hash = blake3::hash(body.as_bytes()).to_hex().to_string();
    let is_html = content_type.starts_with("text/html") || looks_like_html(body);

    let (canonical_url, title, markdown, links) = if is_html {
        let metadata = extract_metadata(body);
        let canonical_url = metadata
            .canonical_url
            .as_deref()
            .and_then(|value| resolve_link_url(final_url, value))
            .unwrap_or_else(|| final_url.clone());
        let title = metadata
            .title
            .or(metadata.og_title)
            .unwrap_or_else(|| canonical_url.as_str().to_string());
        let markdown = extract_markdown(body);
        let links = normalize_links(&canonical_url, extract_links(body, canonical_url.as_str()));
        (canonical_url, title, markdown, links)
    } else {
        (
            final_url.clone(),
            final_url.as_str().to_string(),
            body.trim().to_string(),
            Vec::new(),
        )
    };

    FetchedPage {
        url: requested_url.to_string(),
        canonical_url: canonical_url.to_string(),
        title,
        markdown,
        content_type: content_type.to_string(),
        status,
        truncated,
        rendered,
        raw_html_hash,
        etag,
        last_modified,
        links,
    }
}

pub fn truncate_utf8(input: &str, max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (input.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    (input[..end].to_string(), true)
}

pub fn looks_like_html(body: &str) -> bool {
    let head = body.trim_start();
    head.starts_with('<') || head.to_ascii_lowercase().contains("<html")
}

fn extract_markdown(body: &str) -> String {
    let empty_selectors: &[String] = &[];
    let cleaned = clean_html(body, false, empty_selectors, empty_selectors)
        .unwrap_or_else(|_| body.to_string());
    let main_html = extract_main_content(cleaned.as_str());
    let focused = if main_html.trim().is_empty() {
        cleaned.clone()
    } else {
        clean_html(main_html.as_str(), true, empty_selectors, empty_selectors).unwrap_or(main_html)
    };

    let focused_markdown = normalize_markdown(html_to_markdown(focused.as_str()));
    if !focused_markdown.is_empty() {
        return focused_markdown;
    }

    normalize_markdown(html_to_markdown(cleaned.as_str()))
}

fn normalize_markdown(markdown: String) -> String {
    markdown
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn normalize_links(base_url: &Url, extracted: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for link in extracted {
        let Some(url) = resolve_link_url(base_url, link.as_str()) else {
            continue;
        };
        let value = url.to_string();
        if seen.insert(value.clone()) {
            links.push(value);
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::extract_page_from_body;

    #[test]
    fn extract_page_prefers_main_content_and_canonical_url() {
        let requested = Url::parse("https://example.com/post?utm_source=test").expect("url");
        let final_url = Url::parse("https://example.com/post?utm_source=test").expect("url");
        let html = r#"
            <html lang="en">
              <head>
                <title>Ignored title</title>
                <link rel="canonical" href="/canonical-post" />
              </head>
              <body>
                <nav>navigation</nav>
                <article>
                  <h1>Main title</h1>
                  <p>Important content.</p>
                </article>
              </body>
            </html>
        "#;
        let page = extract_page_from_body(
            &requested,
            &final_url,
            "text/html",
            200,
            false,
            false,
            html,
            None,
            None,
        );
        assert_eq!(page.canonical_url, "https://example.com/canonical-post");
        assert!(page.markdown.contains("Important content."));
        assert!(!page.markdown.contains("navigation"));
    }
}
