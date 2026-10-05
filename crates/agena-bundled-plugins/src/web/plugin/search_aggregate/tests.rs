use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use super::*;
use WebSearchEngine::{Baidu, Bing, DuckDuckGo};

fn input() -> CrawlWebSearchInput {
    serde_json::from_value(serde_json::json!({"query":"fixture"})).unwrap()
}

#[tokio::test]
async fn partial_pagination_and_cached_sources_remain_visible_in_machine_and_model_feedback() {
    let input: CrawlWebSearchInput =
        serde_json::from_value(serde_json::json!({"query":"fixture","engine":"bing,baidu"}))
            .unwrap();
    let output = search(
        &input,
        limits(10),
        Duration::from_secs(2),
        |engine, _, _| async move {
            Ok(WebSearchResponse {
                results: vec![row(&format!("https://example.com/{engine}"), "Guide")],
                warnings: if engine == Bing {
                    vec![agena_web::SearchIssue {
                        kind: agena_web::SearchIssueKind::RateLimited,
                        message: "rate_limited: later page returned 429".into(),
                        http_status: Some(429),
                        retry_after_secs: Some(75),
                    }]
                } else {
                    Vec::new()
                },
                pages_fetched: 1,
                cache_hit: engine == Baidu,
                rendered: false,
            })
        },
    )
    .await
    .unwrap();
    assert_eq!(output.results.len(), 2);
    assert!(output.partial);
    let tool = output.into_tool_output().unwrap();
    assert!(tool.output_text.contains("retry after 75s"));
    let payload = tool.payload.unwrap();
    assert_eq!(payload["engine_reports"][0]["status"], "partial");
    assert_eq!(
        payload["engine_reports"][0]["issue"]["retry_after_secs"],
        75
    );
    assert_eq!(payload["engine_reports"][1]["cache_hit"], true);
}

fn limits(max_results: usize) -> HtmlSearchLimits {
    HtmlSearchLimits {
        max_results,
        max_results_per_engine: 10,
        max_pages_per_engine: 1,
    }
}

#[test]
fn discovery_schema_and_help_describe_aggregate_selection_and_total_limit() {
    use agena_plugin_host::sdk::Plugin;
    let manifest = super::super::WebPlugin::new().manifest();
    let tool = manifest
        .tools
        .iter()
        .find(|tool| tool.name == "search")
        .unwrap();
    let schema = tool.input_schema();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for engine in ["baidu,google", " BAIDU, google,baidu ", "auto", "bing"] {
        assert!(validator.is_valid(&serde_json::json!({"query":"fixture", "engine":engine})));
    }
    assert!(
        schema["properties"]["engine"]["description"]
            .as_str()
            .unwrap()
            .contains("concurrently")
    );
    assert!(
        schema["properties"]["max_results"]["description"]
            .as_str()
            .unwrap()
            .contains("Maximum total")
    );
    let help = tool.help_text().unwrap();
    for (name, maximum) in [("max_results_per_engine", 50), ("max_pages_per_engine", 5)] {
        assert_eq!(schema["properties"][name]["minimum"], 1);
        assert_eq!(schema["properties"][name]["maximum"], maximum);
        assert!(
            schema["properties"][name]["description"]
                .as_str()
                .unwrap()
                .contains("HTML only")
        );
    }
    assert!(schema["properties"].get("offset").is_none());
    for behavior in [
        "HTML auto",
        "concurrently",
        "deduplicates",
        "engine_errors",
        "not fetched-page evidence",
        "max_results_per_engine",
        "max_pages_per_engine",
        "never triggers extra pages",
        "No global offset",
        "comma-separated list",
        "baidu,google",
        "auto must stand alone",
    ] {
        assert!(help.contains(behavior));
    }
}

fn row(url: &str, title: &str) -> WebSearchResult {
    WebSearchResult {
        title: title.into(),
        url: url.into(),
        description: format!("Preview of {title}"),
        source: "Example publisher".into(),
        engine: String::new(),
    }
}

#[tokio::test(start_paused = true)]
async fn auto_starts_all_sources_concurrently_and_completion_order_does_not_change_results() {
    let mut snapshots = Vec::new();
    for reverse in [false, true] {
        let mut input = input();
        if reverse {
            input.engine = Some(WebSearchEngineSelection::Auto);
        }
        // No source can complete until all eight have started. Serial dispatch
        // would hit the source deadlines instead of returning these results.
        let barrier = tokio::sync::Barrier::new(8);
        let output = search(&input, limits(5), Duration::from_secs(1), |engine, _, _| {
            let barrier = &barrier;
            async move {
                barrier.wait().await;
                let delay = match engine {
                    DuckDuckGo => 10,
                    Bing => 20,
                    Baidu => 30,
                    _ => 15,
                };
                tokio::time::sleep(Duration::from_millis(if reverse {
                    40 - delay
                } else {
                    delay
                }))
                .await;
                Ok(vec![
                    row(&format!("https://example.com/{engine}"), engine.as_ref()),
                    row("https://example.com/shared", "Shared"),
                ])
            }
        })
        .await
        .unwrap();
        assert_eq!(
            output.attempted_engines,
            [
                "duckduckgo",
                "bing",
                "baidu",
                "yandex",
                "google",
                "yahoo",
                "brave",
                "naver"
            ]
        );
        assert_eq!(output.successful_engines, output.attempted_engines);
        assert!(!output.partial);
        assert_eq!(output.results.len(), 5);
        assert_eq!(output.results[0].result.url, "https://example.com/shared");
        snapshots.push(serde_json::to_value(output).unwrap());
    }
    assert_eq!(snapshots[0], snapshots[1]);
}

struct Active(Arc<AtomicUsize>);

impl Active {
    fn new(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count.clone())
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn source_error_and_whole_source_timeout_keep_completed_results_and_cancel_pending_work() {
    let active = Arc::new(AtomicUsize::new(0));
    let started = tokio::time::Instant::now();
    let output = search(
        &input(),
        limits(5),
        Duration::from_secs(5),
        |engine, _, _| {
            let active = &active;
            async move {
                let _guard = Active::new(active);
                match engine {
                    DuckDuckGo => Ok(vec![row("https://example.com/guide", "Guide")]),
                    Bing => Err(PluginError::internal("fixture HTTP 503")),
                    Baidu => {
                        // Simulated pages each fit within the deadline, but their
                        // total must still be capped by one whole-source deadline.
                        for _ in 0..3 {
                            tokio::time::sleep(Duration::from_secs(3)).await;
                        }
                        Ok(vec![row("https://example.com/late", "Late")])
                    }
                    _ => Err(PluginError::internal("fixture captcha_required")),
                }
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(5)
    );
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(output.results.len(), 1);
    assert_eq!(output.successful_engines, ["duckduckgo"]);
    assert!(output.partial);
    assert_eq!(output.engine_errors.len(), 7);
    assert!(output.engine_errors[0].contains("fixture HTTP 503"));
    assert!(output.engine_errors[1].contains("timed out after 5000 ms"));
    let tool = output.into_tool_output().unwrap();
    assert!(tool.output_text.contains("Search partially degraded"));
    assert!(tool.output_text.contains("bing:"));
    assert!(tool.output_text.contains("baidu:"));
    assert_eq!(tool.payload.unwrap()["partial"], true);
}

#[tokio::test(start_paused = true)]
async fn caller_cancellation_drops_every_source_without_detached_tasks() {
    let active = Arc::new(AtomicUsize::new(0));
    let input = input();
    let mut future = Box::pin(search(
        &input,
        limits(5),
        Duration::from_secs(30),
        |_, _, _| {
            let active = &active;
            async move {
                let _guard = Active::new(active);
                std::future::pending::<SdkResult<Vec<WebSearchResult>>>().await
            }
        },
    ));
    assert!(futures_util::poll!(&mut future).is_pending());
    assert_eq!(active.load(Ordering::SeqCst), 8);
    drop(future);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn all_failed_or_timed_out_is_an_error_but_successful_empty_searches_are_valid() {
    let failed = search(
        &input(),
        limits(5),
        Duration::from_secs(1),
        |engine, _, _| async move {
            if engine == Baidu {
                std::future::pending::<()>().await;
            }
            Err::<Vec<WebSearchResult>, _>(PluginError::internal("fixture failure"))
        },
    )
    .await
    .unwrap_err();
    let message = failed.diagnostic_message();
    assert!(message.contains("all search engines failed"));
    assert_eq!(
        failed.diagnostic.data.as_ref().unwrap()["engine_reports"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(
        failed.failure.recovery,
        agena_failure::RecoveryDirective::ChooseAlternative
    );
    for engine in WebSearchEngine::ALL {
        assert!(message.contains(engine.label()));
    }

    let empty = search(
        &input(),
        limits(5),
        Duration::from_secs(1),
        |engine, _, _| async move {
            if engine == Bing {
                Err::<Vec<WebSearchResult>, _>(PluginError::internal("fixture failure"))
            } else {
                Ok(Vec::new())
            }
        },
    )
    .await
    .unwrap();
    assert!(empty.results.is_empty());
    assert!(empty.partial);
    assert_eq!(empty.successful_engines.len(), 7);
    assert!(
        !empty
            .successful_engines
            .iter()
            .any(|engine| engine == "bing")
    );
    assert!(empty.to_text().contains("Search partially degraded"));

    let empty = search(
        &input(),
        limits(5),
        Duration::from_secs(1),
        |_, _, _| async { Ok(Vec::new()) },
    )
    .await
    .unwrap();
    assert!(empty.results.is_empty());
    assert!(!empty.partial);
    assert_eq!(empty.successful_engines.len(), 8);
}

#[tokio::test]
async fn explicit_selection_only_queries_that_engine_and_preserves_its_error_kind() {
    for expected in WebSearchEngine::ALL {
        let selection: WebSearchEngineSelection =
            serde_json::from_value(serde_json::json!(expected.label())).unwrap();
        let mut input = input();
        input.engine = Some(selection);
        let called = Mutex::new(Vec::new());
        let output = search(&input, limits(5), Duration::from_secs(1), |engine, _, _| {
            called.lock().unwrap().push(engine);
            async { Ok(vec![row("https://example.com/guide", "Guide")]) }
        })
        .await
        .unwrap();
        assert_eq!(*called.lock().unwrap(), [expected]);
        assert_eq!(output.engine, expected.as_ref());
        assert_eq!(output.results[0].engines, [expected.as_ref()]);

        let error = search(&input, limits(5), Duration::from_secs(1), |_, _, _| async {
            Err::<Vec<WebSearchResult>, _>(PluginError::invalid_params("fixture policy check"))
        })
        .await
        .unwrap_err();
        assert_eq!(error.kind, PluginError::invalid_params("fixture").kind);
    }
}

#[tokio::test(start_paused = true)]
async fn explicit_csv_queries_only_unique_selected_sources_concurrently_with_stable_fusion() {
    let input: CrawlWebSearchInput = serde_json::from_value(serde_json::json!({
        "query":"fixture", "engine":" BAIDU, Google,baidu ",
        "max_results_per_engine": 25, "max_pages_per_engine": 3, "max_results": 3
    }))
    .unwrap();
    let limits = HtmlSearchLimits::from_input(&input, &WebSearchConfig::default());
    let mut snapshots = Vec::new();
    for reverse in [false, true] {
        let barrier = tokio::sync::Barrier::new(2);
        let called = Mutex::new(Vec::new());
        let output = search(
            &input,
            limits,
            Duration::from_secs(1),
            |engine, candidates, pages| {
                called.lock().unwrap().push(engine);
                assert!(matches!(engine, Baidu | WebSearchEngine::Google));
                assert_eq!((candidates, pages), (25, 3));
                let barrier = &barrier;
                async move {
                    barrier.wait().await;
                    tokio::time::sleep(Duration::from_millis(if (engine == Baidu) ^ reverse {
                        5
                    } else {
                        10
                    }))
                    .await;
                    Ok(vec![
                        row(&format!("https://example.com/{engine}"), engine.label()),
                        row("https://example.com/shared", engine.label()),
                    ])
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(
            called.into_inner().unwrap(),
            [Baidu, WebSearchEngine::Google]
        );
        assert_eq!(output.engine, "baidu,google");
        assert_eq!(output.attempted_engines, ["baidu", "google"]);
        assert_eq!(output.successful_engines, output.attempted_engines);
        assert_eq!(output.results.len(), 3);
        assert_eq!(output.results[0].result.url, "https://example.com/shared");
        assert_eq!(output.results[0].engines, ["baidu", "google"]);
        assert_eq!(output.results[0].result.engine, "baidu");
        assert!(!output.partial);
        assert!(output.to_text().contains("baidu,google (aggregated HTML)"));
        snapshots.push(output.into_tool_output().unwrap().payload.unwrap());
    }
    assert_eq!(snapshots[0], snapshots[1]);
}

#[tokio::test(start_paused = true)]
async fn explicit_csv_preserves_success_on_error_or_timeout_and_reports_all_failed() {
    let input =
        serde_json::from_value(serde_json::json!({"query":"fixture", "engine":"baidu,google"}))
            .unwrap();
    for timeout in [false, true] {
        let output = search(
            &input,
            limits(5),
            Duration::from_secs(1),
            |engine, _, _| async move {
                if engine == Baidu {
                    return Ok(vec![row("https://example.com/guide", "Guide")]);
                }
                if timeout {
                    std::future::pending::<()>().await;
                }
                Err::<Vec<WebSearchResult>, _>(PluginError::internal("fixture failure"))
            },
        )
        .await
        .unwrap();
        assert!(output.partial);
        assert_eq!(output.successful_engines, ["baidu"]);
        assert_eq!(output.results.len(), 1);
        assert_eq!(output.engine_errors.len(), 1);
        assert!(output.engine_errors[0].starts_with("google:"));
    }

    let error = search(&input, limits(5), Duration::from_secs(1), |_, _, _| async {
        Err::<Vec<WebSearchResult>, _>(PluginError::internal("fixture failure"))
    })
    .await
    .unwrap_err();
    assert!(
        error
            .diagnostic_message()
            .contains("all search engines failed")
    );
    assert!(error.diagnostic_message().contains("baidu:"));
    assert!(error.diagnostic_message().contains("google:"));

    let input =
        serde_json::from_value(serde_json::json!({"query":"fixture", "engine":"baidu,BAIDU"}))
            .unwrap();
    let error = search(&input, limits(5), Duration::from_secs(1), |_, _, _| async {
        Err::<Vec<WebSearchResult>, _>(PluginError::invalid_params("single-source failure"))
    })
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginError::invalid_params("fixture").kind);
    assert_eq!(error.diagnostic_message(), "single-source failure");
}

#[test]
fn fusion_promotes_consensus_deduplicates_each_source_and_keeps_metadata_attribution() {
    let results = merge_results(
        vec![
            (
                DuckDuckGo,
                vec![
                    row("https://example.com/first", "First"),
                    row("https://EXAMPLE.com:443/shared#section", "DDG shared"),
                    row("https://example.com/shared", "Duplicate"),
                ],
            ),
            (Bing, vec![row("https://example.com/shared", "Bing shared")]),
            (Baidu, vec![row("https://example.com/third", "Third")]),
        ],
        &input(),
        3,
    );
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].result.url, "https://example.com/shared");
    assert_eq!(results[0].engines, ["duckduckgo", "bing"]);
    assert_eq!(results[0].result.engine, "bing");
    assert_eq!(results[0].result.title, "Bing shared");
    assert_eq!(results[0].result.description, "Preview of Bing shared");
    assert_eq!(results[0].result.source, "Example publisher");
    assert_eq!(results[1].result.title, "First");
    assert_eq!(results[2].result.title, "Third");

    // Repeated rows from one source must not outrank a genuinely shared URL.
    let results = merge_results(
        vec![
            (
                DuckDuckGo,
                vec![row("https://example.com/spam", "Spam"); 10],
            ),
            (Bing, vec![row("https://example.com/shared", "Shared")]),
            (Baidu, vec![row("https://example.com/shared", "Shared")]),
        ],
        &input(),
        10,
    );
    assert_eq!(results[0].result.url, "https://example.com/shared");
    assert_eq!(results[1].engines, ["duckduckgo"]);
}

#[test]
fn fusion_preserves_url_identity_including_queries_case_scheme_ports_and_encoding() {
    let urls = [
        "https://example.com/article?id=1",
        "https://example.com/article?id=2",
        "https://example.com/Article?id=1",
        "http://example.com/article?id=1",
        "https://example.com:8443/article?id=1",
        "https://example.com/article?a=1&b=2",
        "https://example.com/article?b=2&a=1",
        "https://example.com/article?a=1&a=2",
        "https://example.com/article?a=2&a=1",
        "https://example.com/a%2Fb?q=x%26y%3Dz&signature=A%2BB",
        "https://example.com/a/b?q=x&y=z&signature=A+B",
        "https://example.com/",
    ];
    let mut rows: Vec<_> = urls.iter().map(|url| row(url, "Fixture")).collect();
    rows.push(row("https://EXAMPLE.com:443#fragment", "Same root"));
    rows.push(row(
        "https://example.com/article?id=1#section",
        "Same article",
    ));
    rows.push(row(
        "https://user:secret@example.com/private",
        "Invalid credentials",
    ));
    rows.push(row("javascript:alert(1)", "Invalid scheme"));
    rows.push(row("not a URL", "Invalid URL"));
    let results = merge_results(vec![(DuckDuckGo, rows)], &input(), 50);
    let actual: Vec<_> = results.iter().map(|row| row.result.url.as_str()).collect();
    assert_eq!(actual, urls);
}

#[test]
fn filters_apply_before_final_limit_and_exclusions_win_for_every_source() {
    let mut input = input();
    input.allowed_domains = vec!["example.com".into()];
    input.blocked_domains = vec!["ads.example.com".into()];
    let results = merge_results(
        vec![
            (
                DuckDuckGo,
                vec![
                    row("https://ads.example.com", "Blocked"),
                    row("https://docs.example.com/1", "One"),
                ],
            ),
            (
                Bing,
                vec![
                    row("https://other.test", "Outside"),
                    row("https://example.com/2", "Two"),
                ],
            ),
            (
                Baidu,
                vec![
                    row("https://example.com/3", "Three"),
                    row("https://example.com/4", "Four"),
                ],
            ),
        ],
        &input,
        2,
    );
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].result.title, "Three");
    assert_eq!(results[1].result.title, "One");
}

#[tokio::test]
async fn payload_and_text_expose_aggregate_provenance_and_preserve_existing_result_fields() {
    let tool = search(
        &input(),
        limits(1),
        Duration::from_secs(1),
        |_, _, _| async { Ok(vec![row("https://example.com/guide", "Guide")]) },
    )
    .await
    .unwrap()
    .into_tool_output()
    .unwrap();
    let payload = tool.payload.unwrap();
    assert_eq!(payload["engine"], "auto");
    assert_eq!(
        payload["limits"],
        serde_json::json!({
            "max_results": 1, "max_results_per_engine": 10, "max_pages_per_engine": 1
        })
    );
    assert!(
        tool.output_text
            .contains("10 candidates and 1 page(s) per engine")
    );
    assert_eq!(payload["results"].as_array().unwrap().len(), 1);
    let result = &payload["results"][0];
    assert_eq!(result["title"], "Guide");
    assert_eq!(result["url"], "https://example.com/guide");
    assert_eq!(result["description"], "Preview of Guide");
    assert_eq!(result["source"], "Example publisher");
    assert_eq!(result["engine"], "duckduckgo");
    assert_eq!(
        result["engines"],
        serde_json::json!([
            "duckduckgo",
            "bing",
            "baidu",
            "yandex",
            "google",
            "yahoo",
            "brave",
            "naver"
        ])
    );
    assert!(tool.output_text.contains("aggregated HTML"));
    assert!(
        tool.output_text
            .contains("Engines: duckduckgo, bing, baidu")
    );
    assert!(tool.output_text.contains("not fetched-page evidence"));
}

#[test]
fn search_budgets_use_independent_defaults_and_configured_ceilings() {
    let defaults = HtmlSearchLimits::from_input(&input(), &WebSearchConfig::default());
    assert_eq!(defaults.max_results, 5);
    assert_eq!(defaults.max_results_per_engine, 10);
    assert_eq!(defaults.max_pages_per_engine, 1);

    let config = WebSearchConfig {
        default_limit: 3,
        max_limit: 12,
        default_results_per_engine: 9,
        max_results_per_engine: 25,
        default_pages_per_engine: 2,
        max_pages_per_engine: 3,
        ..Default::default()
    };
    let configured = HtmlSearchLimits::from_input(&input(), &config);
    assert_eq!(configured.max_results, 3);
    assert_eq!(configured.max_results_per_engine, 9);
    assert_eq!(configured.max_pages_per_engine, 2);

    let mut input = input();
    input.max_results = Some(50);
    let output_only = HtmlSearchLimits::from_input(&input, &config);
    assert_eq!(output_only.max_results, 12);
    assert_eq!(output_only.max_results_per_engine, 9);
    assert_eq!(output_only.max_pages_per_engine, 2);

    input.max_results_per_engine = Some(50);
    input.max_pages_per_engine = Some(5);
    let capped = HtmlSearchLimits::from_input(&input, &config);
    assert_eq!(capped.max_results, 12);
    assert_eq!(capped.max_results_per_engine, 25);
    assert_eq!(capped.max_pages_per_engine, 3);
}

#[tokio::test]
async fn final_limit_does_not_limit_source_candidates_or_hide_consensus_below_the_cap() {
    // A shared candidate below the final cap in every source must still win.
    // Changing only the output cap must send identical budgets to all sources.
    for max_results in [1, 20] {
        let calls = Mutex::new(Vec::new());
        let limits = HtmlSearchLimits {
            max_results,
            max_results_per_engine: 3,
            max_pages_per_engine: 2,
        };
        let output = search(
            &input(),
            limits,
            Duration::from_secs(1),
            |engine, candidates, pages| {
                calls.lock().unwrap().push((engine, candidates, pages));
                async move {
                    Ok(vec![
                        row(&format!("https://example.com/{engine}/1"), "First"),
                        row(&format!("https://example.com/{engine}/2"), "Second"),
                        row("https://example.com/shared", "Shared third candidate"),
                    ])
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(
            calls.into_inner().unwrap(),
            WebSearchEngine::ALL.map(|engine| (engine, 3, 2))
        );
        assert_eq!(output.results.len(), max_results.min(17));
        assert_eq!(output.results[0].result.url, "https://example.com/shared");
        assert_eq!(output.results[0].engines.len(), 8);
    }
}

#[tokio::test]
async fn filtering_uses_all_fetched_candidates_without_refilling_sources() {
    let mut input = input();
    input.engine = Some(WebSearchEngineSelection::Engines(vec![Bing]));
    input.allowed_domains = vec!["docs.example.com".into()];
    for (max_results, expected_count) in [(1, 1), (5, 2)] {
        let calls = AtomicUsize::new(0);
        let output = search(
            &input,
            limits(max_results),
            Duration::from_secs(1),
            |_, _, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                async {
                    Ok(vec![
                        row("https://outside.test/", "Filtered"),
                        row("https://docs.example.com/1", "Kept"),
                        row("https://docs.example.com/2", "Kept too"),
                    ])
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(output.results.len(), expected_count);
        assert_eq!(output.results[0].result.url, "https://docs.example.com/1");
    }
}
