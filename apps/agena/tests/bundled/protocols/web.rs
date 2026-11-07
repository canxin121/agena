use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use agena_failure::FailureCategory;
use agena_testkit::{ModelFeedbackKind, SettingsLayer, TestAgenaBuilder, TestResult};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};

use super::super::{allow_permissions, fixture};

#[derive(Default)]
struct SearchPeer {
    requests: Mutex<Vec<(BTreeMap<String, String>, HeaderMap)>>,
    endpoint: OnceLock<String>,
    private_fetches: AtomicUsize,
}

async fn search(
    State(peer): State<Arc<SearchPeer>>,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    {
        let mut requests = peer.requests.lock().unwrap();
        if requests.len() >= 8 {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "search fixture request budget exceeded",
            )
                .into_response();
        }
        requests.push((query.clone(), headers));
    }
    if query
        .get("q")
        .is_some_and(|query| query == "search-http-error")
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [("retry-after", "1")],
            "fixture search unavailable",
        )
            .into_response();
    }
    Json(json!({
        "results": [
            {
                "url": "https://docs.fixture.example/article#first",
                "title": "Local protocol search",
                "content": "<b>loopback-search-result</b> from the local fixture"
            },
            {
                "url": "https://docs.fixture.example/article#duplicate",
                "title": "Duplicate",
                "content": "must be deduplicated"
            },
            {
                "url": "https://blocked.fixture.example/hidden",
                "title": "Blocked",
                "content": "must be filtered"
            },
            {
                "url": "https://outside.example/not-allowed",
                "title": "Outside allowlist",
                "content": "must be filtered"
            },
            {"title": "Malformed row without a URL"}
        ],
        "unresponsive_engines": []
    }))
    .into_response()
}

fn configured_fixture(setup_peer: Arc<SearchPeer>) -> TestAgenaBuilder {
    fixture()
        .expect_plugin("agena.web")
        .before_start(move |case| async move {
            let private_peer = setup_peer.clone();
            let server = case
                .serve_http(
                    Router::new()
                        .route("/search", get(search))
                        .route(
                            "/private",
                            get(move || {
                                let peer = private_peer.clone();
                                async move {
                                    peer.private_fetches.fetch_add(1, Ordering::SeqCst);
                                    "a public fetch must never reach this private fixture"
                                }
                            }),
                        )
                        .with_state(setup_peer.clone()),
                )
                .await?;
            setup_peer.endpoint.set(server.base_url()).unwrap();
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.web"]["package"] = json!({"kind": "static"});
            config["plugins"]["list"]["agena.web"]["settings"] = json!({
                "network": {"proxy": ""},
                "search": {
                    "provider": "searxng",
                    "endpoint": server.url("/search"),
                    "allow_private_endpoint": true,
                    "default_limit": 2,
                    "max_limit": 4
                }
            });
            case.write_config(SettingsLayer::Global, config).await
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub(super) async fn configured_search_uses_real_http() -> TestResult {
    let peer = Arc::new(SearchPeer::default());
    configured_fixture(peer.clone())
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let result = user
                .call_tool(
                    "web.search",
                    json!({
                        "query": "  local-search-marker  ",
                        "max_results": 2,
                        "allowed_domains": ["fixture.example"],
                        "blocked_domains": ["blocked.fixture.example"]
                    }),
                )
                .await?;
            result.assert_succeeded()?;
            result.assert_text_contains("loopback-search-result")?;
            let data = result.data()?;
            ensure!(
                data["engine"] == "searxng",
                "wrong configured search backend: {data}"
            );
            ensure!(data["query"] == "local-search-marker");
            ensure!(data["effective_limit"] == 2);
            ensure!(data["provider_result_count"] == 5);
            ensure!(data["filtered_result_count"] == 3);
            ensure!(
                data["partial"] == true,
                "malformed provider row was not reported"
            );
            let results = data["results"]
                .as_array()
                .context("search output has no result list")?;
            ensure!(
                results.len() == 1,
                "deduplication or domain filters failed: {data}"
            );
            ensure!(results[0]["url"] == "https://docs.fixture.example/article");
            ensure!(
                results[0]["description"] == "loopback-search-result from the local fixture",
                "search HTML snippet was not normalized: {data}"
            );

            user.call_tool("web.search", json!({"query": "search-http-error"}))
                .await?
                .assert_failed()?;
            user.call_tool(
                "web.search",
                json!({"query": "invalid-limit", "max_results": 0}),
            )
            .await?
            .assert_failed()?;

            // SearXNG's explicit private-endpoint allowance applies to that
            // configured service only. It must not weaken ordinary web.fetch.
            let private_url = format!(
                "{}/private",
                peer.endpoint
                    .get()
                    .context("search fixture endpoint missing")?
            );
            let private = user
                .call_tool(
                    "web.fetch",
                    json!({"url": private_url, "render_js": false, "use_cache": false}),
                )
                .await?;
            private.assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
            assert_eq!(
                private.failure()?.category,
                FailureCategory::PermissionDenied
            );
            ensure!(peer.private_fetches.load(Ordering::SeqCst) == 0);

            let records = peer.requests.lock().unwrap();
            ensure!(
                records.len() == 2,
                "search validation, retry or routing made unexpected requests: {records:#?}"
            );
            for (index, (query, headers)) in records.iter().enumerate() {
                let expected = if index == 0 {
                    "local-search-marker"
                } else {
                    "search-http-error"
                };
                ensure!(query.get("q").map(String::as_str) == Some(expected));
                ensure!(query.get("format").map(String::as_str) == Some("json"));
                ensure!(query.get("categories").map(String::as_str) == Some("general"));
                ensure!(
                    headers.get("accept").and_then(|value| value.to_str().ok())
                        == Some("application/json")
                );
                ensure!(headers.get("authorization").is_none());
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_search_permission_decline_precedes_http() -> TestResult {
    let peer = Arc::new(SearchPeer::default());
    configured_fixture(peer.clone())
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let mut policy =
                serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            policy["network"] = json!({
                "loopback":"ask", "private":"deny", "internet":"deny"
            });
            user.client()
                .set_session_permission(user.id(), serde_json::from_value(policy)?)
                .await?;

            let run = user
                .start_tool(
                    "web.search",
                    json!({"query":"declined-search-must-not-reach-service"}),
                )
                .await?;
            let permission = run.run().wait_permission().await?;
            let presented = serde_json::to_value(&permission.request.request)?;
            let target = format!("{}/search", peer.endpoint.get().context("search endpoint")?);
            assert_eq!(presented.pointer("/action/target"), Some(&json!(target)));
            ensure!(
                peer.requests.lock().unwrap().is_empty(),
                "search ran before user approval"
            );
            permission.mark_presented().await?;
            permission.deny_once().await?;
            let declined = run.wait_settled().await?;
            declined.assert_failure_kind(ModelFeedbackKind::UserDeclined)?;
            ensure!(
                peer.requests.lock().unwrap().is_empty(),
                "declined search contacted its provider"
            );
            assert_eq!(peer.private_fetches.load(Ordering::SeqCst), 0);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_open_validates_typed_targets_before_browser_or_network_io() -> TestResult {
    let peer = Arc::new(SearchPeer::default());
    configured_fixture(peer.clone())
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let private_url = format!(
                "{}/private",
                peer.endpoint.get().context("fixture endpoint")?
            );
            for input in [
                json!({}),
                json!({"url":"","preview_id":null}),
                json!({"preview_id":"  "}),
                json!({"url":private_url,"preview_id":"preview-1"}),
            ] {
                let invalid = user.call_tool("web.browser_open", input).await?;
                invalid.assert_failure_kind(ModelFeedbackKind::InvalidInput)?;
                assert_eq!(peer.private_fetches.load(Ordering::SeqCst), 0);
            }

            // These are syntactically valid single targets. They reach their
            // distinct authorization checks and must be classified as policy
            // denials, before Chrome can be acquired or contacted.
            let public_target = user
                .call_tool(
                    "web.browser_open",
                    json!({"url":format!("  {private_url}  ")}),
                )
                .await?;
            public_target.assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
            let unowned_preview = user
                .call_tool("web.browser_open", json!({"preview_id":"  preview-1  "}))
                .await?;
            unowned_preview.assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
            assert_eq!(peer.private_fetches.load(Ordering::SeqCst), 0);
            ensure!(peer.requests.lock().unwrap().is_empty());
            Ok(())
        })
        .await
}
