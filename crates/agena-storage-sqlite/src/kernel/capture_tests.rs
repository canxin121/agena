use std::{future::Future, task::Poll};

use agena_domain::{CommandOutputStream, DocumentMutation, ViewBlock};

use super::{
    event_tests::{screen, tool as running_tool},
    tests::fixture,
    *,
};

#[tokio::test]
async fn synchronous_pipe_capture_admits_real_cursors_while_the_database_writer_cannot_progress() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = running_tool(&ports).await;
        let text = tool.begin_text(RequestId::new()).await.unwrap().value;
        let log = tool.begin_log(RequestId::new()).await.unwrap().value;
        let document = tool.begin_document(RequestId::new()).await.unwrap().value;
        let terminal = tool.begin_terminal(RequestId::new()).await.unwrap().value;
        let watch = ports
            .reader
            .watch_content(session, text.id())
            .await
            .unwrap();
        // Infrastructure fixture deliberately withholds the sole writer
        // connection. Native capture runs on an OS reader thread with no
        // Tokio runtime. A database/async wait here cannot pass this barrier.
        let mut blocked = admin.get_sqlite_connection_pool().acquire().await.unwrap();
        let transaction = blocked.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let mut finish = Box::pin(tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"observed":true}),
            },
        ));
        std::future::poll_fn(|cx| {
            assert!(finish.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let reader_text = text.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let result = (|| {
                let first = reader_text.try_append("first 界\r\n\0")?;
                let second = reader_text.try_append("second 🙂")?;
                let log_cursor =
                    log.try_append_at(4321, CommandOutputStream::Stderr, "\u{1b}[31merr\r\n")?;
                document.try_apply(DocumentMutation::Insert {
                    after: None,
                    block: ViewBlock::Text {
                        id: Some("body".into()),
                        text: "document".into(),
                    },
                })?;
                let base = terminal.try_snapshot(screen("before"))?;
                let end = terminal.try_patch(base, screen("after"), vec![0, 1])?;
                Ok::<_, KernelError>((
                    first,
                    second,
                    log_cursor,
                    base,
                    end,
                    terminal.id(),
                    document.id(),
                    log.id(),
                ))
            })();
            send.send(result).unwrap();
        });
        let (first, second, log_cursor, base, end, terminal_id, document_id, log_id) = receive
            .recv_timeout(Duration::from_secs(3))
            .expect("pipe admission waited on the database")
            .unwrap();
        thread.join().unwrap();
        assert_eq!(first.offset, "first 界\r\n\0".len() as u64);
        assert_eq!(second.offset, "first 界\r\n\0second 🙂".len() as u64);
        assert_eq!(watch.head().unwrap().accepted, second.offset);
        assert_eq!(watch.head().unwrap().committed, 0);
        assert_eq!(base.sequence, 1);
        assert_eq!(end.sequence, 2);
        assert_eq!(log_cursor.sequence, 1);
        transaction.rollback().await.unwrap();
        drop(blocked);
        finish.await.unwrap();
        let text_page = ports
            .reader
            .text_range(session, text.id(), None, 1024)
            .await
            .unwrap();
        assert_eq!(text_page.bytes, "first 界\r\n\0second 🙂".as_bytes());
        assert_eq!(text_page.stream.committed, second.offset);
        assert_eq!(text_page.stream.state, CaptureState::Complete);
        let snapshot = ports
            .reader
            .event_snapshot(session, terminal_id)
            .await
            .unwrap();
        assert_eq!(snapshot.through, end);
        assert_eq!(snapshot.terminal.as_deref(), Some(&screen("after")));
        assert_eq!(
            ports
                .reader
                .event_snapshot(session, document_id)
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
                .event_page(session, log_id, None, 10)
                .await
                .unwrap()
                .records[0]
                .timestamp_ms,
            4321
        );
        assert!(text.try_append("after completion").is_err());
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn an_activity_output_opened_and_ended_in_one_batch_never_installs_a_live_writer() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = running_tool(&ports).await;
        let activity = tool
            .begin_activity(
                RequestId::new(),
                BackgroundActivityStart {
                    kind: BackgroundActivityKind::Process,
                    label: "same batch".into(),
                },
            )
            .await
            .unwrap()
            .value;
        let (text, log, document, end) = tokio::join!(biased;
            activity.begin_text(RequestId::new()),
            activity.begin_log(RequestId::new()),
            activity.begin_document(RequestId::new()),
            activity.end(RequestId::new(),BackgroundActivityOutcome::Stopped),
        );
        let text = text.unwrap();
        let log = log.unwrap();
        let document = document.unwrap();
        let end = end.unwrap();
        assert_eq!(text.position.sequence, end.position.sequence);
        assert_eq!(log.position.sequence, end.position.sequence);
        assert_eq!(document.position.sequence, end.position.sequence);
        assert!(text.value.try_append("revived").is_err());
        assert!(text.value.append("revived").await.is_err());
        assert!(
            log.value
                .try_append(CommandOutputStream::Stdout, "revived")
                .is_err()
        );
        assert!(
            document
                .value
                .try_apply(DocumentMutation::Remove {
                    block_id: "absent".into()
                })
                .is_err()
        );
        for id in [text.value.id(), log.value.id(), document.value.id()] {
            let view = ports
                .reader
                .watch_content(session, id)
                .await
                .unwrap()
                .head()
                .unwrap();
            assert_eq!(view.state, CaptureState::Complete);
            assert_eq!(view.accepted, view.committed);
        }
        assert_eq!(
            ports
                .reader
                .activity(session, activity.id())
                .await
                .unwrap()
                .outcome,
            Some(BackgroundActivityOutcome::Stopped)
        );
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM agena_content_streams WHERE capture_state='open'",
        )
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        assert_eq!(open, 0);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn attempt_sources_opened_in_their_owners_terminal_batch_stay_sealed() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, round, tool) = running_tool(&ports).await;
        let (text, events, end) = tokio::join!(biased;
            tool.begin_text(RequestId::new()),
            tool.begin_terminal(RequestId::new()),
            tool.complete(RequestId::new(),ToolResult {phase:ToolPhase::Succeeded,value:serde_json::json!({})}),
        );
        let text = text.unwrap();
        let events = events.unwrap();
        let end = end.unwrap();
        assert_eq!(text.position.sequence, end.position.sequence);
        assert_eq!(events.position.sequence, end.position.sequence);
        assert!(text.value.try_append("revived").is_err());
        assert!(events.value.try_snapshot(screen("revived")).is_err());
        assert_eq!(
            ports
                .reader
                .watch_content(session, text.value.id())
                .await
                .unwrap()
                .head()
                .unwrap()
                .state,
            CaptureState::Complete
        );
        let (round_text, end) = tokio::join!(biased;
            round.begin_text(RequestId::new(),false),
            round.finish(RequestId::new(),None,true),
        );
        let round_text = round_text.unwrap();
        assert_eq!(round_text.position.sequence, end.unwrap().position.sequence);
        assert!(round_text.value.try_append("revived").is_err());
        assert_eq!(
            ports
                .reader
                .watch_content(session, round_text.value.id())
                .await
                .unwrap()
                .head()
                .unwrap()
                .state,
            CaptureState::Complete
        );
        let (_second, round, tool) = running_tool(&ports).await;
        let session = round.session_id();
        let (source, cancel) = tokio::join!(biased;
            tool.begin_log(RequestId::new()),
            ports.sessions.cancel(RequestId::new(),session,round.execution_id()),
        );
        let source = source.unwrap();
        assert_eq!(source.position.sequence, cancel.unwrap().position.sequence);
        assert!(
            source
                .value
                .try_append(CommandOutputStream::Stderr, "revived")
                .is_err()
        );
        assert_eq!(
            ports
                .reader
                .watch_content(session, source.value.id())
                .await
                .unwrap()
                .head()
                .unwrap()
                .state,
            CaptureState::Interrupted
        );
        ports.shutdown().await.unwrap();
    }
}
