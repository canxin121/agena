//! Agent identity, delivery and lifecycle race contracts.
use super::*;

#[tokio::test]
async fn spawn_is_async_and_final_handoff_is_durable_once_then_followup_reuses_identity() {
    let fixture = Fixture::new(2).await;
    let child = fixture
        .manager
        .spawn_agent(fixture.session.id, None, agent_request())
        .await
        .unwrap();
    assert_eq!(child.status, AgentStatus::Running);
    fixture.calls(1).await;
    fixture.finish(0, "child result");
    let initial = settled_agent(&fixture, child.agent_id).await;
    assert_eq!(initial.status, AgentStatus::Completed);
    let next = fixture
        .manager
        .followup_agent(
            fixture.session.id,
            None,
            AgentMessageRequest {
                target: "worker".into(),
                message: "check one more thing".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(next.agent_id, child.agent_id);
    fixture.calls(2).await;
    fixture.finish(1, "followup result");
    let final_status = settled_agent(&fixture, child.agent_id).await;
    assert!(final_status.started_at_ms > initial.started_at_ms);
    assert_eq!(final_status.status, AgentStatus::Completed);
    let deliveries = fixture
        .manager
        .kernel()
        .reader
        .pending_deliveries(session_id(fixture.session.id).unwrap(), None, 64)
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 2);
    assert_eq!(deliveries[0].payload["agent_id"], child.agent_id);
    assert!(
        deliveries
            .iter()
            .any(|d| d.payload["text"].as_str().unwrap().contains("child result"))
    );
    assert!(deliveries.iter().any(|d| {
        d.payload["text"]
            .as_str()
            .unwrap()
            .contains("followup result")
    }));
    drop(deliveries);
    assert_eq!(
        fixture
            .manager
            .reconcile_background_agents(128)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .pending_deliveries(session_id(fixture.session.id).unwrap(), None, 64)
            .await
            .unwrap()
            .len(),
        2
    );
    fixture.close().await;
}

#[tokio::test]
async fn live_followup_is_fenced_to_same_execution_and_final_reply_uses_last_round() {
    let fixture = Fixture::new(2).await;
    let child = fixture
        .manager
        .spawn_agent(fixture.session.id, None, agent_request())
        .await
        .unwrap();
    fixture.calls(1).await;
    let id = session_id(child.agent_id).unwrap();
    let execution = fixture
        .manager
        .kernel()
        .reader
        .active_executions(id, None, 1)
        .await
        .unwrap()
        .executions[0]
        .id;
    fixture
        .manager
        .followup_agent(
            fixture.session.id,
            None,
            AgentMessageRequest {
                target: "worker".into(),
                message: "Preserve this explicit instruction.".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .pending_deliveries(id, None, 64)
            .await
            .unwrap()[0]
            .target_execution,
        Some(execution)
    );
    fixture.finish(0, "initial answer");
    fixture.calls(2).await;
    assert!(
        serde_json::to_string(&fixture.provider.requests.lock().unwrap()[1].turns)
            .unwrap()
            .contains("Preserve this explicit instruction.")
    );
    assert!(
        fixture
            .manager
            .kernel()
            .reader
            .pending_deliveries(id, None, 64)
            .await
            .unwrap()
            .is_empty()
    );
    fixture.finish(1, "guided answer");
    assert_eq!(
        settled_agent(&fixture, child.agent_id).await.status,
        AgentStatus::Completed
    );
    assert_eq!(
        fixture
            .manager
            .final_text(child.agent_id, 4096)
            .await
            .unwrap()
            .as_deref(),
        Some("guided answer")
    );
    let rounds = fixture
        .manager
        .kernel()
        .reader
        .rounds(id, None, 64)
        .await
        .unwrap();
    assert_eq!(rounds.len(), 2);
    assert!(rounds.iter().all(|round| round.execution_id == execution));
    drop(rounds);
    // A plain message after completion is accepted but cannot start the idle child.
    fixture
        .manager
        .message_agent(
            fixture.session.id,
            None,
            AgentMessageRequest {
                target: "worker".into(),
                message: "queued for next time".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .recover_background_deliveries(128)
            .await
            .unwrap(),
        0
    );
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 2);
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .pending_deliveries(id, None, 64)
            .await
            .unwrap()
            .len(),
        1
    );
    fixture.close().await;
}

#[tokio::test]
async fn parent_child_sibling_mailboxes_are_scoped_and_wait_returns_no_message_body() {
    let fixture = Fixture::new(2).await;
    let a = fixture
        .manager
        .spawn_agent(fixture.session.id, None, agent_request())
        .await
        .unwrap();
    let mut req = agent_request();
    req.name = "reviewer".into();
    let b = fixture
        .manager
        .spawn_agent(fixture.session.id, None, req)
        .await
        .unwrap();
    fixture.calls(2).await;
    for (from, to, text) in [
        (fixture.session.id, "worker".to_string(), "parent to child"),
        (a.agent_id, "parent".to_string(), "child to parent"),
        (a.agent_id, "reviewer".to_string(), "sibling to sibling"),
    ] {
        fixture
            .manager
            .message_agent(
                from,
                None,
                AgentMessageRequest {
                    target: to,
                    message: text.into(),
                },
            )
            .await
            .unwrap();
    }
    for recipient in [fixture.session.id, a.agent_id, b.agent_id] {
        let wake = fixture
            .manager
            .wait_agent(
                recipient,
                None,
                AgentWaitRequest {
                    timeout_ms: Some(50),
                },
            )
            .await
            .unwrap();
        assert!(matches!(wake.reason, AgentWakeReason::Message));
        let json = serde_json::to_value(wake).unwrap();
        assert_eq!(json, serde_json::json!({"reason":"message"}));
    }
    let outsider = fixture
        .manager
        .create_session(SessionCreateRequest {
            title: "outside".into(),
            parent_session_id: None,
        })
        .await
        .unwrap();
    assert!(
        fixture
            .manager
            .message_agent(
                a.agent_id,
                None,
                AgentMessageRequest {
                    target: outsider.id.to_string(),
                    message: "cross-group".into()
                }
            )
            .await
            .is_err()
    );
    assert!(
        fixture
            .manager
            .message_agent(
                outsider.id,
                None,
                AgentMessageRequest {
                    target: a.agent_id.to_string(),
                    message: "cross-group".into()
                }
            )
            .await
            .is_err()
    );
    assert!(
        fixture
            .manager
            .followup_agent(
                a.agent_id,
                None,
                AgentMessageRequest {
                    target: "reviewer".into(),
                    message: "control sibling".into()
                }
            )
            .await
            .is_err()
    );
    assert!(
        fixture
            .manager
            .cancel_agent(
                a.agent_id,
                None,
                AgentTargetRequest {
                    target: "reviewer".into()
                }
            )
            .await
            .is_err()
    );
    let mut nested = agent_request();
    nested.name = "nested".into();
    assert!(
        fixture
            .manager
            .spawn_agent(a.agent_id, None, nested)
            .await
            .is_err()
    );
    for target in [a.agent_id, b.agent_id] {
        fixture
            .manager
            .cancel_agent(
                fixture.session.id,
                None,
                AgentTargetRequest {
                    target: target.to_string(),
                },
            )
            .await
            .unwrap();
    }
    fixture.close().await;
}

#[tokio::test]
async fn wait_subscribes_before_snapshot_and_handles_arrival_timeout_and_cancellation() {
    let fixture = Fixture::new(1).await;
    let child = fixture
        .manager
        .spawn_agent(fixture.session.id, None, agent_request())
        .await
        .unwrap();
    fixture.calls(1).await;
    let manager = fixture.manager.clone();
    let parent = fixture.session.id;
    let wait = tokio::spawn(async move {
        manager
            .wait_agent(
                parent,
                None,
                AgentWaitRequest {
                    timeout_ms: Some(5000),
                },
            )
            .await
    });
    tokio::task::yield_now().await;
    fixture
        .manager
        .message_agent(
            child.agent_id,
            None,
            AgentMessageRequest {
                target: "parent".into(),
                message: "arrived during wait".into(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), wait)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .reason,
        AgentWakeReason::Message
    ));
    assert!(matches!(
        fixture
            .manager
            .wait_agent(
                child.agent_id,
                None,
                AgentWaitRequest {
                    timeout_ms: Some(1)
                }
            )
            .await
            .unwrap()
            .reason,
        AgentWakeReason::Timeout
    ));
    assert!(
        fixture
            .manager
            .wait_agent(
                child.agent_id,
                None,
                AgentWaitRequest {
                    timeout_ms: Some(0)
                }
            )
            .await
            .is_err()
    );
    fixture
        .manager
        .cancel_agent(
            parent,
            None,
            AgentTargetRequest {
                target: "worker".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        settled_agent(&fixture, child.agent_id).await.status,
        AgentStatus::Cancelled
    );
    // No callback can borrow authority from an inactive tool ID.
    assert!(
        fixture
            .manager
            .wait_agent(parent, Some(999), AgentWaitRequest::default())
            .await
            .is_err()
    );
    fixture.close().await;
}

#[tokio::test]
async fn compact_roster_is_rebuilt_from_runtime_metadata_and_child_controls_are_denied() {
    let fixture = Fixture::new(1).await;
    let child = fixture
        .manager
        .spawn_agent(fixture.session.id, None, agent_request())
        .await
        .unwrap();
    fixture.calls(1).await;
    let id = session_id(child.agent_id).unwrap();
    let config = fixture
        .manager
        .kernel()
        .reader
        .configuration(id)
        .await
        .unwrap();
    for tool in ["agent.spawn", "agent.followup", "agent.cancel"] {
        assert!(config.capability_denied_tool_names.contains(tool));
    }
    for tool in ["agent.message", "agent.wait"] {
        assert!(!config.capability_denied_tool_names.contains(tool));
    }
    drop(config);
    let roster = fixture.manager.agent_roster(id).await.unwrap();
    assert!(roster.contains("worker"));
    assert!(roster.contains("target `parent`"));
    assert!(!roster.contains("finish the delegated work"));
    assert!(roster.len() < 2048);
    fixture.finish(0, "completed answer that must never enter the roster");
    settled_agent(&fixture, child.agent_id).await;
    let roster = fixture
        .manager
        .agent_roster(session_id(fixture.session.id).unwrap())
        .await
        .unwrap();
    assert!(roster.contains("completed"));
    assert!(!roster.contains("completed answer"));
    fixture.close().await;
}

async fn live_callback(fixture: &Fixture) -> (i64, Arc<RunControl>) {
    let session = session_id(fixture.session.id).unwrap();
    let execution = fixture
        .manager
        .kernel()
        .sessions
        .send(RequestId::new(), session, document("gateway callback"))
        .await
        .unwrap()
        .value;
    let attempt = fixture
        .manager
        .kernel()
        .executor
        .claim(RequestId::new(), session, execution.id)
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
                call_id: "gateway".into(),
                name: "tools_call".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let mut state = (*fixture.manager.state.load_full()).clone();
    state.tools = Arc::new(Semaphore::new(1));
    let state = Arc::new(state);
    fixture.manager.state.store(state.clone());
    let control = Arc::new(RunControl {
        session,
        execution: execution.id,
        model: fixture.options().model,
        state: state.clone(),
        cancel: CancellationToken::new(),
        phase: StdMutex::new(agena_domain::ExecutionPhase::ExecutingTools),
        done: Notify::new(),
        budget: None,
        failure: StdMutex::new(None),
        settled: std::sync::atomic::AtomicBool::new(false),
    });
    let call = tool.id().get();
    let permit = state.tools.clone().acquire_owned().await.unwrap();
    fixture.manager.tools.lock().unwrap().insert(
        (session.get(), call),
        LiveTool {
            tool,
            control: control.clone(),
            permit: Arc::new(tokio::sync::Mutex::new(Some(permit))),
        },
    );
    (call, control)
}

#[tokio::test]
async fn gateway_wait_releases_the_only_tool_permit_and_reacquires_it_after_mail_delivery() {
    let fixture = Fixture::new(0).await;
    let (call, control) = live_callback(&fixture).await;
    let parent = fixture.session.id;
    let child = fixture
        .manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            fixture.session.workspace_id,
            "sender".into(),
            SessionConfiguration {
                parent_session_id: Some(control.session),
                relation_kind: agena_domain::SessionRelationKind::Subagent,
                agent: Some(SessionAgentConfiguration {
                    name: "sender".into(),
                    status: AgentStatus::Created,
                    started_at_ms: None,
                    finished_at_ms: None,
                    failure: None,
                    limits: Default::default(),
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let manager = fixture.manager.clone();
    let waiting = tokio::spawn(async move {
        manager
            .wait_agent(
                parent,
                Some(call),
                AgentWaitRequest {
                    timeout_ms: Some(5000),
                },
            )
            .await
    });
    // This acquire times out if agent.wait keeps the gateway's capacity lease.
    let permit = tokio::time::timeout(
        Duration::from_secs(2),
        control.state.tools.clone().acquire_owned(),
    )
    .await
    .unwrap()
    .unwrap();
    fixture
        .manager
        .message_agent(
            child.get(),
            None,
            AgentMessageRequest {
                target: "parent".into(),
                message: "reply using the freed capacity".into(),
            },
        )
        .await
        .unwrap();
    drop(permit);
    assert_eq!(
        waiting.await.unwrap().unwrap().reason,
        AgentWakeReason::Message
    );
    assert_eq!(control.state.tools.available_permits(), 0);
    fixture
        .manager
        .tools
        .lock()
        .unwrap()
        .remove(&(parent, call));
    assert_eq!(control.state.tools.available_permits(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn wait_yields_to_queued_user_input_and_cancellation_without_leaking_capacity() {
    let fixture = Fixture::new(0).await;
    let (call, control) = live_callback(&fixture).await;
    let parent = fixture.session.id;
    let manager = fixture.manager.clone();
    let waiting = tokio::spawn(async move {
        manager
            .wait_agent(
                parent,
                Some(call),
                AgentWaitRequest {
                    timeout_ms: Some(5000),
                },
            )
            .await
    });
    let permit = tokio::time::timeout(
        Duration::from_secs(2),
        control.state.tools.clone().acquire_owned(),
    )
    .await
    .unwrap()
    .unwrap();
    fixture
        .manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            control.session,
            document("new user steering"),
        )
        .await
        .unwrap();
    drop(permit);
    assert_eq!(
        waiting.await.unwrap().unwrap().reason,
        AgentWakeReason::UserInput
    );
    // A separate callback without pending user input exercises cancellation while asleep.
    fixture
        .manager
        .tools
        .lock()
        .unwrap()
        .remove(&(parent, call));
    fixture.close().await;

    let fixture = Fixture::new(0).await;
    let (call, control) = live_callback(&fixture).await;
    let parent = fixture.session.id;
    let manager = fixture.manager.clone();
    let waiting = tokio::spawn(async move {
        manager
            .wait_agent(parent, Some(call), AgentWaitRequest::default())
            .await
    });
    let permit = tokio::time::timeout(
        Duration::from_secs(2),
        control.state.tools.clone().acquire_owned(),
    )
    .await
    .unwrap()
    .unwrap();
    control.cancel.cancel();
    drop(permit);
    assert!(matches!(waiting.await.unwrap(), Err(AppError::Cancelled)));
    assert_eq!(control.state.tools.available_permits(), 1);
    fixture
        .manager
        .tools
        .lock()
        .unwrap()
        .remove(&(parent, call));
    fixture.close().await;
}

#[tokio::test]
async fn followup_racing_with_completion_always_delivers_the_instruction_and_reuses_the_agent() {
    let fixture = Fixture::new(2).await;
    let child = fixture
        .manager
        .spawn_agent(fixture.session.id, None, agent_request())
        .await
        .unwrap();
    fixture.calls(1).await;
    let manager = fixture.manager.clone();
    let parent = fixture.session.id;
    let followup = tokio::spawn(async move {
        manager
            .followup_agent(
                parent,
                None,
                AgentMessageRequest {
                    target: "worker".into(),
                    message: "racing guidance must be read".into(),
                },
            )
            .await
    });
    fixture.finish(0, "first result");
    assert_eq!(followup.await.unwrap().unwrap().agent_id, child.agent_id);
    fixture.calls(2).await;
    assert!(
        serde_json::to_string(&fixture.provider.requests.lock().unwrap()[1].turns)
            .unwrap()
            .contains("racing guidance must be read")
    );
    fixture.finish(1, "final result");
    assert_eq!(
        settled_agent(&fixture, child.agent_id).await.status,
        AgentStatus::Completed
    );
    fixture.close().await;
}

#[tokio::test]
async fn cancelling_then_following_up_resumes_the_same_session_with_its_original_limits() {
    let fixture = Fixture::new(2).await;
    let mut request = agent_request();
    request.max_tokens = Some(100);
    let child = fixture
        .manager
        .spawn_agent(fixture.session.id, None, request)
        .await
        .unwrap();
    fixture.calls(1).await;
    fixture
        .manager
        .cancel_agent(
            fixture.session.id,
            None,
            AgentTargetRequest {
                target: "worker".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        settled_agent(&fixture, child.agent_id).await.status,
        AgentStatus::Cancelled
    );
    let handle = fixture
        .manager
        .followup_agent(
            fixture.session.id,
            None,
            AgentMessageRequest {
                target: "worker".into(),
                message: "resume after explicit cancellation".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(handle.agent_id, child.agent_id);
    fixture.calls(2).await;
    assert_eq!(
        fixture.provider.requests.lock().unwrap()[1].max_output_tokens,
        Some(100)
    );
    fixture.finish(1, "resumed");
    assert_eq!(
        settled_agent(&fixture, child.agent_id).await.status,
        AgentStatus::Completed
    );
    fixture.close().await;
}

#[tokio::test]
async fn accepted_agent_message_enters_the_parent_prompt_once_at_the_next_boundary() {
    let fixture = Fixture::new(3).await;
    fixture.submit("coordinate the work").await;
    fixture.calls(1).await;
    let child = fixture
        .manager
        .spawn_agent(fixture.session.id, None, agent_request())
        .await
        .unwrap();
    fixture.calls(2).await;
    fixture
        .manager
        .message_agent(
            child.agent_id,
            None,
            AgentMessageRequest {
                target: "parent".into(),
                message: "unique child finding".into(),
            },
        )
        .await
        .unwrap();
    fixture.finish(0, "parent preliminary answer");
    fixture.calls(3).await;
    {
        let requests = fixture.provider.requests.lock().unwrap();
        let prompt = serde_json::to_string(&requests[2]).unwrap();
        assert!(prompt.contains("unique child finding"));
        assert!(prompt.contains("Agent roster"));
        assert!(prompt.contains("worker"));
    }
    fixture.finish(2, "parent integrated answer");
    fixture.idle(fixture.session.id).await;
    let history = fixture
        .manager
        .kernel()
        .reader
        .history(session_id(fixture.session.id).unwrap(), None, 128)
        .await
        .unwrap();
    assert_eq!(history.entries.iter().filter(|e|matches!(&e.payload,EntryPayload::Notice {text} if text.contains("unique child finding"))).count(),1);
    drop(history);
    fixture
        .manager
        .cancel_agent(
            fixture.session.id,
            None,
            AgentTargetRequest {
                target: "worker".into(),
            },
        )
        .await
        .unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn agent_identity_capacity_is_bounded_and_names_cannot_shadow_routing() {
    let fixture = Fixture::new(8).await;
    for name in ["parent", "self", "123", "bad-name"] {
        let mut request = agent_request();
        request.name = name.into();
        assert!(
            fixture
                .manager
                .spawn_agent(fixture.session.id, None, request)
                .await
                .is_err()
        );
    }
    let mut children = Vec::new();
    for index in 0..8 {
        let mut request = agent_request();
        request.name = format!("worker{index}");
        let child = fixture
            .manager
            .spawn_agent(fixture.session.id, None, request)
            .await
            .unwrap();
        fixture.calls(index + 1).await;
        fixture.finish(index, "done");
        settled_agent(&fixture, child.agent_id).await;
        children.push(child);
    }
    // The limit covers retained identities, including idle agents. It is
    // independent of the shared provider-input memory admission budget.
    assert!(
        fixture
            .manager
            .spawn_agent(fixture.session.id, None, agent_request())
            .await
            .is_err()
    );
    let roster = fixture
        .manager
        .agent_roster(session_id(fixture.session.id).unwrap())
        .await
        .unwrap();
    assert_eq!(roster.matches("status=").count(), 8);
    for child in children {
        fixture
            .manager
            .cancel_agent(
                fixture.session.id,
                None,
                AgentTargetRequest {
                    target: child.agent_id.to_string(),
                },
            )
            .await
            .unwrap();
    }
    fixture.close().await;
}
