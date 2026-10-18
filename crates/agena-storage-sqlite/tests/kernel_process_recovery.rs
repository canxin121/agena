//! Kill/reopen through the new kernel's public capabilities and repository
//! composition. The child only operates on the parent's owned temporary path.

use std::{
    io::{BufRead, Write},
    process::{Child, Command, Stdio},
    time::Duration,
};

use agena_domain::{ActivityId, ComposerDocument, ComposerNode, ContentId, ExecutionId};
use agena_storage::backend::kernel::KernelPorts;
use agena_storage::kernel::*;
use agena_storage_sqlite::open_kernel_runtime;

#[derive(serde::Serialize, serde::Deserialize)]
struct Evidence {
    session: SessionId,
    sent: CommitReceipt<ExecutionView>,
    request: RequestId,
    tool: ToolId,
    draft: ToolId,
    interaction: InteractionId,
    stream: ContentStreamView,
    accepted: TextCursor,
    blob: BlobDescriptor,
    attachment_message: EntryId,
    preparation_request: RequestId,
    unpublished_blob: BlobDescriptor,
    compaction: CompactionEvidence,
    activity: ActivityEvidence,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ActivityEvidence {
    session: SessionId,
    execution: ExecutionId,
    activity: ActivityId,
    tool: ToolId,
    stream: ContentStreamView,
}

async fn background_fixture(
    ports: &KernelPorts,
    workspace: i64,
) -> (ActivityEvidence, ActivityHandle, TextOutput) {
    let session = ports
        .sessions
        .create(
            RequestId::new(),
            workspace,
            "independent background recovery".into(),
        )
        .await
        .unwrap()
        .value
        .id;
    let sent = ports
        .sessions
        .send(RequestId::new(), session, input())
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
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "background-launch".into(),
                name: "scripted".into(),
                input: serde_json::json!({"no_external_effect":true}),
            },
        )
        .await
        .unwrap()
        .value;
    let activity = tool
        .begin_activity(
            RequestId::new(),
            BackgroundActivityStart {
                kind: BackgroundActivityKind::Process,
                label: "scripted activity remains after parent".into(),
            },
        )
        .await
        .unwrap()
        .value;
    let output = activity.begin_text(RequestId::new()).await.unwrap().value;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"launched":activity.id()}),
        },
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let mut watch = ports
        .reader
        .watch_content(session, output.id())
        .await
        .unwrap();
    let prefix = committed_text();
    output.append(&prefix).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if watch.changed().await.unwrap().committed == prefix.len() as u64 {
                break;
            }
        }
    })
    .await
    .unwrap();
    let stream = (*watch.head().unwrap()).clone();
    assert_eq!(stream.state, CaptureState::Open);
    (
        ActivityEvidence {
            session,
            execution: sent.id,
            activity: activity.id(),
            tool: tool.id(),
            stream,
        },
        activity,
        output,
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CompactionEvidence {
    session: SessionId,
    execution: ExecutionId,
    checkpoint: EntryId,
    summary: ContentStreamView,
    suffix: Vec<EntryId>,
    unfinished_round: RoundId,
    unfinished_summary: ContentId,
}

async fn compaction_fixture(ports: &KernelPorts, workspace: i64) -> CompactionEvidence {
    let session = ports
        .sessions
        .create(RequestId::new(), workspace, "compaction recovery".into())
        .await
        .unwrap()
        .value
        .id;
    let mut users = Vec::new();
    let mut execution = None;
    for _ in 0..3 {
        let sent = ports
            .sessions
            .send(RequestId::new(), session, input())
            .await
            .unwrap()
            .value;
        users.push(sent.message_id);
        execution = Some(sent.id);
    }
    let execution = execution.unwrap();
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, execution)
        .await
        .unwrap()
        .value;
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
    output.append("durable compaction 世界\n\0").await.unwrap();
    let checkpoint = compact
        .commit_compaction(
            RequestId::new(),
            output.id(),
            users[1],
            Some(serde_json::json!({"opaque_compaction":[null,true,"世界"]})),
        )
        .await
        .unwrap()
        .value;
    let summary = (*ports
        .reader
        .watch_content(session, output.id())
        .await
        .unwrap()
        .head()
        .unwrap())
    .clone();
    let unfinished = attempt
        .begin_compaction(RequestId::new(), 2)
        .await
        .unwrap()
        .value;
    let pending = unfinished
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    pending
        .append("unfinished replacement must not become a checkpoint")
        .await
        .unwrap();
    let late = ports
        .sessions
        .send(RequestId::new(), session, input())
        .await
        .unwrap()
        .value;
    CompactionEvidence {
        session,
        execution,
        checkpoint,
        summary,
        suffix: vec![users[2], late.message_id],
        unfinished_round: unfinished.id(),
        unfinished_summary: pending.id(),
    }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn input() -> ComposerDocument {
    ComposerDocument(vec![ComposerNode::Text {
        text: "kill/reopen fixture 世界\n\0".into(),
    }])
}

fn committed_text() -> String {
    "checkpoint 世界🙂\n\0".repeat(4096)
}

fn managed_bytes() -> Vec<u8> {
    (0..128 * 1024 + 31)
        .map(|n| ((n * 73 + n / 89) % 256) as u8)
        .collect()
}

#[test]
fn killed_kernel_recovers_committed_facts_and_interrupts_pending_authorization() {
    const CHILD_URL: &str = "AGENA_KERNEL_RECOVERY_TEST_URL";
    const CHILD_ROOT: &str = "AGENA_KERNEL_RECOVERY_TEST_ROOT";
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    if let Ok(url) = std::env::var(CHILD_URL) {
        runtime.block_on(async {
            let kernel = open_kernel_runtime(&url).await.unwrap();
            let workspace = kernel
                .workspaces
                .create(std::env::var(CHILD_ROOT).unwrap())
                .await
                .unwrap();
            let ports = &kernel.ports;
            let compaction = compaction_fixture(ports, workspace.id).await;
            let (activity, _activity_handle, _activity_output) =
                background_fixture(ports, workspace.id).await;
            let session = ports
                .sessions
                .create(RequestId::new(), workspace.id, "process recovery".into())
                .await
                .unwrap()
                .value
                .id;
            let request = RequestId::new();
            let sent = ports
                .sessions
                .send(request, session, input())
                .await
                .unwrap();
            let attempt = ports
                .executor
                .claim(RequestId::new(), session, sent.value.id)
                .await
                .unwrap()
                .value;
            let round = attempt
                .begin_round(RequestId::new(), vec![sent.value.message_id])
                .await
                .unwrap()
                .value;
            let tool = round
                .begin_tool(
                    RequestId::new(),
                    ToolStart {
                        call_id: "started-before-kill".into(),
                        name: "fixture".into(),
                        input: serde_json::json!({"synthetic":true}),
                    },
                )
                .await
                .unwrap()
                .value;
            let interaction = tool
                .request_interaction(
                    RequestId::new(),
                    InteractionRequest::Permission {
                        action_key: "fixture".into(),
                        description: "unanswered authorization".into(),
                    },
                )
                .await
                .unwrap()
                .value;
            let draft = round
                .begin_tool_draft(
                    RequestId::new(),
                    "unfinished-arguments".into(),
                    "tools_call".into(),
                )
                .await
                .unwrap()
                .value;
            let bytes = managed_bytes();
            let attachment = ports
                .sessions
                .prepare_blob(
                    RequestId::new(),
                    session,
                    bytes.len() as u64,
                    &mut bytes.as_slice(),
                )
                .await
                .unwrap();
            let queued = ports
                .sessions
                .send_with_blobs(
                    RequestId::new(),
                    session,
                    input(),
                    std::slice::from_ref(&attachment),
                )
                .await
                .unwrap();
            let preparation_request = RequestId::new();
            let unpublished = ports
                .sessions
                .prepare_blob(
                    preparation_request,
                    session,
                    bytes.len() as u64,
                    &mut bytes.as_slice(),
                )
                .await
                .unwrap();
            let output = tool.begin_text(RequestId::new()).await.unwrap().value;
            let mut watch = ports
                .reader
                .watch_content(session, output.id())
                .await
                .unwrap();
            let prefix = committed_text();
            output.append(&prefix).await.unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if watch.changed().await.unwrap().committed == prefix.len() as u64 {
                        break;
                    }
                }
            })
            .await
            .unwrap();
            let accepted = output
                .append("accepted tail which must reset after kill")
                .await
                .unwrap();
            let stream = watch.head().unwrap();
            assert!(stream.committed < accepted.offset);
            let evidence = Evidence {
                session,
                sent,
                request,
                tool: tool.id(),
                draft: draft.id(),
                interaction,
                stream: (*stream).clone(),
                accepted,
                blob: attachment.descriptor().clone(),
                attachment_message: queued.value.message_id,
                preparation_request,
                unpublished_blob: unpublished.descriptor().clone(),
                compaction,
                activity,
            };
            println!(
                "KERNEL_RECOVERY_READY {}",
                serde_json::to_string(&evidence).unwrap()
            );
            std::io::stdout().flush().unwrap();
            loop {
                std::thread::park();
            }
        });
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        root.path().join("kernel.db").display()
    );
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("killed_kernel_recovers_committed_facts_and_interrupts_pending_authorization")
            .arg("--nocapture")
            .env(CHILD_URL, &url)
            .env(CHILD_ROOT, root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line.unwrap();
            if let Some((_, json)) = line.split_once("KERNEL_RECOVERY_READY ") {
                let _ = sender.send(serde_json::from_str::<Evidence>(json).unwrap());
                break;
            }
        }
    });
    let evidence = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("child's committed boundary");
    reader.join().unwrap();
    runtime.block_on(async {
        assert!(open_kernel_runtime(&url).await.is_err());
    });
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    runtime.block_on(async {
        let kernel = open_kernel_runtime(&url).await.unwrap();
        let ports = kernel.ports;
        let activity = &evidence.activity;
        assert_eq!(
            ports
                .reader
                .execution(activity.session, activity.execution)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Succeeded
        );
        assert_eq!(
            ports
                .reader
                .tool(activity.session, activity.tool)
                .await
                .unwrap()
                .phase,
            ToolPhase::Succeeded
        );
        let recovered = ports
            .reader
            .activity(activity.session, activity.activity)
            .await
            .unwrap();
        assert_eq!(
            recovered.outcome,
            Some(BackgroundActivityOutcome::Interrupted {
                diagnostic: "service restarted; external activity outcome is unknown".into()
            })
        );
        assert!(
            ports
                .reader
                .active_activities(activity.session, None, 10)
                .await
                .unwrap()
                .activities
                .is_empty()
        );
        let head = ports
            .reader
            .watch_content(activity.session, activity.stream.id)
            .await
            .unwrap()
            .head()
            .unwrap();
        assert_eq!(head.state, CaptureState::Interrupted);
        assert_eq!(head.activity_id, Some(activity.activity));
        assert_eq!(head.committed, activity.stream.committed);
        assert_ne!(head.epoch, activity.stream.epoch);
        let recovered_text = ports
            .reader
            .text_range(activity.session, activity.stream.id, None, 256 * 1024)
            .await
            .unwrap();
        assert_eq!(recovered_text.bytes, committed_text().as_bytes());
        assert!(matches!(
            ports
                .reader
                .text_range(
                    activity.session,
                    activity.stream.id,
                    Some(activity.stream.text_cursor().unwrap()),
                    100
                )
                .await,
            Err(KernelError::ContentReset)
        ));
        let compact = &evidence.compaction;
        assert_eq!(
            ports
                .reader
                .round(compact.session, compact.unfinished_round)
                .await
                .unwrap()
                .phase,
            ProviderRoundPhase::Interrupted
        );
        assert_eq!(
            ports
                .reader
                .watch_content(compact.session, compact.unfinished_summary)
                .await
                .unwrap()
                .head()
                .unwrap()
                .state,
            CaptureState::Interrupted
        );
        ports
            .sessions
            .retry(RequestId::new(), compact.session, compact.execution)
            .await
            .unwrap();
        let attempt = ports
            .executor
            .claim(RequestId::new(), compact.session, compact.execution)
            .await
            .unwrap()
            .value;
        let next = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let prompt = next.prompt().await.unwrap();
        let mut expected = vec![compact.checkpoint];
        expected.extend_from_slice(&compact.suffix);
        assert_eq!(
            prompt
                .entries
                .iter()
                .map(|entry| entry.entry.id)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            prompt.sources.as_slice(),
            std::slice::from_ref(&compact.summary)
        );
        assert_eq!(prompt.replays.len(), 1);
        assert_eq!(
            prompt.replays[0].provider_state,
            Some(serde_json::json!({"opaque_compaction":[null,true,"世界"]}))
        );
        let text = ports
            .reader
            .text_range_at(compact.session, &compact.summary, None, 1000)
            .await
            .unwrap();
        assert_eq!(text.bytes, "durable compaction 世界\n\0".as_bytes());
        assert_eq!(
            ports
                .reader
                .entry_blobs(evidence.session, evidence.attachment_message)
                .await
                .unwrap()
                .as_slice(),
            vec![evidence.blob.clone()]
        );
        let blob = ports
            .reader
            .open_blob(evidence.session, evidence.blob.id)
            .await
            .unwrap();
        let mut copied = Vec::new();
        blob.copy_to(&mut copied).await.unwrap();
        assert_eq!(copied, managed_bytes());
        let bytes = managed_bytes();
        let recovered = ports
            .sessions
            .prepare_blob(
                evidence.preparation_request,
                evidence.session,
                bytes.len() as u64,
                &mut bytes.as_slice(),
            )
            .await
            .unwrap();
        assert_eq!(recovered.descriptor(), &evidence.unpublished_blob);
        let execution = ports
            .reader
            .execution(evidence.session, evidence.sent.value.id)
            .await
            .unwrap();
        assert_eq!(execution.phase, ExecutionPhase::Interrupted);
        assert_eq!(execution.generation, evidence.sent.value.generation);
        let range = ports
            .reader
            .text_range(evidence.session, evidence.stream.id, None, 256 * 1024)
            .await
            .unwrap();
        assert_eq!(range.bytes, committed_text().as_bytes());
        assert_eq!(range.stream.committed, evidence.stream.committed);
        assert_eq!(range.stream.accepted, range.stream.committed);
        assert_eq!(range.stream.state, CaptureState::Interrupted);
        assert_ne!(range.stream.epoch, evidence.stream.epoch);
        let mut frozen = evidence.stream.clone();
        frozen.accepted = frozen.committed;
        assert!(matches!(
            ports
                .reader
                .text_range_at(evidence.session, &frozen, None, 100)
                .await,
            Err(KernelError::ContentReset)
        ));
        assert!(matches!(
            ports
                .reader
                .text_range(
                    evidence.session,
                    evidence.stream.id,
                    Some(evidence.accepted),
                    100
                )
                .await,
            Err(KernelError::ContentReset)
        ));
        assert_eq!(
            ports
                .reader
                .tool(evidence.session, evidence.tool)
                .await
                .unwrap()
                .phase,
            ToolPhase::Interrupted
        );
        assert_eq!(
            ports
                .reader
                .tool(evidence.session, evidence.draft)
                .await
                .unwrap()
                .phase,
            ToolPhase::Interrupted
        );
        assert_eq!(
            ports
                .reader
                .interaction(evidence.session, evidence.interaction)
                .await
                .unwrap()
                .decision,
            Some(InteractionDecision::Cancel)
        );
        assert_eq!(
            ports
                .sessions
                .send(evidence.request, evidence.session, input())
                .await
                .unwrap(),
            evidence.sent
        );
        assert_eq!(
            ports
                .reader
                .history(evidence.session, None, 100)
                .await
                .unwrap()
                .entries
                .len(),
            5
        );
        assert!(
            ports
                .sessions
                .reply(
                    RequestId::new(),
                    evidence.session,
                    evidence.interaction,
                    InteractionDecision::Permission {
                        reply: agena_domain::PermissionReply {
                            request_id: evidence.interaction.get().to_string(),
                            kind: agena_domain::PermissionReplyKind::AllowOnce,
                            reason: None,
                            scope: None
                        },
                        allowed: true
                    },
                    None
                )
                .await
                .is_err()
        );
        ports.shutdown().await.unwrap();
    });
}
