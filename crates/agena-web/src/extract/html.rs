//! Preserve explicit content regions; use Readability for unstructured pages.

use dom_query::Document;
use dom_smoothie::{Config, Readability};
use url::Url;

use crate::{PageContentStatus, resolve_link_url};

pub(super) struct ExtractedHtml {
    pub canonical_url: Url,
    pub title: String,
    pub markdown: String,
    pub links: Vec<String>,
    pub strategy: String,
    pub status: PageContentStatus,
    pub warnings: Vec<String>,
}

pub(super) fn extract(body: &str, final_url: &Url) -> ExtractedHtml {
    const MAX_ELEMENTS: usize = 50_000;
    const MAX_DEPTH: usize = 256;
    let document = Document::from(body);
    let base = document
        .select("base[href]")
        .attr("href")
        .and_then(|href| resolve_link_url(final_url, &href))
        .filter(|url| url.username().is_empty() && url.password().is_none())
        .unwrap_or_else(|| final_url.clone());
    let canonical_url = document
        .select("link[rel~='canonical'][href]")
        .attr("href")
        .and_then(|href| resolve_link_url(&base, &href))
        .filter(|url| url.username().is_empty() && url.password().is_none())
        .unwrap_or_else(|| final_url.clone());
    let title = [
        document.select("title").text().to_string(),
        document
            .select("meta[property='og:title']")
            .attr("content")
            .map(|s| s.to_string())
            .unwrap_or_default(),
        document.select_single("h1").text().to_string(),
    ]
    .into_iter()
    .find(|s| !s.trim().is_empty())
    .unwrap_or_else(|| final_url.to_string());
    let mut extracted = ExtractedHtml {
        canonical_url,
        title: title.trim().to_owned(),
        markdown: String::new(),
        links: Vec::new(),
        strategy: "dom_smoothie".into(),
        status: PageContentStatus::Readable,
        warnings: Vec::new(),
    };
    let elements = document.select("*");
    if elements.length() > MAX_ELEMENTS
        || elements
            .nodes()
            .iter()
            .any(|node| node.ancestors_it(Some(MAX_DEPTH)).count() >= MAX_DEPTH)
    {
        extracted.status = PageContentStatus::TooComplex;
        extracted.warnings.push("HTML exceeded the 50,000-element or 256-level extraction budget; no complete text is available.".into());
        return extracted;
    }
    let mut seen = std::collections::HashSet::new();
    for node in document.select("a[href]").nodes() {
        if let Some(url) = node
            .attr("href")
            .and_then(|href| resolve_link_url(&base, &href))
            && url.username().is_empty()
            && url.password().is_none()
            && seen.insert(url.to_string())
        {
            extracted.links.push(url.to_string());
            if extracted.links.len() == 2000 {
                extracted
                    .warnings
                    .push("Only the first 2000 unique page links were retained.".into());
                break;
            }
        }
    }
    // Detect document-level interstitials before removing their scripts/forms.
    // Words such as 'captcha' inside an ordinary article are not challenges.
    let title_lower = extracted.title.to_lowercase();
    let challenge = matches!(
        title_lower.trim_end_matches(['.', '…']),
        "just a moment"
            | "access denied"
            | "attention required! | cloudflare"
            | "verify you are human"
            | "robot or human?"
    ) || document
        .select("#challenge-form, #challenge-running, form[action*='/sorry/']")
        .exists();
    let has_script = document
        .select("script:not([type='application/ld+json']):not([type='application/json'])")
        .exists();
    let noscript = document.select("noscript").text().to_lowercase();
    let js_notice = noscript.contains("enable javascript")
        || noscript.contains("javascript is required")
        || noscript.contains("启用 javascript");
    document.select("script, style, noscript, template, iframe, canvas, svg, form, input, button, nav, [role='navigation'], [hidden], [aria-hidden='true'], body > header, body > footer").remove();

    // Resolve inline Markdown links against the actual document/base URL,
    // never the publisher's potentially unrelated canonical hint.
    for node in document.select("a[href], img[src]").nodes() {
        let attribute = if node.is("a") { "href" } else { "src" };
        let Some(raw) = node.attr(attribute) else {
            continue;
        };
        // Inline links keep fragments; crawl frontier URLs intentionally do not.
        if let Ok(url) = base.join(raw.trim())
            && matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
        {
            node.set_attr(attribute, url.as_str());
        } else {
            node.remove_attr(attribute);
        }
        node.remove_attr("srcset");
    }
    // MathML often carries an exact TeX representation alongside visual nodes.
    for annotation in document
        .select("annotation[encoding='application/x-tex']")
        .nodes()
    {
        let math = annotation
            .ancestors_it(None)
            .find(|ancestor| ancestor.is("math"));
        if let Some(math) = math {
            let tex = format!("${}$", annotation.text());
            math.set_text(tex);
        }
    }
    let main = document.select("main, [role='main']");
    let articles = document.select("article");
    let semantic_html = |selection: &dom_query::Selection<'_>| {
        let ids: std::collections::HashSet<_> =
            selection.nodes().iter().map(|node| node.id).collect();
        selection
            .nodes()
            .iter()
            .filter(|node| {
                !node
                    .ancestors_it(None)
                    .any(|ancestor| ids.contains(&ancestor.id))
            })
            .map(|node| node.html().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let selected = if main.exists() && !main.text().trim().is_empty() {
        extracted.strategy = "semantic_main".into();
        semantic_html(&main)
    } else if articles.exists() && !articles.text().trim().is_empty() {
        extracted.strategy = if articles.length() > 1 {
            "multiple_articles"
        } else {
            "semantic_article"
        }
        .into();
        semantic_html(&articles)
    } else {
        let config = Config {
            max_elements_to_parse: MAX_ELEMENTS,
            ..Default::default()
        };
        match Readability::new(document.html().as_ref(), Some(base.as_str()), Some(config))
            .and_then(|mut reader| reader.parse())
        {
            Ok(article) if !article.content.trim().is_empty() => article.content.to_string(),
            _ => {
                extracted.strategy = "clean_body".into();
                extracted
                    .warnings
                    .push("Readability found no article; returning cleaned page content.".into());
                document.select("body").html().to_string()
            }
        }
    };
    extracted.markdown = htmd::convert(&selected)
        .map(super::normalize_markdown)
        .unwrap_or_default();
    if extracted.markdown.is_empty() && !document.select("body").text().trim().is_empty() {
        extracted.markdown = document.select("body").text().trim().to_owned();
        extracted.strategy = "plain_text_fallback".into();
        extracted.warnings.push(
            "Markdown conversion produced no text; returning visible text without structure."
                .into(),
        );
    }
    let shell_text = document
        .select("body")
        .text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let sparse_shell = shell_text.is_empty()
        || (shell_text.chars().count() < 200
            && (shell_text.starts_with("enable javascript")
                || shell_text.starts_with("please enable javascript")
                || shell_text.starts_with("you need to enable javascript")
                || shell_text.starts_with("javascript is required")
                || matches!(
                    shell_text.trim_end_matches(['.', '…']),
                    "loading" | "please wait" | "加载中"
                )));
    extracted.status = if challenge {
        PageContentStatus::Blocked
    } else if sparse_shell && (js_notice || has_script) {
        PageContentStatus::RequiresJavascript
    } else if extracted.markdown.trim().is_empty() {
        PageContentStatus::Empty
    } else {
        PageContentStatus::Readable
    };
    match extracted.status {
        PageContentStatus::Blocked => extracted.warnings.push("The response is an access/verification page, not the requested content.".into()),
        PageContentStatus::RequiresJavascript => extracted.warnings.push("The HTTP response has little readable content and indicates JavaScript is required; browser rendering may be needed.".into()),
        PageContentStatus::Empty => extracted.warnings.push("The response contains no readable page text.".into()),
        _ => {}
    }
    extracted
}
