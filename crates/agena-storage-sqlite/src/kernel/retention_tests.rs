use agena_domain::{CommandOutputStream, DocumentMutation, ViewBlock};

use super::{
    blob_tests::fixture,
    content_tests::round,
    event_tests::{screen, tool},
    tests::sql,
    *,
};

#[tokio::test]
async fn text_retention_declares_byte_gap_and_reclaims_bounded_blocks_without_rebasing_cursors() {
    for memory in [true, false] {
        let (ports, maintenance, admin, _root, url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        let chunk = "界🙂a\n".repeat(8192);
        let mut expected = Vec::new();
        for _ in 0..16 {
            output.append(&chunk).await.unwrap();
            expected.extend_from_slice(chunk.as_bytes());
        }
        assert!(matches!(
            maintenance
                .prune_content(RequestId::new(), session, output.id(), 1)
                .await,
            Err(KernelError::Conflict(_))
        ));
        let finished = output.finish(RequestId::new()).await.unwrap();
        let requested_before = 10 * 32768 + 3;
        let mut before = requested_before;
        while std::str::from_utf8(&expected[..before as usize]).is_err() {
            before += 1;
        }
        let request = RequestId::new();
        let mut watch = ports.reader.watch(session, None).unwrap();
        let first = maintenance
            .prune_content(request, session, output.id(), requested_before)
            .await
            .unwrap();
        assert_eq!(first.value.stream.retained_start, before);
        assert_eq!(first.value.stream.epoch, finished.value.epoch);
        assert_eq!(first.value.stream.committed, expected.len() as u64);
        assert_eq!(first.value.blocks_removed, 8);
        assert!(first.value.pending_reclamation);
        assert_eq!(watch.recv().await.unwrap().cursor.position, first.position);
        assert!(
            matches!(ports.reader.text_range(session,output.id(),None,20).await,Err(KernelError::ContentGap { requested:0,retained_start,.. }) if retained_start==before)
        );
        let second = maintenance
            .prune_content(RequestId::new(), session, output.id(), before)
            .await
            .unwrap();
        assert_eq!(second.value.blocks_removed, 2);
        assert!(!second.value.pending_reclamation);
        let mut replay_watch = ports.reader.watch(session, None).unwrap();
        assert_eq!(
            maintenance
                .prune_content(request, session, output.id(), requested_before)
                .await
                .unwrap(),
            first
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), replay_watch.recv())
                .await
                .is_err()
        );
        let mut cursor = TextCursor {
            content: output.id(),
            epoch: finished.value.epoch,
            offset: before,
        };
        let mut bytes = Vec::new();
        loop {
            let page = ports
                .reader
                .text_range(session, output.id(), Some(cursor), 17003)
                .await
                .unwrap();
            bytes.extend_from_slice(&page.bytes);
            cursor = page.next;
            if !page.has_more {
                break;
            }
        }
        assert_eq!(bytes, &expected[before as usize..]);
        let gap: (i64, i64, String) =
            sqlx::query_as("SELECT logical_start,logical_end,reason FROM agena_content_gaps")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
        assert_eq!(gap, (0, before as i64, "retention".into()));
        let id = output.id();
        drop(output);
        drop(round);
        drop(maintenance);
        drop(admin);
        ports.shutdown().await.unwrap();
        if !memory {
            let reopened = open_session_kernel(&url).await.unwrap();
            let page = reopened
                .reader
                .text_range(
                    session,
                    id,
                    Some(TextCursor {
                        content: id,
                        epoch: finished.value.epoch,
                        offset: before,
                    }),
                    32,
                )
                .await
                .unwrap();
            assert_eq!(page.bytes, &expected[before as usize..before as usize + 32]);
            assert_eq!(page.stream.retained_start, before);
            reopened.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn retention_rejects_foreign_owner_and_rolls_gap_revision_and_reclamation_back_together() {
    for memory in [true, false] {
        let (ports, maintenance, admin, _root, _url) = fixture(memory).await;
        let (session, _execution, round) = round(&ports).await;
        let (other, _other_execution, _other_round) = super::content_tests::round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output
            .append(&"immutable text\n".repeat(8192))
            .await
            .unwrap();
        output.finish(RequestId::new()).await.unwrap();
        assert!(matches!(
            maintenance
                .prune_content(RequestId::new(), other, output.id(), 32768)
                .await,
            Err(KernelError::Ownership)
        ));
        let revision = ports
            .reader
            .history(session, None, 10)
            .await
            .unwrap()
            .entries
            .last()
            .unwrap()
            .revision;
        sql(&admin,"CREATE TRIGGER reject_prune BEFORE DELETE ON agena_content_blocks BEGIN SELECT RAISE(ABORT,'injected prune failure'); END").await;
        let request = RequestId::new();
        assert!(
            maintenance
                .prune_content(request, session, output.id(), 32768)
                .await
                .is_err()
        );
        assert_eq!(
            ports
                .reader
                .text_range(session, output.id(), None, 16)
                .await
                .unwrap()
                .stream
                .retained_start,
            0
        );
        assert_eq!(
            ports
                .reader
                .history(session, None, 10)
                .await
                .unwrap()
                .entries
                .last()
                .unwrap()
                .revision,
            revision
        );
        let gaps: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_content_gaps")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        assert_eq!(gaps, 0);
        sql(&admin, "DROP TRIGGER reject_prune").await;
        maintenance
            .prune_content(request, session, output.id(), 32768)
            .await
            .unwrap();
        assert!(
            ports
                .reader
                .history(session, None, 10)
                .await
                .unwrap()
                .entries
                .last()
                .unwrap()
                .revision
                > revision
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn event_retention_keeps_log_order_and_a_replayable_document_and_terminal_checkpoint() {
    for memory in [true, false] {
        let (ports, maintenance, admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let log = tool.begin_log(RequestId::new()).await.unwrap().value;
        for index in 0..100 {
            log.append_at(
                index,
                if index % 2 == 0 {
                    CommandOutputStream::Stdout
                } else {
                    CommandOutputStream::Stderr
                },
                &format!("{index}: 界\r\n\0"),
            )
            .await
            .unwrap();
        }
        log.finish(RequestId::new()).await.unwrap();
        // Warm the immutable body before retention. Reusing it must never
        // bypass the newly committed floor or revive the pruned prefix.
        let warm = ports
            .reader
            .event_page(session, log.id(), None, 7)
            .await
            .unwrap();
        assert_eq!(warm.records[0].sequence, 1);
        drop(warm);
        maintenance
            .prune_content(RequestId::new(), session, log.id(), 17)
            .await
            .unwrap();
        assert!(matches!(
            ports.reader.event_page(session, log.id(), None, 100).await,
            Err(KernelError::ContentGap {
                retained_start: 17,
                ..
            })
        ));
        let mut cursor = EventCursor {
            content: log.id(),
            epoch: log.epoch(),
            sequence: 17,
        };
        let mut records = Vec::new();
        loop {
            let page = ports
                .reader
                .event_page(session, log.id(), Some(cursor), 7)
                .await
                .unwrap();
            cursor = page.next;
            records.extend(page.records.iter().cloned());
            if !page.has_more {
                break;
            }
        }
        assert_eq!(records.len(), 83);
        for (index, record) in (17..100).zip(records) {
            assert_eq!(record.sequence, index + 1);
            assert_eq!(record.timestamp_ms, index as i64);
            assert_eq!(
                record.payload,
                EventPayload::Log {
                    channel: if index % 2 == 0 {
                        CommandOutputStream::Stdout
                    } else {
                        CommandOutputStream::Stderr
                    },
                    text: format!("{index}: 界\r\n\0")
                }
            );
        }
        let document = tool.begin_document(RequestId::new()).await.unwrap().value;
        document
            .apply(DocumentMutation::Insert {
                after: None,
                block: ViewBlock::Text {
                    id: Some("body".into()),
                    text: "body".into(),
                },
            })
            .await
            .unwrap();
        for _ in 0..150 {
            document
                .apply(DocumentMutation::AppendText {
                    block_id: "body".into(),
                    text: " 世界🙂".into(),
                })
                .await
                .unwrap();
        }
        document.finish(RequestId::new()).await.unwrap();
        let terminal = tool.begin_terminal(RequestId::new()).await.unwrap().value;
        let mut base = terminal.snapshot(screen("initial")).await.unwrap();
        for index in 0..150 {
            let mut changed = screen(&format!("frame {index}"));
            changed.cells.truncate(1);
            base = terminal.patch(base, changed, vec![0]).await.unwrap();
        }
        terminal.finish(RequestId::new()).await.unwrap();
        for (id, epoch) in [
            (document.id(), document.epoch()),
            (terminal.id(), terminal.epoch()),
        ] {
            let before = ports.reader.event_snapshot(session, id).await.unwrap();
            sql(&admin,&format!("UPDATE agena_content_checkpoints SET logical_sequence=65 WHERE content_id='{id}' AND logical_sequence=64")).await;
            assert!(matches!(
                maintenance
                    .prune_content(RequestId::new(), session, id, 100)
                    .await,
                Err(KernelError::Storage(_))
            ));
            assert_eq!(
                ports
                    .reader
                    .event_snapshot(session, id)
                    .await
                    .unwrap()
                    .stream
                    .retained_start,
                0
            );
            sql(&admin,&format!("UPDATE agena_content_checkpoints SET logical_sequence=64 WHERE content_id='{id}' AND logical_sequence=65")).await;
            let pruned = maintenance
                .prune_content(RequestId::new(), session, id, 100)
                .await
                .unwrap();
            assert_eq!(pruned.value.stream.retained_start, 63);
            let after = ports.reader.event_snapshot(session, id).await.unwrap();
            assert_eq!(after.document, before.document);
            assert_eq!(after.terminal, before.terminal);
            assert_eq!(after.through, before.through);
            assert!(matches!(
                ports.reader.event_page(session, id, None, 1).await,
                Err(KernelError::ContentGap {
                    retained_start: 63,
                    ..
                })
            ));
            let mut cursor = EventCursor {
                content: id,
                epoch,
                sequence: 63,
            };
            let mut projection = None;
            loop {
                let page = ports
                    .reader
                    .event_page(session, id, Some(cursor), 7)
                    .await
                    .unwrap();
                cursor = page.next;
                for record in &page.records {
                    if let Some(projection) = projection.as_mut() {
                        EventProjection::apply(projection, record).unwrap();
                    } else {
                        projection = Some(EventProjection::from_checkpoint(record).unwrap());
                    }
                }
                if !page.has_more {
                    break;
                }
            }
            let replay = projection.unwrap().snapshot(after.stream.clone()).unwrap();
            assert_eq!(replay.document, before.document);
            assert_eq!(replay.terminal, before.terminal);
        }
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn text_retention_aligns_its_declared_gap_without_changing_canonical_positions() {
    for memory in [true, false] {
        let (ports, maintenance, _admin, _diagnostics, _root) =
            snapshot_tests::fixture(memory).await;
        let (session, round, _tool) = event_tests::tool(&ports).await;
        let text = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        text.append("A界🙂B").await.unwrap();
        text.finish(RequestId::new()).await.unwrap();
        let pruned = maintenance
            .prune_content(RequestId::new(), session, text.id(), 2)
            .await
            .unwrap()
            .value;
        assert_eq!(pruned.stream.retained_start, 4);
        let cursor = TextCursor {
            content: text.id(),
            epoch: pruned.stream.epoch,
            offset: 4,
        };
        let retained = ports
            .reader
            .text_range(session, text.id(), Some(cursor), 1024)
            .await
            .unwrap();
        assert_eq!(retained.bytes, "🙂B".as_bytes());
        assert_eq!(retained.next.offset, 9);
        ports.shutdown().await.unwrap();
    }
}
