//! Opt-in profiling of stable serial pagination; no Agena service is started.
use super::*;
use std::path::{Path, PathBuf};

fn legacy(root: &Path, matcher: &GlobMatcher, offset: usize, limit: usize) -> (Vec<PathBuf>, bool) {
    let mut paths = Vec::new();
    let mut skipped = 0;
    for entry in walk_builder(root, false).build() {
        let entry = entry.unwrap();
        if entry.path() == root {
            continue;
        }
        let relative = normalize_path_for_display(entry.path().strip_prefix(root).unwrap());
        if !matcher.is_match(&relative) {
            continue;
        }
        if skipped < offset {
            skipped += 1;
            continue;
        }
        if paths.len() == limit {
            return (paths, true);
        }
        paths.push(entry.into_path());
    }
    (paths, false)
}

#[test]
#[ignore = "opt-in optimized glob profiling; set AGENA_GLOB_BENCH_OUTPUT to retain evidence"]
fn repository_glob_equivalence_benchmark() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut workloads = Vec::new();
    for (pattern, offset, limit) in [
        ("**/*.rs", 0, 200),
        ("**/*.rs", 1000, 1000),
        ("**/absent_agena_benchmark.xyz", 0, 200),
    ] {
        let matcher = Glob::new(pattern).unwrap().compile_matcher();
        let baseline = legacy(&root, &matcher, offset, limit);
        let enhanced = || {
            let (paths, reason) = collect_matches(
                &root,
                &matcher,
                offset,
                limit,
                false,
                &ScanOptions::default(),
            )
            .unwrap();
            assert!(
                reason
                    .as_ref()
                    .is_none_or(|reason| reason == "result page limit reached")
            );
            (paths, reason.is_some())
        };
        for _ in 0..2 {
            assert_eq!(enhanced(), baseline);
            assert_eq!(legacy(&root, &matcher, offset, limit), baseline);
        }
        let mut before = Vec::new();
        let mut after = Vec::new();
        for round in 0..21 {
            for version in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
                let start = Instant::now();
                let result = if version == 0 {
                    legacy(&root, &matcher, offset, limit)
                } else {
                    enhanced()
                };
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(result, baseline);
                if version == 0 {
                    before.push(elapsed);
                } else {
                    after.push(elapsed);
                }
            }
        }
        let median = |values: &[f64]| {
            let mut values = values.to_vec();
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        workloads.push(serde_json::json!({"pattern":pattern,"offset":offset,"limit":limit,"records":baseline.0.len(),"more":baseline.1,"previous_median_ms":median(&before),"enhanced_median_ms":median(&after),"previous_ms":before,"enhanced_ms":after}));
    }
    let report = serde_json::json!({
        "profile":if cfg!(debug_assertions) {"debug"} else {"optimized"},
        "corpus":"Current worktree crates/; normal ignore rules; sorted serial pagination",
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "rounds":21,"warmups":2,"order":"alternating","workloads":workloads,
        "limitations":["Warm cache, shared host load; no end-to-end or cross-platform claim.","All paths and pagination completeness checked every iteration.","Reference isolates previous walk/filter loop, not all old product rendering."]
    });
    if let Some(path) = std::env::var_os("AGENA_GLOB_BENCH_OUTPUT") {
        std::fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(path),
            serde_json::to_string_pretty(&report).unwrap() + "\n",
        )
        .unwrap();
    }
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
