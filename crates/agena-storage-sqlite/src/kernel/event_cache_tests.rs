//! Public capability regressions for cached event authorization and ranges.

use agena_domain::CommandOutputStream;
use sha2::{Digest, Sha256};

use super::{event_tests::tool, snapshot_tests::fixture, *};

async fn append_records(output: &LogOutput, range: std::ops::Range<u64>) -> Vec<EventRecord> {
    let mut records = Vec::new();
    for index in range {
        let channel = if index % 2 == 0 {
            CommandOutputStream::Stdout
        } else {
            CommandOutputStream::Stderr
        };
        let timestamp_ms = index as i64 - 100;
        let text = format!("{index:04}: 界🙂\0\r\n");
        let cursor = output
            .append_at(timestamp_ms, channel.clone(), &text)
            .await
            .unwrap();
        assert_eq!(cursor.sequence, index + 1);
        records.push(EventRecord {
            sequence: cursor.sequence,
            timestamp_ms,
            payload: EventPayload::Log { channel, text },
        });
    }
    records
}

async fn committed_head(
    reader: &KernelReader,
    session: SessionId,
    content: ContentId,
    through: u64,
) -> ContentStreamView {
    let mut watch = reader.watch_content(session, content).await.unwrap();
    // This is a hang guard for an acknowledged checkpoint, not a latency gate.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let head = watch.head().unwrap();
            if head.committed >= through {
                return (*head).clone();
            }
            drop(head);
            watch.changed().await.unwrap();
        }
    })
    .await
    .expect("the live source must publish its committed checkpoint")
}

fn assert_read_transactions_closed(diagnostics: &KernelDiagnostics) {
    let metrics = diagnostics.snapshot();
    assert_eq!(
        metrics.read_transactions_started,
        metrics.read_transactions_completed
    );
    assert_eq!(metrics.read_transactions_abandoned, 0);
    assert_eq!(metrics.read_transaction_active_scopes, 0);
}

#[tokio::test]
async fn cache_budget_reclamation_preserves_sequential_log_reads_with_slow_views() {
    const PACKET_BYTES: usize = 64 * 1024;
    const BLOCKS: u64 = 320;
    fn packet(index: u64) -> String {
        let mut text = "k".repeat(PACKET_BYTES);
        let prefix = format!("{index:04}:界🙂\0\r\n");
        text.replace_range(..prefix.len(), &prefix);
        text
    }

    for memory in [true, false] {
        let (ports, _maintenance, admin, diagnostics, _root) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_log(RequestId::new()).await.unwrap().value;
        let mut expected_digest = Sha256::new();
        for index in 0..BLOCKS {
            let text = packet(index);
            assert_eq!(text.len(), PACKET_BYTES);
            expected_digest.update(text.as_bytes());
            let channel = if index % 2 == 0 {
                CommandOutputStream::Stdout
            } else {
                CommandOutputStream::Stderr
            };
            let cursor = output
                .append_at(index as i64 - 500, channel, &text)
                .await
                .unwrap();
            assert_eq!(cursor.sequence, index + 1);
        }
        let finished = output.finish(RequestId::new()).await.unwrap().value;
        let physical_blocks: i64 =
            sqlx::query_scalar("SELECT count(*) FROM agena_content_blocks WHERE content_id=?")
                .bind(output.id().to_string())
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
        assert_eq!(physical_blocks, BLOCKS as i64);

        // These are real consumer allocations from the same source. Cache
        // ownership must yield before their existing leases are released.
        let mut slow = Vec::new();
        for index in [0, 80, 160, 240] {
            let page = ports
                .reader
                .event_page(
                    session,
                    output.id(),
                    Some(EventCursor {
                        content: output.id(),
                        epoch: finished.epoch,
                        sequence: index,
                    }),
                    1,
                )
                .await
                .unwrap();
            assert_eq!(page.records.len(), 1);
            slow.push((index, page));
        }
        let rejected_before = diagnostics.read_budget().rejected;
        let mut actual_digest = Sha256::new();
        let mut cursor = None;
        let mut records = 0u64;
        let mut bytes = 0usize;
        loop {
            let page = ports
                .reader
                .event_page(session, output.id(), cursor, 32)
                .await
                .unwrap();
            assert!(!page.records.is_empty());
            assert_eq!(page.stream.state, CaptureState::Complete);
            assert_eq!(
                (page.stream.accepted, page.stream.committed),
                (BLOCKS, BLOCKS)
            );
            for record in &page.records {
                assert_eq!(record.sequence, records + 1);
                assert_eq!(record.timestamp_ms, records as i64 - 500);
                let EventPayload::Log { channel, text } = &record.payload else {
                    panic!("the log source must retain its payload kind");
                };
                assert_eq!(
                    *channel,
                    if records.is_multiple_of(2) {
                        CommandOutputStream::Stdout
                    } else {
                        CommandOutputStream::Stderr
                    }
                );
                assert_eq!(text, &packet(records));
                actual_digest.update(text.as_bytes());
                bytes += text.len();
                records += 1;
            }
            assert_eq!(page.next.sequence, records);
            cursor = Some(page.next);
            if !page.has_more {
                break;
            }
        }
        assert_eq!(records, BLOCKS);
        assert_eq!(bytes, BLOCKS as usize * PACKET_BYTES);
        let actual_digest: [u8; 32] = actual_digest.finalize().into();
        let expected_digest: [u8; 32] = expected_digest.finalize().into();
        assert_eq!(actual_digest, expected_digest);
        assert!(
            diagnostics.read_budget().rejected > rejected_before,
            "the sequential scan must actually reclaim the full source cache"
        );
        for (index, page) in &slow {
            assert_eq!(page.records[0].sequence, index + 1);
            let EventPayload::Log { text, .. } = &page.records[0].payload else {
                panic!("the retained slow view must remain a log record");
            };
            assert_eq!(text, &packet(*index));
        }
        assert_read_transactions_closed(&diagnostics);
        drop(slow);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cached_event_reads_refresh_live_heads_seal_and_respect_earlier_frozen_heads() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, diagnostics, _root) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_log(RequestId::new()).await.unwrap().value;
        let prefix = "界".repeat(24 * 1024);
        let cursor = output
            .append_at(-101, CommandOutputStream::Stdout, &prefix)
            .await
            .unwrap();
        assert_eq!(cursor.sequence, 1);
        let frozen = committed_head(&ports.reader, session, output.id(), 1).await;
        assert_eq!((frozen.accepted, frozen.committed), (1, 1));
        assert_eq!(frozen.state, CaptureState::Open);
        let mut expected = vec![EventRecord {
            sequence: 1,
            timestamp_ms: -101,
            payload: EventPayload::Log {
                channel: CommandOutputStream::Stdout,
                text: prefix,
            },
        }];
        let old = ports
            .reader
            .event_page(session, output.id(), None, 1)
            .await
            .unwrap();
        assert_eq!(old.records.as_slice(), expected.as_slice());
        assert_eq!(old.stream.accepted, 1);

        expected.extend(append_records(&output, 1..4).await);
        let before = diagnostics.snapshot();
        let fresh = ports
            .reader
            .event_page(session, output.id(), None, 1)
            .await
            .unwrap();
        assert_eq!(fresh.records.as_slice(), &expected[..1]);
        assert_eq!(fresh.stream.accepted, 4);
        assert_eq!(fresh.stream.state, CaptureState::Open);
        assert!(fresh.has_more);
        assert_eq!(
            old.stream.accepted, 1,
            "an already returned view remains immutable"
        );
        assert_eq!(
            diagnostics.snapshot().read_transactions_started,
            before.read_transactions_started
        );
        let current = ports
            .reader
            .event_page(session, output.id(), None, 16)
            .await
            .unwrap();
        assert_eq!(current.records.as_slice(), expected.as_slice());
        assert_eq!(current.next.sequence, 4);

        output.finish(RequestId::new()).await.unwrap();
        let sealed = ports
            .reader
            .event_page(session, output.id(), None, 16)
            .await
            .unwrap();
        assert_eq!(sealed.records.as_slice(), expected.as_slice());
        assert_eq!(sealed.stream.state, CaptureState::Complete);
        assert_eq!((sealed.stream.accepted, sealed.stream.committed), (4, 4));
        let before = diagnostics.snapshot();
        let sealed_again = ports
            .reader
            .event_page(session, output.id(), Some(cursor), 16)
            .await
            .unwrap();
        assert_eq!(sealed_again.records.as_slice(), &expected[1..]);
        assert_eq!(sealed_again.stream.state, CaptureState::Complete);
        assert!(!sealed_again.has_more);
        assert_eq!(
            diagnostics.snapshot().read_transactions_started,
            before.read_transactions_started
        );

        // A fully warm current read must not turn an earlier committed boundary
        // into the newer head, including after the live source has been sealed.
        let fixed = ports
            .reader
            .event_page_at(session, &frozen, None, 16)
            .await
            .unwrap();
        assert_eq!(fixed.records.as_slice(), &expected[..1]);
        assert_eq!((fixed.stream.accepted, fixed.stream.committed), (1, 1));
        assert_eq!(fixed.next.sequence, 1);
        assert!(!fixed.has_more);
        assert!(matches!(
            ports
                .reader
                .event_page_at(
                    session,
                    &frozen,
                    Some(EventCursor {
                        sequence: 2,
                        ..cursor
                    }),
                    1,
                )
                .await,
            Err(KernelError::ContentReset)
        ));
        let mut wrong_epoch = frozen.clone();
        wrong_epoch.epoch = ContentEpoch::new();
        assert!(matches!(
            ports
                .reader
                .event_page_at(session, &wrong_epoch, None, 1)
                .await,
            Err(KernelError::ContentReset)
        ));
        assert_read_transactions_closed(&diagnostics);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn shifted_event_cursors_share_verified_bodies_without_starting_sql_transactions() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, diagnostics, _root) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_log(RequestId::new()).await.unwrap().value;
        let expected = append_records(&output, 0..96).await;
        let finished = output.finish(RequestId::new()).await.unwrap().value;
        let before = diagnostics.snapshot();
        let warm = ports
            .reader
            .event_page(session, output.id(), None, 96)
            .await
            .unwrap();
        assert_eq!(warm.records.as_slice(), expected.as_slice());
        assert_eq!(warm.stream.state, CaptureState::Complete);
        let cold = diagnostics.snapshot();
        assert!(cold.read_transactions_started > before.read_transactions_started);
        assert!(cold.event_block_queries > before.event_block_queries);
        let mut requested_records = 0;
        for (after, limit) in [
            (1, 1),
            (7, 17),
            (19, 11),
            (31, 32),
            (63, 17),
            (95, 1),
            (96, 8),
        ] {
            let page = ports
                .reader
                .event_page(
                    session,
                    output.id(),
                    Some(EventCursor {
                        content: output.id(),
                        epoch: finished.epoch,
                        sequence: after,
                    }),
                    limit,
                )
                .await
                .unwrap();
            let end = (after as usize + limit).min(expected.len());
            assert_eq!(page.records.as_slice(), &expected[after as usize..end]);
            assert_eq!(page.next.sequence, end as u64);
            assert_eq!(page.has_more, end < expected.len());
            requested_records += (end - after as usize) as u64;
        }
        let hot = diagnostics.snapshot();
        assert_eq!(
            hot.read_transactions_started,
            cold.read_transactions_started
        );
        assert_eq!(hot.event_block_queries, cold.event_block_queries);
        assert_eq!(hot.event_block_rows, cold.event_block_rows);
        assert_eq!(
            hot.event_block_validated_bytes,
            cold.event_block_validated_bytes
        );
        assert_eq!(
            hot.event_page_decoded_records - cold.event_page_decoded_records,
            requested_records
        );
        assert_read_transactions_closed(&diagnostics);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cached_event_membership_is_not_inherited_by_foreign_or_rewound_sessions_or_after_withdrawal()
 {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, diagnostics, _root) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_log(RequestId::new()).await.unwrap().value;
        let expected = append_records(&output, 0..48).await;
        output.finish(RequestId::new()).await.unwrap();
        let history = ports.reader.history(session, None, 10).await.unwrap();
        let user = &history.entries[0];
        assert!(matches!(user.payload, EntryPayload::User { .. }));
        let message = user.message_id;
        let execution = user.execution_id;
        drop(history);
        ports
            .sessions
            .cancel(RequestId::new(), session, execution)
            .await
            .unwrap();
        let child = ports
            .sessions
            .fork(RequestId::new(), session, output.entry_id())
            .await
            .unwrap()
            .value
            .id;
        let rewound = ports
            .sessions
            .rewind(RequestId::new(), session, message)
            .await
            .unwrap()
            .value
            .session
            .id;
        let foreign = ports
            .sessions
            .create(RequestId::new(), 1, "foreign event reader".into())
            .await
            .unwrap()
            .value
            .id;
        let held = ports
            .reader
            .event_page(session, output.id(), None, 48)
            .await
            .unwrap();
        let inherited = ports
            .reader
            .event_page(child, output.id(), None, 48)
            .await
            .unwrap();
        assert_eq!(held.records.as_slice(), expected.as_slice());
        assert_eq!(inherited.records.as_slice(), expected.as_slice());
        let before_foreign = diagnostics.snapshot();
        for outsider in [foreign, rewound] {
            assert!(matches!(
                ports
                    .reader
                    .event_page(outsider, output.id(), None, 8)
                    .await,
                Err(KernelError::Ownership)
            ));
        }
        assert_eq!(
            diagnostics.snapshot().event_block_queries,
            before_foreign.event_block_queries
        );

        // The source remains physically present and authorized in the fork;
        // withdrawing its origin must still revoke the origin's warm context.
        ports
            .sessions
            .withdraw(RequestId::new(), session, message)
            .await
            .unwrap();
        assert!(matches!(
            ports.reader.event_page(session, output.id(), None, 8).await,
            Err(KernelError::Ownership)
        ));
        assert_eq!(held.records.as_slice(), expected.as_slice());
        // A writer may invalidate the shared family conservatively. Reauthorize
        // the surviving branch before checking reuse by another native cursor.
        let authorized_again = ports
            .reader
            .event_page(child, output.id(), None, 48)
            .await
            .unwrap();
        assert_eq!(authorized_again.records.as_slice(), expected.as_slice());
        let before_child = diagnostics.snapshot();
        let suffix = ports
            .reader
            .event_page(
                child,
                output.id(),
                Some(EventCursor {
                    content: output.id(),
                    epoch: output.epoch(),
                    sequence: 13,
                }),
                17,
            )
            .await
            .unwrap();
        assert_eq!(suffix.records.as_slice(), &expected[13..30]);
        assert_eq!(
            diagnostics.snapshot().read_transactions_started,
            before_child.read_transactions_started
        );
        assert_read_transactions_closed(&diagnostics);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn retention_invalidates_warm_event_floors_in_every_branch_and_earlier_frozen_heads() {
    for memory in [true, false] {
        let (ports, maintenance, _admin, diagnostics, _root) = fixture(memory).await;
        let (session, _round, tool) = tool(&ports).await;
        let output = tool.begin_log(RequestId::new()).await.unwrap().value;
        let expected = append_records(&output, 0..96).await;
        let frozen = output.finish(RequestId::new()).await.unwrap().value;
        let child = ports
            .sessions
            .fork(RequestId::new(), session, output.entry_id())
            .await
            .unwrap()
            .value
            .id;
        for branch in [session, child] {
            let warm = ports
                .reader
                .event_page(branch, output.id(), None, 96)
                .await
                .unwrap();
            assert_eq!(warm.records.as_slice(), expected.as_slice());
            assert_eq!(warm.stream.retained_start, 0);
        }
        let retained = maintenance
            .prune_content(RequestId::new(), session, output.id(), 17)
            .await
            .unwrap()
            .value;
        assert_eq!(retained.stream.retained_start, 17);
        assert_eq!(retained.stream.epoch, frozen.epoch);
        for branch in [session, child] {
            assert!(matches!(
                ports.reader.event_page(branch, output.id(), None, 8).await,
                Err(KernelError::ContentGap {
                    requested: 0,
                    retained_start: 17,
                    ..
                })
            ));
            let cursor = EventCursor {
                content: output.id(),
                epoch: frozen.epoch,
                sequence: 17,
            };
            let suffix = ports
                .reader
                .event_page(branch, output.id(), Some(cursor), 9)
                .await
                .unwrap();
            assert_eq!(suffix.records.as_slice(), &expected[17..26]);
            assert_eq!(suffix.stream.retained_start, 17);
            let before = diagnostics.snapshot();
            assert!(matches!(
                ports
                    .reader
                    .event_page(
                        branch,
                        output.id(),
                        Some(EventCursor {
                            sequence: 16,
                            ..cursor
                        }),
                        1,
                    )
                    .await,
                Err(KernelError::ContentGap {
                    requested: 16,
                    retained_start: 17,
                    ..
                })
            ));
            assert_eq!(
                diagnostics.snapshot().read_transactions_started,
                before.read_transactions_started
            );
            assert!(matches!(
                ports.reader.event_page_at(branch, &frozen, None, 8).await,
                Err(KernelError::ContentGap {
                    retained_start: 17,
                    ..
                })
            ));
            let fixed_suffix = ports
                .reader
                .event_page_at(branch, &frozen, Some(cursor), 9)
                .await
                .unwrap();
            assert_eq!(fixed_suffix.records.as_slice(), &expected[17..26]);
            assert_eq!(fixed_suffix.stream.retained_start, 17);
        }
        assert_read_transactions_closed(&diagnostics);
        ports.shutdown().await.unwrap();
    }
}
