use super::*;
use agena_runtime::SessionExecutionCommandService;
use std::time::Duration;

fn run_options(provider: &str, model: &str) -> agena_runtime::SessionRunOptions {
    agena_runtime::SessionRunOptions {
        model: ModelRef::new(provider, model),
        model_supports_parallel_tool_calls: None,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: Some(0.0),
        max_output_tokens: Some(64),
    }
}

#[tokio::test]
async fn user_send_returns_committed_parts_without_waiting_for_the_provider() {
    let manager = manager_with_provider(Arc::new(HangingProvider {
        model: ModelId::new("hanging-model"),
    }))
    .await;
    let session = create(&manager, "immediate committed input").await;
    let (published_tx, mut published_rx) = tokio::sync::mpsc::unbounded_channel();
    let _subscription = manager.session_store().subscribe(
        session.id,
        Arc::new(move |change| {
            if let SessionChange::PartAdded { part, .. } = change
                && part.role == PartRole::User
            {
                let _ = published_tx.send(part.part_id);
            }
        }),
    );
    let document = ComposerDocument(vec![
        ComposerNode::Text {
            text: "first input part".to_owned(),
        },
        ComposerNode::Text {
            text: "second input part".to_owned(),
        },
    ]);
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        manager.submit_user_run(agena_runtime::SessionUserRunRequest::new(
            session.id,
            run_options("hanging", "hanging-model"),
            document.clone(),
        )),
    )
    .await
    .expect("send acknowledgement must not wait for a provider response")
    .expect("accept user message");
    let receipt = outcome.receipt.expect("accepted execution identity");

    // Read immediately, with no polling or yield to let the background task
    // catch up. Every submitted part must already be durable at this boundary.
    let persisted = manager.store.load_session(session.id).await.unwrap();
    let user_parts = persisted
        .parts()
        .iter()
        .filter(|part| part.role == PartRole::User)
        .collect::<Vec<_>>();
    assert_eq!(user_parts.len(), 3, "one marker and both input parts");
    assert_eq!(user_parts[0].kind, "run");
    assert_eq!(
        user_parts[0].content["execution_id"],
        receipt.execution_id.to_string()
    );
    assert_eq!(user_parts[1].content["text"], "first input part");
    assert_eq!(user_parts[2].content["text"], "second input part");
    let published = std::iter::from_fn(|| published_rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        published,
        user_parts
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>(),
        "all user parts must reach live subscribers before acknowledgement"
    );
    assert!(
        user_parts
            .iter()
            .all(|part| part.state == PartState::Completed)
    );
    assert!(manager.execution_registry.is_active(session.id).await);
    assert!(
        !persisted
            .parts()
            .iter()
            .any(|part| { part.role == PartRole::Assistant && part.kind == "text" })
    );

    let cancellation = manager
        .cancel_active_execution_with_outcome(session.id)
        .await
        .expect("cancel unanswered execution");
    assert_eq!(cancellation.restored_user_message, Some(document));
    assert!(cancellation.restored_user_run_id.is_some());
}

#[tokio::test]
async fn provider_failure_does_not_reject_an_already_committed_send() {
    let manager = manager_with_provider(Arc::new(StartupFailureProvider {
        model: ModelId::new("failure-model"),
    }))
    .await;
    let session = create(&manager, "accepted input and failed provider").await;
    manager
        .submit_user_run(agena_runtime::SessionUserRunRequest::new(
            session.id,
            run_options("startup-failure", "failure-model"),
            ComposerDocument(vec![ComposerNode::Text {
                text: "keep this committed message".to_owned(),
            }]),
        ))
        .await
        .expect("provider failure must not make the client resubmit committed input");
    tokio::time::timeout(
        Duration::from_secs(5),
        manager.execution_registry.wait_until_released(session.id),
    )
    .await
    .expect("failed provider releases its execution");
    let persisted = manager.store.load_session(session.id).await.unwrap();
    assert_eq!(
        persisted
            .parts()
            .iter()
            .filter(|part| part.role == PartRole::User)
            .count(),
        2,
        "one committed user marker and one text part survive the provider failure"
    );
    assert!(persisted.parts().iter().any(|part| part.kind == "error"));
}

#[tokio::test]
async fn submission_failure_is_returned_and_releases_the_execution_slot() {
    let manager = test_manager().await;
    let session = create(&manager, "rejected submission").await;
    let (submitted_tx, submitted_rx) = tokio::sync::oneshot::channel();
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        manager.start_registered_with_admission(
            session.id,
            agena_domain::ExecutionSource::User,
            ExecutionConversationTarget::NewTurn,
            "rejected user input fixture",
            ExecutionAdmission {
                restore_document: Some(ComposerDocument(vec![ComposerNode::Text {
                    text: "restore this draft".to_owned(),
                }])),
                submission: Some(submitted_rx),
                ..Default::default()
            },
            move |_manager, _control, _steer_rx| async move {
                let _submission = submitted_tx;
                Err::<(), _>(crate::AppError::Config(
                    "input preparation failed".to_owned(),
                ))
            },
        ),
    )
    .await
    .expect("a failed preparation must release the acknowledgement waiter")
    .expect_err("a message that was never stored cannot be acknowledged");
    assert!(error.to_string().contains("input preparation failed"));
    assert!(!manager.execution_registry.is_active(session.id).await);
    assert!(
        manager
            .store
            .load_session(session.id)
            .await
            .unwrap()
            .parts()
            .is_empty()
    );
}
