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
