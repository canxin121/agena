use super::{
    tests::{document, fixture},
    *,
};
use serde_json::json;

async fn attempt(ports: &KernelPorts) -> (SessionId, ExecutionView, AttemptHandle) {
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "stop boundary".into())
        .await
        .unwrap()
        .value
        .id;
    let execution = ports
        .sessions
        .send(RequestId::new(), session, document("work"))
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    (session, execution, attempt)
}

async fn decision(attempt: &AttemptHandle) -> StopDecision {
    attempt
        .prepare_stop(RequestId::new(), StopConditions::default())
        .await
        .unwrap()
        .value
}

async fn observe(attempt: &AttemptHandle, succeeded: bool) {
    attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value
        .finish(RequestId::new(), None, succeeded)
        .await
        .unwrap();
}

#[tokio::test]
async fn stop_preparation_and_guidance_have_one_atomic_order() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        for index in 0..8 {
            let (session, execution, attempt) = attempt(&ports).await;
            let key = format!("stop-race-{index}");
            let payload = json!({"text":"guidance before stop","wake":false});
            let deliver = ports.executor.deliver_to_execution(
                RequestId::new(),
                session,
                execution.id,
                key,
                payload,
            );
            let prepare = attempt.prepare_stop(RequestId::new(), StopConditions::default());
            let (delivered, prepared) = if index % 2 == 0 {
                tokio::join!(deliver, prepare)
            } else {
                let (prepared, delivered) = tokio::join!(prepare, deliver);
                (delivered, prepared)
            };
            match (delivered, prepared.unwrap().value) {
                (Ok(delivery), StopDecision::Continue) => {
                    assert_eq!(delivery.value.target_execution, Some(execution.id));
                    ports
                        .executor
                        .consume_delivery(
                            RequestId::new(),
                            session,
                            delivery.value.id,
                            execution.message_id,
                            "guidance before stop".into(),
                        )
                        .await
                        .unwrap();
                    assert_eq!(decision(&attempt).await, StopDecision::Continue);
                    observe(&attempt, true).await;
                    assert_eq!(decision(&attempt).await, StopDecision::Ready);
                }
                (Err(KernelError::Conflict(_)), StopDecision::Ready) => {}
                other => panic!("guidance and stop did not have one order: {other:?}"),
            }
            attempt
                .finish(RequestId::new(), ExecutionPhase::Succeeded)
                .await
                .unwrap();
        }
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn ready_stop_fence_resets_for_a_hook_round_and_never_crosses_retry_generations() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, execution, attempt) = attempt(&ports).await;
        assert_eq!(decision(&attempt).await, StopDecision::Ready);
        let payload = json!({"text":"only an active model round accepts this","wake":false});
        assert!(matches!(
            ports
                .executor
                .deliver_to_execution(
                    RequestId::new(),
                    session,
                    execution.id,
                    "fenced-guidance".into(),
                    payload.clone()
                )
                .await,
            Err(KernelError::Conflict(_))
        ));
        // Generic events published after Ready belong to the next wake and
        // cannot resurrect a completed attempt merely by arriving late.
        ports
            .executor
            .deliver(
                RequestId::new(),
                session,
                "new-business-event".into(),
                json!({"text":"next wake"}),
            )
            .await
            .unwrap();
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let guidance = ports
            .executor
            .deliver_to_execution(
                RequestId::new(),
                session,
                execution.id,
                "round-guidance".into(),
                payload.clone(),
            )
            .await
            .unwrap()
            .value;
        round.finish(RequestId::new(), None, true).await.unwrap();
        assert_eq!(decision(&attempt).await, StopDecision::Continue);
        ports
            .sessions
            .cancel(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        ports
            .sessions
            .retry(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        let retried = ports
            .executor
            .claim(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value;
        assert!(matches!(
            attempt
                .prepare_stop(RequestId::new(), StopConditions::default())
                .await,
            Err(KernelError::StaleAttempt)
        ));
        ports
            .executor
            .deliver_to_execution(
                RequestId::new(),
                session,
                execution.id,
                "retry-guidance".into(),
                payload,
            )
            .await
            .unwrap();
        assert_ne!(retried.generation(), attempt.generation());
        let pending = ports
            .reader
            .pending_deliveries(session, None, 16)
            .await
            .unwrap();
        assert!(!pending.iter().any(|delivery| delivery.id == guidance.id));
        retried
            .finish(RequestId::new(), ExecutionPhase::Cancelled)
            .await
            .unwrap();
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn consumed_business_delivery_requires_successful_frozen_round_observation() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, execution, attempt) = attempt(&ports).await;
        let frozen = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let delivery = ports
            .executor
            .deliver(
                RequestId::new(),
                session,
                "business-result".into(),
                json!({"text":"actual result"}),
            )
            .await
            .unwrap()
            .value;
        ports
            .executor
            .consume_delivery(
                RequestId::new(),
                session,
                delivery.id,
                execution.message_id,
                "actual result".into(),
            )
            .await
            .unwrap();
        frozen.finish(RequestId::new(), None, true).await.unwrap();
        assert_eq!(decision(&attempt).await, StopDecision::Continue);
        observe(&attempt, false).await;
        assert_eq!(decision(&attempt).await, StopDecision::Continue);
        observe(&attempt, true).await;
        assert_eq!(decision(&attempt).await, StopDecision::Ready);
        // Stop diagnostics are kept as notices without forcing an endless
        // hook -> diagnostic -> extra model call cycle.
        let diagnostic = ports
            .executor
            .deliver(
                RequestId::new(),
                session,
                "hook-diagnostic".into(),
                json!({"text":"hook observation","wake":false}),
            )
            .await
            .unwrap()
            .value;
        ports
            .executor
            .consume_delivery(
                RequestId::new(),
                session,
                diagnostic.id,
                execution.message_id,
                "hook observation".into(),
            )
            .await
            .unwrap();
        assert_eq!(decision(&attempt).await, StopDecision::Ready);
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn background_activity_and_queued_input_release_the_execution_without_a_ready_stop() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, execution, attempt) = attempt(&ports).await;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "launch".into(),
                    name: "fixture".into(),
                    input: json!({}),
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
                    label: "controlled process".into(),
                },
            )
            .await
            .unwrap()
            .value;
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({"launched":true}),
            },
        )
        .await
        .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        assert_eq!(decision(&attempt).await, StopDecision::AwaitingBackground);
        // Background waiting does not fence new guidance; Finish still
        // resolves its race with a pending targeted delivery atomically.
        let guidance = ports
            .executor
            .deliver_to_execution(
                RequestId::new(),
                session,
                execution.id,
                "waiting-guidance".into(),
                json!({"text":"more work","wake":false}),
            )
            .await
            .unwrap()
            .value;
        assert!(matches!(
            attempt
                .finish(RequestId::new(), ExecutionPhase::Succeeded)
                .await,
            Err(KernelError::PendingDelivery(_))
        ));
        ports
            .executor
            .consume_delivery(
                RequestId::new(),
                session,
                guidance.id,
                execution.message_id,
                "more work".into(),
            )
            .await
            .unwrap();
        observe(&attempt, true).await;
        assert_eq!(decision(&attempt).await, StopDecision::AwaitingBackground);
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        activity
            .end(
                RequestId::new(),
                BackgroundActivityOutcome::Succeeded {
                    value: json!({"exit_code":0}),
                },
            )
            .await
            .unwrap();
        let next = ports
            .sessions
            .send(RequestId::new(), session, document("next input"))
            .await
            .unwrap()
            .value;
        let current = ports
            .executor
            .claim(RequestId::new(), session, next.id)
            .await
            .unwrap()
            .value;
        let queued = ports
            .sessions
            .send(RequestId::new(), session, document("queued input"))
            .await
            .unwrap()
            .value;
        assert_eq!(decision(&current).await, StopDecision::AwaitingBackground);
        ports
            .sessions
            .cancel(RequestId::new(), session, queued.id)
            .await
            .unwrap();
        assert_eq!(decision(&current).await, StopDecision::Ready);
        current
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn observed_once_delivery_can_stop_before_scheduler_ack_but_future_wakes_still_wait() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, execution, attempt) = attempt(&ports).await;
        let key = "scheduler:once-job:exact-occurrence";
        let conditions = StopConditions {
            scheduler_waiting: false,
            pending_once_delivery_keys: vec![key.into()],
        };
        assert_eq!(
            attempt
                .prepare_stop(RequestId::new(), conditions.clone())
                .await
                .unwrap()
                .value,
            StopDecision::AwaitingBackground
        );
        let wrong = ports
            .executor
            .deliver(
                RequestId::new(),
                session,
                "scheduler:once-job:other-occurrence".into(),
                json!({"text":"other occurrence"}),
            )
            .await
            .unwrap()
            .value;
        ports
            .executor
            .consume_delivery(
                RequestId::new(),
                session,
                wrong.id,
                execution.message_id,
                "other occurrence".into(),
            )
            .await
            .unwrap();
        observe(&attempt, true).await;
        assert_eq!(
            attempt
                .prepare_stop(RequestId::new(), conditions.clone())
                .await
                .unwrap()
                .value,
            StopDecision::AwaitingBackground
        );
        let exact = ports
            .executor
            .deliver(
                RequestId::new(),
                session,
                key.into(),
                json!({"text":"once fired"}),
            )
            .await
            .unwrap()
            .value;
        ports
            .executor
            .consume_delivery(
                RequestId::new(),
                session,
                exact.id,
                execution.message_id,
                "once fired".into(),
            )
            .await
            .unwrap();
        assert_eq!(
            attempt
                .prepare_stop(RequestId::new(), conditions.clone())
                .await
                .unwrap()
                .value,
            StopDecision::Continue
        );
        observe(&attempt, true).await;
        assert_eq!(
            attempt
                .prepare_stop(
                    RequestId::new(),
                    StopConditions {
                        scheduler_waiting: true,
                        ..conditions.clone()
                    }
                )
                .await
                .unwrap()
                .value,
            StopDecision::AwaitingBackground
        );
        assert_eq!(
            attempt
                .prepare_stop(RequestId::new(), conditions)
                .await
                .unwrap()
                .value,
            StopDecision::Ready
        );
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn successful_compaction_observes_deliveries_removed_from_the_current_prompt() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, execution, attempt) = attempt(&ports).await;
        let delivered = ports
            .executor
            .deliver(
                RequestId::new(),
                session,
                "compacted-delivery".into(),
                json!({"text":"important background result"}),
            )
            .await
            .unwrap()
            .value;
        let notice = ports
            .executor
            .consume_delivery(
                RequestId::new(),
                session,
                delivered.id,
                execution.message_id,
                "important background result".into(),
            )
            .await
            .unwrap()
            .value;
        assert_eq!(decision(&attempt).await, StopDecision::Continue);
        let compact = attempt
            .begin_compaction(RequestId::new(), 128)
            .await
            .unwrap()
            .value;
        let summary = compact
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        summary
            .append("Preserved background result in the summary")
            .await
            .unwrap();
        compact
            .commit_compaction(RequestId::new(), summary.id(), notice, None)
            .await
            .unwrap();
        let next = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        assert!(
            !next
                .prompt()
                .await
                .unwrap()
                .entries
                .iter()
                .any(|input| input.entry.id == notice)
        );
        next.finish(RequestId::new(), None, true).await.unwrap();
        assert_eq!(decision(&attempt).await, StopDecision::Ready);
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        ports.shutdown().await.unwrap();
    }
}
