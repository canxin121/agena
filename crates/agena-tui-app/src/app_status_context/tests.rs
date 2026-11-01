use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

#[tokio::test]
async fn closed_ui_cancels_active_commands_and_does_not_start_queued_work() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let (commands, command_rx) = tokio::sync::mpsc::channel(crate::UI_COMMAND_QUEUE_CAPACITY);
    let started = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicUsize::new(0));
    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    for _ in 0..crate::UI_COMMAND_MAX_CONCURRENCY + 3 {
        let started = started.clone();
        let cancelled = cancelled.clone();
        let command: crate::UiCommand = Box::pin(async move {
            let _guard = Guard(cancelled);
            started.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
            Box::new(|_: &mut App| {}) as crate::UiCompletion
        });
        commands.send(command).await.unwrap();
    }
    let actor = tokio::spawn(run_command_actor(command_rx, tx));
    tokio::time::timeout(Duration::from_secs(3), async {
        while started.load(Ordering::SeqCst) < crate::UI_COMMAND_MAX_CONCURRENCY {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(rx);
    tokio::time::timeout(Duration::from_secs(3), actor)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while cancelled.load(Ordering::SeqCst) < crate::UI_COMMAND_MAX_CONCURRENCY {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        started.load(Ordering::SeqCst),
        crate::UI_COMMAND_MAX_CONCURRENCY
    );
    assert!(commands.is_closed());
}

#[tokio::test]
async fn closing_ui_unblocks_completion_delivery_without_running_more_commands() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.send(crate::AppMessage::CatalogInvalidated)
        .await
        .unwrap();
    let (commands, command_rx) = tokio::sync::mpsc::channel(1);
    let command: crate::UiCommand =
        Box::pin(async { Box::new(|_: &mut App| {}) as crate::UiCompletion });
    commands.send(command).await.unwrap();
    let actor = tokio::spawn(run_command_actor(command_rx, tx));
    tokio::task::yield_now().await;
    rx.close();
    tokio::time::timeout(Duration::from_secs(3), actor)
        .await
        .unwrap()
        .unwrap();
    assert!(commands.is_closed());
}

fn app() -> (tempfile::TempDir, App) {
    let directory = tempfile::tempdir().unwrap();
    let backend = crate::TuiBackend::isolated_test_backend(directory.path().to_path_buf());
    let mut app = App::new_with_backend(backend, Default::default(), crate::I18n::english());
    app.draft_store_path = directory.path().join("drafts.json");
    app.prompt_history_path = directory.path().join("history.jsonl");
    (directory, app)
}

#[tokio::test]
async fn queue_rejection_completes_the_request_without_running_its_operation() {
    for closed in [false, true] {
        let (_directory, mut app) = app();
        if closed {
            drop(app.command_rx.take());
        } else {
            for _ in 0..crate::UI_COMMAND_QUEUE_CAPACITY {
                let command: crate::UiCommand =
                    Box::pin(async { Box::new(|_: &mut App| {}) as crate::UiCompletion });
                assert!(app.command_tx.try_send(command).is_ok());
            }
        }
        app.session_load.loading = true;
        let calls = Arc::new(AtomicUsize::new(0));
        let received = calls.clone();
        let ran = Arc::new(AtomicBool::new(false));
        let operation_ran = ran.clone();
        app.dispatch_backend_operation(
            move |_| async move {
                operation_ran.store(true, Ordering::SeqCst);
                Ok::<_, anyhow::Error>(())
            },
            move |app, result| {
                assert!(result.is_err());
                app.session_load.loading = false;
                received.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "failure must not reenter the input handler"
        );
        assert!(app.session_load.loading);
        assert!(app.apply_deferred_ui_completions());
        assert!(!app.apply_deferred_ui_completions());
        assert!(!app.session_load.loading);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!ran.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn panic_in_creation_or_polling_still_finishes_the_ui_request() {
    for panic_during_creation in [false, true] {
        let (_directory, mut app) = app();
        app.start_command_actor();
        app.session_load.loading = true;
        let calls = Arc::new(AtomicUsize::new(0));
        let received = calls.clone();
        app.dispatch_backend_operation(
            move |_| {
                assert!(!panic_during_creation, "synthetic creation panic");
                async move {
                    assert!(panic_during_creation, "synthetic polling panic");
                    Ok::<_, anyhow::Error>(())
                }
            },
            move |app, result| {
                assert!(result.is_err());
                app.session_load.loading = false;
                received.fetch_add(1, Ordering::SeqCst);
            },
        );
        let message = tokio::time::timeout(Duration::from_secs(3), app.rx.recv())
            .await
            .unwrap()
            .unwrap();
        app.handle_message(message);
        assert!(!app.session_load.loading);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!app.command_actor.as_ref().unwrap().is_finished());
    }
}

#[tokio::test]
async fn accepted_commands_keep_the_original_success_completion() {
    let (_directory, mut app) = app();
    app.start_command_actor();
    app.session_load.loading = true;
    app.dispatch_backend_operation(
        |_| async { Ok::<_, anyhow::Error>(42) },
        |app, result| {
            assert_eq!(result.unwrap(), 42);
            app.session_load.loading = false;
        },
    );
    let message = tokio::time::timeout(Duration::from_secs(3), app.rx.recv())
        .await
        .unwrap()
        .unwrap();
    app.handle_message(message);
    assert!(!app.session_load.loading);
}

#[tokio::test]
async fn deferred_failures_have_a_per_turn_budget_and_do_not_reenter_retries() {
    let (_directory, mut app) = app();
    let calls = Arc::new(AtomicUsize::new(0));
    for _ in 0..50 {
        let calls = calls.clone();
        app.deferred_ui_completions.push_back(Box::new(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
        }));
    }
    assert!(app.apply_deferred_ui_completions());
    assert_eq!(calls.load(Ordering::SeqCst), 32);
    assert!(app.apply_deferred_ui_completions());
    assert_eq!(calls.load(Ordering::SeqCst), 50);
    assert!(!app.apply_deferred_ui_completions());
    let retried = calls.clone();
    app.deferred_ui_completions.push_back(Box::new(move |app| {
        app.deferred_ui_completions.push_back(Box::new(move |_| {
            retried.fetch_add(1, Ordering::SeqCst);
        }));
    }));
    assert!(app.apply_deferred_ui_completions());
    assert_eq!(calls.load(Ordering::SeqCst), 50);
    assert!(app.apply_deferred_ui_completions());
    assert_eq!(calls.load(Ordering::SeqCst), 51);
}
