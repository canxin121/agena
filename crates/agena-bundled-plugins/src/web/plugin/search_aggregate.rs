//! Concurrent HTML search, conservative URL deduplication and rank fusion.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::time::Duration;

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeOutput};
use agena_web::{WebSearchEngine, WebSearchResponse, WebSearchResult, canonicalize_url};
use futures_util::future::join_all;
use serde::Serialize;

use super::{
    CrawlWebSearchInput, WebSearchConfig, WebSearchEngineSelection, clamp_limit, domain_allowed,
};

#[derive(Debug, Clone, Copy, Serialize)]
pub(super) struct HtmlSearchLimits {
    pub(super) max_results: usize,
    pub(super) max_results_per_engine: usize,
    pub(super) max_pages_per_engine: usize,
}

impl HtmlSearchLimits {
    pub(super) fn from_input(input: &CrawlWebSearchInput, config: &WebSearchConfig) -> Self {
        Self {
            max_results: clamp_limit(
                input.max_results,
                config.default_limit as usize,
                config.max_limit as usize,
            ),
            max_results_per_engine: clamp_limit(
                input.max_results_per_engine,
                config.default_results_per_engine as usize,
                config.max_results_per_engine as usize,
            ),
            max_pages_per_engine: clamp_limit(
                input.max_pages_per_engine,
                config.default_pages_per_engine as usize,
                config.max_pages_per_engine as usize,
            ),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct HtmlSearchOutput {
    query: String,
    engine: String,
    attempted_engines: Vec<String>,
    successful_engines: Vec<String>,
    limits: HtmlSearchLimits,
    results: Vec<MergedResult>,
    partial: bool,
    engine_errors: Vec<String>,
    engine_reports: Vec<EngineReport>,
}

#[derive(Debug, Serialize)]
struct EngineReport {
    engine: String,
    status: &'static str,
    result_count: usize,
    pages_fetched: usize,
    cache_hit: bool,
    rendered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue: Option<agena_web::SearchIssue>,
}

#[derive(Debug, Serialize)]
struct MergedResult {
    // Keep the existing title/url/description/source/engine fields. `engine`
    // identifies the best-ranked appearance supplying the displayed metadata.
    #[serde(flatten)]
    result: WebSearchResult,
    engines: Vec<String>,
}

pub(super) async fn search<F, Fut, Batch>(
    input: &CrawlWebSearchInput,
    limits: HtmlSearchLimits,
    timeout: Duration,
    search_engine: F,
) -> SdkResult<HtmlSearchOutput>
where
    F: Fn(WebSearchEngine, usize, usize) -> Fut,
    Fut: Future<Output = SdkResult<Batch>>,
    Batch: Into<WebSearchResponse>,
{
    let selection = input
        .engine
        .as_ref()
        .unwrap_or(&WebSearchEngineSelection::Auto);
    let engines = selection.engines();
    // Fixed fanout (at most eight). These futures run in the caller's task,
    // preserving host callback authority and cancellation without detached work.
    // Time out each complete source, including permission, pacing, DNS and all
    // pages. A slow source cannot discard another source's completed results.
    let outcomes = join_all(engines.iter().copied().map(|engine| {
        let search_engine = &search_engine;
        async move {
            tokio::time::timeout(
                timeout,
                search_engine(
                    engine,
                    limits.max_results_per_engine,
                    limits.max_pages_per_engine,
                ),
            )
            .await
            .unwrap_or_else(|_| {
                Err(super::search_error_to_plugin(
                    agena_web::CrawlError::SearchUnavailable {
                        provider: engine.label(),
                        issue: agena_web::SearchIssue {
                            kind: agena_web::SearchIssueKind::Timeout,
                            message: format!(
                                "search timed out after {} ms, including permission, DNS and queueing",
                                timeout.as_millis()
                            ),
                            http_status: None,
                            retry_after_secs: None,
                        },
                    },
                ))
            })
        }
    }))
    .await;

    // join_all preserves selection order, so network completion order cannot
    // affect rank ties, representative snippets, provenance or diagnostics.
    let mut batches = Vec::new();
    let mut successful_engines = Vec::new();
    let mut engine_errors = Vec::new();
    let mut engine_reports = Vec::new();
    for (engine, outcome) in engines.iter().copied().zip(outcomes) {
        match outcome {
            Ok(batch) => {
                let batch = batch.into();
                engine_reports.push(EngineReport {
                    engine: engine.to_string(),
                    status: if !batch.warnings.is_empty() {
                        "partial"
                    } else if batch.results.is_empty() {
                        "empty"
                    } else {
                        "ok"
                    },
                    result_count: batch.results.len(),
                    pages_fetched: batch.pages_fetched,
                    cache_hit: batch.cache_hit,
                    rendered: batch.rendered,
                    issue: batch.warnings.first().cloned(),
                });
                engine_errors.extend(
                    batch
                        .warnings
                        .iter()
                        .map(|issue| format!("{engine}: {issue}")),
                );
                successful_engines.push(engine.to_string());
                batches.push((engine, batch.results));
            }
            Err(error) if engines.len() == 1 => return Err(error),
            Err(error) => {
                engine_reports.push(EngineReport {
                    engine: engine.to_string(),
                    status: "unavailable",
                    result_count: 0,
                    pages_fetched: 0,
                    cache_hit: false,
                    rendered: false,
                    issue: error
                        .diagnostic
                        .data
                        .as_ref()
                        .and_then(|data| data.get("search_issue"))
                        .and_then(|value| serde_json::from_value(value.clone()).ok()),
                });
                engine_errors.push(format!("{engine}: {}", error.diagnostic_message()));
            }
        }
    }
    super::ensure_search_available(successful_engines.len(), &engine_errors).map_err(
        |mut error| {
            error.diagnostic.data = Some(serde_json::json!({"engine_reports": engine_reports}));
            error
        },
    )?;
    Ok(HtmlSearchOutput {
        query: input.query.trim().to_owned(),
        engine: selection.label(),
        attempted_engines: engines.iter().map(ToString::to_string).collect(),
        successful_engines,
        limits,
        results: merge_results(batches, input, limits.max_results),
        partial: !engine_errors.is_empty(),
        engine_errors,
        engine_reports,
    })
}

struct RankedResult {
    merged: MergedResult,
    score: f64,
    best_position: (usize, usize),
}

fn merge_results(
    batches: Vec<(WebSearchEngine, Vec<WebSearchResult>)>,
    input: &CrawlWebSearchInput,
    limit: usize,
) -> Vec<MergedResult> {
    // Reciprocal rank fusion: sum 1/(60 + rank), with one vote per engine/URL.
    // Scores are ranking signals, not probabilities or factual confidence.
    const RRF_K: f64 = 60.0;
    let mut merged: BTreeMap<String, RankedResult> = BTreeMap::new();
    for (engine_index, (engine, results)) in batches.into_iter().enumerate() {
        let mut seen = BTreeSet::new();
        // Source adapters already enforce retrieval budgets. Keep every fetched
        // candidate for filtering/fusion, including ranks below the final cap.
        for (index, mut result) in results.into_iter().enumerate() {
            let Ok(url) = canonicalize_url(&result.url) else {
                continue;
            };
            // Preserve query values/order/repeated keys, path case, scheme and
            // nondefault port. Fragments and default ports do not identify a
            // different fetched document. Do not resolve result links here.
            let url = url.to_string();
            if !domain_allowed(&url, &input.allowed_domains, &input.blocked_domains)
                || !seen.insert(url.clone())
            {
                continue;
            }
            result.url = url.clone();
            result.engine = engine.to_string();
            let position = (index + 1, engine_index);
            let score = 1.0 / (RRF_K + position.0 as f64);
            match merged.entry(url) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(RankedResult {
                        merged: MergedResult {
                            result,
                            engines: vec![engine.to_string()],
                        },
                        score,
                        best_position: position,
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let existing = entry.get_mut();
                    existing.score += score;
                    existing.merged.engines.push(engine.to_string());
                    if position < existing.best_position {
                        existing.best_position = position;
                        existing.merged.result = result;
                    }
                }
            }
        }
    }
    let mut ranked: Vec<_> = merged.into_values().collect();
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.best_position.cmp(&right.best_position))
            .then_with(|| left.merged.result.url.cmp(&right.merged.result.url))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|ranked| ranked.merged)
        .collect()
}

impl HtmlSearchOutput {
    pub(super) fn into_tool_output(self) -> SdkResult<ToolInvokeOutput> {
        let text = self.to_text();
        let summary = if self.attempted_engines.len() > 1 {
            format!(
                "{} results · {}/{} engines",
                self.results.len(),
                self.successful_engines.len(),
                self.attempted_engines.len()
            )
        } else {
            format!("{} results · {}", self.results.len(), self.engine)
        };
        let title = format!("web search {}", self.query);
        let payload =
            serde_json::to_value(self).map_err(|error| PluginError::internal_error(&error))?;
        Ok(ToolInvokeOutput::from_parts(
            title,
            summary,
            text,
            Some(payload),
            BTreeMap::new(),
            Vec::new(),
        ))
    }

    fn to_text(&self) -> String {
        let mode = if self.attempted_engines.len() > 1 {
            format!("{} (aggregated HTML)", self.engine)
        } else {
            self.engine.clone()
        };
        let mut lines = vec![format!(
            "Found {} web search result(s) for '{}' via {mode}.",
            self.results.len(),
            self.query
        )];
        lines.push(format!(
            "Attempted engines: {}. Successful engines: {}.",
            self.attempted_engines.join(", "),
            self.successful_engines.join(", ")
        ));
        lines.push(format!(
            "HTML budgets: up to {} candidates and {} page(s) per engine, including the first; final cap {}. Filtering/deduplication may return fewer; no automatic refill or global offset.",
            self.limits.max_results_per_engine,
            self.limits.max_pages_per_engine,
            self.limits.max_results,
        ));
        if self.partial {
            lines.push(format!(
                "Search partially degraded: {}",
                self.engine_errors.join("; ")
            ));
            if self.results.is_empty() {
                lines.push("No usable results were returned by the available sources. Unavailable engines did not establish that no matches exist; try another selected engine or a broader query, and respect retry_after_secs instead of immediately repeating blocked requests.".into());
            }
        }
        lines.push(format!(
            "Source results before filtering: {}.",
            self.engine_reports
                .iter()
                .map(|report| format!(
                    "{}={} ({}, {} page(s){})",
                    report.engine,
                    report.result_count,
                    report.status,
                    report.pages_fetched,
                    if report.cache_hit { ", cached" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ));
        if !self.results.is_empty() {
            lines.push("\nSnippets are previews, not fetched-page evidence. Fetch 1-3 relevant URLs before answering factual questions; use fetch with `prompt` for focused extraction. Multiple engines finding a URL does not independently verify its contents.".into());
        }
        for (index, merged) in self.results.iter().enumerate() {
            let result = &merged.result;
            lines.push(format!(
                "\n{}. {}\nURL: {}\nSource: {}\nEngines: {}\nDescription: {}",
                index + 1,
                result.title,
                result.url,
                result.source,
                merged.engines.join(", "),
                result.description
            ));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests;
