use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn page(index: usize, count: usize) -> WebSearchPage {
    let cards = (0..count).map(|n| format!(r#"<li class="b_algo"><h2><a href="https://example.com/{index}/{n}">Result {index}/{n}</a></h2><p>Snippet</p></li>"#)).collect::<String>();
    WebSearchPage {
        final_url: url::Url::parse(BING_BASE).unwrap(),
        status: 200,
        html: format!("<ol id=b_results>{cards}</ol>"),
        retry_after_secs: None,
    }
}

async fn no_render(_: url::Url, _: Duration) -> Result<Option<WebSearchPage>, CrawlError> {
    panic!("must not render this response")
}

#[tokio::test(start_paused = true)]
async fn duckduckgo_follows_actual_next_tokens_and_stops_when_no_continuation_is_offered() {
    let service = WebSearchCoordinator::default();
    let options = WebSearchOptions {
        engine: WebSearchEngine::DuckDuckGo,
        limit: 50,
        max_pages: 5,
        ..Default::default()
    };
    let calls = AtomicUsize::new(0);
    let response = service
        .run(
            "fixture",
            &options,
            false,
            |_, index, continuation| {
                calls.fetch_add(1, Ordering::SeqCst);
                if index == 0 {
                    assert!(continuation.is_none());
                } else {
                    assert_eq!(index, 1);
                    assert!(continuation.unwrap().form.unwrap().contains(&("s".into(), "27".into())));
                }
                let mut page = page(index, 0);
                page.final_url = url::Url::parse(DDG_HTML_URL).unwrap();
                page.html = format!(r#"<div class=result><a class=result__a href=https://example.com/{index}>Result</a></div>"#);
                if index == 0 {
                    page.html.push_str(r#"<form method=post action=/html/><input name=q value=fixture><input name=s value=27><input name=vqd value=opaque><input type=submit value=Next></form>"#);
                }
                std::future::ready(Ok(page))
            },
            no_render,
        )
        .await
        .unwrap();
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.pages_fetched, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn source_budgets_deduplication_and_exhaustion_bound_real_page_requests() {
    for (limit, max_pages, size, expected_pages, expected_results) in [
        (10, 1, 3, 1, 3),
        (12, 5, 10, 2, 12),
        (4, 1, 10, 1, 4),
        (50, 2, 3, 2, 6),
        (usize::MAX, usize::MAX, 1, 5, 5),
    ] {
        let service = WebSearchCoordinator::default();
        let calls = AtomicUsize::new(0);
        let options = WebSearchOptions {
            limit,
            max_pages,
            ..Default::default()
        };
        let response = service
            .run(
                "fixture",
                &options,
                false,
                |_, index, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(page(index, size)))
                },
                no_render,
            )
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), expected_pages);
        assert_eq!(response.pages_fetched, expected_pages);
        assert_eq!(response.results.len(), expected_results);
    }
    let service = WebSearchCoordinator::default();
    let options = WebSearchOptions {
        limit: 50,
        max_pages: 5,
        ..Default::default()
    };
    let response = service
        .run(
            "repeated",
            &options,
            false,
            |_, _, _| async { Ok(page(0, 1)) },
            no_render,
        )
        .await
        .unwrap();
    assert_eq!(response.pages_fetched, 2);
    assert_eq!(response.results.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn later_page_rate_limit_or_timeout_preserves_prior_results_and_sets_cooldown() {
    for slow in [false, true] {
        let service = WebSearchCoordinator::default();
        let options = WebSearchOptions {
            limit: 50,
            max_pages: 5,
            timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let response = service
            .run(
                "fixture",
                &options,
                false,
                |_, index, _| async move {
                    let mut result = page(index, 1);
                    if index > 0 {
                        if slow {
                            tokio::time::sleep(Duration::from_secs(20)).await;
                        }
                        result.status = 429;
                        result.retry_after_secs = Some(90);
                    }
                    Ok(result)
                },
                no_render,
            )
            .await
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.pages_fetched, 1);
        assert_eq!(response.warnings.len(), 1);
        assert_eq!(
            response.warnings[0].kind,
            if slow {
                SearchIssueKind::Timeout
            } else {
                SearchIssueKind::RateLimited
            }
        );
        let blocked = service
            .run(
                "another query",
                &options,
                false,
                |_, _, _| async { panic!("cooldown must not send a request") },
                no_render,
            )
            .await
            .unwrap_err();
        assert!(blocked.to_string().contains("cooling down"));
    }
}

#[tokio::test(start_paused = true)]
async fn retry_after_is_shared_across_queries_and_expires_without_retry_storms() {
    let service = WebSearchCoordinator::default();
    let options = WebSearchOptions::default();
    let error = service
        .run(
            "first",
            &options,
            false,
            |_, _, _| async {
                let mut result = page(0, 0);
                result.status = 429;
                result.retry_after_secs = Some(90);
                Ok(result)
            },
            no_render,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("90s"));
    for _ in 0..3 {
        assert!(
            service
                .run(
                    "different",
                    &options,
                    false,
                    |_, _, _| async { panic!("no repeated request") },
                    no_render
                )
                .await
                .is_err()
        );
    }
    tokio::time::advance(Duration::from_secs(91)).await;
    assert_eq!(
        service
            .run(
                "different",
                &options,
                false,
                |_, _, _| async { Ok(page(0, 1)) },
                no_render
            )
            .await
            .unwrap()
            .results
            .len(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn duplicate_queries_share_work_but_cache_keys_keep_independent_budgets_and_render_modes() {
    let service = WebSearchCoordinator::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let options = WebSearchOptions::default();
    let fetch = |_, _, _| async {
        calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(page(0, 10))
    };
    let (a, b) = tokio::join!(
        service.run("fixture", &options, false, fetch, no_render),
        service.run("fixture", &options, false, fetch, no_render)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!a.unwrap().cache_hit);
    assert!(b.unwrap().cache_hit);
    let more = WebSearchOptions {
        limit: 20,
        max_pages: 2,
        ..options.clone()
    };
    service
        .run("fixture", &more, false, fetch, no_render)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    service
        .run("fixture", &options, true, fetch, no_render)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    tokio::time::advance(CACHE_TTL).await;
    assert!(
        !service
            .run("fixture", &options, false, fetch, no_render)
            .await
            .unwrap()
            .cache_hit
    );
}

#[tokio::test(start_paused = true)]
async fn same_engine_queries_are_serialized_and_paced_and_cancellation_releases_the_gate() {
    let service = WebSearchCoordinator::default();
    let options = WebSearchOptions::default();
    let started = Instant::now();
    let (a, b) = tokio::join!(
        service.run(
            "one",
            &options,
            false,
            |_, _, _| async { Ok(page(0, 1)) },
            no_render
        ),
        service.run(
            "two",
            &options,
            false,
            |_, _, _| async { Ok(page(0, 1)) },
            no_render
        )
    );
    a.unwrap();
    b.unwrap();
    assert!(started.elapsed() >= PAGE_DELAY);
    let mut cancelled = Box::pin(service.run(
        "slow",
        &options,
        false,
        |_, _, _| std::future::pending(),
        no_render,
    ));
    assert!(futures_util::poll!(&mut cancelled).is_pending());
    drop(cancelled);
    assert!(
        service
            .run(
                "after cancel",
                &options,
                false,
                |_, _, _| async { Ok(page(0, 1)) },
                no_render
            )
            .await
            .is_ok()
    );
}

#[tokio::test(start_paused = true)]
async fn empty_results_are_rechecked_and_configured_pacing_is_respected() {
    let delay = Duration::from_secs(7);
    let service = WebSearchCoordinator::new(delay);
    let options = WebSearchOptions::default();
    let started = Instant::now();
    let empty = service
        .run(
            "fixture",
            &options,
            false,
            |_, _, _| async {
                let mut response = page(0, 0);
                response.html = "<div class=b_no>No results found</div>".into();
                Ok(response)
            },
            no_render,
        )
        .await
        .unwrap();
    assert!(empty.results.is_empty());
    let response = service
        .run(
            "fixture",
            &options,
            false,
            |_, _, _| async { Ok(page(0, 1)) },
            no_render,
        )
        .await
        .unwrap();
    assert_eq!(response.results.len(), 1);
    assert!(!response.cache_hit);
    assert!(started.elapsed() >= delay);
}

#[tokio::test(start_paused = true)]
async fn successful_cached_results_remain_available_during_cooldown() {
    let service = WebSearchCoordinator::default();
    let options = WebSearchOptions::default();
    service
        .run(
            "cached",
            &options,
            false,
            |_, _, _| async { Ok(page(0, 1)) },
            no_render,
        )
        .await
        .unwrap();
    service
        .run(
            "blocked",
            &options,
            false,
            |_, _, _| async {
                let mut response = page(0, 0);
                response.status = 429;
                Ok(response)
            },
            no_render,
        )
        .await
        .unwrap_err();
    let response = service
        .run(
            "cached",
            &options,
            false,
            |_, _, _| async { panic!("cache hit must not send a request") },
            no_render,
        )
        .await
        .unwrap();
    assert!(response.cache_hit);
    assert_eq!(response.results.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn only_javascript_shells_get_one_bounded_browser_attempt_and_captcha_does_not() {
    let service = WebSearchCoordinator::default();
    let options = WebSearchOptions::default();
    let renders = AtomicUsize::new(0);
    let response = service
        .run(
            "shell",
            &options,
            true,
            |_, _, _| async {
                let mut result = page(0, 0);
                result.html = "<noscript><a href='/httpservice/retry/enablejs'>Enable JavaScript</a></noscript>".into();
                Ok(result)
            },
            |_, remaining| {
                assert!(remaining < options.timeout);
                renders.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(Some(page(0, 1))))
            },
        )
        .await
        .unwrap();
    assert_eq!(response.results.len(), 1);
    assert!(response.rendered);
    assert_eq!(renders.load(Ordering::SeqCst), 1);
    let error = service
        .run(
            "challenge",
            &options,
            true,
            |_, _, _| async {
                let mut result = page(0, 0);
                result.html = "<form id=captcha-form>Verify</form>".into();
                Ok(result)
            },
            no_render,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("captcha_required"));
}
