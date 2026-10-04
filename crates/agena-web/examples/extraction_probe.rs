//! Extract local fixture HTML without any network or Agena tool calls.
use serde_json::{Value, json};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("supply fixture JSON")?;
    let fixtures: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    for fixture in fixtures {
        let html = fixture["html"].as_str().ok_or("missing HTML")?;
        let url = url::Url::parse(
            fixture["url"]
                .as_str()
                .unwrap_or("https://fixture.example/article"),
        )?;
        let page = agena_web::extract_page_from_body(
            &url,
            &url,
            "text/html",
            200,
            false,
            false,
            html,
            None,
            None,
        );
        let text = format!("{}\n{}", page.title, page.markdown.replace("\\_", "_"));
        let markers = |key: &str| {
            fixture[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| text.contains(v.as_str().unwrap()))
                .cloned()
                .collect::<Vec<_>>()
        };
        println!(
            "{}",
            json!({"fixture":fixture["name"],"backend":"agena_semantic_dom_smoothie_htmd", "facts_found":markers("facts"),"noise_found":markers("noise"),"page":page})
        );
    }
    Ok(())
}
