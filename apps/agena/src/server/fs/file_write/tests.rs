use std::{sync::mpsc, time::Duration};

use axum::body::Bytes;
use tokio::sync::{Semaphore, oneshot};

use super::{AppError, remove_path, rename_path, transform_file_atomically, write_file_atomically};

// These probes deliberately hold namespace readers while a writer queues.
// Keep the probes together so one cannot block another probe's setup.
static PROBES: Semaphore = Semaphore::const_new(1);

#[test]
fn opened_file_growth_stops_after_the_first_byte_over_budget() {
    use std::io::Seek as _;

    let fixture = tempfile::tempfile().unwrap();
    fixture.set_len(7).unwrap();
    let inspected_len = fixture.metadata().unwrap().len();
    let mut shared_cursor = fixture.try_clone().unwrap();
    // The opened inode grows after inspection, independently of path changes.
    fixture.set_len(64 * 1024).unwrap();
    let result = super::read_opened_bounded(fixture, inspected_len, 1024).unwrap();
    assert!(matches!(result, super::FileRead::TooLarge));
    assert_eq!(shared_cursor.stream_position().unwrap(), 1025);
}

#[tokio::test]
async fn external_changes_during_transform_are_preserved_and_reported_as_conflicts() {
    let _probe = PROBES.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("target");
    std::fs::write(&path, b"original").unwrap();
    let external = path.clone();
    let result = transform_file_atomically(path.clone(), 1024, move |original| {
        assert_eq!(original.unwrap(), b"original");
        std::fs::write(external, b"external edit").unwrap();
        Ok((Some(Bytes::from_static(b"replacement")), ()))
    })
    .await;
    assert!(matches!(result, Err(AppError::Conflict { .. })));
    assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn cancelled_transform_keeps_its_file_lock_until_the_worker_finishes() {
    let _probe = PROBES.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("target");
    std::fs::write(&path, b"original").unwrap();
    let (started, started_rx) = oneshot::channel();
    let (finish, finish_rx) = mpsc::channel();
    let worker = tokio::spawn(transform_file_atomically(path.clone(), 1024, move |_| {
        let _ = started.send(());
        finish_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok((Some(Bytes::from_static(b"replaced")), ()))
    }));
    tokio::time::timeout(Duration::from_secs(10), started_rx)
        .await
        .unwrap()
        .unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    let subsequent = tokio::spawn(write_file_atomically(
        path.clone(),
        Bytes::from_static(b"saved later"),
        true,
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ran_early = subsequent.is_finished();
    finish.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), subsequent)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        !ran_early,
        "a cancelled waiter cannot unlock a still-running transform"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"saved later");
}

#[tokio::test]
async fn namespace_changes_wait_for_in_progress_file_transforms() {
    let _probe = PROBES.acquire().await.unwrap();
    for rename in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let directory = fixture.path().join("original-directory");
        let moved = fixture.path().join("moved-directory");
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("target");
        std::fs::write(&path, b"original").unwrap();
        let (started, started_rx) = oneshot::channel();
        let (finish, finish_rx) = mpsc::channel();
        let worker = tokio::spawn(transform_file_atomically(path.clone(), 1024, move |_| {
            let _ = started.send(());
            finish_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok((Some(Bytes::from_static(b"replaced")), ()))
        }));
        tokio::time::timeout(Duration::from_secs(10), started_rx)
            .await
            .unwrap()
            .unwrap();
        let from = directory.clone();
        let to = moved.clone();
        let namespace = tokio::spawn(async move {
            if rename {
                rename_path(from, to).await
            } else {
                remove_path(from).await
            }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let ran_early = namespace.is_finished();
        finish.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), namespace)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!ran_early, "namespace mutation must wait for publication");
        assert!(!directory.exists());
        if rename {
            assert_eq!(std::fs::read(moved.join("target")).unwrap(), b"replaced");
        } else {
            assert!(!moved.exists());
        }
    }
}

#[tokio::test]
async fn a_panicking_transform_preserves_the_file_and_does_not_poison_future_edits() {
    let _probe = PROBES.acquire().await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("target");
    std::fs::write(&path, b"original").unwrap();
    let result =
        transform_file_atomically::<()>(path.clone(), 1024, |_| panic!("injected transform panic"))
            .await;
    assert!(matches!(result, Err(AppError::Internal { .. })));
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    transform_file_atomically(path.clone(), 1024, |original| {
        assert_eq!(original.unwrap(), b"original");
        Ok((Some(Bytes::from_static(b"recovered")), ()))
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"recovered");
}
