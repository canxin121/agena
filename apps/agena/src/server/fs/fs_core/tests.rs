use axum::{
    Json, Router,
    body::Body,
    extract::Query,
    http::{HeaderMap, Request, StatusCode},
    routing::{get, post},
};
use http_body_util::BodyExt as _;
use tower::ServiceExt as _;

use super::{ReadChunkQuery, UploadQuery, fs_read_chunk, fs_upload};

async fn read_chunk(
    path: &std::path::Path,
    offset: usize,
    limit: usize,
) -> super::ApiResult<super::ReadChunkResponse> {
    fs_read_chunk(Query(ReadChunkQuery {
        path: Some(path.to_string_lossy().into_owned()),
        offset: Some(offset),
        limit: Some(limit),
    }))
    .await
    .map(|Json(response)| response)
}

#[tokio::test]
async fn small_positive_chunks_advance_through_utf8_and_round_trip() {
    let fixture = tempfile::tempdir().expect("file fixture");
    let path = fixture.path().join("utf8.txt");
    let original = "😀你好éZ";
    std::fs::write(&path, original).expect("fixture content");
    for limit in 1..=8 {
        let mut offset = 0;
        let mut recovered = String::new();
        loop {
            let chunk = read_chunk(&path, offset, limit).await.expect("UTF-8 chunk");
            assert!(
                !chunk.content.is_empty(),
                "limit {limit} made no progress at {offset}"
            );
            recovered.push_str(&chunk.content);
            assert_eq!(chunk.loaded_bytes, recovered.len());
            assert_eq!(chunk.total_bytes, original.len());
            if !chunk.has_more {
                assert_eq!(chunk.next_offset, None);
                break;
            }
            let next = chunk.next_offset.expect("next chunk");
            assert!(
                next > offset,
                "limit {limit} returned a non-advancing offset"
            );
            offset = next;
        }
        assert_eq!(recovered, original, "limit {limit}");
    }
}

#[tokio::test]
async fn incomplete_utf8_at_eof_is_rejected_instead_of_returning_the_same_offset() {
    let fixture = tempfile::tempdir().expect("file fixture");
    let path = fixture.path().join("incomplete.txt");
    for bytes in [&b"hello\xf0\x9f\x98"[..], &b"\xe4\xbd"[..], &b"abc\xc3"[..]] {
        std::fs::write(&path, bytes).expect("incomplete fixture");
        let mut offset = 0;
        let mut rejected = false;
        for _ in 0..bytes.len() + 1 {
            match read_chunk(&path, offset, 4).await {
                Err(super::AppError::BadRequest { .. }) => {
                    rejected = true;
                    break;
                }
                Err(error) => panic!("unexpected error: {error}"),
                Ok(chunk) => {
                    if !chunk.has_more || chunk.next_offset == Some(offset) {
                        break;
                    }
                    offset = chunk.next_offset.expect("next offset");
                }
            }
        }
        assert!(
            rejected,
            "incomplete UTF-8 must be rejected: {bytes:?}, last offset {offset}"
        );
    }
}

#[tokio::test]
async fn metadata_only_reads_keep_the_zero_limit_contract() {
    let fixture = tempfile::tempdir().expect("file fixture");
    let path = fixture.path().join("metadata.txt");
    std::fs::write(&path, "😀x").expect("fixture");
    let meta = read_chunk(&path, 0, 0).await.expect("metadata");
    assert_eq!(meta.total_bytes, 5);
    assert_eq!(meta.limit, 0);
    assert_eq!(meta.loaded_bytes, 0);
    assert_eq!(meta.content, "");
    assert!(meta.has_more);
    assert_eq!(meta.next_offset, Some(0));
    let eof = read_chunk(&path, 5, 0).await.expect("EOF metadata");
    assert!(!eof.has_more);
    assert_eq!(eof.next_offset, None);
    assert!(read_chunk(&path, 6, 4).await.is_err());
    assert!(
        read_chunk(&path, 1, 4).await.is_err(),
        "offset inside a code point is invalid"
    );
}

#[tokio::test]
async fn http_chunks_keep_complete_utf8_across_the_default_boundary() {
    let fixture = tempfile::tempdir().expect("file fixture");
    let path = fixture.path().join("boundary.txt");
    let original = format!("{}😀中é", "x".repeat(super::DEFAULT_READ_CHUNK_LIMIT - 1));
    std::fs::write(&path, &original).expect("fixture");
    let app = Router::new().route("/read-chunk", get(fs_read_chunk));
    let mut offset = 0;
    let mut recovered = String::new();
    loop {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/read-chunk?path={}&offset={offset}",
                        urlencoding::encode(path.to_str().unwrap())
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("HTTP chunk");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes();
        let chunk: super::ReadChunkResponse = serde_json::from_slice(&bytes).expect("chunk schema");
        recovered.push_str(&chunk.content);
        assert!(chunk.loaded_bytes > offset);
        assert!(chunk.content.len() <= chunk.limit);
        if !chunk.has_more {
            break;
        }
        offset = chunk.next_offset.expect("next offset");
    }
    assert_eq!(recovered, original);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_no_overwrite_uploads_have_exactly_one_winner() {
    let fixture = tempfile::tempdir().expect("upload fixture");
    let path = fixture.path().join("uploaded.bin");
    let app = Router::new().route("/upload", post(fs_upload));
    let start = std::sync::Arc::new(tokio::sync::Barrier::new(24));
    let mut workers = tokio::task::JoinSet::new();
    for marker in 0..24u8 {
        let app = app.clone();
        let start = start.clone();
        let uri = format!(
            "/upload?directory={}&path=uploaded.bin&overwrite=false",
            urlencoding::encode(fixture.path().to_str().unwrap())
        );
        workers.spawn(async move {
            start.wait().await;
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .body(Body::from(vec![marker; 8192]))
                        .unwrap(),
                )
                .await
                .expect("upload response");
            (marker, response.status())
        });
    }
    let mut winners = Vec::new();
    while let Some(result) = workers.join_next().await {
        let (marker, status) = result.expect("upload task");
        if status.is_success() {
            winners.push(marker);
        } else {
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "existing target is rejected"
            );
        }
    }
    assert_eq!(
        winners.len(),
        1,
        "multiple no-overwrite uploads claimed success: {winners:?}"
    );
    assert_eq!(
        std::fs::read(&path).expect("uploaded content"),
        vec![winners[0]; 8192]
    );
}

#[tokio::test]
async fn no_overwrite_upload_preserves_existing_files_and_symlinks() {
    let fixture = tempfile::tempdir().expect("upload fixture");
    let existing = fixture.path().join("existing.bin");
    std::fs::write(&existing, b"preserved").expect("existing file");
    let names = if cfg!(unix) {
        vec!["existing.bin", "link.bin", "dangling.bin"]
    } else {
        vec!["existing.bin"]
    };
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&existing, fixture.path().join("link.bin")).expect("symlink");
        std::os::unix::fs::symlink("missing.bin", fixture.path().join("dangling.bin"))
            .expect("dangling symlink");
    }
    for name in names {
        let result = fs_upload(
            HeaderMap::new(),
            Query(UploadQuery {
                directory: Some(fixture.path().to_string_lossy().into_owned()),
                path: Some(name.to_owned()),
                overwrite: false,
            }),
            axum::body::Bytes::from_static(b"replacement"),
        )
        .await;
        assert!(result.is_err(), "must preserve {name}");
    }
    assert_eq!(
        std::fs::read(existing).expect("existing content"),
        b"preserved"
    );
    assert!(!fixture.path().join("missing.bin").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn atomic_writes_preserve_existing_symlinks_and_file_mode() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = tempfile::tempdir().expect("file fixture");
    let target = fixture.path().join("script.sh");
    let link = fixture.path().join("script-link");
    std::fs::write(&target, b"original").expect("target");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o751)).expect("mode");
    std::os::unix::fs::symlink(&target, &link).expect("link");
    let directory = fixture.path().to_string_lossy().into_owned();
    let _ = super::fs_write(
        HeaderMap::new(),
        Query(super::ProjectDirQuery {
            directory: Some(directory.clone()),
        }),
        Json(super::WriteBody {
            path: Some("script-link".to_owned()),
            content: Some("written through link".to_owned()),
        }),
    )
    .await
    .expect("write through link");
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link remains")
            .is_symlink()
    );
    assert_eq!(
        std::fs::read(&target).expect("target bytes"),
        b"written through link"
    );
    assert_eq!(
        std::fs::metadata(&target)
            .expect("target mode")
            .permissions()
            .mode()
            & 0o777,
        0o751
    );
    let _ = fs_upload(
        HeaderMap::new(),
        Query(UploadQuery {
            directory: Some(directory),
            path: Some("script-link".to_owned()),
            overwrite: true,
        }),
        axum::body::Bytes::from_static(b"uploaded through link"),
    )
    .await
    .expect("upload through link");
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link remains")
            .is_symlink()
    );
    assert_eq!(
        std::fs::read(&target).expect("target bytes"),
        b"uploaded through link"
    );
    assert_eq!(
        std::fs::metadata(&target)
            .expect("target mode")
            .permissions()
            .mode()
            & 0o777,
        0o751
    );
    assert_eq!(
        std::fs::read_dir(fixture.path()).expect("entries").count(),
        2
    );
}

#[cfg(unix)]
#[tokio::test]
async fn atomic_writes_refuse_read_only_and_dangling_link_targets() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = tempfile::tempdir().expect("file fixture");
    let target = fixture.path().join("read-only");
    std::fs::write(&target, b"preserved").expect("target");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444))
        .expect("read-only mode");
    std::os::unix::fs::symlink("missing", fixture.path().join("dangling")).expect("dangling link");
    for name in ["read-only", "dangling"] {
        let directory = fixture.path().to_string_lossy().into_owned();
        assert!(
            super::fs_write(
                HeaderMap::new(),
                Query(super::ProjectDirQuery {
                    directory: Some(directory.clone()),
                }),
                Json(super::WriteBody {
                    path: Some(name.to_owned()),
                    content: Some("replacement".to_owned()),
                })
            )
            .await
            .is_err(),
            "file write must reject {name}"
        );
        assert!(
            fs_upload(
                HeaderMap::new(),
                Query(UploadQuery {
                    directory: Some(directory),
                    path: Some(name.to_owned()),
                    overwrite: true,
                }),
                axum::body::Bytes::from_static(b"replacement")
            )
            .await
            .is_err(),
            "upload must reject {name}"
        );
    }
    assert_eq!(
        std::fs::read(&target).expect("preserved target"),
        b"preserved"
    );
    assert!(
        std::fs::symlink_metadata(fixture.path().join("dangling"))
            .expect("link remains")
            .is_symlink()
    );
    assert!(!fixture.path().join("missing").exists());
    assert_eq!(
        std::fs::read_dir(fixture.path()).expect("entries").count(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_readers_see_complete_file_replacements() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let fixture = tempfile::tempdir().expect("file fixture");
    let path = fixture.path().join("shared.bin");
    const SIZE: usize = 256 * 1024;
    std::fs::write(&path, vec![b'a'; SIZE]).expect("initial content");
    let start = Arc::new(tokio::sync::Barrier::new(5));
    let remaining = Arc::new(AtomicUsize::new(4));
    let mut tasks = tokio::task::JoinSet::new();
    for marker in b'b'..=b'e' {
        let start = start.clone();
        let remaining = remaining.clone();
        let directory = fixture.path().to_string_lossy().into_owned();
        tasks.spawn(async move {
            start.wait().await;
            for _ in 0..8 {
                if marker % 2 == 0 {
                    let _ = fs_upload(
                        HeaderMap::new(),
                        Query(UploadQuery {
                            directory: Some(directory.clone()),
                            path: Some("shared.bin".to_owned()),
                            overwrite: true,
                        }),
                        axum::body::Bytes::from(vec![marker; SIZE]),
                    )
                    .await
                    .expect("concurrent upload");
                } else {
                    let _ = super::fs_write(
                        HeaderMap::new(),
                        Query(super::ProjectDirQuery {
                            directory: Some(directory.clone()),
                        }),
                        Json(super::WriteBody {
                            path: Some("shared.bin".to_owned()),
                            content: Some(char::from(marker).to_string().repeat(SIZE)),
                        }),
                    )
                    .await
                    .expect("concurrent file write");
                }
            }
            remaining.fetch_sub(1, Ordering::SeqCst);
            0
        });
    }
    tasks.spawn(async move {
        start.wait().await;
        let mut reads = 0;
        loop {
            let content = tokio::fs::read(&path).await.expect("concurrent reader");
            assert_eq!(
                content.len(),
                SIZE,
                "a replacement must not expose a truncated file"
            );
            let marker = content[0];
            assert!((b'a'..=b'e').contains(&marker));
            assert!(
                content.iter().all(|byte| *byte == marker),
                "a replacement must not mix writer payloads"
            );
            reads += 1;
            if remaining.load(Ordering::SeqCst) == 0 {
                break;
            }
        }
        reads
    });
    let reads = tokio::time::timeout(std::time::Duration::from_secs(20), async move {
        let mut reads = 0;
        while let Some(result) = tasks.join_next().await {
            reads += result.expect("file task");
        }
        reads
    })
    .await
    .expect("concurrent file operations deadline");
    assert!(reads > 0);
    assert_eq!(
        std::fs::read_dir(fixture.path()).expect("entries").count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_truncation_does_not_produce_stuck_chunk_cursors() {
    let fixture = tempfile::tempdir().expect("file fixture");
    let path = fixture.path().join("changing.txt");
    std::fs::write(&path, vec![b'x'; 8192]).expect("initial file");
    let writer_path = path.clone();
    let start = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let writer_start = start.clone();
    let writer = tokio::spawn(async move {
        writer_start.wait().await;
        for _ in 0..64 {
            tokio::fs::write(&writer_path, b"")
                .await
                .expect("external truncate");
            tokio::fs::write(&writer_path, vec![b'x'; 8192])
                .await
                .expect("external rewrite");
        }
    });
    start.wait().await;
    for _ in 0..64 {
        match read_chunk(&path, 0, 4096).await {
            Ok(chunk) if chunk.has_more => {
                assert!(!chunk.content.is_empty());
                assert!(chunk.next_offset.is_some_and(|offset| offset > 0));
            }
            Ok(chunk) => assert_eq!(chunk.next_offset, None),
            Err(super::AppError::BadRequest { .. }) => {} // The file changed during this read.
            Err(error) => panic!("unexpected read error: {error}"),
        }
    }
    writer.await.expect("external writer");
}

#[cfg(unix)]
#[tokio::test]
async fn filesystem_write_errors_preserve_original_files_and_leave_no_partial_creates() {
    const CHILD_ENV: &str = "AGENA_FS_WRITE_FAILURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let mut command =
            tokio::process::Command::new(std::env::current_exe().expect("test binary"));
        command.args([
            "--exact",
            "server::fs::fs_core::tests::filesystem_write_errors_preserve_original_files_and_leave_no_partial_creates",
            "--nocapture",
        ]).env(CHILD_ENV, "1").kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
            .await
            .expect("write-failure child deadline")
            .expect("write-failure child");
        assert!(
            output.status.success(),
            "write-failure child failed: {}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let fixture = tempfile::tempdir().expect("write-failure fixture");
    let original = b"original contents must survive";
    for name in ["upload-existing", "write-existing"] {
        std::fs::write(fixture.path().join(name), original).expect("original file");
    }
    // The resource limit and ignored signal apply only to this dedicated
    // child process. The kernel permits a partial write and then returns
    // EFBIG, providing a real write failure without filling a developer disk.
    unsafe {
        assert_ne!(libc::signal(libc::SIGXFSZ, libc::SIG_IGN), libc::SIG_ERR);
        let mut limit = std::mem::zeroed::<libc::rlimit>();
        assert_eq!(libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit), 0);
        limit.rlim_cur = 4096;
        assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
    }
    let directory = fixture.path().to_string_lossy().into_owned();
    let mut damaged = Vec::new();
    for (name, existing, upload) in [
        ("upload-existing", true, true),
        ("write-existing", true, false),
        ("upload-new", false, true),
        ("write-new", false, false),
    ] {
        let failed = if upload {
            fs_upload(
                HeaderMap::new(),
                Query(UploadQuery {
                    directory: Some(directory.clone()),
                    path: Some(name.to_owned()),
                    overwrite: existing,
                }),
                axum::body::Bytes::from(vec![b'x'; 16_384]),
            )
            .await
            .is_err()
        } else {
            super::fs_write(
                HeaderMap::new(),
                Query(super::ProjectDirQuery {
                    directory: Some(directory.clone()),
                }),
                Json(super::WriteBody {
                    path: Some(name.to_owned()),
                    content: Some("x".repeat(16_384)),
                }),
            )
            .await
            .is_err()
        };
        assert!(
            failed,
            "the filesystem must reject the oversized {name} write"
        );
        let path = fixture.path().join(name);
        if existing {
            if std::fs::read(&path).expect("original file survives") != original {
                damaged.push(name);
            }
        } else if path.exists() {
            damaged.push(name);
        }
    }
    assert!(
        damaged.is_empty(),
        "failed writes damaged or partially created targets: {damaged:?}"
    );
    assert_eq!(
        std::fs::read_dir(fixture.path())
            .expect("fixture entries")
            .count(),
        2,
        "failed writes must clean up their staging files"
    );
}
