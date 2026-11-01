use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use std::time::Duration;

async fn fixture() -> (
    SqliteKernel,
    SessionId,
    AttemptHandle,
    RoundHandle,
    TextOutput,
) {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/content-consistency")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Content publication".into())
        .await
        .unwrap()
        .value
        .id;
    let execution = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Capture output".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    (runtime, session, attempt, round, output)
}

#[tokio::test]
async fn abandoned_capture_publishes_its_durable_terminal_revision() {
    let (runtime, session, attempt, round, output) = fixture().await;
    output.append("accepted tail").await.unwrap();
    let content = output.id();
    let entry = output.entry_id();
    let before = runtime
        .ports
        .reader
        .control_snapshot(session, 16)
        .await
        .unwrap()
        .control_version;
    let revision = runtime
        .ports
        .reader
        .entry(session, entry)
        .await
        .unwrap()
        .revision;
    let mut stream = runtime
        .ports
        .reader
        .watch_content(session, content)
        .await
        .unwrap();
    assert!(!stream.is_snapshot());
    let mut changes = runtime.ports.reader.watch(session, None).unwrap();
    let previous = changes.head().unwrap().position;
    drop(output);

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if stream.head().unwrap().state == CaptureState::Interrupted {
                break;
            }
            stream.changed().await.unwrap();
        }
    })
    .await
    .expect("abandoned producer must become terminal");
    let after = runtime
        .ports
        .reader
        .control_snapshot(session, 16)
        .await
        .unwrap();
    let current = runtime.ports.reader.entry(session, entry).await.unwrap();
    assert_eq!(current.content_state, Some(CaptureState::Interrupted));
    assert!(current.revision > revision);
    assert_eq!(
        runtime
            .ports
            .reader
            .text_range(session, content, None, 1024)
            .await
            .unwrap()
            .bytes,
        b"accepted tail"
    );
    assert!(
        after.control_version > before,
        "terminal capture must advance the durable control version"
    );
    drop((after, current));
    let change = tokio::time::timeout(Duration::from_secs(3), changes.recv())
        .await
        .expect("terminal capture must publish a session change")
        .unwrap();
    assert_eq!(change.session, session);
    assert_eq!(change.kind, ChangeKind::Content);
    assert!(change.affected_entries.contains(&entry));
    assert!(!change.requires_snapshot);
    assert!(change.cursor.position > previous);
    let cursor = change.cursor.clone();
    drop(change);
    assert!(matches!(changes.try_recv(), Ok(None)));
    let mut resumed = runtime.ports.reader.watch(session, Some(cursor)).unwrap();
    assert!(!resumed.needs_snapshot());
    assert!(matches!(resumed.try_recv(), Ok(None)));
    assert!(!stream.is_snapshot());
    assert!(matches!(stream.changed().await, Err(KernelError::Closed)));
    round.finish(RequestId::new(), None, false).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Failed)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn ordinary_byte_checkpoint_does_not_publish_control_changes() {
    let (runtime, session, attempt, round, output) = fixture().await;
    let before = runtime
        .ports
        .reader
        .control_snapshot(session, 16)
        .await
        .unwrap()
        .control_version;
    let mut stream = runtime
        .ports
        .reader
        .watch_content(session, output.id())
        .await
        .unwrap();
    let mut changes = runtime.ports.reader.watch(session, None).unwrap();
    let payload = "x".repeat(64 * 1024);
    output.append(&payload).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let head = stream.head().unwrap();
            if head.committed == payload.len() as u64 {
                assert_eq!(head.state, CaptureState::Open);
                break;
            }
            drop(head);
            stream.changed().await.unwrap();
        }
    })
    .await
    .expect("the admitted block must checkpoint");
    assert_eq!(
        runtime
            .ports
            .reader
            .control_snapshot(session, 16)
            .await
            .unwrap()
            .control_version,
        before
    );
    assert!(matches!(changes.try_recv(), Ok(None)));
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}
