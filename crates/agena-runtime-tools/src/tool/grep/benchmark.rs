//! Opt-in comparison of the prior serial engine and the bounded implementation.
//! Both run in the same binary/profile. This never starts an Agena service.
use super::*;
use sha2::{Digest, Sha256};

fn previous_engine(root: &Path, pattern: &str) -> Vec<String> {
    let matcher = grep_regex::RegexMatcher::new(pattern).unwrap();
    let mut rows = Vec::new();
    for entry in walk_builder(root, false).build() {
        let entry = entry.unwrap();
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = normalize_path_for_display(entry.path().strip_prefix(root).unwrap());
        SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(b'\0'))
            .heap_limit(Some(SEARCHER_HEAP_BYTES))
            .build()
            .search_path(
                &matcher,
                entry.path(),
                grep_searcher::sinks::Lossy(|line, text| {
                    rows.push(format!(
                        "{path}:{line}: {}",
                        text.trim_end_matches(['\r', '\n'])
                    ));
                    Ok(rows.len() < MAX_MATCHES)
                }),
            )
            .unwrap();
        if rows.len() >= MAX_MATCHES {
            break;
        }
    }
    rows
}

#[test]
#[ignore = "opt-in filesystem benchmark; set AGENA_SEARCH_BENCH_OUTPUT to retain evidence"]
fn embedded_search_equivalence_benchmark() {
    let root = tempfile::tempdir().unwrap();
    let mut corpus_hash = Sha256::new();
    let mut corpus_bytes = 0;
    for directory in 0..64 {
        std::fs::create_dir(root.path().join(format!("dir{directory:03}"))).unwrap();
        for file in 0..32 {
            let relative = format!("dir{directory:03}/file{file:03}.txt");
            let mut contents = "ordinary source line with no relevant identifier\n".repeat(180);
            if file == 0 && directory % 4 == 0 {
                contents.push_str("agena_bench_needle\n");
            }
            corpus_bytes += contents.len();
            corpus_hash.update(relative.as_bytes());
            corpus_hash.update(contents.as_bytes());
            std::fs::write(root.path().join(relative), contents).unwrap();
        }
    }
    let mut workloads = Vec::new();
    for pattern in ["agena_bench_needle", "missing_benchmark_identifier"] {
        let input: GrepToolInput =
            serde_json::from_value(serde_json::json!({"pattern":pattern})).unwrap();
        let enhanced = || {
            let result = collect_matches(
                root.path(),
                root.path(),
                &input,
                None,
                &SearchLimits::default(),
            )
            .unwrap();
            assert!(result.stats.scan_complete());
            result
                .records
                .iter()
                .map(GrepRecord::display)
                .collect::<Vec<_>>()
        };
        let reference = previous_engine(root.path(), pattern);
        assert_eq!(enhanced(), reference);
        for _ in 0..2 {
            std::hint::black_box(previous_engine(root.path(), pattern));
            std::hint::black_box(enhanced());
        }
        let mut previous_ms = Vec::new();
        let mut enhanced_ms = Vec::new();
        for round in 0..21 {
            for variant in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
                let started = Instant::now();
                let rows = if variant == 0 {
                    previous_engine(root.path(), pattern)
                } else {
                    enhanced()
                };
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(rows, reference);
                if variant == 0 {
                    previous_ms.push(elapsed);
                } else {
                    enhanced_ms.push(elapsed);
                }
            }
        }
        let median = |samples: &[f64]| {
            let mut values = samples.to_vec();
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        workloads.push(serde_json::json!({
            "pattern":pattern, "records":reference.len(),
            "result_sha256":hex::encode(Sha256::digest(reference.join("\n").as_bytes())),
            "previous_ms":previous_ms, "enhanced_ms":enhanced_ms,
            "previous_median_ms":median(&previous_ms), "enhanced_median_ms":median(&enhanced_ms),
        }));
    }
    let report = serde_json::json!({
        "profile":if cfg!(debug_assertions) {"debug"} else {"optimized"},
        "os":std::env::consts::OS, "arch":std::env::consts::ARCH,
        "corpus_files":2048, "corpus_bytes":corpus_bytes,
        "corpus_sha256":hex::encode(corpus_hash.finalize()),
        "rounds":21, "warmups":2, "order":"alternating",
        "limitations":[
            "Synthetic local warm-cache corpus; no OS cache clearing or load isolation.",
            "Both implementations in the same Cargo test binary/profile; no product end-to-end or cross-platform claim.",
            "Prior search loop excludes unrelated payload rendering; equality checked every iteration.",
            "Additional guardrails and structured records are included in enhanced timing."
        ],
        "workloads":workloads,
    });
    if let Some(path) = std::env::var_os("AGENA_SEARCH_BENCH_OUTPUT") {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path);
        std::fs::write(path, serde_json::to_string_pretty(&report).unwrap() + "\n").unwrap();
    }
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
