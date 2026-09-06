use std::{path::Path, time::Duration};

use axum::{body::Body, http::StatusCode, response::Response};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};

async fn replace(directory: &Path, body: Value) -> Response {
    super::send(
        directory,
        "replace-content",
        "unused",
        Body::from(serde_json::to_vec(&body).unwrap()),
    )
    .await
}

#[tokio::test]
async fn misspelled_revision_fields_cannot_silently_disable_conflict_checks() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("target");
    std::fs::write(&target, b"old").unwrap();
    for body in [
        json!({"query": "old", "replace": "new", "paths": ["target"], "expectedRevision": "a".repeat(64)}),
        json!({"replace": "new", "match": {"path": "target", "expected": "old", "startOffset": 0, "endOffset": 3, "expectedRevison": "a".repeat(64)}}),
    ] {
        let response = replace(fixture.path(), body).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
    }
}

#[tokio::test]
async fn expanded_directories_respect_visibility_but_explicit_files_are_still_selected() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir(fixture.path().join(".git")).unwrap();
    std::fs::write(fixture.path().join(".gitignore"), b"ignored.txt\n").unwrap();
    std::fs::create_dir(fixture.path().join("scope")).unwrap();
    for name in ["visible", ".hidden", "ignored.txt"] {
        std::fs::write(fixture.path().join("scope").join(name), b"old").unwrap();
    }
    let response = replace(fixture.path(), json!({"query": "old", "replace": "new", "paths": ["scope"], "includeHidden": false, "respectGitignore": true})).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read(fixture.path().join("scope/visible")).unwrap(),
        b"new"
    );
    for name in [".hidden", "ignored.txt"] {
        assert_eq!(
            std::fs::read(fixture.path().join("scope").join(name)).unwrap(),
            b"old",
            "{name}"
        );
    }
    let response = replace(fixture.path(), json!({"query": "old", "replace": "new", "paths": ["scope/.hidden", "scope/ignored.txt"], "includeHidden": false, "respectGitignore": true})).await;
    assert_eq!(response.status(), StatusCode::OK);
    for name in [".hidden", "ignored.txt"] {
        assert_eq!(
            std::fs::read(fixture.path().join("scope").join(name)).unwrap(),
            b"new"
        );
    }
}

async fn search(directory: &Path, paths: &[&str], query: &str) -> Value {
    let response = super::send(
        directory,
        "search-content",
        "unused",
        Body::from(serde_json::to_vec(&json!({ "query": query, "paths": paths })).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn search_returns_content_revisions_and_single_replacements_reject_stale_files() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("target");
    std::fs::write(&target, b"abc").unwrap();
    let found = search(fixture.path(), &["target"], "a").await;
    let revision = found["files"][0]["revision"].as_str().unwrap();
    assert_eq!(
        revision,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // The selected character still matches, but the rest of the file changed.
    std::fs::write(&target, b"abc with an external edit").unwrap();
    let response = replace(fixture.path(), json!({
        "replace": "A", "match": { "path": "target", "expected": "a", "expectedRevision": revision, "startOffset": 0, "endOffset": 1 }
    })).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"abc with an external edit"
    );
    let refreshed = search(fixture.path(), &["target"], "a").await;
    assert_ne!(refreshed["files"][0]["revision"], revision);
    let response = replace(fixture.path(), json!({
        "replace": "A", "match": { "path": "target", "expected": "a", "expectedRevision": refreshed["files"][0]["revision"], "startOffset": 0, "endOffset": 1 }
    })).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"Abc with an external edit"
    );
}

#[tokio::test]
async fn stale_bulk_replacements_report_only_the_files_already_published() {
    let fixture = tempfile::tempdir().unwrap();
    for name in ["first", "changed", "last"] {
        std::fs::write(fixture.path().join(name), b"old").unwrap();
    }
    let found = search(fixture.path(), &["first", "changed", "last"], "old").await;
    let revisions: serde_json::Map<String, Value> = found["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            (
                file["path"].as_str().unwrap().to_owned(),
                file["revision"].clone(),
            )
        })
        .collect();
    std::fs::write(fixture.path().join("changed"), b"old, edited externally").unwrap();
    let response = replace(fixture.path(), json!({
        "query": "old", "replace": "new", "paths": ["first", "changed", "last"], "expectedRevisions": revisions,
    })).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["details"]["completed"]["fileCount"], 1);
    assert_eq!(
        response["details"]["completed"]["files"][0]["relativePath"],
        "first"
    );
    assert_eq!(response["details"]["remaining"], 1);
    assert_eq!(std::fs::read(fixture.path().join("first")).unwrap(), b"new");
    assert_eq!(
        std::fs::read(fixture.path().join("changed")).unwrap(),
        b"old, edited externally"
    );
    assert_eq!(std::fs::read(fixture.path().join("last")).unwrap(), b"old");
}

#[tokio::test]
async fn revision_scope_and_digest_validation_happen_before_any_mutation() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("target"), b"old").unwrap();
    let found = search(fixture.path(), &["target"], "old").await;
    for (expected, status) in [
        (json!({}), StatusCode::CONFLICT),
        (json!({ "target": "invalid" }), StatusCode::BAD_REQUEST),
        (
            json!({ "missing": found["files"][0]["revision"] }),
            StatusCode::CONFLICT,
        ),
        (
            json!({ "target": found["files"][0]["revision"], "./target": found["files"][0]["revision"] }),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = replace(fixture.path(), json!({"query": "old", "replace": "new", "paths": ["target"], "expectedRevisions": expected})).await;
        assert_eq!(response.status(), status);
        assert_eq!(
            std::fs::read(fixture.path().join("target")).unwrap(),
            b"old"
        );
    }
}

#[tokio::test]
async fn replacements_accept_exactly_50_mib_and_reject_one_extra_byte() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("target");
    let original = "a".repeat(50 * 1024);
    std::fs::write(&target, &original).unwrap();
    let response = replace(
        fixture.path(),
        json!({"query": "a", "replace": "b".repeat(1024), "paths": ["target"]}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    super::assert_file_bytes(&target, super::FILE_LIMIT, b'b');
    let original = original + "x";
    std::fs::write(&target, &original).unwrap();
    let response = replace(
        fixture.path(),
        json!({"query": "a", "replace": "b".repeat(1024), "paths": ["target"]}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_distinct_matches_preserve_every_successful_edit() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("target");
    let mut original = (0..8)
        .map(|index| format!("TOKEN{index}|"))
        .collect::<String>();
    original.push_str(&"a".repeat(512 * 1024));
    std::fs::write(&target, &original).unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..8 {
        let directory = fixture.path().to_path_buf();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            let response = replace(&directory, json!({
                "replace": format!("VALUE{index}"),
                "match": { "path": "target", "expected": format!("TOKEN{index}"), "startOffset": index * 7, "endOffset": index * 7 + 6 }
            })).await;
            assert_eq!(response.status(), StatusCode::OK);
        });
    }
    tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .unwrap();
    let expected = original.replace("TOKEN", "VALUE");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), expected);
}

#[tokio::test]
async fn literal_replacement_cannot_publish_an_oversized_file() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("target");
    let original = "a".repeat(64 * 1024);
    std::fs::write(&target, &original).unwrap();
    let response = replace(
        fixture.path(),
        json!({
            "query": "a", "replace": "b".repeat(1024), "paths": ["target"],
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "64 MiB output must be rejected before writing"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
}

#[tokio::test]
async fn capture_expansion_cannot_publish_an_oversized_file() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("target");
    let original = "a".repeat(32 * 1024);
    std::fs::write(&target, &original).unwrap();
    let response = replace(
        fixture.path(),
        json!({
            "query": "(a+)", "isRegex": true, "replace": "$1".repeat(2048), "paths": ["target"],
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "even a single match can expand beyond the file limit"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
}

#[cfg(unix)]
#[tokio::test]
async fn failed_batch_reports_completed_files_and_stops_before_remaining_files() {
    use std::os::unix::fs::PermissionsExt as _;
    let fixture = tempfile::tempdir().unwrap();
    for name in ["first", "readonly", "last"] {
        std::fs::write(fixture.path().join(name), b"old").unwrap();
    }
    std::fs::set_permissions(
        fixture.path().join("readonly"),
        std::fs::Permissions::from_mode(0o444),
    )
    .unwrap();
    let response = replace(
        fixture.path(),
        json!({
            "query": "old", "replace": "new", "paths": ["first", "readonly", "last"],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(std::fs::read(fixture.path().join("first")).unwrap(), b"new");
    assert_eq!(
        std::fs::read(fixture.path().join("readonly")).unwrap(),
        b"old"
    );
    assert_eq!(std::fs::read(fixture.path().join("last")).unwrap(), b"old");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["details"]["completed"]["fileCount"], 1);
    assert_eq!(body["details"]["completed"]["replacementCount"], 1);
    assert_eq!(
        body["details"]["completed"]["files"][0]["relativePath"],
        "first"
    );
    assert!(
        body["details"]["failedPath"]
            .as_str()
            .unwrap()
            .ends_with("/readonly")
    );
    assert_eq!(body["details"]["remaining"], 1);
}

#[cfg(unix)]
#[tokio::test]
async fn kernel_write_failures_leave_single_and_bulk_replacement_targets_intact() {
    const CHILD_ENV: &str = "AGENA_CONTENT_REPLACE_FAILURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child.args([
            "--exact",
            "server::app::tests::content_replace::kernel_write_failures_leave_single_and_bulk_replacement_targets_intact",
            "--nocapture",
        ]).env(CHILD_ENV, "1").kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(20), child.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "replacement child failed: {}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    for name in ["single", "bulk"] {
        std::fs::write(fixture.path().join(name), b"old").unwrap();
    }
    // Only this test subprocess receives the resource limit and signal handler.
    unsafe {
        assert_ne!(libc::signal(libc::SIGXFSZ, libc::SIG_IGN), libc::SIG_ERR);
        let mut limit = std::mem::zeroed::<libc::rlimit>();
        assert_eq!(libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit), 0);
        limit.rlim_cur = 4096;
        assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
    }
    let mut damaged = Vec::new();
    for name in ["single", "bulk"] {
        let body = if name == "single" {
            json!({"replace": "x".repeat(16384), "match": {"path": name, "expected": "old", "startOffset": 0, "endOffset": 3}})
        } else {
            json!({"replace": "x".repeat(16384), "query": "old", "paths": [name]})
        };
        let response = replace(fixture.path(), body).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        if std::fs::read(fixture.path().join(name)).unwrap() != b"old" {
            damaged.push(name);
        }
    }
    assert!(
        damaged.is_empty(),
        "failed replacement damaged targets: {damaged:?}"
    );
    assert_eq!(
        std::fs::read_dir(fixture.path()).unwrap().count(),
        2,
        "no staging files remain"
    );
}
