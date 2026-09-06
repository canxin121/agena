use std::{sync::mpsc, time::Duration};

use tokio::sync::{Semaphore, oneshot};

use super::{AppError, process_body};

#[tokio::test]
async fn cancelled_body_worker_retains_its_slot_until_it_finishes() {
    static SLOTS: Semaphore = Semaphore::const_new(1);
    let permit = SLOTS.acquire().await.unwrap();
    let (started, started_rx) = oneshot::channel();
    let (finish, finish_rx) = mpsc::channel();
    let worker = tokio::spawn(process_body(permit, move || {
        let _ = started.send(());
        finish_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("release body worker");
        Ok(())
    }));
    tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .unwrap()
        .unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    assert!(
        SLOTS.try_acquire().is_err(),
        "the cancelled HTTP waiter must not admit another parser"
    );
    finish.send(()).unwrap();
    let _permit = tokio::time::timeout(Duration::from_secs(5), SLOTS.acquire())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn failed_body_workers_release_their_slot_and_report_failure() {
    static SLOTS: Semaphore = Semaphore::const_new(1);
    let permit = SLOTS.acquire().await.unwrap();
    let result = process_body::<()>(permit, || panic!("injected body worker panic")).await;
    assert!(matches!(result, Err(AppError::Internal { .. })));
    let _permit = SLOTS
        .try_acquire()
        .expect("panicked worker released its slot");
}
