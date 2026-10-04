use url::Url;

use crate::{FetchedPage, PageContentStatus};

mod html;

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
    let extracted = if is_html_response(content_type, body) {
        html::extract(body, final_url)
    } else {
        html::ExtractedHtml {
            canonical_url: final_url.clone(),
            title: final_url.to_string(),
            markdown: body.to_string(),
            links: Vec::new(),
            strategy: "passthrough".into(),
            status: if body.trim().is_empty() {
                PageContentStatus::Empty
            } else {
                PageContentStatus::Readable
            },
            warnings: Vec::new(),
        }
    };
    let (markdown, clipped) = truncate_utf8(&extracted.markdown, 2 * 1024 * 1024);
    let mut warnings = extracted.warnings;
    if clipped {
        warnings.push("Extracted text was clipped to 2 MiB.".into());
    }

    FetchedPage {
        url: requested_url.to_string(),
        final_url: final_url.to_string(),
        canonical_url: extracted.canonical_url.to_string(),
        title: crate::preview_text(&extracted.title, 512),
        markdown,
        content_type: content_type.to_string(),
        status,
        truncated: truncated || clipped,
        rendered,
        extraction_backend: ExtractionBackend::Readability,
        extraction_strategy: extracted.strategy,
        content_status: extracted.status,
        warnings,
        raw_html_hash,
        etag,
        last_modified,
        links: extracted.links,
    }
}

fn is_html_response(content_type: &str, body: &str) -> bool {
    match content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "text/html" | "application/xhtml+xml" => true,
        "" | "application/octet-stream" => looks_like_html(body),
        _ => false,
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

fn normalize_markdown(markdown: String) -> String {
    // Trailing spaces can be meaningful inside code and Markdown hard breaks.
    markdown.trim_matches('\n').to_string()
}

#[cfg(test)]
mod regression;

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
