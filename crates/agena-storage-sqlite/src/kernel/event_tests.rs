use super::{
    tests::{document, fixture, sql},
    *,
};
use agena_domain::{
    CommandOutputStream, ContentKind, DocumentMutation, TerminalAttributes, TerminalCellRun,
    TerminalSnapshot, ViewBlock,
};

pub(super) async fn tool(ports: &KernelPorts) -> (SessionId, RoundHandle, ToolHandle) {
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "events".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = ports
        .sessions
        .send(RequestId::new(), session, document("events"))
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "events".into(),
                name: "fixture".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    (session, round, tool)
}

pub(super) fn screen(text: &str) -> TerminalSnapshot {
    TerminalSnapshot {
        rows: 2,
        cols: 80,
        cursor_row: 0,
        cursor_col: 0,
        cursor_visible: true,
        alternate_screen: false,
        bracketed_paste: true,
        application_cursor: false,
        cells: vec![
            TerminalCellRun {
                row: 0,
                col: 0,
                text: text.into(),
                attributes: TerminalAttributes::default(),
            },
            TerminalCellRun {
                row: 1,
                col: 0,
                text: "unchanged".into(),
                attributes: TerminalAttributes::default(),
            },
        ],
    }
}

async fn events(
    ports: &KernelPorts,
    session: SessionId,
    content: ContentId,
    limit: usize,
) -> Vec<EventRecord> {
    let mut after = None;
    let mut events = Vec::new();
    loop {
        let page = ports
            .reader
            .event_page(session, content, after, limit)
            .await
            .unwrap();
        assert!(page.records.len() <= limit);
        assert!(page.next.sequence >= after.map_or(0, |cursor: EventCursor| cursor.sequence));
        after = Some(page.next);
        events.extend(page.records.iter().cloned());
        if !page.has_more {
            return events;
        }
    }
}

#[tokio::test]
async fn log_packets_keep_interleaving_time_bytes_and_bounded_pages_without_token_commands() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, round, tool) = tool(&ports).await;
        let output = tool.begin_log(RequestId::new()).await.unwrap().value;
        let mut watch = ports
            .reader
            .watch_content(session, output.id())
            .await
            .unwrap();
        let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_command_results")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        let mut expected = Vec::new();
        for index in 0..2048 {
            let channel = if index % 2 == 0 {
                CommandOutputStream::Stdout
            } else {
                CommandOutputStream::Stderr
            };
            let text = if index == 19 {
                "huge 界🙂\r\n\0\u{1b}[31m".repeat(4096)
            } else {
                format!("{index} 界🙂\r\n\0\u{1b}[31m")
            };
            let timestamp_ms = 100000 + index / 3;
            let cursor = output
                .append_at(timestamp_ms, channel.clone(), &text)
                .await
                .unwrap();
            assert_eq!(cursor.sequence, index as u64 + 1);
            expected.push(EventRecord {
                sequence: cursor.sequence,
                timestamp_ms,
                payload: EventPayload::Log { channel, text },
            });
        }
        assert_eq!(watch.changed().await.unwrap().accepted, 2048);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_command_results")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap(),
            commands
        );
        assert_eq!(events(&ports, session, output.id(), 31).await, expected);
        assert!(matches!(
            ports
                .reader
                .text_range(session, output.id(), None, 100)
                .await,
            Err(KernelError::Invalid(_))
        ));
        assert!(matches!(
            ports
                .reader
                .event_page(
                    session,
                    output.id(),
                    Some(EventCursor {
                        content: output.id(),
                        epoch: ContentEpoch::new(),
                        sequence: 0
                    }),
                    10
                )
                .await,
            Err(KernelError::ContentReset)
        ));
        round.finish(RequestId::new(), None, true).await.unwrap();
        output
            .append_at(999999, CommandOutputStream::Stderr, "after provider round")
            .await
            .unwrap();
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"exit_code":0}),
            },
        )
        .await
        .unwrap();
        let page = ports
            .reader
            .event_page(session, output.id(), None, usize::MAX)
            .await
            .unwrap();
        assert_eq!(page.stream.kind, ContentKind::Log);
        assert_eq!(page.stream.state, CaptureState::Complete);
        assert!(page.records.len() <= 256);
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM agena_content_blocks WHERE content_id=?")
                .bind(output.id().to_string())
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
        assert!(count < 32, "packet envelope amplification: {count}");
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn small_log_pages_reuse_verified_bodies_and_decode_only_the_requested_rows() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, admin, _root, _url) =
            tests::fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_log(RequestId::new()).await.unwrap().value;
        let mut expected = Vec::new();
        for index in 0..8192 {
            let channel = if index % 2 == 0 {
                CommandOutputStream::Stdout
            } else {
                CommandOutputStream::Stderr
            };
            let text = format!("{index:08}:packet!\n");
            assert_eq!(text.len(), 17);
            let timestamp_ms = index / 3 - 100;
            let cursor = output
                .append_at(timestamp_ms, channel.clone(), &text)
                .await
                .unwrap();
            expected.push(EventRecord {
                sequence: cursor.sequence,
                timestamp_ms,
                payload: EventPayload::Log { channel, text },
            });
        }
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"exit_code":0}),
            },
        )
        .await
        .unwrap();
        let (blocks, body_bytes): (i64, i64) = sqlx::query_as(
            "SELECT count(*),sum(length(body)) FROM agena_content_blocks WHERE content_id=?",
        )
        .bind(output.id().to_string())
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        assert!(blocks > 1 && blocks < 32);
        let before = diagnostics.snapshot();
        assert_eq!(events(&ports, session, output.id(), 32).await, expected);
        let first = diagnostics.snapshot();
        // 256 small pages read and verify each immutable physical body once,
        // rather than once per page or once per observer. Prefix records are
        // not decoded merely to seek the next native sequence.
        assert_eq!(
            first.event_block_queries - before.event_block_queries,
            blocks as u64
        );
        assert_eq!(
            first.event_block_rows - before.event_block_rows,
            blocks as u64
        );
        assert_eq!(
            first.event_block_validated_bytes - before.event_block_validated_bytes,
            body_bytes as u64
        );
        assert_eq!(
            first.event_page_decoded_records - before.event_page_decoded_records,
            8192
        );
        assert_eq!(events(&ports, session, output.id(), 32).await, expected);
        let repeated = diagnostics.snapshot();
        assert_eq!(repeated.event_block_queries, first.event_block_queries);
        assert_eq!(
            repeated.event_block_validated_bytes,
            first.event_block_validated_bytes
        );
        assert_eq!(
            repeated.event_page_decoded_records - first.event_page_decoded_records,
            8192
        );
        let page = ports
            .reader
            .event_page(
                session,
                output.id(),
                Some(EventCursor {
                    content: output.id(),
                    epoch: output.epoch(),
                    sequence: 7999,
                }),
                1,
            )
            .await
            .unwrap();
        assert_eq!(page.records.as_slice(), &expected[7999..8000]);
        assert_eq!(
            diagnostics.snapshot().event_page_decoded_records - repeated.event_page_decoded_records,
            1
        );
        drop(page);
        assert!(diagnostics.read_cache().bytes <= 16 * 1024 * 1024);

        let other = ports
            .sessions
            .create(RequestId::new(), 1, "foreign reader".into())
            .await
            .unwrap()
            .value
            .id;
        let before_foreign = diagnostics.snapshot().event_block_queries;
        assert!(matches!(
            ports.reader.event_page(other, output.id(), None, 32).await,
            Err(KernelError::Ownership)
        ));
        assert_eq!(diagnostics.snapshot().event_block_queries, before_foreign);
        ports.shutdown().await.unwrap();
        assert_eq!(diagnostics.read_cache().bytes, 0);
        assert_eq!(diagnostics.read_budget().bytes, 0);
    }
}

#[tokio::test]
async fn document_admission_and_indexed_replay_use_the_same_atomic_projection() {
    for memory in [true, false] {
        let (ports, admin, _root, url) = fixture(memory).await;
        let (session, round, tool) = tool(&ports).await;
        let output = tool.begin_document(RequestId::new()).await.unwrap().value;
        let start = ports
            .reader
            .event_snapshot(session, output.id())
            .await
            .unwrap();
        assert_eq!(start.through.sequence, 1);
        assert!(start.document.as_ref().unwrap().blocks.is_empty());
        output
            .apply(DocumentMutation::Insert {
                after: None,
                block: ViewBlock::Text {
                    id: Some("body".into()),
                    text: "  世界\n\0".into(),
                },
            })
            .await
            .unwrap();
        for _ in 0..150 {
            output
                .apply(DocumentMutation::AppendText {
                    block_id: "body".into(),
                    text: "🙂\r\n".into(),
                })
                .await
                .unwrap();
        }
        let before = ports
            .reader
            .event_snapshot(session, output.id())
            .await
            .unwrap();
        assert!(matches!(
            output
                .apply(DocumentMutation::Remove {
                    block_id: "missing".into()
                })
                .await,
            Err(KernelError::Invalid(_))
        ));
        let after = ports
            .reader
            .event_snapshot(session, output.id())
            .await
            .unwrap();
        assert_eq!(before.through, after.through);
        assert_eq!(before.document, after.document);
        let finished = output.finish(RequestId::new()).await.unwrap();
        assert_eq!(finished.value.committed, 152);
        let restored = ports
            .reader
            .event_snapshot(session, output.id())
            .await
            .unwrap();
        assert_eq!(restored.document, before.document);
        assert_eq!(restored.through, before.through);
        assert_eq!(restored.stream.state, CaptureState::Complete);
        let mut replay = EventProjection::new(EventKind::Structured);
        for record in events(&ports, session, output.id(), 7).await {
            replay.apply(&record).unwrap();
        }
        assert_eq!(replay.document(), before.document.as_deref());
        let latest: i64 = sqlx::query_scalar(
            "SELECT max(logical_sequence) FROM agena_content_checkpoints WHERE content_id=?",
        )
        .bind(output.id().to_string())
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        assert_eq!(latest, 128);
        let id = output.id();
        drop(output);
        drop(tool);
        drop(round);
        drop(admin);
        ports.shutdown().await.unwrap();
        if !memory {
            let reopened = open_session_kernel(&url).await.unwrap();
            let snapshot = reopened.reader.event_snapshot(session, id).await.unwrap();
            assert_eq!(snapshot.document, before.document);
            reopened.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn terminal_patches_reject_missing_stale_and_changed_geometry_bases_and_restore_unchanged_rows()
 {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_terminal(RequestId::new()).await.unwrap().value;
        let missing = EventCursor {
            content: output.id(),
            epoch: output.epoch(),
            sequence: 1,
        };
        assert!(
            output
                .patch(missing, screen("without base"), vec![0, 1])
                .await
                .is_err()
        );
        let mut cursor = output.snapshot(screen("initial")).await.unwrap();
        for index in 0..130 {
            let mut changed = screen(&format!("frame {index} 界"));
            changed.cells.truncate(1);
            changed.cursor_row = 1;
            if index == 0 {
                let mut wrong = changed.clone();
                wrong.cols = 81;
                assert!(matches!(
                    output.patch(cursor, wrong, vec![0]).await,
                    Err(KernelError::Conflict(_))
                ));
                assert!(matches!(
                    output
                        .patch(
                            EventCursor {
                                epoch: ContentEpoch::new(),
                                ..cursor
                            },
                            changed.clone(),
                            vec![0]
                        )
                        .await,
                    Err(KernelError::ContentReset)
                ));
            }
            cursor = output.patch(cursor, changed, vec![0]).await.unwrap();
        }
        let live = ports
            .reader
            .event_snapshot(session, output.id())
            .await
            .unwrap();
        assert_eq!(live.terminal.as_ref().unwrap().cells[1].text, "unchanged");
        output.finish(RequestId::new()).await.unwrap();
        let restored = ports
            .reader
            .event_snapshot(session, output.id())
            .await
            .unwrap();
        assert_eq!(restored.terminal, live.terminal);
        assert_eq!(restored.through, cursor);
        assert_eq!(restored.stream.state, CaptureState::Complete);
        let mut replay = EventProjection::new(EventKind::Terminal);
        for event in events(&ports, session, output.id(), 3).await {
            replay.apply(&event).unwrap();
        }
        assert_eq!(replay.terminal(), live.terminal.as_deref());
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn large_atomic_terminal_frames_use_their_own_admission_and_read_budget() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_terminal(RequestId::new()).await.unwrap().value;
        let mut large = screen("large");
        large.rows = 80;
        large.cols = 80;
        large.cells.clear();
        let text = format!("x{}", "\u{301}".repeat(64));
        for row in 0..80 {
            for col in 0..80 {
                large.cells.push(TerminalCellRun {
                    row,
                    col,
                    text: text.clone(),
                    attributes: TerminalAttributes::default(),
                });
            }
        }
        assert!(serde_json::to_vec(&large).unwrap().len() > 1024 * 1024);
        let cursor = output.snapshot(large.clone()).await.unwrap();
        output.finish(RequestId::new()).await.unwrap();
        let page = ports
            .reader
            .event_page(session, output.id(), None, 1)
            .await
            .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.next, cursor);
        assert!(!page.has_more);
        let restored = ports
            .reader
            .event_snapshot(session, output.id())
            .await
            .unwrap();
        assert_eq!(restored.terminal.as_deref(), Some(&large));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn event_blocks_checkpoints_and_terminal_facts_roll_back_together() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_document(RequestId::new()).await.unwrap().value;
        sql(&admin,"CREATE TABLE injected_failure (id INTEGER REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED)").await;
        sql(&admin,"CREATE TRIGGER reject_terminal AFTER INSERT ON agena_command_results BEGIN INSERT INTO injected_failure VALUES (-1); END").await;
        sql(&admin,"CREATE TRIGGER reject_checkpoint BEFORE UPDATE OF committed_head ON agena_content_streams WHEN NEW.capture_state='open' BEGIN SELECT RAISE(ABORT,'hold ordinary checkpoints'); END").await;
        output
            .apply(DocumentMutation::Insert {
                after: None,
                block: ViewBlock::Text {
                    id: Some("body".into()),
                    text: "pending 世界".into(),
                },
            })
            .await
            .unwrap();
        let request = RequestId::new();
        let result = ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"ok":true}),
        };
        assert!(matches!(
            tool.complete(request, result.clone()).await,
            Err(KernelError::Storage(_))
        ));
        assert_eq!(
            ports.reader.tool(session, tool.id()).await.unwrap().phase,
            ToolPhase::Running
        );
        let pending = ports
            .reader
            .event_page(session, output.id(), None, 10)
            .await
            .unwrap();
        assert_eq!(pending.stream.committed, 1);
        assert_eq!(pending.stream.accepted, 2);
        assert_eq!(pending.stream.state, CaptureState::Open);
        sql(&admin, "DROP TRIGGER reject_terminal").await;
        tool.complete(request, result).await.unwrap();
        assert_eq!(
            ports
                .reader
                .event_snapshot(session, output.id())
                .await
                .unwrap()
                .document
                .as_ref()
                .unwrap()
                .blocks
                .len(),
            1
        );
        assert_eq!(
            ports
                .reader
                .event_page(session, output.id(), None, 10)
                .await
                .unwrap()
                .stream
                .state,
            CaptureState::Complete
        );
        sql(&admin, "DROP TRIGGER reject_checkpoint").await;
        sql(&admin, "DROP TABLE injected_failure").await;
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn process_success_and_capture_interruption_keep_independent_committed_outcomes() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_log(RequestId::new()).await.unwrap().value;
        output
            .append(CommandOutputStream::Stdout, "captured prefix\n世界")
            .await
            .unwrap();
        let request = RequestId::new();
        let interrupted = output
            .interrupt(request, "pipe read failed".into())
            .await
            .unwrap();
        assert_eq!(interrupted.value.state, CaptureState::Interrupted);
        assert_eq!(
            interrupted.value.capture_error.as_deref(),
            Some("pipe read failed")
        );
        assert_eq!(
            interrupted,
            output
                .interrupt(request, "pipe read failed".into())
                .await
                .unwrap()
        );
        assert!(matches!(
            output.finish(RequestId::new()).await,
            Err(KernelError::Conflict(_))
        ));
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"exit_code":0}),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ports.reader.tool(session, tool.id()).await.unwrap().phase,
            ToolPhase::Succeeded
        );
        let page = ports
            .reader
            .event_page(session, output.id(), None, 10)
            .await
            .unwrap();
        assert_eq!(page.stream.state, CaptureState::Interrupted);
        assert_eq!(
            page.stream.capture_error.as_deref(),
            Some("pipe read failed")
        );
        assert_eq!(page.stream.accepted, page.stream.committed);
        assert!(
            matches!(&page.records[0].payload,EventPayload::Log { text,.. } if text=="captured prefix\n世界")
        );
        ports.shutdown().await.unwrap();
    }
}
