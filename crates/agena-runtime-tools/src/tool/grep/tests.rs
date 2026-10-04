use super::*;
use serde_json::json;

fn input(value: serde_json::Value) -> GrepToolInput {
    serde_json::from_value(value).unwrap()
}
fn run(root: &Path, value: serde_json::Value) -> SearchResult {
    let input = input(value);
    let limits = SearchLimits {
        max_results: input.max_results.unwrap_or(500) as usize,
        ..SearchLimits::default()
    };
    collect_matches(root, root, &input, None, &limits).unwrap()
}

#[test]
fn parallel_batches_preserve_order_context_and_global_result_budgets() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..48 {
        std::fs::write(
            root.path().join(format!("file-{index:03}.txt")),
            if index % 11 == 0 {
                "hit\n\0binary"
            } else {
                "before\nhit\nmiddle\nhit\nafter\n"
            },
        )
        .unwrap();
    }
    for mode in ["content", "files", "count"] {
        for limit in [3, 27, 500] {
            let mut value = json!({"pattern":"hit", "mode":mode});
            if mode == "content" {
                value["before_context"] = json!(1);
                value["after_context"] = json!(1);
            }
            let input = input(value);
            let serial = collect_matches(
                root.path(),
                root.path(),
                &input,
                None,
                &SearchLimits {
                    parallelism: 1,
                    max_results: limit,
                    ..SearchLimits::default()
                },
            )
            .unwrap();
            let parallel = collect_matches(
                root.path(),
                root.path(),
                &input,
                None,
                &SearchLimits {
                    parallelism: 4,
                    max_results: limit,
                    ..SearchLimits::default()
                },
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(&serial.records).unwrap(),
                serde_json::to_value(&parallel.records).unwrap()
            );
            assert_eq!(serial.stats.stop_reason, parallel.stats.stop_reason);
            assert_eq!(serial.returned_matches, parallel.returned_matches);
            assert!(parallel.returned_matches <= limit);
        }
    }
}

#[test]
fn batch_boundaries_prove_exact_limits_and_reserve_disjoint_bytes() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..16 {
        std::fs::write(root.path().join(format!("file-{index:03}")), "hit\n").unwrap();
    }
    let input = input(json!({"pattern":"hit"}));
    let limits = SearchLimits {
        max_results: 16,
        ..SearchLimits::default()
    };
    let exact = collect_matches(root.path(), root.path(), &input, None, &limits).unwrap();
    assert_eq!(exact.returned_matches, 16);
    assert!(exact.stats.scan_complete());
    std::fs::write(root.path().join("file-016"), "hit\n").unwrap();
    let more = collect_matches(root.path(), root.path(), &input, None, &limits).unwrap();
    assert_eq!(more.returned_matches, 16);
    assert_eq!(more.stats.stop_reason, Some(StopReason::Results));
    for bytes in [0, 3, 4, 7, 8, 64, 68] {
        let result = collect_matches(
            root.path(),
            root.path(),
            &input,
            None,
            &SearchLimits {
                max_total_bytes: bytes,
                ..SearchLimits::default()
            },
        )
        .unwrap();
        assert!(result.stats.searched_bytes <= bytes);
        assert_eq!(result.returned_matches as u64, (bytes / 4).min(17));
        assert_eq!(result.stats.scan_complete(), bytes == 68);
    }
}
fn lines(result: &SearchResult) -> Vec<(u64, &str, bool)> {
    result
        .records
        .iter()
        .filter_map(|record| match record {
            GrepRecord::Line {
                line,
                text,
                matched,
                ..
            } => Some((*line, text.as_str(), *matched)),
            _ => None,
        })
        .collect()
}

#[test]
fn literal_case_and_whitespace_patterns_keep_their_meaning() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "x.y\nX.Y\nxay\n  x.y  \n").unwrap();
    assert_eq!(
        lines(&run(
            root.path(),
            json!({"pattern":"x.y","fixed_strings":true})
        ))
        .len(),
        2
    );
    assert_eq!(
        lines(&run(
            root.path(),
            json!({"pattern":"x.y","fixed_strings":true,"case":"insensitive"})
        ))
        .len(),
        3
    );
    assert_eq!(
        lines(&run(
            root.path(),
            json!({"pattern":"x.y","fixed_strings":true,"case":"smart"})
        ))
        .len(),
        3
    );
    assert_eq!(
        lines(&run(
            root.path(),
            json!({"pattern":"X.Y","fixed_strings":true,"case":"smart"})
        ))
        .len(),
        1
    );
    assert_eq!(
        lines(&run(
            root.path(),
            json!({"pattern":"  x.y  ","fixed_strings":true})
        )),
        [(4, "  x.y  ", true)]
    );
}

#[test]
fn includes_are_or_combined_and_exclusion_wins() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("nested")).unwrap();
    for path in ["a.rs", "b.ts", "c.txt", "nested/d.rs"] {
        std::fs::write(root.path().join(path), "hit\n").unwrap();
    }
    let result = run(
        root.path(),
        json!({"pattern":"hit","mode":"files","include":"**/*.rs","includes":["*.ts"],"exclude":["nested/**"]}),
    );
    assert_eq!(
        result.records,
        [
            GrepRecord::File {
                path: "a.rs".into()
            },
            GrepRecord::File {
                path: "b.ts".into()
            }
        ]
    );
    assert!(result.stats.scan_complete());
}

#[test]
fn context_is_ordered_deduplicated_and_separate_from_matching_lines() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("a.txt"),
        "before\nhit\nbetween\nhit\nafter\n",
    )
    .unwrap();
    let result = run(
        root.path(),
        json!({"pattern":"hit","before_context":1,"after_context":1,"max_results":2}),
    );
    assert_eq!(
        lines(&result),
        [
            (1, "before", false),
            (2, "hit", true),
            (3, "between", false),
            (4, "hit", true),
            (5, "after", false)
        ]
    );
    assert_eq!(result.returned_matches, 2);
    assert!(!result.stats.truncated());
    let capped = run(
        root.path(),
        json!({"pattern":"hit","before_context":1,"after_context":1,"max_results":1}),
    );
    assert_eq!(
        lines(&capped),
        [
            (1, "before", false),
            (2, "hit", true),
            (3, "between", false)
        ]
    );
    assert_eq!(capped.stats.stop_reason, Some(StopReason::Results));
}

#[test]
fn exact_limits_are_complete_and_an_additional_match_proves_truncation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "hit\nhit\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "miss\n").unwrap();
    let result = run(root.path(), json!({"pattern":"hit","max_results":2}));
    assert!(!result.stats.truncated());
    std::fs::write(root.path().join("b.txt"), "hit\n").unwrap();
    let result = run(root.path(), json!({"pattern":"hit","max_results":2}));
    assert_eq!(result.records.len(), 2);
    assert_eq!(result.stats.stop_reason, Some(StopReason::Results));
}

#[test]
fn count_limits_files_without_capping_line_counts_and_files_return_once() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "hit hit\n".repeat(1000)).unwrap();
    let result = run(
        root.path(),
        json!({"pattern":"hit","mode":"count","max_results":1}),
    );
    assert_eq!(
        result.records,
        [GrepRecord::Count {
            path: "a.txt".into(),
            count: 1000,
            complete: true
        }]
    );
    assert!(!result.stats.truncated());
    let result = run(
        root.path(),
        json!({"pattern":"hit","mode":"files","max_results":1}),
    );
    assert_eq!(result.records.len(), 1);
    assert!(!result.stats.truncated());
    std::fs::write(root.path().join("b.txt"), "hit\n").unwrap();
    for mode in ["count", "files"] {
        let result = run(
            root.path(),
            json!({"pattern":"hit","mode":mode,"max_results":1}),
        );
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.stats.stop_reason, Some(StopReason::Results));
    }
}

#[test]
fn shortened_text_and_incomplete_scan_are_distinct() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("a.txt"),
        format!("hit{}\n", "😀中".repeat(5000)),
    )
    .unwrap();
    let result = run(root.path(), json!({"pattern":"hit"}));
    assert!(result.stats.scan_complete());
    assert!(result.stats.truncated());
    assert!(
        matches!(&result.records[0], GrepRecord::Line { text_truncated: true, text, .. } if text.len() <= MAX_LINE_BYTES)
    );
    let limits = SearchLimits {
        max_output_bytes: 100,
        ..SearchLimits::default()
    };
    let result = collect_matches(
        root.path(),
        root.path(),
        &input(json!({"pattern":"hit"})),
        None,
        &limits,
    )
    .unwrap();
    assert!(result.records.is_empty());
    assert_eq!(result.stats.stop_reason, Some(StopReason::Output));
    // Count mode must say "partial" if an oversized line aborts the same file.
    std::fs::write(
        root.path().join("a.txt"),
        format!("hit\n{}\n", "x".repeat(3 * 1024 * 1024)),
    )
    .unwrap();
    let result = run(root.path(), json!({"pattern":"hit","mode":"count"}));
    assert_eq!(
        result.records,
        [GrepRecord::Count {
            path: "a.txt".into(),
            count: 1,
            complete: false
        }]
    );
    assert!(!result.stats.scan_complete());
}

#[test]
fn observed_binary_data_discards_earlier_text_matches_and_buffers_are_reused_safely() {
    let root = tempfile::tempdir().unwrap();
    let mut bytes = "hit\n".repeat(20000).into_bytes();
    bytes.push(0);
    std::fs::write(root.path().join("a.bin"), bytes).unwrap();
    std::fs::write(root.path().join("b.txt"), "hit\n").unwrap();
    let result = run(root.path(), json!({"pattern":"hit","mode":"count"}));
    assert_eq!(
        result.records,
        [GrepRecord::Count {
            path: "b.txt".into(),
            count: 1,
            complete: true
        }]
    );
    assert_eq!(result.stats.binary_files, 1);
}

#[test]
fn cancellation_deadline_and_total_bytes_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "hit\n").unwrap();
    let input = input(json!({"pattern":"hit"}));
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        collect_matches(
            root.path(),
            root.path(),
            &input,
            Some(&cancel),
            &SearchLimits::default()
        ),
        Err(ToolError::Cancelled)
    ));
    for (limits, expected) in [
        (
            SearchLimits {
                max_duration: Duration::ZERO,
                ..SearchLimits::default()
            },
            StopReason::Deadline,
        ),
        (
            SearchLimits {
                max_total_bytes: 2,
                ..SearchLimits::default()
            },
            StopReason::Bytes,
        ),
    ] {
        let result = collect_matches(root.path(), root.path(), &input, None, &limits).unwrap();
        assert_eq!(result.stats.stop_reason, Some(expected));
        assert!(!result.stats.scan_complete());
    }
}

#[test]
fn malformed_options_fail_instead_of_silently_weakening_the_search() {
    let parsed =
        GrepToolInput::parse_input(json!({"pattern":"  literal  ","fixed_strings":true})).unwrap();
    assert_eq!(parsed.pattern, "  literal  ");
    for value in [
        json!({"pattern":"x","max_results":0}),
        json!({"pattern":"x","after_context":21}),
    ] {
        assert!(GrepToolInput::parse_input(value).is_err());
    }
    assert!(
        serde_json::from_value::<GrepToolInput>(json!({"pattern":"x","fixed_string":true}))
            .is_err()
    );
    let root = tempfile::tempdir().unwrap();
    for value in [
        json!({"pattern":"[","fixed_strings":false}),
        json!({"pattern":"x","mode":"files","before_context":1}),
        json!({"pattern":"x","includes":["["]}),
    ] {
        assert!(
            collect_matches(
                root.path(),
                root.path(),
                &input(value),
                None,
                &SearchLimits::default()
            )
            .is_err()
        );
    }
}
