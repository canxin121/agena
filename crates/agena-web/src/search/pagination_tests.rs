use super::*;

fn row(url: &str) -> WebSearchResult {
    WebSearchResult {
        title: "Fixture".into(),
        url: url.into(),
        description: String::new(),
        source: "example.com".into(),
        engine: "bing".into(),
    }
}

#[tokio::test]
async fn search_pagination_stops_at_either_budget_and_counts_the_first_page() {
    for (limit, max_pages, page_size, expected_pages, expected_results) in [
        (10, 1, 3, 1, 3),
        (12, 5, 10, 2, 12),
        (10, 5, 10, 1, 10),
        (4, 1, 10, 1, 4),
        (50, 2, 3, 2, 6),
        (usize::MAX, usize::MAX, 1, 5, 5),
        (usize::MAX, usize::MAX, 60, 1, 50),
    ] {
        let mut requested_pages = Vec::new();
        let results = collect_search_pages(limit, max_pages, |page| {
            requested_pages.push(page);
            std::future::ready(Ok((0..page_size)
                .map(|index| row(&format!("https://example.com/{page}/{index}")))
                .collect()))
        })
        .await
        .unwrap();
        assert_eq!(requested_pages, (0..expected_pages).collect::<Vec<_>>());
        assert_eq!(results.len(), expected_results);
    }
}

#[tokio::test]
async fn search_pagination_deduplicates_across_pages_before_counting_candidates() {
    let mut requested_pages = Vec::new();
    let results = collect_search_pages(4, 5, |page| {
        requested_pages.push(page);
        let urls = match page {
            0 => vec!["https://example.com/a", "https://example.com/b"],
            1 => vec![
                "https://EXAMPLE.com:443/a#repeated",
                "https://example.com/b",
                "https://example.com/c",
                "https://example.com/d",
                "https://example.com/e",
            ],
            _ => panic!("candidate budget must stop further requests"),
        };
        std::future::ready(Ok(urls.into_iter().map(row).collect()))
    })
    .await
    .unwrap();
    assert_eq!(requested_pages, [0, 1]);
    assert_eq!(
        results.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(),
        [
            "https://example.com/a",
            "https://example.com/b",
            "https://example.com/c",
            "https://example.com/d",
        ]
    );
}

#[tokio::test]
async fn search_pagination_stops_on_empty_or_repeated_pages_including_empty_first_page() {
    for (empty_first, repeat) in [(true, false), (false, false), (false, true)] {
        let mut requested_pages = Vec::new();
        let results = collect_search_pages(50, 5, |page| {
            requested_pages.push(page);
            let rows = match page {
                0 if empty_first => Vec::new(),
                0 => vec![row("https://example.com/a")],
                1 if repeat => vec![
                    row("https://EXAMPLE.com:443/a#same-page"),
                    row("javascript:alert(1)"),
                ],
                1 => Vec::new(),
                _ => panic!("no-new-URL page must stop further requests"),
            };
            std::future::ready(Ok(rows))
        })
        .await
        .unwrap();
        assert_eq!(requested_pages.len(), if empty_first { 1 } else { 2 });
        assert_eq!(results.len(), usize::from(!empty_first));
    }
}

#[tokio::test]
async fn search_pagination_propagates_source_failure_without_requesting_another_page() {
    let mut requested_pages = Vec::new();
    let error = collect_search_pages(50, 5, |page| {
        requested_pages.push(page);
        std::future::ready(match page {
            0 => Ok(vec![row("https://example.com/a")]),
            1 => Err(CrawlError::SearchProvider {
                provider: "bing",
                message: "fixture failure".into(),
            }),
            _ => panic!("a failed page must stop further requests"),
        })
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("fixture failure"));
    assert_eq!(requested_pages, [0, 1]);
}
