use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_sqlite::open_kernel_runtime;

#[tokio::test]
async fn a_queued_supervisor_cannot_claim_a_later_retry_generation() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/claim-generation")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Attempt ownership".into())
        .await
        .unwrap()
        .value
        .id;
    let original = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Queued input".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    runtime
        .ports
        .sessions
        .cancel_queued(RequestId::new(), session, original.id)
        .await
        .unwrap();
    let next = runtime
        .ports
        .sessions
        .continue_execution(RequestId::new(), session, original.message_id)
        .await
        .unwrap()
        .value;
    assert_eq!(next.id, original.id);
    assert_eq!(next.generation, original.generation.next().unwrap());
    let control_before = *runtime.ports.reader.control_version(session).await.unwrap();
    let stale = runtime
        .ports
        .executor
        .claim_next_generation(RequestId::new(), session, original.id, original.generation)
        .await;
    assert!(
        matches!(stale, Err(KernelError::StaleAttempt)),
        "the old supervisor must not acquire generation {}",
        next.generation.get()
    );
    let queued = runtime
        .ports
        .reader
        .execution(session, next.id)
        .await
        .unwrap();
    assert_eq!(queued.phase, ExecutionPhase::Queued);
    assert_eq!(queued.generation, next.generation);
    assert_eq!(
        *runtime.ports.reader.control_version(session).await.unwrap(),
        control_before
    );
    drop(queued);
    let owner = runtime
        .ports
        .executor
        .claim_next_generation(RequestId::new(), session, next.id, next.generation)
        .await
        .unwrap()
        .value;
    assert_eq!(owner.generation(), next.generation);
    owner
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn a_cancelled_queue_head_keeps_its_notice_and_stays_out_of_the_next_prompt() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime.workspaces.ensure_id("/cancel-next").await.unwrap();
    let ports = &runtime.ports;
    let session = ports
        .sessions
        .create(RequestId::new(), workspace, "Stop before claim".into())
        .await
        .unwrap()
        .value
        .id;
    let mut queued = Vec::new();
    for text in ["interrupted input", "next input"] {
        let document = ComposerDocument(vec![ComposerNode::Text { text: text.into() }]);
        queued.push(
            ports
                .sessions
                .send(RequestId::new(), session, document)
                .await
                .unwrap()
                .value,
        );
    }
    let [head, next] = queued.as_slice() else {
        unreachable!();
    };
    let cancel = |request, id, generation| {
        ports.executor.cancel_next_generation(
            request,
            session,
            id,
            generation,
            "SessionStart stop".into(),
        )
    };
    assert!(matches!(
        cancel(RequestId::new(), next.id, next.generation).await,
        Err(KernelError::QueueOutOfOrder)
    ));
    ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                message_queue_paused: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        cancel(RequestId::new(), head.id, head.generation).await,
        Err(KernelError::QueuePaused)
    ));
    ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                message_queue_paused: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        cancel(RequestId::new(), head.id, head.generation.next().unwrap()).await,
        Err(KernelError::StaleAttempt)
    ));

    let request = RequestId::new();
    let stopped = cancel(request, head.id, head.generation).await.unwrap();
    assert_eq!(stopped.value.phase, ExecutionPhase::Cancelled);
    assert_eq!(
        cancel(request, head.id, head.generation)
            .await
            .unwrap()
            .position,
        stopped.position
    );
    let head_entry = ports.reader.entry(session, head.message_id).await.unwrap();
    assert!(!head_entry.execution_started);
    let history = ports.reader.history(session, None, 16).await.unwrap();
    assert_eq!(
        history.entries.iter().filter(|entry| {
            entry.message_id == head.message_id
                && matches!(&entry.payload, EntryPayload::Notice { text } if text == "SessionStart stop")
        }).count(),
        1,
    );

    let attempt = ports
        .executor
        .claim_next_generation(RequestId::new(), session, next.id, next.generation)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let prompt = ports.reader.prompt(session, round.id()).await.unwrap();
    assert!(
        prompt
            .entries
            .iter()
            .all(|item| item.entry.id != head.message_id)
    );
    assert!(
        prompt
            .entries
            .iter()
            .any(|item| item.entry.id == next.message_id)
    );
    drop(prompt);
    drop(history);
    drop(head_entry);
    runtime.ports.clone().shutdown().await.unwrap();
}

#[test]
fn legacy_claim_next_wire_shape_has_no_generation_field() {
    use agena_storage::backend::kernel::Command;
    let session = SessionId::try_from(1).unwrap();
    let execution = agena_domain::ExecutionId::new();
    let epoch = agena_domain::KernelEpoch::new();
    let legacy = serde_json::json!({"ClaimNext": {
        "session": session,
        "execution": execution,
        "epoch": epoch,
    }});
    let command: Command = serde_json::from_value(legacy.clone()).unwrap();
    assert!(matches!(
        &command,
        Command::ClaimNext {
            expected_generation: None,
            ..
        }
    ));
    assert_eq!(serde_json::to_value(command).unwrap(), legacy);
    let guarded = Command::ClaimNext {
        session,
        execution,
        epoch,
        expected_generation: Some(AttemptGeneration::INITIAL),
    };
    assert_eq!(
        serde_json::to_value(guarded).unwrap()["ClaimNext"]["expected_generation"],
        1
    );
}

#[tokio::test]
async fn stale_attempt_cancellation_cannot_fence_a_retry_or_its_live_content() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/cancel-generation")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Cancellation ownership".into())
        .await
        .unwrap()
        .value
        .id;
    let original = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "retry me".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    runtime
        .ports
        .sessions
        .cancel_attempt(RequestId::new(), session, original.id, original.generation)
        .await
        .unwrap();
    let cancelled = runtime
        .ports
        .reader
        .execution_attempt(session, original.id, original.generation)
        .await
        .unwrap();
    // A cancelled unclaimed input can own callbacks without a provider start.
    // Both raw queue cancellation and guarded cancellation are exercised here.
    assert_eq!(cancelled.owner_epoch, Some(runtime.ports.reader.epoch()));
    drop(cancelled);
    let next = runtime
        .ports
        .sessions
        .continue_execution(RequestId::new(), session, original.message_id)
        .await
        .unwrap()
        .value;
    let owner = runtime
        .ports
        .executor
        .claim_next_generation(RequestId::new(), session, next.id, next.generation)
        .await
        .unwrap()
        .value;
    let round = owner
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    output.append("before stale cancellation").await.unwrap();

    let stale = runtime
        .ports
        .sessions
        .cancel_attempt(RequestId::new(), session, original.id, original.generation)
        .await;
    assert!(matches!(stale, Err(KernelError::StaleAttempt)));
    // This exercises live-content preflight as well as the transactional
    // generation check. A stale cancel must not install a terminal gate.
    output.append("after stale cancellation").await.unwrap();
    let running = runtime
        .ports
        .reader
        .execution_attempt(session, next.id, next.generation)
        .await
        .unwrap();
    assert_eq!(running.phase, ExecutionPhase::Running);
    assert_eq!(running.current_generation, next.generation);
    drop(running);

    let cancelled = runtime
        .ports
        .sessions
        .cancel_attempt(RequestId::new(), session, next.id, next.generation)
        .await
        .unwrap()
        .value;
    assert_eq!(cancelled.phase, ExecutionPhase::Cancelled);
    assert!(output.append("too late").await.is_err());
    runtime.ports.clone().shutdown().await.unwrap();
}

#[test]
fn legacy_cancel_wire_shape_keeps_its_idempotency_digest() {
    use agena_storage::backend::kernel::Command;
    let session = SessionId::try_from(1).unwrap();
    let execution = agena_domain::ExecutionId::new();
    let legacy = serde_json::json!({"Cancel": {
        "session": session, "execution": execution,
    }});
    let decoded: Command = serde_json::from_value(legacy.clone()).unwrap();
    assert!(matches!(
        &decoded,
        Command::Cancel {
            expected_generation: None,
            ..
        }
    ));
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    let guarded = Command::Cancel {
        session,
        execution,
        expected_generation: Some(AttemptGeneration::INITIAL),
        epoch: Some(agena_domain::KernelEpoch::new()),
    };
    assert_eq!(
        serde_json::to_value(guarded).unwrap()["Cancel"]["expected_generation"],
        1,
    );
}
