use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
};

use agena_domain::{CommandOutputStream, DocumentMutation, ViewBlock};
use serde_json::{Value, json};
use tokio::io::AsyncWrite;

use super::{
    snapshot_tests::fixture,
    tests::{document, sql},
    *,
};

fn verify(bytes: &[u8], report: &ExportReport) -> Vec<Value> {
    let lines = bytes
        .split_inclusive(|byte| *byte == b'\n')
        .collect::<Vec<_>>();
    assert!(lines.iter().all(|line| line.last() == Some(&b'\n')));
    let values = lines
        .iter()
        .map(|line| serde_json::from_slice::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values.first().unwrap()["kind"], "header");
    assert_eq!(values[0]["data"]["version"], 1);
    assert_eq!(values.last().unwrap()["kind"], "footer");
    let footer: ExportReport =
        serde_json::from_value(values.last().unwrap()["data"].clone()).unwrap();
    assert_eq!(&footer, report);
    assert_eq!(report.records as usize, lines.len() - 1);
    let prefix = &bytes[..report.bytes as usize];
    assert_eq!(report.sha256, <[u8; 32]>::from(Sha256::digest(prefix)));
    assert_eq!(prefix, lines[..lines.len() - 1].concat());
    assert_eq!(
        report.entries as usize,
        values.iter().filter(|v| v["kind"] == "entry").count()
    );
    values
}

#[tokio::test]
async fn archive_preserves_order_raw_opaque_json_binary_blobs_and_all_content_codecs() {
    for memory in [true, false] {
        let (ports, maintenance, admin, diagnostics, _root) = fixture(memory).await;
        let (session, round, tool) = event_tests::tool(&ports).await;
        let text = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        let reasoning = round
            .begin_text(RequestId::new(), true)
            .await
            .unwrap()
            .value;
        let text_body = "  \u{754c}\u{1f642}\r\n\0\u{1b}[31m".repeat(10_000);
        text.append(&text_body).await.unwrap();
        text.append(&text_body).await.unwrap();
        text.finish(RequestId::new()).await.unwrap();
        reasoning
            .append("private reasoning \u{754c}\n")
            .await
            .unwrap();
        reasoning.finish(RequestId::new()).await.unwrap();
        let log = tool.begin_log(RequestId::new()).await.unwrap().value;
        log.append_at(
            12345,
            CommandOutputStream::Stdout,
            "  out\r\n\0\u{1b}[31m\u{754c}",
        )
        .await
        .unwrap();
        log.append_at(12346, CommandOutputStream::Stderr, "err\r next\n")
            .await
            .unwrap();
        log.finish(RequestId::new()).await.unwrap();
        let doc = tool.begin_document(RequestId::new()).await.unwrap().value;
        doc.apply(DocumentMutation::Insert {
            after: None,
            block: ViewBlock::Text {
                id: Some("body".into()),
                text: "  \u{754c}\n".into(),
            },
        })
        .await
        .unwrap();
        doc.apply(DocumentMutation::AppendText {
            block_id: "body".into(),
            text: "\u{1f642}\r\n".into(),
        })
        .await
        .unwrap();
        doc.finish(RequestId::new()).await.unwrap();
        let terminal = tool.begin_terminal(RequestId::new()).await.unwrap().value;
        let cursor = terminal
            .snapshot(event_tests::screen("first"))
            .await
            .unwrap();
        let mut patch = event_tests::screen("next \u{754c}");
        patch.cells.truncate(1);
        terminal.patch(cursor, patch, vec![0]).await.unwrap();
        terminal.finish(RequestId::new()).await.unwrap();
        let interaction = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "archive".into(),
                    description: "permission evidence".into(),
                },
            )
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                interaction,
                super::tests::permission_decision(interaction, true, None),
                None,
            )
            .await
            .unwrap();
        let binary = (0..300_019)
            .map(|i| ((i * 71 + i / 97) % 256) as u8)
            .collect::<Vec<_>>();
        let prepared = ports
            .sessions
            .prepare_blob(
                RequestId::new(),
                session,
                binary.len() as u64,
                &mut binary.as_slice(),
            )
            .await
            .unwrap();
        tool.complete_with_blobs(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({"result":"  \u{754c}\n\0","unknown":[null,true,{"nested":4}]}),
            },
            std::slice::from_ref(&prepared),
        )
        .await
        .unwrap();
        let opaque = json!({"unknown_provider_field":{"escaped":"  \u{754c}\n\0\"\\","replay":[null,1,{"x":[true,false]}]}});
        round
            .finish(RequestId::new(), Some(opaque.clone()), true)
            .await
            .unwrap();
        let snapshot = ports.reader.history_snapshot(session).await.unwrap();
        let expected = snapshot
            .page(None, 200)
            .await
            .unwrap()
            .entries
            .iter()
            .map(|e| e.entry.id.get())
            .collect::<Vec<_>>();
        let mut bytes = Vec::new();
        let report = snapshot.export_jsonl(&mut bytes).await.unwrap();
        let values = verify(&bytes, &report);
        assert_eq!(
            values
                .iter()
                .filter(|v| v["kind"] == "entry")
                .map(|v| v["data"]["facts"]["entry"]["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
        let rounds = values
            .iter()
            .filter(|v| v["kind"] == "round")
            .collect::<Vec<_>>();
        assert_eq!(rounds.len(), 1);
        let raw_state = rounds[0]["data"]["provider_state_json"].as_str().unwrap();
        let stored: String =
            sqlx::query_scalar("SELECT provider_state_json FROM agena_rounds WHERE id=?")
                .bind(round.id().get())
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
        assert_eq!(raw_state, stored);
        assert_eq!(serde_json::from_str::<Value>(raw_state).unwrap(), opaque);
        assert_eq!(
            values.iter().filter(|v| v["kind"] == "round_input").count(),
            1
        );
        assert!(values.iter().any(|v| {
            v["kind"] == "entry"
                && v["data"]["facts"]["interaction"]["decision_json"]
                    == serde_json::to_string(&super::tests::permission_decision(
                        interaction,
                        true,
                        None,
                    ))
                    .unwrap()
        }));
        let mut blocks = BTreeMap::<(String, i64), Vec<u8>>::new();
        let mut media = Vec::new();
        for value in &values {
            let data = &value["data"];
            if value["kind"] == "content_piece" {
                let key = (
                    data["content_id"].as_str().unwrap().to_owned(),
                    data["block_start"].as_i64().unwrap(),
                );
                let body = blocks.entry(key).or_default();
                assert_eq!(body.len() as i64, data["byte_offset"].as_i64().unwrap());
                let piece = hex::decode(data["bytes_hex"].as_str().unwrap()).unwrap();
                assert!(piece.len() <= 64 * 1024);
                body.extend(piece);
            } else if value["kind"] == "blob_piece" {
                assert_eq!(data["byte_offset"].as_u64().unwrap(), media.len() as u64);
                media.extend(hex::decode(data["bytes_hex"].as_str().unwrap()).unwrap());
            }
        }
        assert_eq!(media, binary);
        let rows =
            sqlx::query("SELECT * FROM agena_content_blocks ORDER BY content_id,logical_start")
                .fetch_all(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
        assert_eq!(rows.len(), blocks.len());
        for row in rows {
            let key = (
                row.get::<String, _>("content_id"),
                row.get::<i64, _>("logical_start"),
            );
            let restored = blocks.remove(&key).unwrap();
            assert_eq!(restored, row.get::<Vec<u8>, _>("body"));
            assert_eq!(
                Sha256::digest(&restored).as_slice(),
                row.get::<Vec<u8>, _>("digest")
            );
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_content_checkpoints")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        assert_eq!(
            values
                .iter()
                .filter(|v| v["kind"] == "content_checkpoint")
                .count(),
            count as usize
        );
        drop((snapshot, prepared));
        maintenance.collect_history_snapshots().await.unwrap();
        maintenance.collect_blobs().await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_blob_pins")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap(),
            0
        );
        ports.shutdown().await.unwrap();
        assert_eq!(diagnostics.read_budget().bytes, 0);
    }
}

struct PausedSink {
    bytes: Arc<Mutex<Vec<u8>>>,
    tag: &'static str,
    arrived: Option<oneshot::Sender<()>>,
    resume: Option<oneshot::Receiver<()>>,
    fail: bool,
}
impl AsyncWrite for PausedSink {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        use std::future::Future;
        let this = self.get_mut();
        if bytes.starts_with(format!("{{\"kind\":\"{}\"", this.tag).as_bytes()) {
            if let Some(arrived) = this.arrived.take() {
                let _ = arrived.send(());
            }
            if let Some(resume) = &mut this.resume {
                if Pin::new(resume).poll(cx).is_pending() {
                    return Poll::Pending;
                }
                this.resume = None;
            }
            if this.fail {
                return Poll::Ready(Err(std::io::Error::other("injected sink failure")));
            }
        }
        this.bytes.lock().unwrap().extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
fn paused(
    tag: &'static str,
) -> (
    PausedSink,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    Arc<Mutex<Vec<u8>>>,
) {
    let (arrived, reached) = oneshot::channel();
    let (resume, gate) = oneshot::channel();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    (
        PausedSink {
            bytes: bytes.clone(),
            tag,
            arrived: Some(arrived),
            resume: Some(gate),
            fail: false,
        },
        reached,
        resume,
        bytes,
    )
}

fn assert_measured_reads_completed(diagnostics: &KernelDiagnostics) -> KernelMetricsSnapshot {
    let reads = diagnostics.snapshot();
    assert!(reads.read_transactions_started > 0);
    assert_eq!(
        reads.read_transactions_started,
        reads.read_transactions_completed
    );
    assert_eq!(reads.read_transactions_abandoned, 0);
    assert_eq!(reads.read_transaction_active_scopes, 0);
    assert_eq!(
        reads.read_transaction_duration_us_log2.iter().sum::<u64>(),
        reads.read_transactions_completed
    );
    assert!(reads.read_transaction_max_completed_ns <= reads.read_transaction_total_completed_ns);
    reads
}

#[tokio::test]
async fn failed_read_does_not_claim_an_acknowledged_transaction_end() {
    for memory in [true, false] {
        let (ports, _maintenance, admin, diagnostics, _root) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "expired read root".into())
            .await
            .unwrap()
            .value
            .id;
        let snapshot = ports.reader.history_snapshot(session).await.unwrap();
        // The empty successful page must also explicitly end its transaction.
        assert!(snapshot.page(None, 1).await.unwrap().entries.is_empty());
        let before = assert_measured_reads_completed(&diagnostics);
        // Simulate loss of a pinned root in this owned fixture. The normal
        // error path drops SQLx's transaction and queues its rollback; no
        // completion acknowledgement was observed by our measurement guard.
        sql(&admin, "DELETE FROM agena_history_snapshots").await;
        assert!(matches!(
            snapshot.page(None, 1).await,
            Err(KernelError::Conflict(_))
        ));
        let failed = diagnostics.snapshot();
        assert_eq!(
            failed.read_transactions_started,
            before.read_transactions_started + 1
        );
        assert_eq!(
            failed.read_transactions_completed,
            before.read_transactions_completed
        );
        assert_eq!(
            failed.read_transactions_abandoned,
            before.read_transactions_abandoned + 1
        );
        assert_eq!(failed.read_transaction_active_scopes, 0);
        assert_eq!(
            failed.read_transaction_duration_us_log2,
            before.read_transaction_duration_us_log2
        );
        assert_eq!(
            failed.read_transaction_total_completed_ns,
            before.read_transaction_total_completed_ns
        );
        drop(snapshot);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn every_export_sink_wait_follows_acknowledged_read_rollback() {
    for memory in [true, false] {
        let (ports, _maintenance, admin, diagnostics, _root) = fixture(memory).await;
        let (session, round, tool) = event_tests::tool(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output
            .append(&"archive \u{754c}\r\n".repeat(20_000))
            .await
            .unwrap();
        output.finish(RequestId::new()).await.unwrap();
        let body = vec![0x97; 200_000];
        let blob = ports
            .sessions
            .prepare_blob(
                RequestId::new(),
                session,
                body.len() as u64,
                &mut body.as_slice(),
            )
            .await
            .unwrap();
        tool.complete_with_blobs(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({}),
            },
            std::slice::from_ref(&blob),
        )
        .await
        .unwrap();
        let snapshot = ports.reader.history_snapshot(session).await.unwrap();
        for tag in ["header", "entry", "content_piece", "blob_piece"] {
            let (mut sink, reached, resume, bytes) = paused(tag);
            let root = snapshot.clone();
            let running = tokio::spawn(async move { root.export_jsonl(&mut sink).await });
            tokio::time::timeout(Duration::from_secs(5), reached)
                .await
                .unwrap()
                .unwrap();
            // This observes an actual suspended AsyncWrite, after the exporter
            // has performed the relevant history/fact/content/blob SQL reads.
            // Equality requires the ROLLBACK futures to have finished; Drop
            // alone is deliberately counted as abandoned, never completed.
            let paused_reads = assert_measured_reads_completed(&diagnostics);
            if !memory {
                let checkpoint = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                    .fetch_one(admin.get_sqlite_connection_pool())
                    .await
                    .unwrap();
                assert_eq!(checkpoint.get::<i64, _>(0), 0, "{tag} sink wait pins WAL");
                assert_eq!(checkpoint.get::<i64, _>(1), 0);
            }
            resume.send(()).unwrap();
            let report = tokio::time::timeout(Duration::from_secs(5), running)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            verify(&bytes.lock().unwrap(), &report);
            let finished_reads = assert_measured_reads_completed(&diagnostics);
            assert!(
                finished_reads.read_transactions_completed
                    >= paused_reads.read_transactions_completed
            );
        }
        drop((snapshot, blob));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn paused_export_keeps_membership_and_source_cut_without_pinning_wal_or_control_writes() {
    let (ports, maintenance, admin, diagnostics, _root) = fixture(false).await;
    let (session, round, _tool) = event_tests::tool(&ports).await;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    output.append(&"a".repeat(180_000)).await.unwrap();
    let mut watch = ports
        .reader
        .watch_content(session, output.id())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while watch.head().unwrap().committed == 0 {
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let snapshot = ports.reader.history_snapshot(session).await.unwrap();
    let expected_count = snapshot.info().entry_count;
    let execution = snapshot.page(None, 1).await.unwrap().entries[0]
        .entry
        .execution_id;
    let (mut sink, reached, resume, bytes) = paused("content");
    let root = snapshot.clone();
    let running = tokio::spawn(async move { root.export_jsonl(&mut sink).await });
    tokio::time::timeout(Duration::from_secs(3), reached)
        .await
        .unwrap()
        .unwrap();
    assert_measured_reads_completed(&diagnostics);
    tokio::time::timeout(Duration::from_secs(3), async {
        output.append(&"later".repeat(40_000)).await.unwrap();
        ports
            .sessions
            .send(
                RequestId::new(),
                session,
                document("outside exported membership"),
            )
            .await
            .unwrap();
        ports
            .sessions
            .cancel(RequestId::new(), session, execution)
            .await
            .unwrap();
    })
    .await
    .unwrap();
    let checkpoint = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
    assert_eq!(
        checkpoint.get::<i64, _>(0),
        0,
        "sink wait pinned a read transaction"
    );
    assert_eq!(checkpoint.get::<i64, _>(1), 0);
    resume.send(()).unwrap();
    let report = tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.entries, expected_count);
    let values = verify(&bytes.lock().unwrap(), &report);
    let source = values.iter().find(|v| v["kind"] == "content").unwrap();
    let committed = source["data"]["committed"].as_u64().unwrap();
    assert!(committed > 0 && committed <= 180_000);
    let body = values
        .iter()
        .filter(|v| v["kind"] == "content_piece")
        .flat_map(|v| hex::decode(v["data"]["bytes_hex"].as_str().unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(body, vec![b'a'; committed as usize]);
    assert!(diagnostics.read_budget().peak_bytes < 16 * 1024 * 1024);
    drop((watch, snapshot));
    maintenance.collect_history_snapshots().await.unwrap();
    ports.shutdown().await.unwrap();
    assert_eq!(diagnostics.read_budget().bytes, 0);
}

#[tokio::test]
async fn cancelled_or_failed_sink_releases_blob_read_pins_and_never_writes_success_footer() {
    for fail in [false, true] {
        let (ports, maintenance, admin, diagnostics, _root) = fixture(false).await;
        let (session, _round, tool) = event_tests::tool(&ports).await;
        let body = vec![0x97u8; 200_000];
        let blob = ports
            .sessions
            .prepare_blob(
                RequestId::new(),
                session,
                body.len() as u64,
                &mut body.as_slice(),
            )
            .await
            .unwrap();
        tool.complete_with_blobs(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({}),
            },
            std::slice::from_ref(&blob),
        )
        .await
        .unwrap();
        drop(blob);
        maintenance.collect_blobs().await.unwrap();
        let snapshot = ports.reader.history_snapshot(session).await.unwrap();
        let baseline = diagnostics.read_budget().bytes;
        let (mut sink, reached, resume, bytes) = paused("blob_piece");
        sink.fail = fail;
        let root = snapshot.clone();
        let running = tokio::spawn(async move { root.export_jsonl(&mut sink).await });
        tokio::time::timeout(Duration::from_secs(3), reached)
            .await
            .unwrap()
            .unwrap();
        assert_measured_reads_completed(&diagnostics);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_blob_pins")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap(),
            1
        );
        if fail {
            resume.send(()).unwrap();
            assert!(matches!(
                running.await.unwrap(),
                Err(KernelError::Storage(_))
            ));
        } else {
            running.abort();
            assert!(running.await.unwrap_err().is_cancelled());
        }
        assert_measured_reads_completed(&diagnostics);
        maintenance.collect_blobs().await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_blob_pins")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap(),
            0
        );
        assert_eq!(diagnostics.read_budget().bytes, baseline);
        assert!(
            !String::from_utf8(bytes.lock().unwrap().clone())
                .unwrap()
                .contains("\"kind\":\"footer\"")
        );
        let mut complete = Vec::new();
        let report = snapshot.export_jsonl(&mut complete).await.unwrap();
        verify(&complete, &report);
        drop(snapshot);
        ports.shutdown().await.unwrap();
        assert_eq!(diagnostics.read_budget().bytes, 0);
    }
}

#[tokio::test]
async fn snapshot_export_reads_retired_membership_and_blobs_after_current_edges_are_removed() {
    for memory in [true, false] {
        let (ports, _maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "old membership".into())
            .await
            .unwrap()
            .value
            .id;
        let blob = ports
            .sessions
            .prepare_blob(RequestId::new(), session, 3, &mut &b"\x00\xff\x17"[..])
            .await
            .unwrap();
        ports
            .sessions
            .send_with_blobs(
                RequestId::new(),
                session,
                document("pinned"),
                std::slice::from_ref(&blob),
            )
            .await
            .unwrap();
        let snapshot = ports.reader.history_snapshot(session).await.unwrap();
        ports
            .sessions
            .send(RequestId::new(), session, document("new edge"))
            .await
            .unwrap();
        // The new tail is unrelated to this root; its old tail remains pinned.
        sql(&admin, "DELETE FROM agena_session_segments").await;
        assert!(matches!(
            ports.reader.open_blob(session, blob.descriptor().id).await,
            Err(KernelError::Ownership)
        ));
        let mut bytes = Vec::new();
        let report = snapshot.export_jsonl(&mut bytes).await.unwrap();
        assert_eq!(report.entries, 1);
        let values = verify(&bytes, &report);
        assert!(
            values
                .iter()
                .any(|v| v["kind"] == "blob_piece" && v["data"]["bytes_hex"] == "00ff17")
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn existing_loss_ranges_are_exported_and_new_loss_or_reset_never_gets_a_success_footer() {
    for memory in [true, false] {
        let (ports, maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let (session, round, _tool) = event_tests::tool(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        let original = "\u{754c}\u{1f642}\r\n".repeat(25_000);
        output.append(&original).await.unwrap();
        let head = output.finish(RequestId::new()).await.unwrap().value;
        let snapshot = ports.reader.history_snapshot(session).await.unwrap();
        let (mut sink, reached, resume, bytes) = paused("content");
        let root = snapshot.clone();
        let running = tokio::spawn(async move { root.export_jsonl(&mut sink).await });
        tokio::time::timeout(Duration::from_secs(3), reached)
            .await
            .unwrap()
            .unwrap();
        let pruned = maintenance
            .prune_content(RequestId::new(), session, output.id(), 100_000)
            .await
            .unwrap()
            .value;
        assert!(pruned.stream.retained_start > 0);
        resume.send(()).unwrap();
        assert!(matches!(
            running.await.unwrap(),
            Err(KernelError::ContentGap { .. })
        ));
        assert!(
            !String::from_utf8(bytes.lock().unwrap().clone())
                .unwrap()
                .contains("\"kind\":\"footer\"")
        );
        let mut complete = Vec::new();
        let report = snapshot.export_jsonl(&mut complete).await.unwrap();
        let values = verify(&complete, &report);
        let gaps = values
            .iter()
            .filter(|v| v["kind"] == "content_gap")
            .collect::<Vec<_>>();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0]["data"]["logical_start"], 0);
        assert_eq!(gaps[0]["data"]["logical_end"], pruned.stream.retained_start);
        let suffix = values
            .iter()
            .filter(|v| v["kind"] == "content_piece")
            .flat_map(|v| hex::decode(v["data"]["bytes_hex"].as_str().unwrap()).unwrap())
            .collect::<Vec<_>>();
        let physical_start =
            values.iter().find(|v| v["kind"] == "content").unwrap()["data"]["physical_start"]
                .as_u64()
                .unwrap();
        assert!(physical_start <= pruned.stream.retained_start);
        assert_eq!(suffix, &original.as_bytes()[physical_start as usize..]);
        assert_eq!(
            &suffix[(pruned.stream.retained_start - physical_start) as usize..],
            &original.as_bytes()[pruned.stream.retained_start as usize..]
        );
        assert_eq!(head.committed, original.len() as u64);
        let (mut sink, reached, resume, bytes) = paused("content");
        let root = snapshot.clone();
        let running = tokio::spawn(async move { root.export_jsonl(&mut sink).await });
        tokio::time::timeout(Duration::from_secs(3), reached)
            .await
            .unwrap()
            .unwrap();
        sqlx::query("UPDATE agena_content_streams SET epoch=? WHERE id=?")
            .bind(ContentEpoch::new().to_string())
            .bind(output.id().to_string())
            .execute(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        resume.send(()).unwrap();
        assert!(matches!(
            running.await.unwrap(),
            Err(KernelError::ContentReset)
        ));
        assert!(
            !String::from_utf8(bytes.lock().unwrap().clone())
                .unwrap()
                .contains("\"kind\":\"footer\"")
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn content_corruption_is_detected_before_a_success_footer() {
    let (ports, _maintenance, admin, _diagnostics, _root) = fixture(false).await;
    let (session, round, _tool) = event_tests::tool(&ports).await;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    output.append("immutable bytes\n").await.unwrap();
    output.finish(RequestId::new()).await.unwrap();
    let row = sqlx::query("SELECT * FROM agena_content_blocks WHERE content_id=?")
        .bind(output.id().to_string())
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
    let mut tx = admin.get_sqlite_connection_pool().begin().await.unwrap();
    sqlx::query("DELETE FROM agena_content_blocks WHERE content_id=?")
        .bind(output.id().to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agena_content_blocks VALUES (?,?,?,?,?,?)")
        .bind(output.id().to_string())
        .bind(row.get::<i64, _>("logical_start"))
        .bind(row.get::<i64, _>("logical_end"))
        .bind(row.get::<String, _>("codec"))
        .bind(row.get::<Vec<u8>, _>("digest"))
        .bind(vec![b'x'; row.get::<Vec<u8>, _>("body").len()])
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut bytes = Vec::new();
    let result = ports.reader.export_jsonl(session, &mut bytes).await;
    assert!(matches!(result,Err(KernelError::Storage(ref error)) if error.contains("checksum")));
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains("\"kind\":\"footer\"")
    );
    ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn export_admission_bounds_clone_families_and_is_released_after_cancellation() {
    let (ports, maintenance, _admin, diagnostics, _root) = fixture(false).await;
    let mut handles = Vec::new();
    let mut roots = Vec::new();
    for index in 0..4 {
        let session = ports
            .sessions
            .create(RequestId::new(), 1, format!("export owner {index}"))
            .await
            .unwrap()
            .value
            .id;
        let root = ports.reader.history_snapshot(session).await.unwrap();
        for _ in 0..2 {
            let (mut sink, reached, _resume, _bytes) = paused("header");
            // Keep the gate sender live in the task until it is cancelled.
            let cloned = root.clone();
            let running = tokio::spawn(async move {
                let _gate = _resume;
                cloned.export_jsonl(&mut sink).await
            });
            tokio::time::timeout(Duration::from_secs(3), reached)
                .await
                .unwrap()
                .unwrap();
            handles.push(running);
        }
        assert!(matches!(
            root.export_jsonl(&mut Vec::new()).await,
            Err(KernelError::Busy)
        ));
        roots.push(root);
    }
    let extra = ports
        .sessions
        .create(RequestId::new(), 1, "global limit".into())
        .await
        .unwrap()
        .value
        .id;
    let extra_root = ports.reader.history_snapshot(extra).await.unwrap();
    assert!(matches!(
        extra_root.export_jsonl(&mut Vec::new()).await,
        Err(KernelError::Busy)
    ));
    let first = handles.remove(0);
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let mut bytes = Vec::new();
    let report = extra_root.export_jsonl(&mut bytes).await.unwrap();
    assert_eq!(report.entries, 0);
    verify(&bytes, &report);
    for running in handles {
        running.abort();
        assert!(running.await.unwrap_err().is_cancelled());
    }
    drop((roots, extra_root));
    maintenance.collect_history_snapshots().await.unwrap();
    ports.shutdown().await.unwrap();
    assert_eq!(diagnostics.read_budget().bytes, 0);
}

#[tokio::test]
async fn archived_retained_document_blocks_replay_from_the_declared_checkpoint_floor() {
    use agena_storage::backend::content_events::ImmutableEventBlock;
    for memory in [true, false] {
        let (ports, maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let (session, _round, tool) = event_tests::tool(&ports).await;
        let output = tool.begin_document(RequestId::new()).await.unwrap().value;
        output
            .apply(DocumentMutation::Insert {
                after: None,
                block: ViewBlock::Text {
                    id: Some("body".into()),
                    text: "initial\n".into(),
                },
            })
            .await
            .unwrap();
        for _ in 0..150 {
            output
                .apply(DocumentMutation::AppendText {
                    block_id: "body".into(),
                    text: "\u{754c}\u{1f642}\r\n".into(),
                })
                .await
                .unwrap();
        }
        output.finish(RequestId::new()).await.unwrap();
        let retained = maintenance
            .prune_content(RequestId::new(), session, output.id(), 140)
            .await
            .unwrap()
            .value
            .stream;
        let expected = ports
            .reader
            .event_snapshot(session, output.id())
            .await
            .unwrap();
        assert!(retained.retained_start > 0);
        let mut bytes = Vec::new();
        let report = ports
            .reader
            .export_jsonl(session, &mut bytes)
            .await
            .unwrap();
        let values = verify(&bytes, &report);
        let mut projection = None;
        let mut meta = None;
        let mut block_bytes = Vec::new();
        let replay = |meta: &Value, body: Vec<u8>, projection: &mut Option<EventProjection>| {
            let digest = serde_json::from_value::<Vec<u8>>(meta["sha256"].clone())
                .unwrap()
                .try_into()
                .unwrap();
            let block = ImmutableEventBlock::from_stored(
                meta["logical_start"].as_u64().unwrap(),
                meta["logical_end"].as_u64().unwrap(),
                body,
                digest,
            )
            .unwrap();
            for record in block.iter() {
                let record = record.unwrap();
                if record.sequence <= retained.retained_start {
                    continue;
                }
                if let Some(projection) = projection {
                    projection.apply(&record).unwrap();
                } else {
                    *projection = Some(EventProjection::from_checkpoint(&record).unwrap());
                }
            }
        };
        for value in &values {
            if value["kind"] == "content_block" {
                if let Some(meta) = meta.take() {
                    replay(meta, std::mem::take(&mut block_bytes), &mut projection);
                }
                meta = Some(&value["data"]);
            } else if value["kind"] == "content_piece" {
                block_bytes
                    .extend(hex::decode(value["data"]["bytes_hex"].as_str().unwrap()).unwrap());
            }
        }
        if let Some(meta) = meta {
            replay(meta, block_bytes, &mut projection);
        }
        let restored = projection.unwrap().snapshot(retained).unwrap();
        assert_eq!(restored.document, expected.document);
        assert_eq!(restored.through, expected.through);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn round_inputs_stream_in_pages_once_instead_of_repeating_for_each_output_entry() {
    let (ports, _maintenance, _admin, diagnostics, _root) = fixture(false).await;
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "paged round inputs".into())
        .await
        .unwrap()
        .value
        .id;
    let mut inputs = Vec::new();
    let mut execution = None;
    for index in 0..205 {
        let sent = ports
            .sessions
            .send(
                RequestId::new(),
                session,
                document(&format!("input {index}")),
            )
            .await
            .unwrap()
            .value;
        inputs.push(sent.message_id);
        execution = Some(sent.id);
    }
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, execution.unwrap())
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_round(RequestId::new(), inputs.clone())
        .await
        .unwrap()
        .value;
    for _ in 0..20 {
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output.append("one output\n").await.unwrap();
        output.finish(RequestId::new()).await.unwrap();
    }
    round
        .finish(
            RequestId::new(),
            Some(json!({"opaque":[1,null,true]})),
            true,
        )
        .await
        .unwrap();
    let mut bytes = Vec::new();
    let report = ports
        .reader
        .export_jsonl(session, &mut bytes)
        .await
        .unwrap();
    let values = verify(&bytes, &report);
    assert_eq!(report.entries, 225);
    assert_eq!(values.iter().filter(|v| v["kind"] == "round").count(), 1);
    let exported = values
        .iter()
        .filter(|v| v["kind"] == "round_input")
        .map(|v| v["data"]["entry_id"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        exported,
        inputs.iter().map(|id| id.get()).collect::<Vec<_>>()
    );
    assert!(diagnostics.read_budget().peak_bytes < 16 * 1024 * 1024);
    ports.shutdown().await.unwrap();
}
