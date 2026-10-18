use super::{
    tests::{document, fixture, sql},
    *,
};

pub(super) async fn round(ports: &KernelPorts) -> (SessionId, ExecutionView, RoundHandle) {
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "text".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = ports
        .sessions
        .send(RequestId::new(), session, document("stream"))
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
    (session, sent, round)
}

async fn all(
    ports: &KernelPorts,
    session: SessionId,
    content: ContentId,
    size: usize,
) -> (Vec<u8>, ContentStreamView) {
    let mut cursor = None;
    let mut bytes = Vec::new();
    loop {
        let range = ports
            .reader
            .text_range(session, content, cursor, size)
            .await
            .unwrap();
        assert!(range.bytes.len() <= size);
        bytes.extend_from_slice(&range.bytes);
        if !range.has_more {
            return (bytes, range.stream.clone());
        }
        assert!(!range.bytes.is_empty());
        cursor = Some(range.next);
    }
}

#[tokio::test]
async fn text_is_immediate_byte_exact_bounded_and_committed_with_its_round() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        let mut watch = ports
            .reader
            .watch_content(session, output.id())
            .await
            .unwrap();
        let history = ports.reader.history(session, None, 200).await.unwrap();
        let text_revision = history
            .entries
            .iter()
            .find(|entry| entry.id == output.entry_id())
            .unwrap()
            .revision;
        let command_count: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_command_results")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        sql(&admin,"CREATE TRIGGER reject_checkpoint BEFORE INSERT ON agena_content_blocks BEGIN SELECT RAISE(ABORT,'injected checkpoint failure'); END").await;
        let prefix = "  界🙂\n\0\u{1b}[31m\r\t";
        let accepted = output.append(prefix).await.unwrap();
        assert_eq!(accepted.offset, prefix.len() as u64);
        assert_eq!(watch.changed().await.unwrap().accepted, accepted.offset);
        let live = ports
            .reader
            .text_range(session, output.id(), None, usize::MAX)
            .await
            .unwrap();
        assert_eq!(live.bytes, prefix.as_bytes());
        assert_eq!(live.stream.committed, 0);
        assert_eq!(live.stream.state, CaptureState::Open);
        assert_eq!(
            all(&ports, session, output.id(), 7).await.0,
            prefix.as_bytes()
        );
        assert_eq!(
            ports
                .reader
                .history(session, None, 200)
                .await
                .unwrap()
                .entries
                .iter()
                .find(|entry| entry.id == output.entry_id())
                .unwrap()
                .revision,
            text_revision
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_command_results")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap(),
            command_count
        );
        sql(&admin, "DROP TRIGGER reject_checkpoint").await;
        let chunk = "abcd界🙂\r\n\0".repeat(10000);
        for _ in 0..3 {
            output.append(&chunk).await.unwrap();
        }
        let expected = format!("{prefix}{}", chunk.repeat(3));
        round
            .finish(
                RequestId::new(),
                Some(serde_json::json!({"opaque":"\n\0世界"})),
                true,
            )
            .await
            .unwrap();
        let (actual, head) = all(&ports, session, output.id(), 32771).await;
        assert_eq!(actual, expected.as_bytes());
        assert_eq!(head.state, CaptureState::Complete);
        assert_eq!(head.accepted, head.committed);
        assert!(matches!(
            output.append("late").await,
            Err(KernelError::Conflict(_))
        ));
        let foreign = ports
            .sessions
            .create(RequestId::new(), 1, "other".into())
            .await
            .unwrap()
            .value
            .id;
        assert!(matches!(
            ports
                .reader
                .text_range(foreign, output.id(), None, 100)
                .await,
            Err(KernelError::Ownership)
        ));
        assert!(matches!(
            ports.reader.watch_content(foreign, output.id()).await,
            Err(KernelError::Ownership)
        ));
        assert!(matches!(
            ports
                .reader
                .text_range(
                    session,
                    output.id(),
                    Some(TextCursor {
                        content: output.id(),
                        epoch: ContentEpoch::new(),
                        offset: 0
                    }),
                    100
                )
                .await,
            Err(KernelError::ContentReset)
        ));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn rejected_terminal_commit_keeps_the_tail_for_exact_retry() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, execution, round) = round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        sql(&admin,"CREATE TABLE injected_failure (id INTEGER REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED)").await;
        sql(&admin,"CREATE TRIGGER reject_terminal AFTER INSERT ON agena_command_results BEGIN INSERT INTO injected_failure VALUES (-1); END").await;
        sql(&admin,"CREATE TRIGGER reject_ordinary_checkpoint BEFORE UPDATE OF committed_head ON agena_content_streams WHEN NEW.capture_state='open' BEGIN SELECT RAISE(ABORT,'hold ordinary checkpoints'); END").await;
        let text = "  tail 世界🙂\r\n\0";
        output.append(text).await.unwrap();
        let request = RequestId::new();
        assert!(matches!(
            output.finish(request).await,
            Err(KernelError::Storage(_))
        ));
        let view = ports
            .reader
            .text_range(session, output.id(), None, 100)
            .await
            .unwrap();
        assert_eq!(view.bytes, text.as_bytes());
        assert_eq!(view.stream.state, CaptureState::Open);
        assert_eq!(view.stream.committed, 0);
        assert_eq!(
            ports
                .reader
                .execution(session, execution.id)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Running
        );
        sql(&admin, "DROP TRIGGER reject_terminal").await;
        output.append(" appended after rollback").await.unwrap();
        let finished = output.finish(request).await.unwrap();
        assert_eq!(output.finish(request).await.unwrap(), finished);
        assert_eq!(
            all(&ports, session, output.id(), 13).await.0,
            format!("{text} appended after rollback").as_bytes()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_content_blocks")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap(),
            2
        );
        sql(&admin, "DROP TABLE injected_failure").await;
        sql(&admin, "DROP TRIGGER reject_ordinary_checkpoint").await;
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cancellation_commits_accepted_bytes_and_fences_queued_late_appends() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, execution, round) = round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output.append("accepted before cancel\n世界").await.unwrap();
        // Poll in this order so both are admitted before either response. The
        // content worker must not collect across the cancellation boundary.
        let cancel = ports
            .sessions
            .cancel(RequestId::new(), session, execution.id);
        let late = output.append("must not be accepted");
        let (cancel, late) = tokio::join!(biased; cancel,late);
        assert_eq!(cancel.unwrap().value.phase, ExecutionPhase::Cancelled);
        assert!(matches!(late, Err(KernelError::Conflict(_))));
        let (bytes, head) = all(&ports, session, output.id(), 3).await;
        assert_eq!(bytes, "accepted before cancel\n世界".as_bytes());
        assert_eq!(head.state, CaptureState::Interrupted);
        assert_eq!(head.accepted, head.committed);
        ports
            .sessions
            .retry(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        assert_eq!(
            output.append("old generation").await,
            Err(KernelError::StaleAttempt)
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn tool_output_binds_to_its_entry_and_survives_provider_round_completion() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "t".into(),
                    name: "fixture".into(),
                    input: serde_json::json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        let before = ports.reader.tool(session, tool.id()).await.unwrap();
        let output = tool.begin_text(RequestId::new()).await.unwrap().value;
        let bound = ports.reader.tool(session, tool.id()).await.unwrap();
        assert!(bound.revision > before.revision);
        assert_eq!(bound.outputs.len(), 1);
        assert_eq!(bound.outputs[0].id, output.id());
        output.append("before round end\n").await.unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        output.append("after round end\n").await.unwrap();
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"exit_code":0}),
            },
        )
        .await
        .unwrap();
        let page = ports.reader.history(session, None, 200).await.unwrap();
        assert_eq!(page.entries.len(), 2);
        assert!(matches!(
            page.entries
                .iter()
                .find(|entry| entry.id == output.entry_id())
                .unwrap()
                .payload,
            EntryPayload::Tool { .. }
        ));
        let (bytes, head) = all(&ports, session, output.id(), 11).await;
        assert_eq!(bytes, b"before round end\nafter round end\n");
        assert_eq!(head.state, CaptureState::Complete);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn producer_families_and_clones_abandon_only_after_the_last_owner_leaves() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let request = RequestId::new();
        let first = round.begin_text(request, false).await.unwrap().value;
        let clone = first.clone();
        let second = round.begin_text(request, false).await.unwrap().value;
        assert_eq!(first.id(), second.id());
        let mut watch = ports
            .reader
            .watch_content(session, first.id())
            .await
            .unwrap();
        first.append("retained ").await.unwrap();
        drop(first);
        drop(clone);
        second.append("by replay family").await.unwrap();
        let content = second.id();
        drop(second);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let head = watch.changed().await.unwrap();
                if head.state == CaptureState::Interrupted {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let (bytes, head) = all(&ports, session, content, 10).await;
        assert_eq!(bytes, b"retained by replay family");
        assert_eq!(head.state, CaptureState::Interrupted);
        assert_eq!(head.accepted, head.committed);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn checkpoints_and_shutdown_need_no_caller_flush_and_do_not_emit_token_commands() {
    for memory in [true, false] {
        let (ports, admin, _root, url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        let mut watch = ports
            .reader
            .watch_content(session, output.id())
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_command_results")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        output.append("timed checkpoint\n世界").await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if watch.changed().await.unwrap().committed > 0 {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_command_results")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap(),
            count
        );
        output.append("close checkpoint").await.unwrap();
        let content = output.id();
        ports.shutdown().await.unwrap();
        assert_eq!(watch.head().unwrap().state, CaptureState::Interrupted);
        assert_eq!(
            watch.head().unwrap().accepted,
            watch.head().unwrap().committed
        );
        drop(output);
        drop(round);
        drop(admin);
        if !memory {
            let reopened = open_session_kernel(&url).await.unwrap();
            let (bytes, head) = all(&reopened, session, content, 5).await;
            assert_eq!(bytes, "timed checkpoint\n世界close checkpoint".as_bytes());
            assert_eq!(head.state, CaptureState::Interrupted);
            reopened.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn packet_slicing_does_not_create_a_durable_envelope_for_each_packet() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let small = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        let large = round
            .begin_text(RequestId::new(), true)
            .await
            .unwrap()
            .value;
        let packet = "0123456789abcdefg";
        for _ in 0..8192 {
            small.append(packet).await.unwrap();
        }
        let text = packet.repeat(8192);
        for chunk in text.as_bytes().chunks(text.len() / 4) {
            large
                .append(std::str::from_utf8(chunk).unwrap())
                .await
                .unwrap();
        }
        round.finish(RequestId::new(), None, true).await.unwrap();
        assert_eq!(
            all(&ports, session, small.id(), 32771).await.0,
            text.as_bytes()
        );
        assert_eq!(
            all(&ports, session, large.id(), 32771).await.0,
            text.as_bytes()
        );
        for id in [small.id(), large.id()] {
            let blocks: i64 =
                sqlx::query_scalar("SELECT count(*) FROM agena_content_blocks WHERE content_id=?")
                    .bind(id.to_string())
                    .fetch_one(admin.get_sqlite_connection_pool())
                    .await
                    .unwrap();
            assert!(
                (5..=16).contains(&blocks),
                "unexpected envelope amplification: {blocks}"
            );
        }
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn full_content_budget_preserves_control_admission_and_cancel_commits_every_accepted_byte() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, execution, round) = round(&ports).await;
        sql(&admin,"CREATE TRIGGER hold_checkpoints BEFORE UPDATE OF committed_head ON agena_content_streams WHEN NEW.capture_state='open' BEGIN SELECT RAISE(ABORT,'hold checkpoint budget'); END").await;
        let chunk = "x".repeat(128 * 1024);
        let mut streams = Vec::new();
        let mut total = 0;
        let mut saw_busy = false;
        for _ in 0..32 {
            let output = round
                .begin_text(RequestId::new(), false)
                .await
                .unwrap()
                .value;
            let mut accepted = 0;
            for _ in 0..4 {
                match output.append(&chunk).await {
                    Ok(cursor) => {
                        accepted += chunk.len();
                        total += chunk.len();
                        assert_eq!(cursor.offset, accepted as u64);
                    }
                    Err(KernelError::Busy) => {
                        saw_busy = true;
                        break;
                    }
                    Err(error) => panic!("unexpected append failure: {error}"),
                }
            }
            streams.push((output, accepted));
            if saw_busy {
                break;
            }
        }
        assert!(saw_busy);
        assert!(total <= MAX_CONTENT_PENDING_BYTES);
        assert!(total >= MAX_CONTENT_PENDING_BYTES - 512 * 1024);
        let page = ports
            .reader
            .text_range(session, streams[0].0.id(), None, usize::MAX)
            .await
            .unwrap();
        assert_eq!(page.bytes.len(), 256 * 1024);
        assert!(page.has_more);
        // Content permits and content queue slots do not own the reservation
        // used to admit this control command. Its barrier drains all sources.
        ports
            .sessions
            .cancel(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        for (output, accepted) in streams {
            let head = ports
                .reader
                .text_range(session, output.id(), None, 1)
                .await
                .unwrap()
                .stream
                .clone();
            assert_eq!(head.accepted, accepted as u64);
            assert_eq!(head.committed, head.accepted);
            assert_eq!(head.state, CaptureState::Interrupted);
        }
        sql(&admin, "DROP TRIGGER hold_checkpoints").await;
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn racing_reads_and_checkpoints_never_duplicate_or_omit_a_live_suffix() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        let producer = async {
            for _ in 0..128 {
                output.append(&"界🙂\r\n".repeat(1024)).await.unwrap();
            }
            output.finish(RequestId::new()).await.unwrap();
        };
        let consumer = async {
            let mut cursor = None;
            let mut bytes = Vec::new();
            loop {
                let page = ports
                    .reader
                    .text_range(session, output.id(), cursor, 17003)
                    .await
                    .unwrap();
                bytes.extend_from_slice(&page.bytes);
                cursor = Some(page.next);
                if page.stream.state == CaptureState::Complete && !page.has_more {
                    return bytes;
                }
                if page.bytes.is_empty() {
                    tokio::task::yield_now().await;
                }
            }
        };
        let (_, bytes) = tokio::join!(producer, consumer);
        assert_eq!(bytes, "界🙂\r\n".repeat(1024 * 128).as_bytes());
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn live_source_cap_and_quiet_source_sweep_stay_bounded_and_fair() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let mut outputs = Vec::new();
        for _ in 0..128 {
            outputs.push(
                round
                    .begin_text(RequestId::new(), false)
                    .await
                    .unwrap()
                    .value,
            );
        }
        assert!(matches!(
            round.begin_text(RequestId::new(), false).await,
            Err(KernelError::Busy)
        ));
        let mut watches = Vec::new();
        for output in outputs.iter().take(64) {
            let watch = ports
                .reader
                .watch_content(session, output.id())
                .await
                .unwrap();
            output.append("quiet source").await.unwrap();
            watches.push(watch);
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            for mut watch in watches {
                while watch.head().unwrap().committed == 0 {
                    watch.changed().await.unwrap();
                }
            }
        })
        .await
        .expect("bounded sweeps must visit quiet sources without waiting 500 ms per batch");
        outputs[0].finish(RequestId::new()).await.unwrap();
        let replacement = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        replacement.append("freed source slot").await.unwrap();
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn full_database_checkpoint_retains_accepted_tail_and_writer_can_retry_after_quota_relief() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        sql(&admin, &format!("PRAGMA max_page_count={}", pages + 2)).await;
        let text = "accepted despite failed checkpoint\n".repeat(4096);
        output.append(&text).await.unwrap();
        // This FIFO command returns after the append's checkpoint attempt.
        let watch = ports
            .reader
            .watch_content(session, output.id())
            .await
            .unwrap();
        assert_eq!(watch.head().unwrap().committed, 0);
        assert_eq!(watch.head().unwrap().accepted, text.len() as u64);
        let request = RequestId::new();
        assert!(output.finish(request).await.is_err());
        assert_eq!(watch.head().unwrap().state, CaptureState::Open);
        sql(&admin, "PRAGMA max_page_count=2147483647").await;
        output.finish(request).await.unwrap();
        let (bytes, head) = all(&ports, session, output.id(), 17003).await;
        assert_eq!(bytes, text.as_bytes());
        assert_eq!(head.committed, head.accepted);
        assert_eq!(head.state, CaptureState::Complete);
        ports.shutdown().await.unwrap();
    }
}
