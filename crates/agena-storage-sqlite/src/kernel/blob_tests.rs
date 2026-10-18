use std::{
    pin::Pin,
    task::{Context, Poll},
};

use agena_storage::backend::kernel::KernelMaintenance;
use tokio::io::{AsyncRead, ReadBuf};

use super::{
    tests::{document, sql},
    *,
};

pub(super) async fn fixture(
    memory: bool,
) -> (
    KernelPorts,
    KernelMaintenance,
    Arc<sea_orm::DatabaseConnection>,
    tempfile::TempDir,
    String,
) {
    let root = tempfile::tempdir().unwrap();
    let url = if memory {
        "sqlite::memory:".into()
    } else {
        format!(
            "sqlite://{}?mode=rwc",
            root.path().join("blobs.db").display()
        )
    };
    let db = KernelDatabase::open(&url).await.unwrap();
    let admin = db.writer.clone();
    let (ports, maintenance) = compose_database(db, KernelDiagnostics::default())
        .await
        .unwrap();
    sql(&admin,"INSERT INTO agena_workspaces (id,path,created_at_ms,updated_at_ms) VALUES (1,'isolated blob fixture',0,0)").await;
    (ports, maintenance, admin, root, url)
}
async fn session(ports: &KernelPorts) -> SessionId {
    ports
        .sessions
        .create(RequestId::new(), 1, "managed blob".into())
        .await
        .unwrap()
        .value
        .id
}
fn body(chunks: usize) -> Vec<u8> {
    (0..chunks * 128 * 1024 + 17)
        .map(|n| ((n * 71 + n / 97) % 256) as u8)
        .collect()
}
async fn count(admin: &sea_orm::DatabaseConnection, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn streamed_preparation_replays_and_publishes_atomically_with_message_identity() {
    for memory in [true, false] {
        let (ports, gc, admin, _root, _url) = fixture(memory).await;
        let owner = session(&ports).await;
        let other = session(&ports).await;
        let bytes = body(3);
        let request = RequestId::new();
        let prepared = ports
            .sessions
            .prepare_blob(request, owner, bytes.len() as u64, &mut bytes.as_slice())
            .await
            .unwrap();
        let descriptor = prepared.descriptor().clone();
        assert_eq!(descriptor.sha256, <[u8; 32]>::from(Sha256::digest(&bytes)));
        assert_eq!(count(&admin, "agena_blob_chunks").await, 4);
        assert!(matches!(
            ports.reader.open_blob(owner, descriptor.id).await,
            Err(KernelError::Ownership)
        ));
        assert_eq!(gc.collect_blobs().await.unwrap().retired, 0);
        assert!(matches!(
            ports
                .sessions
                .send_with_blobs(
                    RequestId::new(),
                    other,
                    document("wrong owner"),
                    std::slice::from_ref(&prepared)
                )
                .await,
            Err(KernelError::Ownership)
        ));
        let send = RequestId::new();
        let sent = ports
            .sessions
            .send_with_blobs(
                send,
                owner,
                document("explicit attachment"),
                std::slice::from_ref(&prepared),
            )
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .entry_blobs(owner, sent.value.message_id)
                .await
                .unwrap()
                .as_slice(),
            vec![descriptor.clone()]
        );
        assert!(matches!(
            ports.reader.open_blob(other, descriptor.id).await,
            Err(KernelError::Ownership)
        ));
        let replay = ports
            .sessions
            .prepare_blob(request, owner, bytes.len() as u64, &mut bytes.as_slice())
            .await
            .unwrap();
        assert_eq!(replay.descriptor(), &descriptor);
        assert_eq!(count(&admin, "agena_blob_chunks").await, 4);
        assert_eq!(
            ports
                .sessions
                .send_with_blobs(
                    send,
                    owner,
                    document("explicit attachment"),
                    std::slice::from_ref(&replay)
                )
                .await
                .unwrap(),
            sent
        );
        let reader = ports.reader.open_blob(owner, descriptor.id).await.unwrap();
        let page = reader.range(128 * 1024 - 3, 8).await.unwrap();
        assert_eq!(page.bytes, &bytes[128 * 1024 - 3..128 * 1024 + 5]);
        let mut copied = Vec::new();
        assert_eq!(
            reader.copy_to(&mut copied).await.unwrap(),
            bytes.len() as u64
        );
        assert_eq!(copied, bytes);
        drop(prepared);
        drop(replay);
        drop(reader);
        for _ in 0..3 {
            assert_eq!(gc.collect_blobs().await.unwrap().chunks, 0);
        }
        assert_eq!(count(&admin, "agena_blobs").await, 1);
        ports.shutdown().await.unwrap();
    }
}

struct PausedInput {
    prefix: Vec<u8>,
    offset: usize,
    blocked: Option<oneshot::Sender<()>>,
}
impl AsyncRead for PausedInput {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let input = self.get_mut();
        if input.offset < input.prefix.len() {
            let end = (input.offset + buffer.remaining()).min(input.prefix.len());
            buffer.put_slice(&input.prefix[input.offset..end]);
            input.offset = end;
            Poll::Ready(Ok(()))
        } else {
            if let Some(blocked) = input.blocked.take() {
                let _ = blocked.send(());
            }
            Poll::Pending
        }
    }
}

#[tokio::test]
async fn gc_protects_preparation_and_live_read_families_then_reclaims_bounded_chunks() {
    for memory in [true, false] {
        let (ports, gc, admin, _root, _url) = fixture(memory).await;
        let owner = session(&ports).await;
        let (ready, blocked) = oneshot::channel();
        let sessions = ports.sessions.clone();
        let task = tokio::spawn(async move {
            let mut source = PausedInput {
                prefix: vec![7; 128 * 1024],
                offset: 0,
                blocked: Some(ready),
            };
            sessions
                .prepare_blob(RequestId::new(), owner, 256 * 1024, &mut source)
                .await
        });
        blocked.await.unwrap();
        assert_eq!(count(&admin, "agena_blob_chunks").await, 1);
        assert_eq!(gc.collect_blobs().await.unwrap().retired, 0);
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        for _ in 0..3 {
            gc.collect_blobs().await.unwrap();
        }
        assert_eq!(count(&admin, "agena_blobs").await, 0);
        let bytes = body(17);
        let prepared = ports
            .sessions
            .prepare_blob(
                RequestId::new(),
                owner,
                bytes.len() as u64,
                &mut bytes.as_slice(),
            )
            .await
            .unwrap();
        let sent = ports
            .sessions
            .send_with_blobs(
                RequestId::new(),
                owner,
                document("read pin"),
                std::slice::from_ref(&prepared),
            )
            .await
            .unwrap();
        let reader = ports
            .reader
            .open_blob(owner, prepared.descriptor().id)
            .await
            .unwrap();
        let clone = reader.clone();
        drop(reader);
        drop(prepared);
        // Infrastructure fixture for the future branch/entry-prune command.
        // This is not claimed as public branch deletion coverage.
        sql(
            &admin,
            &format!(
                "DELETE FROM agena_blob_references WHERE entry_id={}",
                sent.value.message_id.get()
            ),
        )
        .await;
        for _ in 0..3 {
            assert_eq!(gc.collect_blobs().await.unwrap().chunks, 0);
        }
        let mut copied = Vec::new();
        clone.copy_to(&mut copied).await.unwrap();
        assert_eq!(copied, bytes);
        drop(clone);
        let mut removed = 0;
        for _ in 0..8 {
            let result = gc.collect_blobs().await.unwrap();
            assert!(result.retired <= 16 && result.chunks <= 8 && result.blobs <= 16);
            removed += result.chunks;
        }
        assert_eq!(removed, 18);
        assert_eq!(count(&admin, "agena_blobs").await, 0);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn failed_ready_or_reference_transactions_leave_no_false_success_and_can_resume() {
    for memory in [true, false] {
        let (ports, gc, admin, _root, _url) = fixture(memory).await;
        let owner = session(&ports).await;
        let request = RequestId::new();
        let bytes = body(2);
        sql(&admin,"CREATE TRIGGER reject_ready BEFORE UPDATE OF state ON agena_blobs WHEN NEW.state='ready' BEGIN SELECT RAISE(ABORT,'injected ready failure'); END").await;
        assert!(
            ports
                .sessions
                .prepare_blob(request, owner, bytes.len() as u64, &mut bytes.as_slice())
                .await
                .is_err()
        );
        assert_eq!(count(&admin, "agena_blob_chunks").await, 3);
        let state: String = sqlx::query_scalar("SELECT state FROM agena_blobs")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        assert_eq!(state, "preparing");
        sql(&admin, "DROP TRIGGER reject_ready").await;
        let prepared = ports
            .sessions
            .prepare_blob(request, owner, bytes.len() as u64, &mut bytes.as_slice())
            .await
            .unwrap();
        assert_eq!(count(&admin, "agena_blob_chunks").await, 3);
        let sent = ports
            .sessions
            .send(RequestId::new(), owner, document("tool result"))
            .await
            .unwrap()
            .value;
        let attempt = ports
            .executor
            .claim(RequestId::new(), owner, sent.id)
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
                    call_id: "large-result".into(),
                    name: "fixture".into(),
                    input: serde_json::Value::Null,
                },
            )
            .await
            .unwrap()
            .value;
        let output = tool.begin_text(RequestId::new()).await.unwrap().value;
        output
            .append("tail must commit with result reference")
            .await
            .unwrap();
        sql(&admin,"CREATE TRIGGER reject_reference BEFORE INSERT ON agena_blob_references BEGIN SELECT RAISE(ABORT,'injected reference failure'); END").await;
        let complete = RequestId::new();
        let result = ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"managed_bytes":bytes.len()}),
        };
        assert!(
            tool.complete_with_blobs(complete, result.clone(), std::slice::from_ref(&prepared))
                .await
                .is_err()
        );
        assert_eq!(
            ports.reader.tool(owner, tool.id()).await.unwrap().phase,
            ToolPhase::Running
        );
        assert_eq!(count(&admin, "agena_blob_references").await, 0);
        assert_eq!(
            ports
                .reader
                .text_range(owner, output.id(), None, 128)
                .await
                .unwrap()
                .stream
                .state,
            CaptureState::Open
        );
        assert_eq!(gc.collect_blobs().await.unwrap().retired, 0);
        sql(&admin, "DROP TRIGGER reject_reference").await;
        tool.complete_with_blobs(complete, result, std::slice::from_ref(&prepared))
            .await
            .unwrap();
        let detail = ports.reader.tool(owner, tool.id()).await.unwrap();
        assert_eq!(detail.phase, ToolPhase::Succeeded);
        assert_eq!(detail.blobs, vec![prepared.descriptor().clone()]);
        let head = ports
            .reader
            .text_range(owner, output.id(), None, 128)
            .await
            .unwrap()
            .stream
            .clone();
        assert_eq!(head.committed, head.accepted);
        assert_eq!(head.state, CaptureState::Complete);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn short_extra_changed_and_corrupt_blob_bytes_are_never_hidden() {
    for memory in [true, false] {
        let (ports, gc, admin, _root, _url) = fixture(memory).await;
        let owner = session(&ports).await;
        assert!(
            ports
                .sessions
                .prepare_blob(RequestId::new(), owner, 4, &mut b"abc".as_slice())
                .await
                .is_err()
        );
        assert!(
            ports
                .sessions
                .prepare_blob(RequestId::new(), owner, 2, &mut b"abc".as_slice())
                .await
                .is_err()
        );
        let request = RequestId::new();
        let prepared = ports
            .sessions
            .prepare_blob(request, owner, 3, &mut b"abc".as_slice())
            .await
            .unwrap();
        assert!(matches!(
            ports
                .sessions
                .prepare_blob(request, owner, 3, &mut b"abd".as_slice())
                .await,
            Err(KernelError::Conflict(_))
        ));
        let empty = ports
            .sessions
            .prepare_blob(RequestId::new(), owner, 0, &mut b"".as_slice())
            .await
            .unwrap();
        let mut copy = Vec::new();
        assert_eq!(empty.reader().copy_to(&mut copy).await.unwrap(), 0);
        sql(&admin, "DROP TRIGGER agena_blob_chunk_immutable").await;
        sql(
            &admin,
            &format!(
                "UPDATE agena_blob_chunks SET body=x'616264' WHERE blob_id='{}'",
                prepared.descriptor().id
            ),
        )
        .await;
        assert!(matches!(
            prepared.reader().range(0, 3).await,
            Err(KernelError::Storage(_))
        ));
        drop(empty);
        drop(prepared);
        for _ in 0..5 {
            gc.collect_blobs().await.unwrap();
        }
        assert_eq!(count(&admin, "agena_blobs").await, 0);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn sqlite_full_during_preparation_is_recoverable_without_repeating_published_facts() {
    for memory in [true, false] {
        let (ports, _gc, admin, _root, _url) = fixture(memory).await;
        let owner = session(&ports).await;
        let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        sql(&admin, &format!("PRAGMA max_page_count={}", pages + 20)).await;
        let bytes = body(2);
        let request = RequestId::new();
        assert!(
            ports
                .sessions
                .prepare_blob(request, owner, bytes.len() as u64, &mut bytes.as_slice())
                .await
                .is_err()
        );
        assert_eq!(count(&admin, "agena_blob_references").await, 0);
        sql(&admin, "PRAGMA max_page_count=2147483647").await;
        let prepared = ports
            .sessions
            .prepare_blob(request, owner, bytes.len() as u64, &mut bytes.as_slice())
            .await
            .unwrap();
        let mut copied = Vec::new();
        prepared.reader().copy_to(&mut copied).await.unwrap();
        assert_eq!(copied, bytes);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn upload_and_pin_admission_are_bounded_and_reads_emit_no_transcript_changes() {
    for memory in [true, false] {
        let (ports, gc, admin, _root, _url) = fixture(memory).await;
        let owner = session(&ports).await;
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let (ready, blocked) = oneshot::channel();
            let sessions = ports.sessions.clone();
            tasks.push(tokio::spawn(async move {
                sessions
                    .prepare_blob(
                        RequestId::new(),
                        owner,
                        1,
                        &mut PausedInput {
                            prefix: Vec::new(),
                            offset: 0,
                            blocked: Some(ready),
                        },
                    )
                    .await
            }));
            blocked.await.unwrap();
        }
        assert!(matches!(
            ports
                .sessions
                .prepare_blob(RequestId::new(), owner, 0, &mut b"".as_slice())
                .await,
            Err(KernelError::Busy)
        ));
        for task in tasks {
            task.abort();
            assert!(matches!(task.await,Err(error) if error.is_cancelled()));
        }
        for _ in 0..4 {
            gc.collect_blobs().await.unwrap();
        }
        assert_eq!(count(&admin, "agena_blobs").await, 0);
        let prepared = ports
            .sessions
            .prepare_blob(RequestId::new(), owner, 0, &mut b"".as_slice())
            .await
            .unwrap();
        ports
            .sessions
            .send_with_blobs(
                RequestId::new(),
                owner,
                document("bounded read pins"),
                std::slice::from_ref(&prepared),
            )
            .await
            .unwrap();
        let mut watch = ports.reader.watch(owner, None).unwrap();
        let mut readers = Vec::new();
        for _ in 0..127 {
            readers.push(
                ports
                    .reader
                    .open_blob(owner, prepared.descriptor().id)
                    .await
                    .unwrap(),
            );
        }
        assert!(matches!(
            ports
                .reader
                .open_blob(owner, prepared.descriptor().id)
                .await,
            Err(KernelError::Busy)
        ));
        let clone = readers[0].clone();
        drop(readers);
        for _ in 0..3 {
            gc.collect_blobs().await.unwrap();
        }
        assert_eq!(count(&admin, "agena_blob_pins").await, 2);
        let another = ports
            .reader
            .open_blob(owner, prepared.descriptor().id)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), watch.recv())
                .await
                .is_err()
        );
        assert!(clone.range(0, 1).await.unwrap().eof);
        drop(another);
        drop(clone);
        drop(prepared);
        ports.shutdown().await.unwrap();
    }
}
