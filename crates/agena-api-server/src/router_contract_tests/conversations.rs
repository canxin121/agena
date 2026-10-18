use super::*;
use agena_api::resource::{BtwAnswer, BtwRequest};
use agena_domain::ConversationMode;
use agena_storage::kernel::{MetadataChange, RequestId, SummaryQuery};

async fn parent_fixture(server: &TestServer, client: &AgenaClient) -> i64 {
    let parent = client
        .create_session(server.workspace_id, "conversation parent", None)
        .await
        .unwrap();
    let application = application_for_test(&server.runtime);
    kernel_fixture::completed_input(
        &application,
        parent.id,
        "Implement the main task, which remains in the parent.",
    )
    .await;
    application
        .update_session_selection(parent.id, options())
        .await
        .unwrap();
    application
        .set_session_permission(
            parent.id,
            agena_domain::PermissionConfig {
                path: Some(agena_domain::PathPermissionConfig {
                    workspace: Some(agena_domain::PathAccessModes {
                        read: Some(agena_domain::PermissionMode::Allow),
                        write: Some(agena_domain::PermissionMode::Allow),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    parent.id
}

fn options() -> RunOptions {
    RunOptions {
        model: Some(agena_api::resource::ModelRef::new_with_adapter(
            "fake",
            "openai_responses",
            "fake-model",
        )),
        ..Default::default()
    }
}

async fn answer(stream: &mut agena_client::BtwSubscription) -> BtwAnswer {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let answer = stream
                .recv()
                .await
                .expect("stream remains open")
                .expect("valid answer");
            if answer.done {
                return answer;
            }
        }
    })
    .await
    .expect("BTW finishes")
}

async fn cleaned(server: &TestServer) {
    let application = application_for_test(&server.runtime);
    let kernel = application.session_kernel().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if kernel
                .reader
                .query_summaries(SummaryQuery {
                    temporary_only: true,
                    limit: 128,
                    ..Default::default()
                })
                .await
                .unwrap()
                .sessions
                .is_empty()
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("temporary question is removed");
}

#[tokio::test]
async fn btw_reads_context_and_tools_without_changing_a_running_parent() {
    let (url, mut requests, provider) = spawn_fake_responses_provider(vec![
        FakeProviderPlan {
            events: ask_user_response_events(),
            release: None,
        },
        FakeProviderPlan {
            events: read_workspace_response_events(),
            release: None,
        },
        FakeProviderPlan {
            events: terminal_response_events(
                "**Read-only answer**: permission fixture",
                "resp_btw",
            ),
            release: None,
        },
    ])
    .await;
    let server = start_test_server(&url).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let parent = submit_test_run(
        &client,
        server.workspace_id,
        "Implement the main task, which remains in the parent.",
    )
    .await
    .session
    .id;
    let waiting = wait_for_execution(&client, parent, |execution| {
        !execution
            .session
            .state
            .pending_interactive_requests()
            .is_empty()
    })
    .await;
    let application = application_for_test(&server.runtime);
    application
        .set_session_permission(
            parent,
            agena_domain::PermissionConfig {
                path: Some(agena_domain::PathPermissionConfig {
                    workspace: Some(agena_domain::PathAccessModes {
                        read: Some(agena_domain::PermissionMode::Allow),
                        write: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let _ = requests.recv().await.unwrap();
    let kernel = application.session_kernel().unwrap();
    let before = kernel_fixture::entries(&application, parent).await;
    let before_version = kernel
        .reader
        .metadata(parent.try_into().unwrap())
        .await
        .unwrap()
        .metadata_version;
    let mut stream = client
        .ask_btw(
            parent,
            BtwRequest {
                question: "Read the fixture and explain it.".into(),
                options: Default::default(),
            },
        )
        .await
        .unwrap();
    let result = answer(&mut stream).await;
    assert_eq!(result.error, None, "{result:?}");
    assert!(result.text.contains("Read-only answer"));
    let first = requests.recv().await.unwrap().to_string();
    assert!(first.contains("temporary /btw question"));
    assert!(first.contains("<conversation_boundary>"));
    assert!(first.contains("Implement the main task"));
    let second = requests.recv().await.unwrap().to_string();
    assert!(
        second.contains("permission fixture"),
        "read-only tool executed"
    );
    cleaned(&server).await;
    let after = kernel_fixture::entries(&application, parent).await;
    assert_eq!(before, after);
    assert_eq!(
        before_version,
        kernel
            .reader
            .metadata(parent.try_into().unwrap())
            .await
            .unwrap()
            .metadata_version
    );
    let after_state = client
        .get_session_state(parent)
        .await
        .unwrap()
        .session
        .state;
    assert_eq!(after_state.as_str(), waiting.session.state.as_str());
    assert_eq!(
        after_state.pending_interactive_requests().len(),
        waiting.session.state.pending_interactive_requests().len()
    );
    provider.await.unwrap();
}

#[tokio::test]
async fn btw_rejects_mutating_tools_even_when_parent_permissions_allow_them() {
    let mut write = read_workspace_response_events();
    for event in &mut write {
        if let Some(args) = event.pointer_mut("/item/arguments")
            && args.as_str().is_some_and(|s| !s.is_empty())
        {
            *args = serde_json::json!({"tool":"fs.apply_patch","input":{"patch":"*** Begin Patch\n*** Add File: should-not-exist.txt\n+bad\n*** End Patch"}}).to_string().into();
        }
    }
    let (url, mut requests, provider) = spawn_fake_responses_provider(vec![
        FakeProviderPlan {
            events: write,
            release: None,
        },
        FakeProviderPlan {
            events: terminal_response_events("This requires a side conversation.", "resp_denied"),
            release: None,
        },
    ])
    .await;
    let server = start_test_server(&url).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let parent = parent_fixture(&server, &client).await;
    let mut stream = client
        .ask_btw(
            parent,
            BtwRequest {
                question: "Try the write tool".into(),
                options: Default::default(),
            },
        )
        .await
        .unwrap();
    assert_eq!(answer(&mut stream).await.error, None);
    let _ = requests.recv().await.unwrap();
    let continuation = requests.recv().await.unwrap().to_string();
    assert!(
        continuation.contains("capability") || continuation.contains("unavailable"),
        "{continuation}"
    );
    assert!(
        !server
            ._workspace
            .path()
            .join("should-not-exist.txt")
            .exists()
    );
    cleaned(&server).await;
    provider.await.unwrap();
}

#[tokio::test]
async fn closing_btw_cancels_only_the_temporary_execution_and_hides_it_from_lists() {
    let (release, wait) = oneshot::channel();
    let (url, mut requests, provider) = spawn_fake_responses_provider(vec![FakeProviderPlan {
        events: terminal_response_events("late answer", "resp_late"),
        release: Some(wait),
    }])
    .await;
    let server = start_test_server(&url).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let parent = parent_fixture(&server, &client).await;
    let application = application_for_test(&server.runtime);
    let kernel = application.session_kernel().unwrap();
    let before = kernel_fixture::entries(&application, parent).await;
    let live_state = AppState::from_application(application_for_test(&server.runtime));
    let mut live = crate::live::subscribe(&live_state, agena_api::Scope::Global).unwrap();
    let stream = client
        .ask_btw(
            parent,
            BtwRequest {
                question: "Question to cancel".into(),
                options: Default::default(),
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), requests.recv())
        .await
        .unwrap()
        .unwrap();
    let visible = kernel
        .reader
        .query_summaries(SummaryQuery {
            limit: 128,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(visible.sessions.len(), 1);
    assert_eq!(visible.sessions[0].id.get(), parent);
    assert_eq!(visible.sessions[0].child_session_count, 0);
    assert_eq!(
        kernel
            .reader
            .workspace_statistics(&[server.workspace_id])
            .await
            .unwrap()[&server.workspace_id]
            .total,
        1
    );
    let child = kernel
        .reader
        .query_summaries(SummaryQuery {
            temporary_only: true,
            limit: 1,
            ..Default::default()
        })
        .await
        .unwrap()
        .sessions[0]
        .id
        .get();
    let mut midstream_live = crate::live::subscribe(&live_state, agena_api::Scope::Global).unwrap();
    // Connecting after creation must classify parts from metadata, rather
    // than depending on having observed the temporary fork's creation.
    // The live temporary execution itself supplies hidden changes; no second
    // request is queued while the fixture provider is deliberately held.
    drop(stream);
    cleaned(&server).await;
    kernel
        .sessions
        .update_metadata(
            RequestId::new(),
            parent.try_into().unwrap(),
            MetadataChange {
                title: Some("parent barrier".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for subscription in [&mut live, &mut midstream_live] {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match subscription.recv().await.expect("filtered feed stays open") {
                    crate::live::LiveItem::SessionChanged(change) => {
                        assert_eq!(change.session_id(), parent, "temporary parts/meta/deletion must stay out of the UI feed");
                        if matches!(change, agena_api::live::SessionChangeResource::SessionMetaUpdated { ref title, .. } if title == "parent barrier") { break; }
                    }
                    crate::live::LiveItem::RuntimeSignal(signal) => assert_ne!(signal.session_id, Some(child)),
                    crate::live::LiveItem::Lagged(_) => panic!("unexpected event pressure"),
                }
            }
        }).await.expect("parent updates still delivered");
    }
    assert_eq!(kernel_fixture::entries(&application, parent).await, before);
    let _ = release.send(());
    provider.await.unwrap();
}

#[tokio::test]
async fn btw_requires_an_owned_stream_and_a_bounded_question() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let parent = parent_fixture(&server, &client).await;
    assert!(
        client
            .command(Command::ForkSession(ForkSessionParams {
                session_id: parent,
                at_message_id: None,
                title: None,
                conversation_mode: Some(ConversationMode::Btw)
            }))
            .await
            .is_err()
    );
    for question in ["  ".to_owned(), "字".repeat(6000)] {
        assert!(
            client
                .ask_btw(
                    parent,
                    BtwRequest {
                        question,
                        options: Default::default()
                    }
                )
                .await
                .is_err()
        );
    }
    cleaned(&server).await;
}

#[tokio::test]
async fn side_identity_and_hidden_boundary_survive_reopening() {
    let (url, mut requests, provider) = spawn_fake_responses_provider(vec![FakeProviderPlan {
        events: terminal_response_events("Side answer", "resp_side"),
        release: None,
    }])
    .await;
    let server = start_test_server(&url).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let parent = parent_fixture(&server, &client).await;
    let CommandResult::Execution(child) = client
        .command(Command::ForkSession(ForkSessionParams {
            session_id: parent,
            at_message_id: None,
            title: None,
            conversation_mode: Some(ConversationMode::Side),
        }))
        .await
        .unwrap()
    else {
        panic!("fork")
    };
    assert_eq!(
        child
            .execution
            .conversation
            .as_ref()
            .unwrap()
            .parent_session_id,
        parent
    );
    let accepted = client
        .submit_message(SubmitRunParams {
            session_id: child.session.id,
            options: Default::default(),
            document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: "Explain the plan only".into(),
            }]),
        })
        .await
        .unwrap();
    let receipt = accepted
        .receipt
        .expect("accepted identity is independent of the Part snapshot");
    assert_ne!(receipt.execution_id, uuid::Uuid::nil());
    let done = wait_for_execution(&client, child.session.id, |execution| {
        execution_text(execution).contains("Side answer") && !execution.session.state.is_running()
    })
    .await;
    assert_eq!(
        done.execution.execution.conversation.unwrap().mode,
        ConversationMode::Side
    );
    assert!(
        !client
            .session_all_parts(child.session.id)
            .await
            .unwrap()
            .iter()
            .any(|part| part.content.to_string().contains("conversation_boundary"))
    );
    let input = requests.recv().await.unwrap().to_string();
    assert!(input.contains("You are in a side conversation"));
    assert!(input.contains("<conversation_boundary>"));
    assert!(input.contains("Explain the plan only"));
    provider.await.unwrap();
}
