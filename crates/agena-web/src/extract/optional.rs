use crate::{CrawlError, FetchedPage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
static EXTRACTORS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionBackend {
    #[default]
    Readability,
    Trafilatura,
}

/// Local HTML-only extraction. This adapter never fetches URLs or installs
/// dependencies. The original transport/canonical metadata remains attached.
pub async fn extract_with_backend(
    mut page: FetchedPage,
    html: &str,
    backend: ExtractionBackend,
) -> Result<FetchedPage, CrawlError> {
    if html.len() > MAX_SOURCE_BYTES {
        return Err(CrawlError::ResponseTooLarge {
            maximum_bytes: MAX_SOURCE_BYTES,
        });
    }
    if backend == ExtractionBackend::Trafilatura
        && page.content_status == crate::PageContentStatus::Readable
        && super::is_html_response(&page.content_type, html)
    {
        let operation = async {
            let _permit = EXTRACTORS.acquire().await.map_err(|error| {
                CrawlError::InvalidInput(format!("extractor admission failed: {error}"))
            })?;
            let python = std::env::var_os("AGENA_EXTRACT_PYTHON")
                .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into());
            let python = tokio::task::spawn_blocking(move || which::which(python)).await
                .map_err(|error| CrawlError::InvalidInput(format!("extractor discovery task failed: {error}")))?
                .map_err(|_| CrawlError::InvalidInput("Trafilatura needs Python with trafilatura installed; set AGENA_EXTRACT_PYTHON".into()))?;
            let mut command = tokio::process::Command::new(python);
            command.args(["-I", "-X", "utf8", "-c", include_str!("trafilatura.py")]);
            let output = agena_process::output_with_input(
                command,
                html.as_bytes(),
                Duration::from_secs(20),
                MAX_TEXT_BYTES + 4096,
            )
            .await?;
            if !output.status.success() {
                return Err(CrawlError::InvalidInput(format!(
                    "HTML extractor exited with {}; raw diagnostics omitted",
                    output.status
                )));
            }
            #[derive(Deserialize)]
            struct ResultBody {
                text: Option<String>,
                error: Option<String>,
            }
            let result: ResultBody = serde_json::from_slice(&output.stdout).map_err(|_| {
                CrawlError::InvalidInput("HTML extractor returned an invalid response".into())
            })?;
            if let Some(text) = result.text.filter(|text| !text.trim().is_empty()) {
                Ok(text)
            } else {
                Err(CrawlError::InvalidInput(match result.error.as_deref() {
                    Some("missing_dependency") => "AGENA_EXTRACT_PYTHON needs the trafilatura package; no automatic installation was attempted",
                    Some("too_large") => "Trafilatura text exceeded the 2 MiB extraction budget",
                    _ => "Trafilatura produced no usable text; inspect the page or explicitly choose readability",
                }.into()))
            }
        };
        page.markdown = tokio::time::timeout(Duration::from_secs(25), operation)
            .await
            .map_err(|_| {
                CrawlError::InvalidInput(
                    "HTML extraction timed out, including admission/startup".into(),
                )
            })??;
        page.extraction_strategy = "python_trafilatura".into();
    }
    page.extraction_backend = backend;
    let (text, clipped) = super::truncate_utf8(&page.markdown, MAX_TEXT_BYTES);
    page.markdown = text;
    if clipped {
        page.truncated = true;
        page.warnings
            .push("Extracted text was clipped to 2 MiB.".into());
    }
    page.title = crate::preview_text(&page.title, 512);
    if page.links.len() > 2000 {
        page.links.truncate(2000);
        page.warnings
            .push("Only the first 2000 unique page links were retained.".into());
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires AGENA_EXTRACT_PYTHON with trafilatura"]
    async fn real_extractor_fixture_comparison() {
        assert!(
            std::env::var_os("AGENA_EXTRACT_PYTHON").is_some(),
            "explicit fixture environment required"
        );
        let fixtures: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../tools/fixtures/web-extraction.json"
        ))
        .unwrap();
        let url = url::Url::parse("https://fixture.invalid/article").unwrap();
        let mut rows = Vec::new();
        for fixture in fixtures {
            let html = fixture["html"].as_str().unwrap();
            for backend in [
                ExtractionBackend::Readability,
                ExtractionBackend::Trafilatura,
            ] {
                let started = std::time::Instant::now();
                let baseline = crate::extract_page_from_body(
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
                let page = extract_with_backend(baseline.clone(), html, backend)
                    .await
                    .unwrap();
                let comparable = page.markdown.replace("\\_", "_");
                let facts = fixture["facts"].as_array().unwrap();
                let noise = fixture["noise"].as_array().unwrap();
                let retained: Vec<_> = facts
                    .iter()
                    .filter(|value| comparable.contains(value.as_str().unwrap()))
                    .collect();
                let remaining_noise: Vec<_> = noise
                    .iter()
                    .filter(|value| comparable.contains(value.as_str().unwrap()))
                    .collect();
                if backend == ExtractionBackend::Readability {
                    assert_eq!(retained.len(), facts.len());
                }
                assert!(!page.markdown.is_empty());
                assert!(
                    !retained.is_empty(),
                    "extractor must retain actual fixture facts"
                );
                rows.push(serde_json::json!({"fixture":fixture["name"],"backend":backend,"fact_count":facts.len(),"retained_facts":retained,"remaining_noise":remaining_noise,"fact_occurrences":facts.iter().map(|value|(value.as_str().unwrap(), comparable.matches(value.as_str().unwrap()).count())).collect::<std::collections::BTreeMap<_,_>>(),"characters":page.markdown.chars().count(),"bytes":page.markdown.len(),"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"markdown":page.markdown}));
            }
        }
        let evidence = serde_json::json!({"tested_on":"2026-10-04","trafilatura":"2.3.0","note":"Three small authored fixtures, one extraction per backend. Markdown escaped underscores are normalized only for marker counting. Fact/noise markers are inspectable, not a representative website quality ranking or latency benchmark. Timings include the existing extraction plus any selected adapter and are single observations, not benchmark estimates.","rows":rows});
        if let Some(path) = std::env::var_os("AGENA_EXTRACTION_EVIDENCE_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }
        eprintln!(
            "AUDIT_REAL_EXTRACTORS: {} fixture/backend comparisons completed",
            evidence["rows"].as_array().unwrap().len()
        );
    }
}
