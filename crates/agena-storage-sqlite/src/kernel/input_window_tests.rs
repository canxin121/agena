use serde_json::json;

use super::{
    snapshot_tests::fixture,
    tests::{document, sql},
    *,
};

async fn seed(ports: &KernelPorts, count: usize) -> (SessionId, Vec<EntryId>, AttemptHandle) {
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "current window".into())
        .await
        .unwrap()
        .value
        .id;
    let mut entries = Vec::new();
    let mut execution = None;
    for index in 0..count {
        let sent = ports
            .sessions
            .send(
                RequestId::new(),
                session,
                document(&format!("user {index}")),
            )
            .await
            .unwrap()
            .value;
        entries.push(sent.message_id);
        execution = Some(sent.id);
    }
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, execution.unwrap())
        .await
        .unwrap()
        .value;
    (session, entries, attempt)
}
fn ids(prompt: &PromptView) -> Vec<EntryId> {
    prompt.entries.iter().map(|entry| entry.entry.id).collect()
}

#[tokio::test]
async fn automatic_rounds_keep_successful_output_when_a_later_round_and_attempt_fail() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let (session, users, attempt) = seed(&ports, 3).await;
        let good = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        assert_eq!(ids(&good.prompt().await.unwrap()), users);
        let output = good
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output.append("successful result").await.unwrap();
        let expected = output.entry_id();
        let opaque = json!({"opaque":{"unknown":[null,true,"\u{754c}"]}});
        good.finish(RequestId::new(), Some(opaque.clone()), true)
            .await
            .unwrap();
        let bad = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let partial = bad.begin_text(RequestId::new(), false).await.unwrap().value;
        partial.append("failed incomplete answer").await.unwrap();
        bad.finish(
            RequestId::new(),
            Some(json!({"must_not_replay":true})),
            false,
        )
        .await
        .unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Failed)
            .await
            .unwrap();
        ports
            .sessions
            .retry(RequestId::new(), session, attempt.execution_id())
            .await
            .unwrap();
        let retry = ports
            .executor
            .claim(RequestId::new(), session, attempt.execution_id())
            .await
            .unwrap()
            .value;
        let next = retry
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let prompt = next.prompt().await.unwrap();
        let mut expected_ids = users.clone();
        expected_ids.push(expected);
        assert_eq!(ids(&prompt), expected_ids);
        assert_eq!(prompt.replays.len(), 1);
        assert_eq!(prompt.replays[0].provider_state, Some(opaque));
        assert_eq!(
            ports
                .reader
                .history(session, None, 200)
                .await
                .unwrap()
                .entries
                .len(),
            5
        );
        // Earlier frozen request facts remain exact after live filtering changes.
        assert!(ids(&bad.prompt().await.unwrap()).contains(&expected));
        assert!(!ids(&bad.prompt().await.unwrap()).contains(&partial.entry_id()));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn changed_compaction_seals_summary_and_keeps_the_entire_concurrently_appended_suffix() {
    for memory in [true, false] {
        let (ports, maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let (session, users, attempt) = seed(&ports, 5).await;
        let history = ports.reader.history_snapshot(session).await.unwrap();
        let compact = attempt
            .begin_compaction(RequestId::new(), 2)
            .await
            .unwrap()
            .value;
        assert_eq!(ids(&compact.prompt().await.unwrap()), users[..2]);
        let output = compact
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output
            .append("  durable \u{754c}\n\0 summary")
            .await
            .unwrap();
        let late_execution = ports
            .sessions
            .send(
                RequestId::new(),
                session,
                document("arrived while compacting"),
            )
            .await
            .unwrap()
            .value;
        let late = late_execution.message_id;
        let request = RequestId::new();
        let opaque = json!({"native_compaction":{"exact":[null,"\u{754c}",true]}});
        let committed = compact
            .commit_compaction(request, output.id(), users[1], Some(opaque.clone()))
            .await
            .unwrap();
        assert_eq!(
            compact
                .commit_compaction(request, output.id(), users[1], Some(opaque.clone()))
                .await
                .unwrap(),
            committed
        );
        assert_eq!(history.page(None, 200).await.unwrap().entries.len(), 5);
        let stream = ports
            .reader
            .watch_content(session, output.id())
            .await
            .unwrap()
            .head()
            .unwrap();
        assert_eq!(stream.state, CaptureState::Complete);
        assert_eq!(stream.accepted, stream.committed);
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let input = round.prompt().await.unwrap();
        let mut expected = vec![committed.value];
        expected.extend_from_slice(&users[2..]);
        assert_eq!(ids(&input), expected);
        assert_eq!(input.sources.len(), 1);
        assert_eq!(input.sources[0].id, output.id());
        assert_eq!(input.sources[0].entry, output.entry_id());
        assert!(
            matches!(input.entries[0].entry.payload,EntryPayload::Compaction {summary,through} if summary==output.id()&&through==users[1])
        );
        assert_eq!(input.replays[0].provider_state, Some(opaque));
        let bytes = ports
            .reader
            .text_range_at(session, &input.sources[0], None, 1000)
            .await
            .unwrap();
        assert_eq!(bytes.bytes, "  durable \u{754c}\n\0 summary".as_bytes());
        assert!(matches!(
            maintenance
                .prune_content(RequestId::new(), session, output.id(), 1)
                .await,
            Err(KernelError::Conflict(_))
        ));
        assert_eq!(ids(&compact.prompt().await.unwrap()), users[..2]);
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        let next = ports
            .executor
            .claim(RequestId::new(), session, late_execution.id)
            .await
            .unwrap()
            .value;
        let next_round = next
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        expected.push(late);
        assert_eq!(ids(&next_round.prompt().await.unwrap()), expected);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn invalid_summary_boundary_or_rejected_commit_preserves_the_old_window_and_accepted_tail() {
    for memory in [true, false] {
        let (ports, _maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let (session, users, attempt) = seed(&ports, 4).await;
        let compact = attempt
            .begin_compaction(RequestId::new(), 2)
            .await
            .unwrap()
            .value;
        let output = compact
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output.append("accepted summary").await.unwrap();
        assert!(matches!(
            compact
                .commit_compaction(RequestId::new(), output.id(), users[3], None)
                .await,
            Err(KernelError::NotFound(_))
        ));
        assert_eq!(
            ports
                .reader
                .watch_content(session, output.id())
                .await
                .unwrap()
                .head()
                .unwrap()
                .state,
            CaptureState::Open
        );
        let checkpoint: Option<i64> = sqlx::query_scalar(
            "SELECT checkpoint_entry_id FROM agena_prompt_windows WHERE session_id=?",
        )
        .bind(session.get())
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        assert_eq!(checkpoint, None);
        sql(&admin,"CREATE TABLE compaction_commit_failure (session_id INTEGER REFERENCES agena_sessions(id) DEFERRABLE INITIALLY DEFERRED)").await;
        sql(&admin,"CREATE TRIGGER reject_compaction_commit AFTER UPDATE OF checkpoint_entry_id ON agena_prompt_windows BEGIN INSERT INTO compaction_commit_failure VALUES (-123); END").await;
        let request = RequestId::new();
        assert!(
            compact
                .commit_compaction(request, output.id(), users[1], None)
                .await
                .is_err()
        );
        assert_eq!(
            ports
                .reader
                .watch_content(session, output.id())
                .await
                .unwrap()
                .head()
                .unwrap()
                .state,
            CaptureState::Open
        );
        assert_eq!(
            ports
                .reader
                .history(session, None, 200)
                .await
                .unwrap()
                .entries
                .len(),
            5
        );
        sql(&admin, "DROP TRIGGER reject_compaction_commit").await;
        let checkpoint = compact
            .commit_compaction(request, output.id(), users[1], None)
            .await
            .unwrap()
            .value;
        let next = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        assert_eq!(
            ids(&next.prompt().await.unwrap()),
            [checkpoint, users[2], users[3]]
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn unchanged_failed_and_cancelled_compactions_do_not_install_checkpoints_or_replay_their_output()
 {
    for outcome in ["unchanged", "failed", "cancelled"] {
        for memory in [true, false] {
            let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
            let (session, users, attempt) = seed(&ports, 3).await;
            let compact = attempt
                .begin_compaction(RequestId::new(), 2)
                .await
                .unwrap()
                .value;
            assert!(matches!(
                compact
                    .begin_tool(
                        RequestId::new(),
                        ToolStart {
                            call_id: "not-a-tool-round".into(),
                            name: "fixture".into(),
                            input: json!({})
                        }
                    )
                    .await,
                Err(KernelError::Conflict(_))
            ));
            let output = compact
                .begin_text(RequestId::new(), false)
                .await
                .unwrap()
                .value;
            output.append("not a committed checkpoint").await.unwrap();
            if outcome == "cancelled" {
                ports
                    .sessions
                    .cancel(RequestId::new(), session, attempt.execution_id())
                    .await
                    .unwrap();
                assert!(
                    compact
                        .commit_compaction(RequestId::new(), output.id(), users[1], None)
                        .await
                        .is_err()
                );
                ports
                    .sessions
                    .retry(RequestId::new(), session, attempt.execution_id())
                    .await
                    .unwrap();
            } else {
                compact
                    .finish(
                        RequestId::new(),
                        Some(json!({"not_for_replay":true})),
                        outcome == "unchanged",
                    )
                    .await
                    .unwrap();
            }
            let active = if outcome == "cancelled" {
                ports
                    .executor
                    .claim(RequestId::new(), session, attempt.execution_id())
                    .await
                    .unwrap()
                    .value
            } else {
                attempt
            };
            let round = active
                .begin_current_round(RequestId::new())
                .await
                .unwrap()
                .value;
            let prompt = round.prompt().await.unwrap();
            assert_eq!(ids(&prompt), users);
            assert!(prompt.replays.is_empty());
            assert!(prompt.sources.is_empty());
            ports.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn competing_compactions_compare_their_base_and_a_checkpoint_cannot_cover_its_unseen_suffix()
{
    for memory in [true, false] {
        let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let (session, users, attempt) = seed(&ports, 6).await;
        let other_execution = ports
            .reader
            .entry(session, users[0])
            .await
            .unwrap()
            .execution_id;
        let other = ports
            .executor
            .claim(RequestId::new(), session, other_execution)
            .await
            .unwrap()
            .value;
        let first = attempt
            .begin_compaction(RequestId::new(), 3)
            .await
            .unwrap()
            .value;
        let second = other
            .begin_compaction(RequestId::new(), 3)
            .await
            .unwrap()
            .value;
        let summary1 = first
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        summary1.append("first summary").await.unwrap();
        let summary2 = second
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        summary2.append("stale summary").await.unwrap();
        let checkpoint = first
            .commit_compaction(RequestId::new(), summary1.id(), users[1], None)
            .await
            .unwrap()
            .value;
        assert!(matches!(
            second
                .commit_compaction(RequestId::new(), summary2.id(), users[2], None)
                .await,
            Err(KernelError::Conflict(_))
        ));
        second.finish(RequestId::new(), None, false).await.unwrap();
        let next = attempt
            .begin_compaction(RequestId::new(), 2)
            .await
            .unwrap()
            .value;
        assert_eq!(ids(&next.prompt().await.unwrap()), [checkpoint, users[2]]);
        let summary = next
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        summary.append("new prefix summary").await.unwrap();
        // Its checkpoint appears physically after the retained user suffix.
        // Treating that checkpoint's position as coverage would erase inputs
        // that were never present in this two-entry request.
        assert!(matches!(
            next.commit_compaction(RequestId::new(), summary.id(), checkpoint, None)
                .await,
            Err(KernelError::NotFound(_))
        ));
        let checkpoint2 = next
            .commit_compaction(RequestId::new(), summary.id(), users[2], None)
            .await
            .unwrap()
            .value;
        let normal = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        assert_eq!(
            ids(&normal.prompt().await.unwrap()),
            [checkpoint2, users[3], users[4], users[5]]
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn oversized_current_input_rolls_back_without_shortening_and_prefix_compaction_makes_progress()
 {
    for memory in [true, false] {
        let (ports, _maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "oversized current input".into())
            .await
            .unwrap()
            .value
            .id;
        let mut users = Vec::new();
        let mut execution = None;
        let body = "\u{754c}".repeat(60_000);
        for index in 0..50 {
            let sent = ports
                .sessions
                .send(
                    RequestId::new(),
                    session,
                    document(&format!("{index}: {body}")),
                )
                .await
                .unwrap()
                .value;
            users.push(sent.message_id);
            execution = Some(sent.id);
        }
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, execution.unwrap())
            .await
            .unwrap()
            .value;
        assert!(matches!(
            attempt.begin_current_round(RequestId::new()).await,
            Err(KernelError::Invalid(_))
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_rounds")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_round_inputs")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap(),
            0
        );
        let compact = attempt
            .begin_compaction(RequestId::new(), 10)
            .await
            .unwrap()
            .value;
        assert_eq!(ids(&compact.prompt().await.unwrap()), users[..10]);
        let summary = compact
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        summary.append("bounded historical prefix").await.unwrap();
        let checkpoint = compact
            .commit_compaction(RequestId::new(), summary.id(), users[9], None)
            .await
            .unwrap()
            .value;
        let next = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let mut expected = vec![checkpoint];
        expected.extend_from_slice(&users[10..]);
        assert_eq!(ids(&next.prompt().await.unwrap()), expected);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn current_window_candidate_walk_uses_membership_indexes_without_decoding_entry_bodies() {
    for memory in [true, false] {
        let (ports, _maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let pool = admin.get_sqlite_connection_pool();
        let plan = sqlx::query(&format!(
            "EXPLAIN QUERY PLAN {}",
            input_window::CANDIDATES_SQL
        ))
        .bind(1i64)
        .bind(0i64)
        .bind(128i64)
        .bind(10001i64)
        .fetch_all(pool)
        .await
        .unwrap();
        let details = plan
            .iter()
            .map(|r| r.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(details.contains("SEARCH m"), "{details}");
        assert!(!details.contains("SCAN e"), "{details}");
        let root: i64 =
            sqlx::query_scalar("SELECT rootpage FROM sqlite_schema WHERE name='agena_entries'")
                .fetch_one(pool)
                .await
                .unwrap();
        let columns = sqlx::query("PRAGMA table_xinfo(agena_entries)")
            .fetch_all(pool)
            .await
            .unwrap();
        let json_column = columns
            .iter()
            .find(|r| r.get::<String, _>("name") == "body_json")
            .unwrap()
            .get::<i64, _>("cid");
        let code = sqlx::query(&format!("EXPLAIN {}", input_window::CANDIDATES_SQL))
            .bind(1i64)
            .bind(0i64)
            .bind(128i64)
            .bind(10001i64)
            .fetch_all(pool)
            .await
            .unwrap();
        let cursors = code
            .iter()
            .filter(|r| r.get::<String, _>("opcode") == "OpenRead" && r.get::<i64, _>("p2") == root)
            .map(|r| r.get::<i64, _>("p1"))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            !code.iter().any(|r| r.get::<String, _>("opcode") == "Column"
                && cursors.contains(&r.get::<i64, _>("p1"))
                && r.get::<i64, _>("p2") == json_column),
            "candidate walk decoded a cold entry body"
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn membership_rewrite_epoch_rejects_an_inflight_compaction_even_with_the_same_checkpoint() {
    for memory in [true, false] {
        let (ports, _maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let (session, users, attempt) = seed(&ports, 3).await;
        let compact = attempt
            .begin_compaction(RequestId::new(), 2)
            .await
            .unwrap()
            .value;
        let output = compact
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output
            .append("summary of superseded membership")
            .await
            .unwrap();
        // Exercise the guard before P7 supplies the destructive membership
        // commands. Appending alone deliberately does not advance this epoch.
        sqlx::query(
            "UPDATE agena_prompt_windows SET rewrite_epoch=rewrite_epoch+1 WHERE session_id=?",
        )
        .bind(session.get())
        .execute(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        assert!(matches!(
            compact
                .commit_compaction(RequestId::new(), output.id(), users[1], None)
                .await,
            Err(KernelError::Conflict(_))
        ));
        assert_eq!(
            ports
                .reader
                .watch_content(session, output.id())
                .await
                .unwrap()
                .head()
                .unwrap()
                .state,
            CaptureState::Open
        );
        compact.finish(RequestId::new(), None, false).await.unwrap();
        let next = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        assert_eq!(ids(&next.prompt().await.unwrap()), users);
        assert!(
            !ports
                .reader
                .history(session, None, 200)
                .await
                .unwrap()
                .entries
                .iter()
                .any(|entry| matches!(entry.payload, EntryPayload::Compaction { .. }))
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn source_capture_searches_both_owners_and_checkpoint_references_without_reading_cold_bodies()
{
    for memory in [true, false] {
        let (ports, _maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let pool = admin.get_sqlite_connection_pool();
        let entry_root: i64 =
            sqlx::query_scalar("SELECT rootpage FROM sqlite_schema WHERE name='agena_entries'")
                .fetch_one(pool)
                .await
                .unwrap();
        let columns = sqlx::query("PRAGMA table_xinfo(agena_entries)")
            .fetch_all(pool)
            .await
            .unwrap();
        let body_column = columns
            .iter()
            .find(|r| r.get::<String, _>("name") == "body_json")
            .unwrap()
            .get::<i64, _>("cid");
        for (query, bindings) in [
            (commands::SOURCE_SIZE_SQL, 1),
            (commands::CAPTURE_SOURCES_SQL, 2),
        ] {
            let plan_sql = format!("EXPLAIN QUERY PLAN {query}");
            let mut plan_query = sqlx::query(&plan_sql);
            for _ in 0..bindings {
                plan_query = plan_query.bind(1i64);
            }
            let details = plan_query
                .fetch_all(pool)
                .await
                .unwrap()
                .iter()
                .map(|r| r.get::<String, _>("detail"))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                details.contains("SEARCH input")
                    && details.contains("SEARCH e USING INTEGER PRIMARY KEY"),
                "{details}"
            );
            assert!(
                details.contains("MULTI-INDEX OR")
                    && details.contains("idx_agena_content_entry (entry_id=?)")
                    && details.contains("(id=?)"),
                "{details}"
            );
            assert!(
                !details.contains("SCAN c")
                    && !details.contains("SCAN e")
                    && !details.contains("SCAN input"),
                "{details}"
            );
            let code_sql = format!("EXPLAIN {query}");
            let mut code_query = sqlx::query(&code_sql);
            for _ in 0..bindings {
                code_query = code_query.bind(1i64);
            }
            let code = code_query.fetch_all(pool).await.unwrap();
            let cursors = code
                .iter()
                .filter(|r| {
                    r.get::<String, _>("opcode") == "OpenRead"
                        && r.get::<i64, _>("p2") == entry_root
                })
                .map(|r| r.get::<i64, _>("p1"))
                .collect::<std::collections::BTreeSet<_>>();
            assert!(
                !code.iter().any(|r| r.get::<String, _>("opcode") == "Column"
                    && cursors.contains(&r.get::<i64, _>("p1"))
                    && r.get::<i64, _>("p2") == body_column),
                "source capture decoded an entry body"
            );
        }
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn queued_input_stays_out_of_followups_and_compaction_never_skips_its_boundary() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let (session, users, attempt) = seed(&ports, 2).await;
        let queued = ports
            .sessions
            .send(RequestId::new(), session, document("queued input"))
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        assert_eq!(ids(&round.prompt().await.unwrap()), users);
        let text = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        text.append("answer appended after queued input")
            .await
            .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        let follow_up = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let mut expected = users.clone();
        expected.push(text.entry_id());
        assert_eq!(ids(&follow_up.prompt().await.unwrap()), expected);
        follow_up
            .finish(RequestId::new(), None, true)
            .await
            .unwrap();
        let compact = attempt
            .begin_compaction(RequestId::new(), 128)
            .await
            .unwrap()
            .value;
        assert_eq!(ids(&compact.prompt().await.unwrap()), users);
        let summary = compact
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        summary.append("summary before queued input").await.unwrap();
        let checkpoint = compact
            .commit_compaction(RequestId::new(), summary.id(), *users.last().unwrap(), None)
            .await
            .unwrap()
            .value;
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        let next = ports
            .executor
            .claim(RequestId::new(), session, queued.id)
            .await
            .unwrap()
            .value;
        let next_round = next
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let next_ids = ids(&next_round.prompt().await.unwrap());
        assert!(next_ids.contains(&checkpoint));
        assert!(
            next_ids.contains(&queued.message_id),
            "compaction must not consume queued input"
        );
        assert!(next_ids.contains(&text.entry_id()));
        assert!(matches!(
            ports
                .sessions
                .cancel_queued(RequestId::new(), session, queued.id)
                .await,
            Err(KernelError::Conflict(_))
        ));
        assert_eq!(
            ports
                .reader
                .execution(session, queued.id)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Running
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cancelling_an_unclaimed_retry_retains_user_input_used_by_an_earlier_attempt() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let (session, users, attempt) = seed(&ports, 1).await;
        attempt
            .finish(RequestId::new(), ExecutionPhase::Failed)
            .await
            .unwrap();
        ports
            .sessions
            .retry(RequestId::new(), session, attempt.execution_id())
            .await
            .unwrap();
        ports
            .sessions
            .cancel_queued(RequestId::new(), session, attempt.execution_id())
            .await
            .unwrap();
        let next = ports
            .sessions
            .send(RequestId::new(), session, document("after cancelled retry"))
            .await
            .unwrap()
            .value;
        let next_attempt = ports
            .executor
            .claim(RequestId::new(), session, next.id)
            .await
            .unwrap()
            .value;
        let round = next_attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        assert_eq!(
            ids(&round.prompt().await.unwrap()),
            [users[0], next.message_id]
        );
        ports.shutdown().await.unwrap();
    }
}
