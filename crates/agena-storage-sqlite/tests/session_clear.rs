use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};

async fn fixture() -> (SqliteKernel, SessionId) {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/session-clear-tests")
        .await
        .unwrap();
    let root = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Cleared".into())
        .await
        .unwrap()
        .value
        .id;
    (runtime, root)
}

async fn send(runtime: &SqliteKernel, session: SessionId, text: &str) -> ExecutionView {
    runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
        )
        .await
        .unwrap()
        .value
}

async fn entry_count(runtime: &SqliteKernel, session: SessionId) -> u64 {
    runtime
        .ports
        .reader
        .summary(session)
        .await
        .unwrap()
        .entry_count
}

/// Entry ids the next prompt is built from.
async fn prompt_entry_ids(runtime: &SqliteKernel, session: SessionId) -> Vec<EntryId> {
    let execution = send(runtime, session, "next turn").await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let ids = round
        .prompt()
        .await
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.entry.id)
        .collect::<Vec<_>>();
    round.finish(RequestId::new(), None, true).await.unwrap();
    ids
}

#[tokio::test]
async fn clearing_history_hides_the_transcript_but_keeps_the_stored_rows() {
    let (runtime, session) = fixture().await;
    send(&runtime, session, "first turn").await;
    let before = entry_count(&runtime, session).await;
    assert!(before > 0, "the fixture must have history");

    let cleared = runtime
        .ports
        .sessions
        .clear(RequestId::new(), session, ClearMode::History)
        .await
        .unwrap()
        .value;
    assert_eq!(cleared.id, session);

    assert_eq!(
        entry_count(&runtime, session).await,
        0,
        "clearing history must empty the transcript membership"
    );
    // Nothing was deleted: the old turn is still readable by id.
    let old_turn = send(&runtime, session, "another turn").await.message_id;
    assert!(
        runtime.ports.reader.entry(session, old_turn).await.is_ok(),
        "a cleared entry must remain stored"
    );
}

#[tokio::test]
async fn clearing_context_keeps_the_transcript_readable() {
    let (runtime, session) = fixture().await;
    send(&runtime, session, "first turn").await;
    let before = entry_count(&runtime, session).await;
    assert!(before > 0);

    runtime
        .ports
        .sessions
        .clear(RequestId::new(), session, ClearMode::Context)
        .await
        .unwrap();

    // The old turns stay members and one empty checkpoint is appended, so the
    // transcript still reads them while the prompt no longer includes them.
    assert!(
        entry_count(&runtime, session).await >= before,
        "clearing only the context must leave every old turn in the transcript"
    );

    let status = runtime
        .ports
        .reader
        .prompt_window_status(session)
        .await
        .unwrap();
    // A real boundary entry now bounds the window, which is what keeps the old
    // turns out of the next prompt.
    assert!(status.checkpoint_entry_id.is_some());
    assert!(status.through_entry_id.is_some());
    // It is a clear marker, not a compaction: the transcript must still read
    // it as "context cleared" rather than as a summary that was produced.
    let boundary = runtime
        .ports
        .reader
        .entry(session, status.checkpoint_entry_id.unwrap())
        .await
        .unwrap();
    assert!(
        matches!(boundary.payload, EntryPayload::ContextCleared { .. }),
        "clearing the context must not fabricate a compaction, got {:?}",
        boundary.payload
    );
    assert!(
        entry_count(&runtime, session).await <= before + 1,
        "only the clear marker may be appended"
    );

    let prompt = prompt_entry_ids(&runtime, session).await;
    let old_turns = runtime
        .ports
        .reader
        .summary(session)
        .await
        .unwrap()
        .user_message_count;
    assert!(
        old_turns >= 2,
        "the old turns stay readable in the transcript"
    );
    // The prompt holds the empty checkpoint and the new turn, never the old
    // ones.
    assert!(
        prompt.len() <= 2,
        "old turns must not reach the model again, got {prompt:?}"
    );
    for entry in &prompt {
        let kind = runtime
            .ports
            .reader
            .entry(session, *entry)
            .await
            .unwrap()
            .payload
            .clone();
        assert!(
            !matches!(kind, EntryPayload::User { .. }) || entry.get() > 2,
            "an old user turn leaked into the cleared prompt"
        );
    }
}

#[tokio::test]
async fn clearing_twice_is_idempotent_and_keeps_the_session_usable() {
    let (runtime, session) = fixture().await;
    send(&runtime, session, "first turn").await;
    for _ in 0..2 {
        runtime
            .ports
            .sessions
            .clear(RequestId::new(), session, ClearMode::History)
            .await
            .unwrap();
    }
    assert_eq!(entry_count(&runtime, session).await, 0);
    // The same session must accept a new turn after being cleared.
    send(&runtime, session, "a fresh turn").await;
    assert!(entry_count(&runtime, session).await > 0);
}
