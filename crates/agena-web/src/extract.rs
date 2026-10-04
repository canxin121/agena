use std::collections::HashSet;

use crw_extract::clean::clean_html;
use crw_extract::markdown::html_to_markdown;
use crw_extract::readability::{extract_links, extract_main_content, extract_metadata};
use url::Url;

use crate::{FetchedPage, resolve_link_url};

mod optional;
pub use optional::{ExtractionBackend, extract_with_backend};

pub fn robots_allows(body: &str, user_agent: &str, url: &str) -> bool {
    let product = user_agent
        .split(|ch: char| !ch.is_ascii_alphabetic() && ch != '-' && ch != '_')
        .next()
        .unwrap_or("");
    !product.is_empty()
        && robotstxt::DefaultMatcher::default().one_agent_allowed_by_robots(body, product, url)
}

/// Decode a bounded transport body using the declared HTTP charset. UTF-8 is
/// the fallback; the returned flag makes replacement characters explicit.
pub fn decode_response_body(bytes: &[u8], content_type: &str) -> (String, bool) {
    let label = content_type.split(';').skip(1).find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches(['"', '\'']))
    });
    let encoding = label
        .and_then(|value| encoding_rs::Encoding::for_label(value.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, _, errors) = encoding.decode(bytes);
    (text.into_owned(), errors)
}

#[allow(clippy::too_many_arguments)]
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
    let is_html =
        content_type.to_ascii_lowercase().starts_with("text/html") || looks_like_html(body);

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
        // A publisher's canonical URL does not change the base of relative
        // links in the fetched document. Honor HTML's explicit <base> instead.
        let base = html_base_url(body, final_url);
        let links = normalize_links(&base, extract_links(body, base.as_str()));
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
        final_url: final_url.to_string(),
        canonical_url: canonical_url.to_string(),
        title,
        markdown,
        content_type: content_type.to_string(),
        status,
        truncated,
        rendered,
        extraction_backend: ExtractionBackend::Readability,
        warnings: Vec::new(),
        raw_html_hash,
        etag,
        last_modified,
        links,
    }
}

fn html_base_url(body: &str, final_url: &Url) -> Url {
    let document = scraper::Html::parse_document(body);
    let selector = scraper::Selector::parse("base[href]").expect("fixed selector");
    document
        .select(&selector)
        .next()
        .and_then(|base| base.value().attr("href"))
        .and_then(|href| resolve_link_url(final_url, href))
        .unwrap_or_else(|| final_url.clone())
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
    use super::*;

    #[test]
    fn canonical_hints_do_not_change_the_base_of_relative_links() {
        let url = Url::parse("https://example.test/docs/page").unwrap();
        for (prefix, expected) in [
            ("", "https://example.test/docs/child"),
            (
                "<base href='/assets/'>",
                "https://example.test/assets/child",
            ),
        ] {
            let html = format!(
                "<html><head>{prefix}<link rel='canonical' href='https://elsewhere.test/news'></head><body><a href='child'>Document</a></body></html>"
            );
            let page = extract_page_from_body(
                &url,
                &url,
                "text/html",
                200,
                false,
                false,
                &html,
                None,
                None,
            );
            assert!(
                page.links.iter().any(|link| link == expected),
                "{:?}",
                page.links
            );
            assert_eq!(page.final_url, url.as_str());
        }
    }

    #[test]
    fn http_scheme_query_order_and_reference_values_are_preserved() {
        let raw = "http://example.test/a?ref=release&z=2&z=1&utm_source=semantic#fragment";
        assert_eq!(
            crate::prepare_fetch_url(raw).unwrap().as_str(),
            raw.split('#').next().unwrap()
        );
        assert!(crate::prepare_fetch_url("https://user:password@example.test/").is_err());
    }

    #[test]
    fn charset_decoding_and_specific_robots_groups_are_checked() {
        let (bytes, _, _) = encoding_rs::GBK.encode("中文网页");
        assert_eq!(
            decode_response_body(&bytes, "text/html; charset=\"gb18030\""),
            ("中文网页".into(), false)
        );
        assert!(decode_response_body(&[0xff], "text/plain").1);
        let robots = "User-agent: Agena\nDisallow: /\nAllow: /public\n\nUser-agent: *\nAllow: /\n";
        assert!(!robots_allows(
            robots,
            "Agena",
            "https://example.test/private"
        ));
        assert!(robots_allows(
            robots,
            "Agena",
            "https://example.test/public"
        ));
    }
}
