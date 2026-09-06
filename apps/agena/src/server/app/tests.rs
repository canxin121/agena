use std::{
    io::{self, Read as _},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    body::{Body, Bytes},
    http::{Request, StatusCode},
    response::Response,
};
use futures_util::StreamExt as _;
use http_body_util::BodyExt as _;
use tokio::sync::{Notify, Semaphore, oneshot};
use tower::ServiceExt as _;

mod content_replace;
mod file_read;

const FILE_LIMIT: usize = 50 * 1024 * 1024;
const JSON_WIRE_LIMIT: usize = 6 * FILE_LIMIT + 64 * 1024;

// Serialize these full-size fixtures and the concurrency probe. This bounds
// test memory and prevents unrelated test requests from occupying probe slots.
static FS_REQUEST_TESTS: Semaphore = Semaphore::const_new(1);

fn request(directory: &Path, endpoint: &str, target: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/workbench/fs/{endpoint}?path={}&overwrite=true",
            urlencoding::encode(target)
        ))
        .header("x-agena-directory", directory.to_str().unwrap())
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}

async fn send(directory: &Path, endpoint: &str, target: &str, body: Body) -> Response {
    super::fs_router::<()>()
        .oneshot(request(directory, endpoint, target, body))
        .await
        .expect("filesystem HTTP response")
}

async fn assert_json_error(response: Response, status: StatusCode) {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()["content-type"], "application/json");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&body).expect("Workbench error envelope");
    assert!(
        error["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty())
    );
    assert!(error.get("success").is_none());
}

fn repeated_chunks(
    pattern: &'static [u8],
    count: usize,
) -> impl futures_util::Stream<Item = Result<Bytes, io::Error>> {
    let chunk_count = 16 * 1024;
    let chunk = Bytes::from(pattern.repeat(chunk_count));
    futures_util::stream::iter((0..count.div_ceil(chunk_count)).map(move |index| {
        let remaining = count - index * chunk_count;
        Ok(chunk.slice(..remaining.min(chunk_count) * pattern.len()))
    }))
}

fn upload_body(size: usize) -> Body {
    Body::from_stream(repeated_chunks(b"x", size))
}

fn write_body(target: &str, pattern: &'static [u8], count: usize, tail: &'static [u8]) -> Body {
    let prefix = Bytes::from(format!(
        "{{\"path\":{},\"content\":\"",
        serde_json::to_string(target).unwrap()
    ));
    Body::from_stream(
        futures_util::stream::iter([Ok(prefix)])
            .chain(repeated_chunks(pattern, count))
            .chain(futures_util::stream::iter([
                Ok(Bytes::from_static(tail)),
                Ok(Bytes::from_static(b"\"}")),
            ])),
    )
}

fn padded_json_body(target: &str, total_bytes: usize) -> Body {
    let prefix = Bytes::from(
        serde_json::to_vec(&serde_json::json!({"path": target, "content": "ok"})).unwrap(),
    );
    let padding = total_bytes - prefix.len();
    Body::from_stream(
        futures_util::stream::iter([Ok(prefix)]).chain(repeated_chunks(b" ", padding)),
    )
}

fn assert_file_bytes(path: &Path, size: usize, byte: u8) {
    let mut file = std::fs::File::open(path).expect("written file");
    assert_eq!(file.metadata().unwrap().len(), size as u64);
    let mut buffer = [0; 64 * 1024];
    let mut observed = 0;
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        assert!(buffer[..count].iter().all(|value| *value == byte));
        observed += count;
    }
    assert_eq!(observed, size);
}

#[tokio::test]
async fn fs_mutations_accept_bodies_above_axum_default() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let size = 3 * 1024 * 1024;
    for endpoint in ["upload", "write"] {
        let body = if endpoint == "upload" {
            Body::from(vec![b'x'; size])
        } else {
            Body::from(
                serde_json::to_vec(
                    &serde_json::json!({"path": endpoint, "content": "x".repeat(size)}),
                )
                .unwrap(),
            )
        };
        let response = send(fixture.path(), endpoint, endpoint, body).await;
        assert_eq!(response.status(), StatusCode::OK, "{endpoint}");
        assert_file_bytes(&fixture.path().join(endpoint), size, b'x');
    }
}

#[tokio::test]
async fn fs_upload_enforces_content_limit_without_content_length() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let response = send(
        fixture.path(),
        "upload",
        "existing",
        upload_body(FILE_LIMIT),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_file_bytes(&fixture.path().join("existing"), FILE_LIMIT, b'x');
    for target in ["existing", "new-parent/new-file"] {
        let response = send(
            fixture.path(),
            "upload",
            target,
            upload_body(FILE_LIMIT + 1),
        )
        .await;
        assert_json_error(response, StatusCode::PAYLOAD_TOO_LARGE).await;
    }
    assert_file_bytes(&fixture.path().join("existing"), FILE_LIMIT, b'x');
    assert!(!fixture.path().join("new-parent").exists());
}

#[tokio::test]
async fn fs_write_enforces_decoded_utf8_byte_limit() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let count = FILE_LIMIT / "😀".len();
    let response = send(
        fixture.path(),
        "write",
        "existing",
        write_body("existing", "😀".as_bytes(), count, b""),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let content = std::fs::read_to_string(fixture.path().join("existing")).unwrap();
    assert_eq!(content.len(), FILE_LIMIT);
    assert!(content.chars().all(|ch| ch == '😀'));
    drop(content);
    std::fs::write(fixture.path().join("existing"), b"preserve me").unwrap();
    for target in ["existing", "new-parent/new-file"] {
        let response = send(
            fixture.path(),
            "write",
            target,
            write_body(target, "😀".as_bytes(), count, b"x"),
        )
        .await;
        assert_json_error(response, StatusCode::PAYLOAD_TOO_LARGE).await;
    }
    assert_eq!(
        std::fs::read(fixture.path().join("existing")).unwrap(),
        b"preserve me"
    );
    assert!(!fixture.path().join("new-parent").exists());
}

#[tokio::test]
async fn fs_write_accepts_worst_case_json_escaping_at_content_limit() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let response = send(
        fixture.path(),
        "write",
        "nul",
        write_body("nul", b"\\u0000", FILE_LIMIT, b""),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_file_bytes(&fixture.path().join("nul"), FILE_LIMIT, 0);
}

#[tokio::test]
async fn fs_write_enforces_encoded_limit_even_with_unknown_length() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let response = send(
        fixture.path(),
        "write",
        "existing",
        padded_json_body("existing", JSON_WIRE_LIMIT),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    for target in ["existing", "new-parent/new-file"] {
        let response = send(
            fixture.path(),
            "write",
            target,
            padded_json_body(target, JSON_WIRE_LIMIT + 1),
        )
        .await;
        assert_json_error(response, StatusCode::PAYLOAD_TOO_LARGE).await;
    }
    assert_eq!(
        std::fs::read(fixture.path().join("existing")).unwrap(),
        b"ok"
    );
    assert!(!fixture.path().join("new-parent").exists());
}

#[tokio::test]
async fn fs_write_preserves_json_rejection_statuses_in_workbench_envelope() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let valid = r#"{"path":"target","content":"ok"}"#;
    for (mime, body, status) in [
        (None, valid, StatusCode::UNSUPPORTED_MEDIA_TYPE),
        (
            Some("text/plain"),
            valid,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (Some("text/json"), valid, StatusCode::UNSUPPORTED_MEDIA_TYPE),
        (
            Some("application/json;invalid"),
            valid,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (Some("application/json"), "{", StatusCode::BAD_REQUEST),
        (
            Some("application/json"),
            r#"{"path":"target","content":42}"#,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            Some("application/json"),
            r#"{"path":"target","content":"ok"} trailing"#,
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let mut request = request(fixture.path(), "write", "target", Body::from(body));
        if let Some(mime) = mime {
            request
                .headers_mut()
                .insert("content-type", mime.parse().unwrap());
        } else {
            request.headers_mut().remove("content-type");
        }
        let response = super::fs_router::<()>().oneshot(request).await.unwrap();
        assert_json_error(response, status).await;
        assert!(!fixture.path().join("target").exists());
    }
    for mime in [
        "application/json; charset=utf-8",
        "application/vnd.agena+json",
        "APPLICATION/JSON",
    ] {
        let mut request = request(fixture.path(), "write", "target", Body::from(valid));
        request
            .headers_mut()
            .insert("content-type", mime.parse().unwrap());
        let response = super::fs_router::<()>().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{mime}");
        assert_eq!(std::fs::read(fixture.path().join("target")).unwrap(), b"ok");
    }
}

#[tokio::test]
async fn fs_mutations_reject_body_transport_errors_before_touching_files() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("existing"), b"preserve me").unwrap();
    for endpoint in ["upload", "write"] {
        for target in ["existing", "new-parent/new-file"] {
            let prefix = Bytes::from(
                serde_json::to_vec(&serde_json::json!({"path": target, "content": "incomplete"}))
                    .unwrap(),
            );
            let body = Body::from_stream(futures_util::stream::iter([
                Ok(prefix),
                Err(io::Error::other("injected body read failure")),
            ]));
            let response = send(fixture.path(), endpoint, target, body).await;
            assert_json_error(response, StatusCode::BAD_REQUEST).await;
        }
    }
    assert_eq!(
        std::fs::read(fixture.path().join("existing")).unwrap(),
        b"preserve me"
    );
    assert!(!fixture.path().join("new-parent").exists());
}

#[tokio::test]
async fn fs_other_mutations_keep_the_default_body_limit() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let body = Body::from(
        serde_json::to_vec(
            &serde_json::json!({"path": "target", "padding": "x".repeat(3 * 1024 * 1024)}),
        )
        .unwrap(),
    );
    let response = send(fixture.path(), "mkdir", "target", body).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!fixture.path().join("target").exists());
}

#[tokio::test]
async fn fs_mutations_bound_body_buffering_concurrency() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let progress = Arc::new(Notify::new());
    let mut releases = Vec::new();
    let mut requests = Vec::new();
    for index in 0..6 {
        let (release, wait) = oneshot::channel();
        releases.push(release);
        let started = started.clone();
        let progress = progress.clone();
        let target = format!("target-{index}");
        let prefix = Bytes::from(
            serde_json::to_vec(&serde_json::json!({"path": target, "content": "ok"})).unwrap(),
        );
        let body = Body::from_stream(futures_util::stream::once(async move {
            started.fetch_add(1, Ordering::SeqCst);
            progress.notify_one();
            let _ = wait.await;
            Ok::<_, io::Error>(prefix)
        }));
        let endpoint = if index % 2 == 0 { "upload" } else { "write" };
        let request = request(fixture.path(), endpoint, &target, body);
        requests.push(tokio::spawn(super::fs_router::<()>().oneshot(request)));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while started.load(Ordering::SeqCst) < 2 {
            progress.notified().await;
        }
    })
    .await
    .expect("two requests should start reading");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let before_release = started.load(Ordering::SeqCst);
    for release in releases {
        let _ = release.send(());
    }
    for request in requests {
        let response = tokio::time::timeout(Duration::from_secs(5), request)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert_eq!(
        before_release, 2,
        "only two requests may buffer or decode file contents at once"
    );
    assert_eq!(started.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn fs_declared_oversized_bodies_are_rejected_before_reading() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("existing"), b"preserve me").unwrap();
    let polled = Arc::new(AtomicUsize::new(0));
    for (endpoint, limit) in [("upload", FILE_LIMIT), ("write", JSON_WIRE_LIMIT)] {
        for target in ["existing", "new-parent/new-file"] {
            let polled = polled.clone();
            let body = Body::from_stream(futures_util::stream::once(async move {
                polled.fetch_add(1, Ordering::SeqCst);
                Ok::<_, io::Error>(Bytes::from_static(b"unexpected read"))
            }));
            let mut request = request(fixture.path(), endpoint, target, body);
            request
                .headers_mut()
                .insert("content-length", (limit + 1).to_string().parse().unwrap());
            let response = super::fs_router::<()>().oneshot(request).await.unwrap();
            assert_json_error(response, StatusCode::PAYLOAD_TOO_LARGE).await;
        }
    }
    assert_eq!(polled.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(fixture.path().join("existing")).unwrap(),
        b"preserve me"
    );
    assert!(!fixture.path().join("new-parent").exists());

    // A caller-provided header cannot replace counting the bytes from Body.
    let mut request = request(
        fixture.path(),
        "upload",
        "existing",
        upload_body(FILE_LIMIT + 1),
    );
    request
        .headers_mut()
        .insert("content-length", "1".parse().unwrap());
    let response = super::fs_router::<()>().oneshot(request).await.unwrap();
    assert_json_error(response, StatusCode::PAYLOAD_TOO_LARGE).await;
    assert_eq!(
        std::fs::read(fixture.path().join("existing")).unwrap(),
        b"preserve me"
    );
}

#[tokio::test]
async fn fs_cancelled_body_reads_release_slots_without_writing() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let progress = Arc::new(Notify::new());
    let mut requests = Vec::new();
    for endpoint in ["upload", "write"] {
        let started = started.clone();
        let progress = progress.clone();
        let body = Body::from_stream(futures_util::stream::once(async move {
            started.fetch_add(1, Ordering::SeqCst);
            progress.notify_one();
            std::future::pending::<Result<Bytes, io::Error>>().await
        }));
        requests.push(tokio::spawn(super::fs_router::<()>().oneshot(request(
            fixture.path(),
            endpoint,
            endpoint,
            body,
        ))));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while started.load(Ordering::SeqCst) < 2 {
            progress.notified().await;
        }
    })
    .await
    .expect("two stalled request bodies");

    let mut queued = tokio::spawn(super::fs_router::<()>().oneshot(request(
        fixture.path(),
        "upload",
        "queued",
        Body::from("ok"),
    )));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut queued)
            .await
            .is_err()
    );
    let cancelled = requests.pop().unwrap();
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    let response = tokio::time::timeout(Duration::from_secs(5), queued)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cancelled = requests.pop().unwrap();
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    assert!(!fixture.path().join("upload").exists());
    assert!(!fixture.path().join("write").exists());
    assert_eq!(std::fs::read(fixture.path().join("queued")).unwrap(), b"ok");
}

#[tokio::test]
async fn fs_content_literal_replacement_preserves_dollar_signs() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("target");
    for replacement in ["$5", "$HOME", "${name}", "$$", "$1-$HOME-$$"] {
        std::fs::write(&target, "item and item").unwrap();
        let body = Body::from(
            serde_json::to_vec(&serde_json::json!({
                "query": "item", "replace": replacement, "isRegex": false, "paths": ["target"]
            }))
            .unwrap(),
        );
        let response = send(fixture.path(), "replace-content", "target", body).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            format!("{replacement} and {replacement}")
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let response: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["replacementCount"], 2);
        assert_eq!(response["fileCount"], 1);
    }
}

#[tokio::test]
async fn fs_content_regex_replacement_keeps_capture_expansion() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("target");
    std::fs::write(&target, "unit=12 unit=34").unwrap();
    let body = Body::from(serde_json::to_vec(&serde_json::json!({
        "query": r"(?P<name>unit)=(\d+)", "replace": "${name}:$2:$$", "isRegex": true, "paths": ["target"]
    })).unwrap());
    let response = send(fixture.path(), "replace-content", "target", body).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "unit:12:$ unit:34:$"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let response: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response["replacementCount"], 2);
}
