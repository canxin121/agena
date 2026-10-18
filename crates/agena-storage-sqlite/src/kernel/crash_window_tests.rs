//! Exact durability boundaries, compiled only into the library test binary.
//! Each child owns a fresh TempDir database; the parent never mutates its env.

use std::{
    io::{BufRead, Write},
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agena_domain::ComposerDocument;

use super::{tests::document, *};

const CHILD_ROOT: &str = "AGENA_KERNEL_CRASH_WINDOW_ROOT";
const CHILD_KIND: &str = "AGENA_KERNEL_CRASH_WINDOW_KIND";
const CHILD_STAGE: &str = "AGENA_KERNEL_CRASH_WINDOW_STAGE";
const CHILD_REQUEST: &str = "AGENA_KERNEL_CRASH_WINDOW_REQUEST";
const CHILD_PREPARATION: &str = "AGENA_KERNEL_CRASH_WINDOW_PREPARATION";
const CHILD_PUBLICATION: &str = "AGENA_KERNEL_CRASH_WINDOW_PUBLICATION";
const MARKER: &str = "KERNEL_CRASH_WINDOW_REACHED ";

struct Spec {
    stage: String,
    request: RequestId,
}

static SPEC: OnceLock<Option<Spec>> = OnceLock::new();
static ARMED: AtomicBool = AtomicBool::new(false);

struct ConfirmationLoss {
    request: RequestId,
    fired_cache: Option<std::sync::Weak<read_cache::ReadCache>>,
    closed: oneshot::Sender<()>,
}
static CONFIRMATION_LOSS: std::sync::Mutex<Option<ConfirmationLoss>> = std::sync::Mutex::new(None);

/// Lose only the acknowledgement of one successful, real COMMIT. The normal
/// error path must classify this as unknown and close its backend. This is
/// controlled confirmation loss, not an operating-system disk fault.
pub(super) fn lose_confirmation(
    committed: std::result::Result<(), sqlx::Error>,
    batch: &[Pending],
    cache: &Arc<read_cache::ReadCache>,
) -> std::result::Result<(), sqlx::Error> {
    if committed.is_ok() {
        let mut injection = CONFIRMATION_LOSS.lock().unwrap();
        if let Some(injection) = injection.as_mut()
            && injection.fired_cache.is_none()
            && batch
                .iter()
                .any(|pending| pending.request == injection.request)
        {
            injection.fired_cache = Some(Arc::downgrade(cache));
            return Err(sqlx::Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "test-only acknowledgement lost after real SQLite COMMIT",
            )));
        }
    }
    committed
}

pub(super) fn writer_closed(cache: &Arc<read_cache::ReadCache>) {
    let mut injection = CONFIRMATION_LOSS.lock().unwrap();
    if injection.as_ref().is_some_and(|injection| {
        injection
            .fired_cache
            .as_ref()
            .is_some_and(|fired| fired.ptr_eq(&Arc::downgrade(cache)))
    }) && let Some(injection) = injection.take()
    {
        let _ = injection.closed.send(());
    }
}

/// A target request is mandatory for command boundaries. Maintenance boundaries
/// are gated by the isolated child spec and the explicit post-setup arm.
pub(super) fn hit(stage: &str, request: Option<RequestId>) {
    let spec = SPEC.get_or_init(|| {
        std::env::var(CHILD_ROOT).ok()?;
        Some(Spec {
            stage: std::env::var(CHILD_STAGE).ok()?,
            request: std::env::var(CHILD_REQUEST).ok()?.parse().ok()?,
        })
    });
    let Some(spec) = spec else { return };
    if !ARMED.load(Ordering::Acquire)
        || spec.stage != stage
        || request.is_some_and(|request| request != spec.request)
    {
        return;
    }
    println!("{MARKER}{stage} {}", spec.request);
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::park();
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Evidence {
    session: SessionId,
    preparation: RequestId,
    publication: RequestId,
    blob: Option<BlobDescriptor>,
    protected: Option<(EntryId, BlobDescriptor)>,
    content: Option<ContentId>,
    watch: Option<WatchCursor>,
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn body(chunks: usize) -> Vec<u8> {
    (0..chunks * 128 * 1024 + 17)
        .map(|n| ((n * 71 + n / 97) % 256) as u8)
        .collect()
}

fn text() -> String {
    // Two complete 64 KiB blocks: the checkpoint has an exact durable head.
    "\0x\r\n".repeat(32 * 1024)
}

fn derived(request: RequestId, tag: &[u8], offset: u64) -> RequestId {
    let mut hash = Sha256::new();
    hash.update(request.to_string());
    hash.update(tag);
    hash.update(offset.to_le_bytes());
    hex::encode(hash.finalize())[..32].parse().unwrap()
}

async fn child(root: &Path, kind: &str) {
    let url = format!("sqlite://{}?mode=rwc", root.join("kernel.db").display());
    let kernel = open_kernel_runtime(&url).await.unwrap();
    let workspace = kernel
        .workspaces
        .create(root.canonicalize().unwrap().display().to_string())
        .await
        .unwrap();
    let ports = &kernel.ports;
    let session = ports
        .sessions
        .create(RequestId::new(), workspace.id, "exact crash window".into())
        .await
        .unwrap()
        .value
        .id;
    let preparation = std::env::var(CHILD_PREPARATION).unwrap().parse().unwrap();
    let publication = std::env::var(CHILD_PUBLICATION).unwrap().parse().unwrap();
    let target = std::env::var(CHILD_REQUEST).unwrap().parse().unwrap();
    let mut evidence = Evidence {
        session,
        preparation,
        publication,
        blob: None,
        protected: None,
        content: None,
        watch: None,
    };
    let bytes = body(2);
    let mut prepared = None;
    let mut round = None;
    let mut output = None;
    let mut watch = None;
    match kind {
        "prepare" | "chunk" | "ready" => {}
        "publish" | "gc" => {
            let blob = ports
                .sessions
                .prepare_blob(
                    preparation,
                    session,
                    bytes.len() as u64,
                    &mut bytes.as_slice(),
                )
                .await
                .unwrap();
            if kind == "gc" {
                let sent = ports
                    .sessions
                    .send_with_blobs(
                        publication,
                        session,
                        document("protected reference"),
                        std::slice::from_ref(&blob),
                    )
                    .await
                    .unwrap();
                evidence.protected = Some((sent.value.message_id, blob.descriptor().clone()));
                let orphan = body(10);
                let orphan = ports
                    .sessions
                    .prepare_blob(
                        RequestId::new(),
                        session,
                        orphan.len() as u64,
                        &mut orphan.as_slice(),
                    )
                    .await
                    .unwrap();
                evidence.blob = Some(orphan.descriptor().clone());
                drop(orphan);
            } else {
                evidence.blob = Some(blob.descriptor().clone());
                // Warm a real projection and leave a real subscriber across
                // the installation/publication boundaries being interrupted.
                ports.reader.history(session, None, 16).await.unwrap();
                let subscribed = ports.reader.watch(session, None).unwrap();
                evidence.watch = subscribed.head().cloned();
                watch = Some(subscribed);
            }
            prepared = Some(blob);
        }
        "open" | "checkpoint" => {
            let execution = ports
                .sessions
                .send(RequestId::new(), session, document("capture"))
                .await
                .unwrap()
                .value;
            let attempt = ports
                .executor
                .claim(RequestId::new(), session, execution.id)
                .await
                .unwrap()
                .value;
            let current = attempt
                .begin_current_round(RequestId::new())
                .await
                .unwrap()
                .value;
            if kind == "checkpoint" {
                let current_output = current
                    .begin_text(RequestId::new(), false)
                    .await
                    .unwrap()
                    .value;
                evidence.content = Some(current_output.id());
                output = Some(current_output);
            }
            round = Some(current);
        }
        _ => panic!("unknown crash fixture kind {kind}"),
    }
    std::fs::write(
        root.join("evidence.json"),
        serde_json::to_vec(&evidence).unwrap(),
    )
    .unwrap();
    ARMED.store(true, Ordering::Release);
    match kind {
        "prepare" | "chunk" | "ready" => {
            ports
                .sessions
                .prepare_blob(
                    preparation,
                    session,
                    bytes.len() as u64,
                    &mut bytes.as_slice(),
                )
                .await
                .unwrap();
        }
        "publish" => {
            ports
                .sessions
                .send_with_blobs(
                    publication,
                    session,
                    publication_input(),
                    std::slice::from_ref(prepared.as_ref().unwrap()),
                )
                .await
                .unwrap();
        }
        "open" => {
            round
                .as_ref()
                .unwrap()
                .begin_text(target, false)
                .await
                .unwrap();
        }
        "checkpoint" => {
            output.as_ref().unwrap().append(&text()).await.unwrap();
        }
        "gc" => {
            kernel.maintenance.collect_blobs().await.unwrap();
        }
        _ => unreachable!(),
    }
    // after_receipt can deliver to the caller while its writer is parked.
    // Keep all handles/pins alive until the parent kills this process.
    let _retained = (prepared, round, output, watch, kernel);
    std::future::pending::<()>().await;
}

fn publication_input() -> ComposerDocument {
    document("exact publication 世界🙂\r\n\0")
}

async fn receipt(database: &sea_orm::DatabaseConnection, request: RequestId) -> Option<String> {
    sqlx::query_scalar("SELECT receipt_json FROM agena_command_results WHERE request_id=?")
        .bind(request.to_string())
        .fetch_optional(database.get_sqlite_connection_pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn lost_real_commit_confirmation_returns_unknown_closes_backend_and_replays_exactly() {
    let (ports, admin, root, url) = super::tests::fixture(false).await;
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "unknown confirmation".into())
        .await
        .unwrap()
        .value
        .id;
    let request = RequestId::new();
    let (closed, closing) = oneshot::channel();
    *CONFIRMATION_LOSS.lock().unwrap() = Some(ConfirmationLoss {
        request,
        fired_cache: None,
        closed,
    });
    let result = ports
        .sessions
        .send(request, session, publication_input())
        .await;
    assert!(
        matches!(result, Err(KernelError::CommitUnknown { request_id, .. })
        if request_id == request)
    );
    assert!(matches!(
        ports.reader.history(session, None, 16).await,
        Err(KernelError::Closed)
    ));
    assert!(matches!(
        ports
            .sessions
            .send(RequestId::new(), session, document("must stay closed"))
            .await,
        Err(KernelError::Closed)
    ));
    tokio::time::timeout(Duration::from_secs(5), closing)
        .await
        .unwrap()
        .unwrap();
    drop((ports, admin));
    let kernel = open_kernel_runtime(&url).await.unwrap();
    let before = receipt(&kernel.database, request)
        .await
        .expect("real COMMIT stored its receipt");
    let stored: CommitReceipt<Mutation> = serde_json::from_str(&before).unwrap();
    let replay = kernel
        .ports
        .sessions
        .send(request, session, publication_input())
        .await
        .unwrap();
    assert_eq!(replay.position, stored.position);
    let Mutation::Execution(execution) = stored.value else {
        panic!("send must return execution")
    };
    assert_eq!(replay.value, execution);
    let again = kernel
        .ports
        .sessions
        .send(request, session, publication_input())
        .await
        .unwrap();
    assert_eq!(again, replay);
    assert_eq!(
        receipt(&kernel.database, request).await.as_deref(),
        Some(before.as_str())
    );
    let history = kernel
        .ports
        .reader
        .history(session, None, 16)
        .await
        .unwrap();
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].id, replay.value.message_id);
    let executions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agena_executions WHERE session_id=?")
            .bind(session.get())
            .fetch_one(kernel.database.get_sqlite_connection_pool())
            .await
            .unwrap();
    assert_eq!(executions, 1);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(kernel.database.get_sqlite_connection_pool())
            .await
            .unwrap()
            .is_empty()
    );
    kernel.ports.shutdown().await.unwrap();
    drop(root);
}

async fn recover(root: &Path, kind: &str, stage: &str, target: RequestId) {
    let evidence: Evidence =
        serde_json::from_slice(&std::fs::read(root.join("evidence.json")).unwrap()).unwrap();
    let url = format!("sqlite://{}?mode=rw", root.join("kernel.db").display());
    let kernel = open_kernel_runtime(&url).await.unwrap();
    let ports = &kernel.ports;
    let database = &kernel.database;
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(database.get_sqlite_connection_pool())
            .await
            .unwrap()
            .is_empty(),
        "{kind} {stage}: recovered foreign keys"
    );
    if matches!(kind, "prepare" | "chunk" | "ready" | "publish") {
        let before = receipt(database, target).await;
        assert_eq!(before.is_some(), stage != "command.before_commit");
        let existing_blob: Option<String> =
            sqlx::query_scalar("SELECT id FROM agena_blobs WHERE session_id=?")
                .bind(evidence.session.get())
                .fetch_optional(database.get_sqlite_connection_pool())
                .await
                .unwrap();
        let chunks: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_blob_chunks")
            .fetch_one(database.get_sqlite_connection_pool())
            .await
            .unwrap();
        match kind {
            "prepare" => {
                assert_eq!(existing_blob.is_some(), before.is_some());
                assert_eq!(chunks, 0);
            }
            "chunk" => assert_eq!(chunks, i64::from(before.is_some())),
            "ready" | "publish" => assert_eq!(chunks, 3),
            _ => unreachable!(),
        }
        let bytes = body(2);
        let prepared = ports
            .sessions
            .prepare_blob(
                evidence.preparation,
                evidence.session,
                bytes.len() as u64,
                &mut bytes.as_slice(),
            )
            .await
            .unwrap();
        if let Some(existing) = existing_blob {
            assert_eq!(prepared.descriptor().id.to_string(), existing);
        }
        if let Some(blob) = &evidence.blob {
            assert_eq!(prepared.descriptor(), blob);
        }
        let sent = ports
            .sessions
            .send_with_blobs(
                evidence.publication,
                evidence.session,
                publication_input(),
                std::slice::from_ref(&prepared),
            )
            .await
            .unwrap();
        assert_eq!(
            ports
                .sessions
                .send_with_blobs(
                    evidence.publication,
                    evidence.session,
                    publication_input(),
                    std::slice::from_ref(&prepared),
                )
                .await
                .unwrap(),
            sent
        );
        if let Some(before) = before {
            assert_eq!(
                receipt(database, target).await.as_deref(),
                Some(before.as_str())
            );
        }
        let history = ports
            .reader
            .history(evidence.session, None, 16)
            .await
            .unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].id, sent.value.message_id);
        assert_eq!(
            ports
                .reader
                .entry_blobs(evidence.session, sent.value.message_id)
                .await
                .unwrap()
                .as_slice(),
            std::slice::from_ref(prepared.descriptor())
        );
        let mut copied = Vec::new();
        ports
            .reader
            .open_blob(evidence.session, prepared.descriptor().id)
            .await
            .unwrap()
            .copy_to(&mut copied)
            .await
            .unwrap();
        assert_eq!(copied, bytes);
        if let Some(old) = evidence.watch {
            let mut resumed = ports
                .reader
                .watch(evidence.session, Some(old.clone()))
                .unwrap();
            assert!(resumed.needs_snapshot());
            ports
                .sessions
                .update_metadata(
                    RequestId::new(),
                    evidence.session,
                    MetadataChange {
                        title: Some("recovered subscription".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            let change = tokio::time::timeout(Duration::from_secs(3), resumed.recv())
                .await
                .unwrap()
                .unwrap();
            assert_ne!(change.cursor.epoch, old.epoch);
        }
    } else if matches!(kind, "open" | "checkpoint") {
        let content = match evidence.content {
            Some(content) => content,
            None => sqlx::query_scalar::<_, String>(
                "SELECT id FROM agena_content_streams WHERE session_id=?",
            )
            .bind(evidence.session.get())
            .fetch_one(database.get_sqlite_connection_pool())
            .await
            .unwrap()
            .parse()
            .unwrap(),
        };
        let range = ports
            .reader
            .text_range(evidence.session, content, None, 256 * 1024)
            .await
            .unwrap();
        let expected = if kind == "checkpoint" && stage != "checkpoint.before_commit" {
            text()
        } else {
            String::new()
        };
        assert_eq!(range.bytes, expected.as_bytes());
        assert_eq!(range.stream.accepted, expected.len() as u64);
        assert_eq!(range.stream.committed, expected.len() as u64);
        assert_eq!(range.stream.state, CaptureState::Interrupted);
        if kind == "open" {
            assert!(receipt(database, target).await.is_some());
        }
    } else if kind == "gc" {
        let orphan = evidence.blob.unwrap();
        let chunks: i64 =
            sqlx::query_scalar("SELECT count(*) FROM agena_blob_chunks WHERE blob_id=?")
                .bind(orphan.id.to_string())
                .fetch_one(database.get_sqlite_connection_pool())
                .await
                .unwrap();
        assert_eq!(chunks, if stage == "gc.before_commit" { 11 } else { 3 });
        // Infrastructure-only clock setup avoids sleeping for the orphan grace
        // period. The collection itself still uses the public maintenance port.
        sqlx::query("UPDATE agena_blobs SET created_at_ms=0 WHERE id=?")
            .bind(orphan.id.to_string())
            .execute(database.get_sqlite_connection_pool())
            .await
            .unwrap();
        for _ in 0..4 {
            let collected = kernel.maintenance.collect_blobs().await.unwrap();
            assert!(collected.chunks <= 8 && collected.retired <= 16);
        }
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_blobs WHERE id=?")
            .bind(orphan.id.to_string())
            .fetch_one(database.get_sqlite_connection_pool())
            .await
            .unwrap();
        assert_eq!(remaining, 0);
        let (message, protected) = evidence.protected.unwrap();
        assert_eq!(
            ports
                .reader
                .entry_blobs(evidence.session, message)
                .await
                .unwrap()
                .as_slice(),
            std::slice::from_ref(&protected)
        );
        let mut copied = Vec::new();
        ports
            .reader
            .open_blob(evidence.session, protected.id)
            .await
            .unwrap()
            .copy_to(&mut copied)
            .await
            .unwrap();
        assert_eq!(copied, body(2));
    }
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(database.get_sqlite_connection_pool())
            .await
            .unwrap()
            .is_empty()
    );
    ports.clone().shutdown().await.unwrap();
}

#[test]
fn killed_kernel_reopens_each_preparation_publication_checkpoint_and_gc_boundary() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    if let Ok(root) = std::env::var(CHILD_ROOT) {
        let kind = std::env::var(CHILD_KIND).unwrap();
        runtime.block_on(child(Path::new(&root), &kind));
        return;
    }
    let cases = [
        ("prepare", "command.before_commit"),
        ("prepare", "command.after_commit"),
        ("chunk", "command.before_commit"),
        ("chunk", "command.after_commit"),
        ("ready", "command.before_commit"),
        ("ready", "command.after_commit"),
        ("publish", "command.before_commit"),
        ("publish", "command.after_commit"),
        ("publish", "command.before_install"),
        ("publish", "command.after_install"),
        ("publish", "command.before_notify"),
        ("publish", "command.after_notify"),
        ("publish", "command.before_receipt"),
        ("publish", "command.after_receipt"),
        ("open", "command.before_install"),
        ("open", "command.after_install"),
        ("checkpoint", "checkpoint.before_commit"),
        ("checkpoint", "checkpoint.after_commit"),
        ("checkpoint", "checkpoint.after_install"),
        ("gc", "gc.before_commit"),
        ("gc", "gc.after_commit"),
        ("gc", "gc.after_install"),
    ];
    for (kind, stage) in cases {
        let root = tempfile::tempdir().unwrap();
        let preparation = RequestId::new();
        let publication = RequestId::new();
        let target = match kind {
            "prepare" => preparation,
            "chunk" => derived(preparation, b"chunk", 0),
            "ready" => derived(preparation, b"ready", 0),
            "publish" => publication,
            _ => RequestId::new(),
        };
        let mut child = ChildGuard(
            ProcessCommand::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("kernel::crash_window_tests::killed_kernel_reopens_each_preparation_publication_checkpoint_and_gc_boundary")
                .arg("--nocapture")
                .env(CHILD_ROOT, root.path())
                .env(CHILD_KIND, kind)
                .env(CHILD_STAGE, stage)
                .env(CHILD_REQUEST, target.to_string())
                .env(CHILD_PREPARATION, preparation.to_string())
                .env(CHILD_PUBLICATION, publication.to_string())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if line.contains(MARKER) {
                    let _ = sender.send(line);
                    break;
                }
            }
        });
        let marker = receiver
            .recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|error| {
                panic!("{kind} {stage}: child did not reach boundary: {error}")
            });
        assert!(marker.contains(&format!("{stage} {target}")));
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        reader.join().unwrap();
        runtime.block_on(recover(root.path(), kind, stage, target));
        println!("kernel crash window verified kind={kind} stage={stage}");
    }
}
