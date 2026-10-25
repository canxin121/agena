use super::*;

#[tokio::test]
async fn non_public_fetch_targets_keep_their_policy_denial_classification() {
    let url = url::Url::parse("http://127.0.0.1:9/private").unwrap();
    let error = resolve_public_network_target(&url).await.unwrap_err();
    assert_eq!(
        error.kind,
        agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
    );
    assert_eq!(
        error.failure.category,
        agena_failure::FailureCategory::PermissionDenied
    );
}
use agena_plugin_host::sdk::{
    PluginErrorKind,
    host_api::{HostCallbackContext, run_in_isolated_host_callback_context},
};

#[tokio::test]
async fn denied_search_sources_keep_typed_denial_and_partial_sources_stay_visible() {
    let input: CrawlWebSearchInput =
        serde_json::from_value(serde_json::json!({"query":"fixture","engine":"bing,google"}))
            .unwrap();
    let limits =
        search_aggregate::HtmlSearchLimits::from_input(&input, &WebSearchConfig::default());
    let denied =
        search_aggregate::search(&input, limits, Duration::from_secs(1), |_, _, _| async {
            Err::<WebSearchResponse, _>(PluginError::from_kind(
                PluginErrorKind::UserDeclined,
                "fixture decline",
            ))
        })
        .await
        .unwrap_err();
    assert_eq!(denied.kind, PluginErrorKind::UserDeclined);
    let partial = search_aggregate::search(
        &input,
        limits,
        Duration::from_secs(1),
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
    let failure_id = error.failure.id.clone();
    let error = crawl_error_to_plugin(plugin_error_to_crawl(error));
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    assert_eq!(error.failure.id, failure_id);
}

#[tokio::test]
async fn browser_denials_keep_their_kind_and_do_not_leak_into_later_calls() {
    let (commands, _receiver) = mpsc::channel(1);
    let errors = Arc::new(std::sync::Mutex::new(VecDeque::new()));
    let client = CdpClient {
        commands,
        navigation_interception_enabled: Arc::new(OnceLock::new()),
        navigation_errors: errors.clone(),
        callback_context: Arc::new(std::sync::Mutex::new(None)),
    };
    push_navigation_failure(
        &errors,
        Some((31, 46)),
        PluginError::from_kind(PluginErrorKind::PolicyDenied, "old callback expired"),
    );
    push_navigation_failure(
        &errors,
        Some((31, 47)),
        PluginError::from_kind(PluginErrorKind::UserDeclined, "current callback declined"),
    );
    let context = HostCallbackContext {
        session_id: Some(31),
        call_id: Some(47),
        ..Default::default()
    };
    run_in_isolated_host_callback_context(context, async {
        assert_eq!(
            client.take_navigation_error().unwrap().kind,
            PluginErrorKind::UserDeclined
        );
        assert!(client.take_navigation_error().is_none());
    })
    .await;
}
