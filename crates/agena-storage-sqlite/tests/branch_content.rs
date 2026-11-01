use agena_domain::{
    CommandOutputStream, ComposerDocument, ComposerNode, DocumentMutation, ViewBlock,
};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use sea_orm::sqlx::Connection;

async fn fixture(url: &str) -> (SqliteKernel, SessionId, AttemptHandle, RoundHandle) {
    let runtime = open_kernel_runtime(url).await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/branch-content-tests")
        .await
        .unwrap();
    let root = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Source".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let execution = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            root,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Stream a response".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), root, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    (runtime, root, attempt, round)
}

#[tokio::test]
async fn nested_branches_freeze_text_and_keep_their_prefix_after_source_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let source_url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("source.db").display()
    );
    let (runtime, root, attempt, round) = fixture(&source_url).await;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    output.append("BEFORE").await.unwrap();
    // Fork itself must checkpoint accepted bytes; this test deliberately does
    // not finish the source or wait for the periodic checkpoint worker.
    let fork = runtime
        .ports
        .sessions
        .fork(RequestId::new(), root, output.entry_id())
        .await
        .unwrap()
        .value
        .id;
    let before = runtime
        .ports
        .reader
        .text_range(fork, output.id(), None, 64)
        .await
        .unwrap();
    assert_eq!(before.bytes, b"BEFORE");
    let frozen = before.stream.clone();
    drop(before);
    let mut watch = runtime
        .ports
        .reader
        .watch_content(fork, output.id())
        .await
        .unwrap();
    assert!(watch.is_snapshot());
    assert_eq!(watch.head().unwrap().accepted, 6);
    assert_eq!(watch.head().unwrap().state, CaptureState::Open);
    assert!(
        !runtime
            .ports
            .reader
            .watch_content(root, output.id())
            .await
            .unwrap()
            .is_snapshot()
    );
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), watch.changed()).await,
        Ok(Err(KernelError::Closed))
    ));
    output.append("-AFTER").await.unwrap();
    assert_eq!(
        runtime
            .ports
            .reader
            .text_range(root, output.id(), None, 64)
            .await
            .unwrap()
            .bytes,
        b"BEFORE-AFTER"
    );
    let nested = runtime
        .ports
        .sessions
        .fork(RequestId::new(), fork, output.entry_id())
        .await
        .unwrap()
        .value
        .id;
    for session in [fork, nested] {
        assert_eq!(
            runtime
                .ports
                .reader
                .text_range(session, output.id(), None, 64)
                .await
                .unwrap()
                .bytes,
            b"BEFORE"
        );
        let mut oversized = frozen.clone();
        oversized.accepted += 1;
        oversized.committed += 1;
        assert!(matches!(
            runtime
                .ports
                .reader
                .text_range_at(session, &oversized, None, 64)
                .await,
            Err(KernelError::ContentReset)
        ));
    }

    // Copy a crash-consistent SQLite image while the source is still open.
    // Recovery of that image rotates its epoch; the branch's trusted cut must
    // continue reading the old committed prefix without inheriting live tail.
    let image = directory.path().join("recovery.db");
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(image.to_string_lossy().as_ref())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    assert!(std::fs::metadata(&image).unwrap().len() > 0);
    let url = format!("sqlite://{}?mode=ro", image.display());
    let mut copied = sea_orm::sqlx::SqliteConnection::connect(&url)
        .await
        .unwrap();
    assert_eq!(
        sea_orm::sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_sessions")
            .fetch_one(&mut copied)
            .await
            .unwrap(),
        3
    );
    copied.close().await.unwrap();
    let content = output.id();
    runtime.ports.clone().shutdown().await.unwrap();
    drop((watch, output, round, attempt, runtime));

    let url = format!("sqlite://{}?mode=rwc", image.display());
    let recovered = open_kernel_runtime(&url).await.unwrap();
    let root_head = recovered
        .ports
        .reader
        .text_range(root, content, None, 64)
        .await
        .unwrap()
        .stream
        .clone();
    assert_ne!(root_head.epoch, frozen.epoch);
    for session in [fork, nested] {
        let range = recovered
            .ports
            .reader
            .text_range(session, content, None, 64)
            .await
            .unwrap();
        assert_eq!(range.bytes, b"BEFORE");
        assert_eq!(range.stream.epoch, frozen.epoch);
        assert_eq!(
            recovered
                .ports
                .reader
                .text_range_at(session, &frozen, None, 64)
                .await
                .unwrap()
                .bytes,
            b"BEFORE"
        );
    }
    assert!(matches!(
        recovered
            .ports
            .reader
            .text_range_at(root, &frozen, None, 64)
            .await,
        Err(KernelError::ContentReset)
    ));
    recovered
        .maintenance
        .prune_content(RequestId::new(), root, content, 6)
        .await
        .unwrap();
    assert!(matches!(
        recovered
            .ports
            .reader
            .text_range(fork, content, None, 64)
            .await,
        Err(KernelError::ContentGap {
            retained_start: 6,
            ..
        })
    ));
    recovered.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn inherited_event_pages_snapshots_losses_and_sources_stop_at_the_fork_cut() {
    let (runtime, root, attempt, round) = fixture("sqlite::memory:").await;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "stream-tool".into(),
                name: "test.stream".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let log = tool.begin_log(RequestId::new()).await.unwrap().value;
    let document = tool.begin_document(RequestId::new()).await.unwrap().value;
    log.append(CommandOutputStream::Stdout, "BEFORE")
        .await
        .unwrap();
    log.record_loss(3, "before loss").unwrap();
    document
        .apply(DocumentMutation::Insert {
            after: None,
            block: ViewBlock::Text {
                id: Some("text".into()),
                text: "BEFORE".into(),
            },
        })
        .await
        .unwrap();
    let fork = runtime
        .ports
        .sessions
        .fork(RequestId::new(), root, log.entry_id())
        .await
        .unwrap()
        .value
        .id;
    let original_document = runtime
        .ports
        .reader
        .event_snapshot(fork, document.id())
        .await
        .unwrap()
        .document
        .clone();
    let mut watch = runtime
        .ports
        .reader
        .watch_content(fork, log.id())
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), watch.changed()).await,
        Ok(Err(KernelError::Closed))
    ));

    log.append(CommandOutputStream::Stdout, "AFTER")
        .await
        .unwrap();
    log.record_loss(5, "after loss").unwrap();
    document
        .apply(DocumentMutation::AppendText {
            block_id: "text".into(),
            text: "-AFTER".into(),
        })
        .await
        .unwrap();
    // A new output can share the already inherited tool entry. Entry
    // membership alone must not grant access to this later source.
    let late = tool.begin_log(RequestId::new()).await.unwrap().value;
    late.append(CommandOutputStream::Stdout, "LATE SOURCE")
        .await
        .unwrap();
    let nested = runtime
        .ports
        .sessions
        .fork(RequestId::new(), fork, log.entry_id())
        .await
        .unwrap()
        .value
        .id;

    for _ in 0..2 {
        assert_eq!(
            runtime
                .ports
                .reader
                .event_page(root, log.id(), None, 16)
                .await
                .unwrap()
                .records
                .len(),
            2
        );
        assert_ne!(
            runtime
                .ports
                .reader
                .event_snapshot(root, document.id())
                .await
                .unwrap()
                .document,
            original_document
        );
        for session in [fork, nested] {
            let page = runtime
                .ports
                .reader
                .event_page(session, log.id(), None, 16)
                .await
                .unwrap();
            assert_eq!(page.records.len(), 1);
            assert!(
                matches!(&page.records[0].payload, EventPayload::Log { text, .. } if text == "BEFORE")
            );
            assert!(!page.has_more);
            assert_eq!(
                runtime
                    .ports
                    .reader
                    .event_snapshot(session, document.id())
                    .await
                    .unwrap()
                    .document,
                original_document
            );
            let losses = runtime
                .ports
                .reader
                .capture_losses(session, log.id(), None, 16)
                .await
                .unwrap();
            assert_eq!(losses.stream.capture_loss.committed_packets, 1);
            assert_eq!(losses.losses.len(), 1);
            assert_eq!(losses.losses[0].bytes, 3);
            assert!(matches!(
                runtime
                    .ports
                    .reader
                    .event_page(session, late.id(), None, 16)
                    .await,
                Err(KernelError::Ownership)
            ));
            assert!(matches!(
                runtime.ports.reader.watch_content(session, late.id()).await,
                Err(KernelError::Ownership)
            ));
        }
    }
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!("done"),
        },
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .ports
            .reader
            .event_snapshot(fork, document.id())
            .await
            .unwrap()
            .document,
        original_document
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .event_page(fork, log.id(), None, 16)
            .await
            .unwrap()
            .records
            .len(),
        1
    );
    runtime.ports.clone().shutdown().await.unwrap();
}
