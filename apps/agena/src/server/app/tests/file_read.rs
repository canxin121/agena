use std::{
    future::Future,
    path::Path,
    pin::Pin,
    sync::{Arc, mpsc},
    task::{Context, Wake, Waker},
    time::Duration,
};

use axum::{body::Body, http::Request, response::Response};
use http_body_util::BodyExt as _;
use tokio::sync::{Notify, oneshot};
use tower::ServiceExt as _;

use super::{FILE_LIMIT, FS_REQUEST_TESTS, StatusCode, assert_json_error};

fn read_request(directory: &Path, endpoint: &str, target: &Path) -> Request<Body> {
    Request::builder()
        .uri(format!(
            "/api/v1/workbench/fs/{endpoint}?path={}",
            urlencoding::encode(target.to_str().unwrap())
        ))
        .header("x-agena-directory", directory.to_str().unwrap())
        .body(Body::empty())
        .unwrap()
}

async fn read(directory: &Path, endpoint: &str, target: &Path) -> Response {
    super::super::fs_router::<()>()
        .oneshot(read_request(directory, endpoint, target))
        .await
        .unwrap()
}

struct WorkFinished(Notify);

impl Wake for WorkFinished {
    fn wake(self: Arc<Self>) {
        self.0.notify_one();
    }
}

// Run one blocking operation, leaving its result ready but unconsumed by the
// HTTP future. A single-thread blocking pool and explicit wake notification
// make the metadata/read race reproducible without sleeps or production hooks.
async fn finish_next_blocking_operation<F: Future>(mut request: Pin<&mut F>) {
    let (started, started_rx) = oneshot::channel();
    let (release, release_rx) = mpsc::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    started_rx.await.unwrap();
    let finished = Arc::new(WorkFinished(Notify::new()));
    let waker = Waker::from(finished.clone());
    let result = request.as_mut().poll(&mut Context::from_waker(&waker));
    release.send(()).unwrap();
    blocker.await.unwrap();
    assert!(
        result.is_pending(),
        "the filesystem work must wait for the gate"
    );
    tokio::time::timeout(Duration::from_secs(10), finished.0.notified())
        .await
        .expect("filesystem worker completed");
}

fn race_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap()
}

#[test]
fn full_read_routes_bound_files_that_grow_after_preflight() {
    race_runtime().block_on(async {
        let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let mut violations = Vec::new();
        for endpoint in ["read", "raw", "download"] {
            let target = fixture.path().join(endpoint);
            std::fs::write(&target, b"original").unwrap();
            let mut request = Box::pin(read(fixture.path(), endpoint, &target));
            // Download first validates its workspace directory.
            if endpoint == "download" {
                finish_next_blocking_operation(request.as_mut()).await;
            }
            finish_next_blocking_operation(request.as_mut()).await;
            std::fs::OpenOptions::new()
                .write(true)
                .open(&target)
                .unwrap()
                .set_len(FILE_LIMIT as u64 + 1)
                .unwrap();
            let response = request.await;
            if response.status() == StatusCode::OK {
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                if bytes.len() > FILE_LIMIT {
                    violations.push(format!("{endpoint} returned {} bytes", bytes.len()));
                } else {
                    assert_eq!(bytes, &b"original"[..]);
                }
            } else {
                assert_json_error(response, StatusCode::PAYLOAD_TOO_LARGE).await;
            }
        }
        assert!(violations.is_empty(), "{}", violations.join("; "));
    });
}

#[test]
fn full_read_routes_recheck_file_type_after_preflight() {
    race_runtime().block_on(async {
        let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let mut violations = Vec::new();
        for endpoint in ["read", "raw", "download"] {
            let target = fixture.path().join(endpoint);
            std::fs::write(&target, b"original").unwrap();
            let mut request = Box::pin(read(fixture.path(), endpoint, &target));
            if endpoint == "download" {
                finish_next_blocking_operation(request.as_mut()).await;
            }
            finish_next_blocking_operation(request.as_mut()).await;
            std::fs::remove_file(&target).unwrap();
            std::fs::create_dir(&target).unwrap();
            let response = request.await;
            match response.status() {
                StatusCode::OK => {
                    let bytes = response.into_body().collect().await.unwrap().to_bytes();
                    assert_eq!(bytes, &b"original"[..]);
                }
                StatusCode::BAD_REQUEST => {
                    assert_json_error(response, StatusCode::BAD_REQUEST).await;
                }
                status => violations.push(format!("{endpoint} returned {status}")),
            }
        }
        assert!(violations.is_empty(), "{}", violations.join("; "));
    });
}

#[tokio::test]
async fn full_text_read_rejects_invalid_utf8_while_binary_routes_preserve_it() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("binary.png");
    for contents in [&b"valid\xff"[..], &b"incomplete\xf0\x9f\x98"[..]] {
        std::fs::write(&target, contents).unwrap();
        for endpoint in ["raw", "download"] {
            let response = read(fixture.path(), endpoint, &target).await;
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(bytes, contents);
        }
        assert_json_error(
            read(fixture.path(), "read", &target).await,
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
}

#[tokio::test]
async fn full_read_routes_preserve_exact_limits_and_response_headers() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("报告.png");
    for size in [0, FILE_LIMIT, FILE_LIMIT + 1] {
        std::fs::File::create(&target)
            .unwrap()
            .set_len(size as u64)
            .unwrap();
        for endpoint in ["read", "raw", "download"] {
            let response = read(fixture.path(), endpoint, &target).await;
            if size > FILE_LIMIT {
                assert_json_error(response, StatusCode::PAYLOAD_TOO_LARGE).await;
                continue;
            }
            assert_eq!(response.status(), StatusCode::OK);
            if endpoint == "read" {
                assert_eq!(response.headers()["content-type"], "text/plain");
            } else {
                assert_eq!(response.headers()["content-type"], "image/png");
                assert_eq!(response.headers()["cache-control"], "no-store");
                let disposition = response.headers()["content-disposition"].to_str().unwrap();
                let kind = if endpoint == "raw" {
                    "inline"
                } else {
                    "attachment"
                };
                assert_eq!(
                    disposition,
                    format!("{kind}; filename=\"__.png\"; filename*=UTF-8''%E6%8A%A5%E5%91%8A.png")
                );
            }
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(bytes.len(), size);
            assert!(bytes.iter().all(|byte| *byte == 0));
        }
    }
}

#[tokio::test]
async fn full_read_routes_reject_missing_paths_and_nonfiles() {
    let _guard = FS_REQUEST_TESTS.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    for endpoint in ["read", "raw", "download"] {
        assert_json_error(
            read(fixture.path(), endpoint, &fixture.path().join("missing")).await,
            StatusCode::NOT_FOUND,
        )
        .await;
        assert_json_error(
            read(fixture.path(), endpoint, fixture.path()).await,
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    #[cfg(unix)]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt as _};
        let fifo = fixture.path().join("fifo");
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a NUL-terminated path owned for the duration of mkfifo.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        for endpoint in ["read", "raw", "download"] {
            assert_json_error(
                tokio::time::timeout(
                    Duration::from_secs(2),
                    read(fixture.path(), endpoint, &fifo),
                )
                .await
                .expect("non-files are rejected without waiting for a FIFO writer"),
                StatusCode::BAD_REQUEST,
            )
            .await;
        }
    }
}
