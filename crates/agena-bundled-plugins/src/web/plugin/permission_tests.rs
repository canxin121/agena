use super::*;

use agena_plugin_host::sdk::PluginErrorKind;

#[tokio::test]
async fn mixed_search_sources_keep_success_and_report_denied_sources() {
    let input: CrawlWebSearchInput =
        serde_json::from_value(serde_json::json!({"query":"fixture","engine":"bing,google"}))
            .unwrap();
    let limits =
        search_aggregate::HtmlSearchLimits::from_input(&input, &WebSearchConfig::default());
    let partial = search_aggregate::search(
        &input,
        limits,
        Duration::from_secs(1),
        None,
        |engine, _, _| async move {
            if engine == WebSearchEngine::Bing {
                Ok(WebSearchResponse::default())
            } else {
                Err(PluginError::from_kind(
                    PluginErrorKind::PolicyDenied,
                    "fixture denial",
                ))
            }
        },
    )
    .await
    .unwrap();
    let partial = serde_json::to_value(partial).unwrap();
    assert_eq!(partial["partial"], true);
    assert_eq!(partial["successful_engines"], serde_json::json!(["bing"]));
    assert_eq!(partial["engine_reports"][0]["status"], "empty");
    assert_eq!(partial["engine_reports"][1]["status"], "unavailable");
}

#[test]
fn crawl_adapters_preserve_access_rejection_identity() {
    let error = PluginError::from_kind(PluginErrorKind::PolicyDenied, "fixture policy");
    let failure_id = error.failure.id;
    let error = crawl_error_to_plugin(plugin_error_to_crawl(error));
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    assert_eq!(error.failure.id, failure_id);
}
