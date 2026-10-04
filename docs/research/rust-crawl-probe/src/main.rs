use serde_json::{Value, json};

// Same extraction sequence as Agena ad235123's extract_markdown/normalize_markdown.
fn baseline(body: &str) -> String {
    let selectors: &[String] = &[];
    let cleaned = crw_extract::clean::clean_html(body, false, selectors, selectors)
        .unwrap_or_else(|_| body.to_string());
    let main = crw_extract::readability::extract_main_content(&cleaned);
    let focused = if main.trim().is_empty() {
        cleaned.clone()
    } else {
        crw_extract::clean::clean_html(&main, true, selectors, selectors).unwrap_or(main)
    };
    let normalize = |s: String| {
        s.lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    };
    let result = normalize(crw_extract::markdown::html_to_markdown(&focused));
    if result.is_empty() {
        normalize(crw_extract::markdown::html_to_markdown(&cleaned))
    } else {
        result
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("supply fixture JSON path")?;
    let fixtures: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    for fixture in fixtures {
        let html = fixture["html"].as_str().ok_or("missing HTML")?;
        let url = fixture["url"]
            .as_str()
            .unwrap_or("https://fixture.example/article");
        let smoothie = dom_smoothie::Readability::new(html, Some(url), None)
            .and_then(|mut r| r.parse())
            .map_err(|e| e.to_string())
            .and_then(|a| {
                htmd::convert(&a.content)
                    .map(|md| (a.title.to_string(), md, String::new()))
                    .map_err(|e| e.to_string())
            });
        let smoothie_xberg = dom_smoothie::Readability::new(html, Some(url), None)
            .and_then(|mut r| r.parse())
            .map_err(|e| e.to_string())
            .and_then(|a| {
                html_to_markdown_rs::convert(&a.content, None)
                    .map(|md| (a.title, md.content.unwrap_or_default(), String::new()))
                    .map_err(|e| e.to_string())
            });
        let tra = trafilatura::extract(html, &trafilatura::Options::default().with_links(true))
            .map(|a| {
                (
                    a.metadata.title.clone(),
                    a.content_markdown(),
                    a.comments_text.clone(),
                )
            })
            .map_err(|e| e.to_string());
        let rs_options = rs_trafilatura::Options {
            output_markdown: true,
            include_links: true,
            include_comments: true,
            include_tables: true,
            url: Some(url.to_string()),
            ..Default::default()
        };
        let rs = rs_trafilatura::extract_with_options(html, &rs_options)
            .map(|a| {
                (
                    a.metadata.title.unwrap_or_default(),
                    a.content_markdown.unwrap_or(a.content_text),
                    a.comments_text.unwrap_or_default(),
                )
            })
            .map_err(|e| e.to_string());
        let metadata = crw_extract::readability::extract_metadata(html);
        let baseline = (
            metadata.title.or(metadata.og_title).unwrap_or_default(),
            baseline(html),
            String::new(),
        );
        for (name, result) in [
            ("agena_crw_0.24.1", Ok(baseline)),
            ("dom_smoothie_0.18.2_htmd", smoothie),
            ("dom_smoothie_0.18.2_xberg_3.17.0", smoothie_xberg),
            ("trafilatura_0.3.0", tra),
            ("rs_trafilatura_0.2.2", rs),
        ] {
            let output = match result {
                Ok((title, markdown, comments)) => {
                    // Markdown escapes are formatting, not missing information.
                    let body_text = markdown.replace("\\_", "_");
                    let available_text = format!("{title}\n{body_text}\n{comments}");
                    let found: Vec<_> = fixture["facts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|m| available_text.contains(m))
                        .collect();
                    let body_found: Vec<_> = fixture["facts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|m| body_text.contains(m))
                        .collect();
                    let noise: Vec<_> = fixture["noise"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|m| available_text.contains(m))
                        .collect();
                    json!({"fixture":fixture["name"],"backend":name,"title":title,"comments":comments,"markdown_bytes":markdown.len(),"facts_found":found,"body_facts_found":body_found,"noise_found":noise,"markdown":markdown})
                }
                Err(error) => json!({"fixture":fixture["name"],"backend":name,"error":error}),
            };
            println!("{}", output);
        }
    }
    Ok(())
}
