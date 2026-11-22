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
async fn sliding_inputs_share_interior_blocks_and_survive_archive_pagination() {
    let (runtime, root, attempt, initial) = fixture("sqlite::memory:").await;
    let user = initial.prompt().await.unwrap().entries[0].entry.id;
    initial.finish(RequestId::new(), None, true).await.unwrap();
    let mut entries = vec![user];
    for number in 1..260 {
        entries.push(
            attempt
                .notice(RequestId::new(), number.to_string())
                .await
                .unwrap()
                .value,
        );
    }

    let mut rounds = Vec::new();
    for start in 0..5 {
        let wanted = &entries[start..start + 256];
        let round = attempt
            .begin_round(RequestId::new(), wanted.to_vec())
            .await
            .unwrap()
            .value;
        let prompt = round.prompt().await.unwrap();
        assert_eq!(
            prompt
                .entries
                .iter()
                .map(|input| input.entry.id)
                .collect::<Vec<_>>(),
            wanted,
        );
        rounds.push(round.id());
        round.finish(RequestId::new(), None, true).await.unwrap();
    }
    let shared: i64 = sea_orm::sqlx::query_scalar(
        "SELECT count(*) FROM agena_round_input_blocks a
         JOIN agena_round_input_blocks b ON a.block_id=b.block_id
         WHERE a.round_id=? AND b.round_id=?",
    )
    .bind(rounds[0].get())
    .bind(rounds[1].get())
    .fetch_one(runtime.database.get_sqlite_connection_pool())
    .await
    .unwrap();
    assert_eq!(shared, 3);
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input-blocks.jsonl");
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    let snapshot = runtime.ports.reader.history_snapshot(root).await.unwrap();
    snapshot.export_jsonl(&mut file).await.unwrap();
    drop((file, snapshot));
    let workspace = runtime
        .workspaces
        .ensure_id("/branch-content-tests")
        .await
        .unwrap();
    let imported = runtime
        .ports
        .sessions
        .import_archive(RequestId::new(), workspace, &path)
        .await
        .unwrap()
        .value
        .id;
    let restored = runtime
        .ports
        .reader
        .rounds(imported, None, 16)
        .await
        .unwrap();
    let windows: Vec<_> = restored
        .iter()
        .filter(|round| round.inputs.len() == 256)
        .collect();
    assert_eq!(windows.len(), 5);
    for (start, round) in windows.into_iter().enumerate() {
        let prompt = runtime
            .ports
            .reader
            .prompt(imported, round.id)
            .await
            .unwrap();
        let positions: Vec<_> = prompt
            .entries
            .iter()
            .map(|input| match &input.entry.payload {
                EntryPayload::User { .. } => 0,
                EntryPayload::Notice { text } => text.parse::<usize>().unwrap(),
                other => panic!("unexpected input {other:?}"),
            })
            .collect();
        assert_eq!(positions, (start..start + 256).collect::<Vec<_>>());
    }
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn sliding_replays_share_blocks_and_archive_restores_frozen_states() {
    use sea_orm::sqlx;

    let (runtime, root, attempt, initial) = fixture("sqlite::memory:").await;
    initial.finish(RequestId::new(), None, true).await.unwrap();
    let mut inputs = Vec::new();
    let mut expected = Vec::new();
    for number in 0..260 {
        let round = attempt
            .begin_round(RequestId::new(), Vec::new())
            .await
            .unwrap()
            .value;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        inputs.push(output.entry_id());
        let state = (number % 3 != 0).then(|| serde_json::json!({"order": number, "text": "甲"}));
        let succeeded = number % 7 != 0;
        let phase = if succeeded {
            ProviderRoundPhase::Succeeded
        } else {
            ProviderRoundPhase::Failed
        };
        round
            .finish(RequestId::new(), state.clone(), succeeded)
            .await
            .unwrap();
        expected.push((phase, state));
    }
    let reader = &runtime.ports.reader;
    let pool = runtime.database.get_sqlite_connection_pool();
    let mut captured = Vec::new();
    for start in 0..5 {
        let round = attempt
            .begin_round(RequestId::new(), inputs[start..start + 256].to_vec())
            .await
            .unwrap()
            .value;
        captured.push(round.id());
        round.finish(RequestId::new(), None, true).await.unwrap();
    }
    let shared: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agena_round_replay_blocks a
         JOIN agena_round_replay_blocks b ON a.block_id=b.block_id
         WHERE a.round_id=? AND b.round_id=?",
    )
    .bind(captured[0].get())
    .bind(captured[1].get())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(shared, 3);
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replay-blocks.jsonl");
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    reader.export_jsonl(root, &mut file).await.unwrap();
    drop(file);
    let workspace = runtime
        .workspaces
        .ensure_id("/branch-content-tests")
        .await
        .unwrap();
    let imported = runtime
        .ports
        .sessions
        .import_archive(RequestId::new(), workspace, &path)
        .await
        .unwrap()
        .value
        .id;
    let restored: Vec<i64> = sqlx::query_scalar(
        "SELECT r.id FROM agena_rounds r WHERE r.session_id=? AND EXISTS(
             SELECT 1 FROM agena_round_replay_blocks q WHERE q.round_id=r.id) ORDER BY r.id",
    )
    .bind(imported.get())
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(restored.len(), 5);
    for (session, rounds) in [
        (root, captured),
        (
            imported,
            restored
                .into_iter()
                .map(|id| RoundId::try_from(id).unwrap())
                .collect(),
        ),
    ] {
        for (start, round) in rounds.into_iter().enumerate() {
            let prompt = reader.prompt(session, round).await.unwrap();
            let facts: Vec<_> = prompt
                .replays
                .iter()
                .map(|replay| (replay.phase, replay.provider_state.clone()))
                .collect();
            assert_eq!(facts, expected[start..start + 256]);
            let budget: (i64, i64) = sqlx::query_as(
                "SELECT sum(b.member_count),sum(b.replay_bytes)
                 FROM agena_round_replay_blocks r CROSS JOIN agena_prompt_replay_blocks b ON b.id=r.block_id
                 WHERE r.round_id=?",
            )
            .bind(round.get())
            .fetch_one(pool)
            .await
            .unwrap();
            let state_bytes: usize = facts
                .iter()
                .filter_map(|(_, state)| state.as_ref())
                .map(|state| serde_json::to_vec(state).unwrap().len())
                .sum();
            assert_eq!(budget, (256, (256 * 256 + state_bytes) as i64));
        }
    }
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn archive_round_sources_cross_byte_and_row_page_boundaries() {
    let (runtime, root, attempt, initial) = fixture("sqlite::memory:").await;
    let diagnostic = "\"".repeat(4096);
    let mut inputs = Vec::new();
    for size in 1..=100 {
        let output = initial
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output.append(&"x".repeat(size)).await.unwrap();
        if size <= 35 {
            output
                .interrupt(RequestId::new(), diagnostic.clone())
                .await
                .unwrap();
        } else {
            output.finish(RequestId::new()).await.unwrap();
        }
        inputs.push(output.entry_id());
    }
    initial.finish(RequestId::new(), None, true).await.unwrap();
    let input_count = inputs.len();
    let captured = attempt
        .begin_round(RequestId::new(), inputs)
        .await
        .unwrap()
        .value;
    assert_eq!(captured.prompt().await.unwrap().sources.len(), input_count);
    captured.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source-pages.jsonl");
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    let reader = &runtime.ports.reader;
    reader.export_jsonl(root, &mut file).await.unwrap();
    drop(file);
    let workspace = runtime
        .workspaces
        .ensure_id("/branch-content-tests")
        .await
        .unwrap();
    let restored = runtime
        .ports
        .sessions
        .import_archive(RequestId::new(), workspace, &path)
        .await
        .unwrap()
        .value
        .id;
    let rounds = reader.rounds(restored, None, 8).await.unwrap();
    let round = rounds
        .iter()
        .find(|round| round.inputs.len() == input_count)
        .unwrap();
    let prompt = reader.prompt(restored, round.id).await.unwrap();
    let mut heads: Vec<_> = prompt.sources.iter().collect();
    heads.sort_unstable_by_key(|head| head.committed);
    assert_eq!(heads.len(), input_count);
    for (index, head) in heads.into_iter().enumerate() {
        assert_eq!(head.committed, index as u64 + 1);
        if index < 35 {
            assert_eq!(head.state, CaptureState::Interrupted);
            assert_eq!(head.capture_error.as_deref(), Some(diagnostic.as_str()));
        } else {
            assert_eq!(head.state, CaptureState::Complete);
            assert!(head.capture_error.is_none());
        }
    }
    drop((prompt, rounds));
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn private_tail_reuse_keeps_the_pinned_snapshot_at_its_original_boundary() {
    let (runtime, root, attempt, _round) = fixture("sqlite::memory:").await;
    let tail = || async {
        sea_orm::sqlx::query_scalar::<_, i64>(
            "SELECT segment_id FROM agena_session_segments WHERE session_id=? ORDER BY sequence DESC LIMIT 1",
        )
        .bind(root.get())
        .fetch_one(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap()
    };
    let original = tail().await;
    let before = attempt
        .notice(RequestId::new(), "Before snapshot".into())
        .await
        .unwrap()
        .value;
    assert_eq!(tail().await, original);

    let snapshot = runtime.ports.reader.history_snapshot(root).await.unwrap();
    attempt
        .notice(RequestId::new(), "After snapshot".into())
        .await
        .unwrap();
    let copied = tail().await;
    assert_ne!(copied, original);
    attempt
        .notice(RequestId::new(), "On the new private tail".into())
        .await
        .unwrap();
    assert_eq!(tail().await, copied);

    runtime
        .ports
        .sessions
        .clear(RequestId::new(), root, ClearMode::History)
        .await
        .unwrap();
    runtime
        .maintenance
        .collect_history_snapshots()
        .await
        .unwrap();
    let page = snapshot.page(None, 8).await.unwrap();
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries.last().unwrap().entry.id, before);
    drop((page, snapshot));
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn selected_history_uses_visible_run_members_and_a_stable_cursor() {
    let (runtime, root, attempt, round) = fixture("sqlite::memory:").await;
    let reader = &runtime.ports.reader;
    let notice = attempt
        .notice(RequestId::new(), "Before the response anchor".into())
        .await
        .unwrap()
        .value;
    let standalone = reader
        .history_window(root, None, &[notice.get()], 2)
        .await
        .unwrap();
    assert_eq!(standalone.entries.len(), 1);
    assert_eq!(standalone.entries[0].id, notice);
    drop(standalone);

    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    let anchor = output.entry_id();
    let branch = runtime
        .ports
        .sessions
        .fork(RequestId::new(), root, anchor)
        .await
        .unwrap()
        .value
        .id;
    let later = attempt
        .notice(RequestId::new(), "Parent only".into())
        .await
        .unwrap()
        .value;

    let runs = reader.message_history(branch, None, 8, 1).await.unwrap();
    assert_eq!(runs.user_message_count, 1);
    let run = runs.messages.iter().find(|run| run.id == anchor).unwrap();
    assert_eq!((run.entry_count, run.loaded_count), (2, 1));
    let older = reader
        .history_window(branch, run.next, &[anchor.get(), anchor.get()], 4)
        .await
        .unwrap();
    assert_eq!(older.entries.len(), 1);
    assert_eq!(older.entries[0].id, notice);
    let all = reader
        .history_window(branch, None, &[anchor.get(), anchor.get()], 4)
        .await
        .unwrap();
    assert_eq!(
        all.entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        [notice, anchor]
    );
    let newest = reader.history_window(root, None, &[], 2).await.unwrap();
    assert_eq!(
        newest
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        [anchor, later]
    );
    let previous = reader
        .history_window(root, newest.next, &[], 2)
        .await
        .unwrap();
    assert_eq!(previous.entries.len(), 2);
    assert!(matches!(
        previous.entries[0].payload,
        EntryPayload::User { .. }
    ));
    assert_eq!(previous.entries[1].id, notice);
    let extra = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    extra.finish(RequestId::new()).await.unwrap();
    let execution = attempt.execution_id();
    let generation = attempt.generation();
    let owned = reader
        .execution_text_sources(root, execution, Some(generation))
        .await
        .unwrap();
    assert_eq!(owned.as_slice(), [extra.id(), output.id()]);
    let inherited = reader
        .execution_text_sources(branch, execution, Some(generation))
        .await
        .unwrap();
    assert_eq!(inherited.as_slice(), [output.id()]);
    assert!(
        reader
            .execution_text_sources(root, execution, Some(generation.next().unwrap()))
            .await
            .unwrap()
            .is_empty()
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn segmented_reply_counts_follow_fork_rewind_and_archive_membership() {
    let (runtime, root, attempt, round) = fixture("sqlite::memory:").await;
    let reader = &runtime.ports.reader;
    let user = round.prompt().await.unwrap().entries[0].entry.id;
    let mut parts = Vec::new();
    for index in 0..130 {
        parts.push(
            attempt
                .notice(RequestId::new(), index.to_string())
                .await
                .unwrap()
                .value,
        );
    }
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    let anchor = output.entry_id();
    parts.push(anchor);
    let queued = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            root,
            ComposerDocument(vec![ComposerNode::Text { text: "Cut".into() }]),
        )
        .await
        .unwrap()
        .value;
    let cut = runtime
        .ports
        .sessions
        .fork(RequestId::new(), root, queued.message_id)
        .await
        .unwrap()
        .value
        .id;
    for index in 0..132 {
        parts.push(
            attempt
                .notice(RequestId::new(), format!("Tail {index}"))
                .await
                .unwrap()
                .value,
        );
    }
    let root_runs = reader.message_history(root, None, 8, 2).await.unwrap();
    let response = root_runs
        .messages
        .iter()
        .find(|run| run.id == anchor)
        .unwrap();
    assert_eq!((response.entry_count, response.loaded_count), (263, 3));
    let mut seen = Vec::new();
    let mut next = None;
    loop {
        let page = reader
            .history_window(root, next, &[user.get(), anchor.get(), anchor.get()], 59)
            .await
            .unwrap();
        seen.extend(page.entries.iter().rev().map(|entry| entry.id));
        next = page.next;
        if next.is_none() {
            break;
        }
    }
    assert_eq!(
        seen,
        parts
            .iter()
            .rev()
            .copied()
            .chain([user])
            .collect::<Vec<_>>()
    );

    output.finish(RequestId::new()).await.unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let next = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), root, queued.id)
        .await
        .unwrap()
        .value;
    next.finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .truncate(RequestId::new(), root, queued.message_id)
        .await
        .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("segmented-reply.jsonl");
    let mut file = tokio::fs::File::create(&archive).await.unwrap();
    let snapshot = reader.history_snapshot(cut).await.unwrap();
    snapshot.export_jsonl(&mut file).await.unwrap();
    drop((file, snapshot));
    let restored = runtime
        .ports
        .sessions
        .import_archive(
            RequestId::new(),
            runtime
                .workspaces
                .ensure_id("/branch-content-tests")
                .await
                .unwrap(),
            &archive,
        )
        .await
        .unwrap()
        .value
        .id;
    for session in [root, cut, restored] {
        let runs = reader.message_history(session, None, 8, 2).await.unwrap();
        assert_eq!(
            runs.messages
                .iter()
                .map(|run| run.entry_count)
                .collect::<Vec<_>>(),
            [1, 131]
        );
        assert!(runs.messages[1].next.is_some());
    }
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn forked_runs_keep_their_members_and_started_count_after_parent_advances() {
    let (runtime, root, attempt, round) = fixture("sqlite::memory:").await;
    let reader = &runtime.ports.reader;
    let notice = attempt
        .notice(RequestId::new(), "Before the fork".into())
        .await
        .unwrap()
        .value;
    let queued = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            root,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Queued at the cut".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let cut = queued.message_id;
    let branch = runtime
        .ports
        .sessions
        .fork(RequestId::new(), root, cut)
        .await
        .unwrap()
        .value
        .id;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    let anchor = output.entry_id();
    output.finish(RequestId::new()).await.unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let pending = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), root, queued.id)
        .await
        .unwrap()
        .value;
    pending
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let root_runs = reader.message_history(root, None, 8, 8).await.unwrap();
    assert_eq!(root_runs.user_message_count, 2);
    assert_eq!(
        root_runs
            .messages
            .iter()
            .find(|run| run.id == anchor)
            .unwrap()
            .entry_count,
        2
    );
    assert!(!root_runs.messages.iter().any(|run| run.id == notice));
    drop(root_runs);
    let newest = reader.message_history(root, None, 1, 8).await.unwrap();
    assert_eq!(newest.messages[0].id, cut);
    assert!(newest.has_more);
    let preceding = reader
        .message_history(root, newest.next, 1, 8)
        .await
        .unwrap();
    assert_eq!(preceding.messages[0].id, anchor);
    assert_eq!(preceding.user_message_count, 2);
    drop((newest, preceding));

    let nested = runtime
        .ports
        .sessions
        .fork(RequestId::new(), branch, cut)
        .await
        .unwrap()
        .value
        .id;
    for session in [branch, nested] {
        let page = reader.message_history(session, None, 8, 8).await.unwrap();
        assert_eq!(page.user_message_count, 1);
        assert!(
            page.messages
                .iter()
                .any(|run| run.id == notice && run.entry_count == 1)
        );
        assert!(
            !page
                .messages
                .iter()
                .any(|run| run.id == cut || run.id == anchor)
        );
        let selected = reader
            .history_window(session, None, &[notice.get(), anchor.get()], 8)
            .await
            .unwrap();
        assert_eq!(selected.entries.len(), 1);
        assert_eq!(selected.entries[0].id, notice);
        assert_eq!(selected.entries[0].response_entry_id, None);
    }

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("branch.jsonl");
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    let snapshot = reader.history_snapshot(nested).await.unwrap();
    let first = snapshot.page(None, 1).await.unwrap();
    assert!(first.has_more);
    let tail = snapshot.page(first.next, 8).await.unwrap();
    assert!(!tail.has_more);
    assert_eq!(
        tail.entries
            .iter()
            .map(|row| row.entry.id)
            .collect::<Vec<_>>(),
        [notice, cut]
    );
    let bytes = runtime.diagnostics.snapshot().snapshot_body_bytes;
    assert!(bytes > 0);
    let report = snapshot.export_jsonl(&mut file).await.unwrap();
    assert_eq!(report.entries, 3);
    assert_eq!(runtime.diagnostics.snapshot().snapshot_body_bytes, bytes);
    drop((snapshot, first, tail));
    drop(file);
    let workspace = runtime
        .workspaces
        .ensure_id("/branch-content-tests")
        .await
        .unwrap();
    let restored = runtime
        .ports
        .sessions
        .import_archive(RequestId::new(), workspace, &path)
        .await
        .unwrap()
        .value
        .id;
    let page = reader.message_history(restored, None, 8, 8).await.unwrap();
    assert_eq!(page.user_message_count, 1);
    let notice = page.entries.iter().find(|entry| {
        matches!(&entry.payload, EntryPayload::Notice { text } if text == "Before the fork")
    }).unwrap();
    assert!(
        page.messages
            .iter()
            .any(|run| run.id == notice.id && run.entry_count == 1)
    );
    drop(page);
    let before_rewind = reader.history_snapshot(root).await.unwrap();
    runtime
        .ports
        .sessions
        .truncate(RequestId::new(), root, cut)
        .await
        .unwrap();
    let remaining = reader.message_history(root, None, 8, 8).await.unwrap();
    assert_eq!(remaining.user_message_count, 1);
    assert!(
        remaining
            .messages
            .iter()
            .all(|run| run.id != cut && run.id != anchor)
    );
    let snapshot = before_rewind.page(None, 8).await.unwrap();
    assert!(snapshot.entries.iter().any(|row| row.entry.id == anchor));
    assert!(snapshot.entries.iter().any(|row| row.entry.id == cut));
    runtime.ports.clone().shutdown().await.unwrap();
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
    let shared_head: i64 = sea_orm::sqlx::query_scalar(
        "SELECT head_owner_session_id FROM agena_branch_sources WHERE session_id=?",
    )
    .bind(nested.get())
    .fetch_one(runtime.database.get_sqlite_connection_pool())
    .await
    .unwrap();
    assert_eq!(shared_head, fork.get());
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
    let activity = tool
        .begin_activity(
            RequestId::new(),
            BackgroundActivityStart {
                kind: BackgroundActivityKind::Monitor,
                label: "Observe the fork cut".into(),
            },
        )
        .await
        .unwrap()
        .value;
    let log = tool.begin_log(RequestId::new()).await.unwrap().value;
    let document = tool.begin_document(RequestId::new()).await.unwrap().value;
    let before = format!("{}{}tail", "A".repeat(64 * 1024), "é".repeat(32 * 1024));
    log.append(CommandOutputStream::Stdout, &before)
        .await
        .unwrap();
    log.record_loss(1, "before loss").unwrap();
    log.record_loss(2, "before loss").unwrap();
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
    let reader = &runtime.ports.reader;
    let frozen_outputs = reader.tool(fork, tool.id()).await.unwrap().outputs.clone();
    assert_eq!(
        frozen_outputs
            .iter()
            .map(|source| source.id)
            .collect::<Vec<_>>(),
        [log.id(), document.id()]
    );
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
    activity
        .end(RequestId::new(), BackgroundActivityOutcome::Stopped)
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
            assert_eq!(
                reader.tool(session, tool.id()).await.unwrap().outputs,
                frozen_outputs
            );
            let page = runtime
                .ports
                .reader
                .event_page(session, log.id(), None, 16)
                .await
                .unwrap();
            assert_eq!(page.records.len(), 1);
            assert!(
                matches!(&page.records[0].payload, EventPayload::Log { text, .. } if text == &before)
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
            assert_eq!(losses.stream.capture_loss.committed_packets, 2);
            assert_eq!(losses.losses.len(), 1);
            assert_eq!(losses.losses[0].bytes, 3);
            let mut inside_window = losses.next;
            inside_window.packet = 1;
            assert!(matches!(
                reader
                    .capture_losses(session, log.id(), Some(inside_window), 16)
                    .await,
                Err(KernelError::Invalid(_))
            ));
            let at_boundary = reader
                .capture_losses(session, log.id(), Some(losses.next), 16)
                .await
                .unwrap();
            assert!(at_boundary.losses.is_empty());
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
            value: serde_json::json!([
                {"resource_id":log.id()},
                {"nested":[{"content_id":document.id()}, {"resource_id":log.id()}]}
            ]),
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

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("frozen-source-activity.jsonl");
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    reader.export_jsonl(nested, &mut file).await.unwrap();
    drop(file);
    let archive = tokio::fs::read_to_string(&path).await.unwrap();
    let offsets: Vec<_> = archive
        .lines()
        .filter_map(|line| {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            (record["kind"] == "content_piece"
                && record["data"]["content_id"] == serde_json::json!(log.id())
                && record["data"]["block_start"] == 0)
                .then(|| record["data"]["byte_offset"].as_u64().unwrap())
        })
        .collect();
    assert_eq!(offsets, [0, 64 * 1024, 128 * 1024]);
    let workspace = runtime
        .workspaces
        .ensure_id("/branch-content-tests")
        .await
        .unwrap();
    let restored = runtime
        .ports
        .sessions
        .import_archive(RequestId::new(), workspace, &path)
        .await
        .unwrap()
        .value
        .id;
    let history = reader.history(restored, None, 16).await.unwrap();
    let tool_id = history
        .entries
        .iter()
        .find_map(|entry| match entry.payload {
            EntryPayload::Tool { tool_id } => Some(tool_id),
            _ => None,
        })
        .unwrap();
    let restored_tool = reader.tool(restored, tool_id).await.unwrap();
    assert_eq!(restored_tool.outputs.len(), 2);
    assert_eq!(restored_tool.activities.len(), 1);
    let restored_activity = reader
        .activity(restored, restored_tool.activities[0])
        .await
        .unwrap();
    assert!(restored_activity.outcome.is_none());
    assert!(restored_activity.finished_at_ms.is_none());
    let restored_log = restored_tool
        .outputs
        .iter()
        .find(|source| source.kind == agena_domain::ContentKind::Log)
        .unwrap();
    let losses = reader
        .capture_losses(restored, restored_log.id, None, 16)
        .await
        .unwrap();
    assert_eq!(restored_log.epoch, losses.stream.epoch);
    assert_eq!(losses.stream.capture_loss.committed_packets, 2);
    assert_eq!(losses.losses[0].bytes, 3);
    let restored_page = reader
        .event_page(restored, restored_log.id, None, 16)
        .await
        .unwrap();
    assert_eq!(restored_page.records.len(), 1);
    assert!(
        matches!(&restored_page.records[0].payload, EventPayload::Log { text, .. } if text == &before)
    );
    let restored_document = restored_tool
        .outputs
        .iter()
        .find(|source| source.kind == agena_domain::ContentKind::Structured)
        .unwrap();
    let pool = runtime.database.get_sqlite_connection_pool();
    let result: String =
        sea_orm::sqlx::query_scalar("SELECT result_json FROM agena_tool_calls WHERE id=?")
            .bind(tool_id.get())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([
            {"resource_id":restored_log.id},
            {"nested":[{"content_id":restored_document.id}, {"resource_id":restored_log.id}]}
        ])
    );

    let reclaimed = runtime
        .maintenance
        .prune_content(RequestId::new(), root, log.id(), 1)
        .await
        .unwrap()
        .value;
    assert_eq!(reclaimed.blocks_removed, 1);
    let pieces: i64 = sea_orm::sqlx::query_scalar(
        "SELECT count(*) FROM agena_content_block_pieces p
         JOIN agena_content_streams s ON s.source_key=p.source_key WHERE s.id=?",
    )
    .bind(log.id().to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(pieces, 0);
    drop((
        restored_page,
        losses,
        restored_activity,
        restored_tool,
        history,
    ));

    runtime.ports.clone().shutdown().await.unwrap();
}
