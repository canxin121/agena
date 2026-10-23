use super::*;

#[tokio::test]
async fn server_queues_messages_across_clients_and_dequeue_cannot_cancel_a_started_turn() {
    let (release_tx, release_rx) = oneshot::channel();
    let (provider_url, mut requests, provider) = spawn_fake_responses_provider(vec![
        FakeProviderPlan {
            events: terminal_response_events("first answer", "queue-first"),
            release: Some(release_rx),
        },
        FakeProviderPlan {
            events: terminal_response_events("queued answer", "queue-next"),
            release: None,
        },
    ])
    .await;
    let server = start_test_server(&provider_url).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let first = submit_test_run(&client, server.workspace_id, "first prompt").await;
    let session = first.session.id;
    tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
        .await
        .unwrap()
        .unwrap();
    let mut queued = Vec::new();
    for prompt in ["remove this queued prompt", "keep this queued prompt"] {
        let state = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            client.submit_message(SubmitRunParams {
                session_id: session,
                options: RunOptions::default(),
                document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                    text: prompt.into(),
                }]),
            }),
        )
        .await
        .expect("queue admission must return while provider is held")
        .unwrap();
        queued.push(state.receipt.unwrap().execution_id);
    }
    drop(client);
    let observer = AgenaClient::new(&server.url).unwrap();
    let state = observer.get_session_state(session).await.unwrap();
    assert_eq!(
        state
            .queued_messages
            .iter()
            .map(|item| item.execution_id)
            .collect::<Vec<_>>(),
        queued
    );
    assert_eq!(
        state.session.state.active_execution().unwrap().execution_id,
        first.receipt.unwrap().execution_id
    );
    let pending_parts = observer
        .session_parts_by_ids(
            session,
            &state
                .queued_messages
                .iter()
                .map(|item| item.message_id)
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    assert!(
        pending_parts
            .parts
            .iter()
            .all(|part| part.run_state.as_deref() == Some("pending"))
    );
    assert!(
        pending_parts
            .parts
            .iter()
            .all(|part| part.content["queue_only"] == true)
    );
    let active_conversation = observer
        .session_run_window(session, 2, 16, None)
        .await
        .unwrap();
    assert_eq!(
        active_conversation.runs.len(),
        1,
        "queued input must not displace the current conversation on refresh"
    );
    assert!(active_conversation.parts.iter().all(|part| {
        !state
            .queued_messages
            .iter()
            .any(|queued| queued.message_id == part.part_id)
    }));
    let removed = observer
        .take_queued_message(session, agena_domain::ExecutionId(queued[0]))
        .await
        .unwrap();
    assert_eq!(removed.text(), "remove this queued prompt");
    let removed_part = observer
        .session_parts_by_ids(session, &[state.queued_messages[0].message_id])
        .await
        .unwrap();
    assert_eq!(
        removed_part.parts[0].run_state.as_deref(),
        Some("cancelled")
    );
    let race = observer
        .take_queued_message(
            session,
            agena_domain::ExecutionId(first.receipt.unwrap().execution_id),
        )
        .await
        .unwrap_err();
    assert_eq!(
        race.problem().unwrap().category,
        agena_failure::FailureCategory::Conflict
    );
    let after = observer.get_session_state(session).await.unwrap();
    assert_eq!(after.queued_messages.len(), 1);
    assert!(
        requests.try_recv().is_err(),
        "the active provider must not be interrupted"
    );
    release_tx.send(()).unwrap();
    let next = tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
        .await
        .unwrap()
        .unwrap();
    let next = serde_json::to_string(&next).unwrap();
    assert!(next.contains("first answer"));
    assert!(next.contains("keep this queued prompt"));
    assert!(!next.contains("remove this queued prompt"));
    let finished = wait_for_execution(&observer, session, |state| {
        state.session.state.active_execution().is_none()
    })
    .await;
    assert!(finished.queued_messages.is_empty());
    let latest = observer
        .session_run_window(session, 2, 16, None)
        .await
        .unwrap();
    assert_eq!(latest.runs.len(), 2);
    assert_eq!(latest.runs[0].run_id, state.queued_messages[1].message_id);
    assert!(
        latest
            .parts
            .iter()
            .all(|part| part.message_id == Some(state.queued_messages[1].message_id))
    );
    let consumed = latest
        .parts
        .iter()
        .find(|part| part.role == "user")
        .unwrap();
    assert_eq!(consumed.run_state.as_deref(), Some("completed"));
    assert_eq!(consumed.content["queue_only"], false);
    let originally_pending = pending_parts
        .parts
        .iter()
        .find(|part| part.part_id == consumed.part_id)
        .unwrap();
    assert!(consumed.revision > originally_pending.revision);
    let previous = observer
        .session_run_window(session, 2, 16, latest.page.next_cursor.as_deref())
        .await
        .unwrap();
    assert_eq!(previous.runs.len(), 2);
    assert!(
        previous
            .parts
            .iter()
            .all(|part| part.message_id != Some(state.queued_messages[0].message_id)),
        "withdrawn input must not become a conversation turn"
    );
    provider.await.unwrap();
}

#[tokio::test]
async fn stopping_with_pending_input_keeps_the_queue_across_clients_until_resume_or_clear() {
    let (release, release_rx) = oneshot::channel();
    let (provider_url, mut requests, provider) = spawn_fake_responses_provider(vec![
        FakeProviderPlan {
            events: terminal_response_events("stopped answer", "stop-active"),
            release: Some(release_rx),
        },
        FakeProviderPlan {
            events: terminal_response_events("resumed answer", "resumed-next"),
            release: None,
        },
    ])
    .await;
    let server = start_test_server(&provider_url).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let first = submit_test_run(&client, server.workspace_id, "active input").await;
    let session = first.session.id;
    tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
        .await
        .unwrap()
        .unwrap();
    for prompt in ["take this input back", "send this retained input"] {
        client
            .submit_message(SubmitRunParams {
                session_id: session,
                options: RunOptions::default(),
                document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                    text: prompt.into(),
                }]),
            })
            .await
            .unwrap();
    }
    client.cancel_run(session, None).await.unwrap();
    drop(client);
    let observer = AgenaClient::new(&server.url).unwrap();
    let stopped = wait_for_execution(&observer, session, |state| {
        state.session.state.active_execution().is_none()
    })
    .await;
    assert!(stopped.queue_paused);
    assert_eq!(stopped.queued_messages.len(), 2);
    assert!(
        requests.try_recv().is_err(),
        "stopping must not drain the queue"
    );
    let conversation = observer
        .session_run_window(session, 10, 16, None)
        .await
        .unwrap();
    assert_eq!(
        conversation.runs.len(),
        1,
        "pending inputs must not become unanswered conversation turns"
    );
    let restored = observer
        .take_queued_message(
            session,
            agena_domain::ExecutionId(stopped.queued_messages[0].execution_id),
        )
        .await
        .unwrap();
    assert_eq!(restored.text(), "take this input back");
    let _ = release.send(());
    let resumed = observer
        .set_message_queue_paused(session, false)
        .await
        .unwrap();
    assert!(!resumed.queue_paused);
    let request = tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
        .await
        .unwrap()
        .unwrap();
    let prompt = serde_json::to_string(&request).unwrap();
    assert!(prompt.contains("send this retained input"));
    assert!(!prompt.contains("take this input back"));
    let finished = wait_for_execution(&observer, session, |state| {
        state.session.state.active_execution().is_none()
    })
    .await;
    assert!(finished.queued_messages.is_empty());
    observer
        .set_message_queue_paused(session, true)
        .await
        .unwrap();
    observer
        .submit_message(SubmitRunParams {
            session_id: session,
            options: RunOptions::default(),
            document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: "clear without sending".into(),
            }]),
        })
        .await
        .unwrap();
    let cleared = observer.clear_message_queue(session).await.unwrap();
    assert!(cleared.queue_paused);
    assert!(cleared.queued_messages.is_empty());
    assert!(requests.try_recv().is_err());
    server.server.abort();
    provider.abort();
}
