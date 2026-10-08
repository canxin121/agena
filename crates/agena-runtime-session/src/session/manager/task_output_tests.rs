use super::*;
use agena_domain::{
    CommandOutputStream, ContentInput, ContentKind, ContentRef, ContentState, SubtaskStatus,
};
use agena_runtime_contracts::part::OperationPart;
use agena_storage::store::{Part, PartDelta};

async fn launch_part(manager: &SessionManager, parent: &Session, call_id: i64) -> Part {
    let run = manager
        .store
        .start_run(
            parent.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .unwrap();
    let mut operation = OperationPart::pending(call_id,
        ToolInvocation::new("tasks.run", StructuredObject::try_from(serde_json::json!({"description":"fixture task","prompt":"private delegated prompt"})).unwrap()),
        TimeRange::default());
    operation.state = agena_domain::ToolResultState::Running;
    manager
        .store
        .append_parts(
            parent.id,
            run,
            vec![NewPart {
                state: PartState::InProgress,
                ..NewPart::pending(
                    "tool_call",
                    PartRole::Assistant,
                    tool_call_from_operation(&operation).as_value(),
                )
            }],
        )
        .await
        .unwrap()
        .remove(0)
}

async fn output_reference(manager: &SessionManager, parent: i64, part_id: i64) -> ContentRef {
    let session = manager.store.load_session(parent).await.unwrap();
    let part = session
        .parts()
        .iter()
        .find(|part| part.part_id == part_id)
        .unwrap();
    operation_from_part(part).unwrap().resources[0].clone()
}

async fn wait_text(manager: &SessionManager, reference: &ContentRef, needle: &str) -> String {
    let hub = manager.session_store().contents().clone();
    let mut changes = hub.subscribe(reference.resource_id).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let text = hub
                .read_text(reference.resource_id, 64 * 1024)
                .await
                .unwrap();
            if text.text.contains(needle) {
                return text.text;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .expect("task output must stream before completion")
}

#[tokio::test]
async fn task_output_streams_text_and_shell_bytes_without_parent_part_mutations() {
    let manager = test_manager().await;
    let parent = create(&manager, "task streaming parent").await;
    let launch = launch_part(&manager, &parent, 1).await;
    let child = manager
        .store
        .create_subagent_session(parent.id, "capture".into(), "child".into())
        .await
        .unwrap();
    let child_run = manager
        .store
        .start_run(
            child,
            "send",
            run_marker_content("send", None, None, None, None),
        )
        .await
        .unwrap();
    let capture = manager
        .start_subtask_output(
            parent.id,
            Some(1),
            child,
            child_run,
            "capture",
            "fixture task",
        )
        .await
        .unwrap()
        .unwrap();
    let reference = output_reference(&manager, parent.id, launch.part_id).await;
    let parent_revision = manager
        .store
        .facade
        .load(parent.id)
        .await
        .unwrap()
        .parts
        .iter()
        .find(|part| part.part_id == launch.part_id)
        .unwrap()
        .revision;
    let text = manager
        .store
        .append_parts(
            child,
            child_run,
            vec![NewPart {
                state: PartState::InProgress,
                ..NewPart::pending("text", PartRole::Assistant, serde_json::json!({"text":""}))
            }],
        )
        .await
        .unwrap()
        .remove(0);
    let text_writer = manager
        .store
        .facade
        .contents()
        .open(child, text.part_id, ContentKind::Text)
        .await
        .unwrap();
    manager.store.update_part(child, text.part_id, PartDelta {
        content: Some(serde_json::json!({"text":"", "resources":[text_writer.resource().reference()]})), ..Default::default()
    }).await.unwrap();
    text_writer
        .append_text("first streamed 中文\n")
        .await
        .unwrap();
    let live = wait_text(&manager, &reference, "first streamed 中文").await;
    assert!(!live.contains("Task completed"));
    assert_eq!(
        manager
            .store
            .facade
            .contents()
            .describe(reference.resource_id)
            .await
            .unwrap()
            .state,
        ContentState::Active
    );

    let shell = OperationPart::pending(
        2,
        ToolInvocation::new(
            "shell.exec",
            StructuredObject::try_from(serde_json::json!({"command":"echo fixture"})).unwrap(),
        ),
        TimeRange::default(),
    );
    let shell = manager
        .store
        .append_parts(
            child,
            child_run,
            vec![NewPart {
                state: PartState::InProgress,
                ..NewPart::pending(
                    "tool_call",
                    PartRole::Assistant,
                    tool_call_from_operation(&shell).as_value(),
                )
            }],
        )
        .await
        .unwrap()
        .remove(0);
    let shell_writer = manager
        .store
        .facade
        .contents()
        .open(child, shell.part_id, ContentKind::Log)
        .await
        .unwrap();
    let mut operation = operation_from_part(&shell).unwrap();
    operation
        .resources
        .push(shell_writer.resource().reference());
    manager
        .store
        .update_part(
            child,
            shell.part_id,
            PartDelta {
                content: Some(tool_call_from_operation(&operation).as_value()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    shell_writer
        .append(ContentInput::Log {
            stream: CommandOutputStream::Stderr,
            text: "live shell diagnostic\n".into(),
        })
        .await
        .unwrap();
    wait_text(&manager, &reference, "live shell diagnostic").await;
    text_writer
        .append_text("second streamed line\n")
        .await
        .unwrap();
    text_writer.finish().await.unwrap();
    shell_writer.finish().await.unwrap();
    operation.state = agena_domain::ToolResultState::Completed;
    operation.output = Some(agena_domain::RawOutput::text("live shell diagnostic\n"));
    manager
        .store
        .update_part(
            child,
            shell.part_id,
            PartDelta {
                content: Some(tool_call_from_operation(&operation).as_value()),
                state: Some(PartState::Completed),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    manager
        .store
        .append_parts(
            child,
            child_run,
            vec![NewPart {
                visibility: PartVisibility::Ai,
                state: PartState::Completed,
                ..NewPart::pending(
                    "text",
                    PartRole::Assistant,
                    serde_json::json!({"text":"hidden model-only content"}),
                )
            }],
        )
        .await
        .unwrap();
    capture.finish(SubtaskStatus::Completed, None).await;
    let page = manager
        .store
        .facade
        .contents()
        .read(reference.resource_id, None, 64 * 1024)
        .await
        .unwrap();
    assert_eq!(page.resource.state, ContentState::Complete);
    assert!(page.chunks.iter().any(|chunk| matches!(&chunk.payload, agena_domain::ContentPayload::Log { stream:CommandOutputStream::Stderr, text } if text.contains("live shell diagnostic"))));
    let full = manager
        .store
        .facade
        .contents()
        .read_text(reference.resource_id, 64 * 1024)
        .await
        .unwrap();
    assert!(full.text.contains("second streamed line"));
    assert!(full.text.contains("[Task completed]"));
    assert_eq!(
        full.text.matches("live shell diagnostic").count(),
        1,
        "completion must not duplicate a streamed body"
    );
    assert!(!full.text.contains("hidden model-only"));
    assert!(!full.text.contains("private delegated prompt"));
    assert_eq!(
        manager
            .store
            .facade
            .load(parent.id)
            .await
            .unwrap()
            .parts
            .iter()
            .find(|part| part.part_id == launch.part_id)
            .unwrap()
            .revision,
        parent_revision
    );
}

#[tokio::test]
async fn dropping_task_capture_drains_output_and_marks_it_interrupted() {
    let manager = test_manager().await;
    let parent = create(&manager, "task cancelled parent").await;
    let launch = launch_part(&manager, &parent, 1).await;
    let child = manager
        .store
        .create_subagent_session(parent.id, "cancel".into(), "child".into())
        .await
        .unwrap();
    let child_run = manager
        .store
        .start_run(
            child,
            "send",
            run_marker_content("send", None, None, None, None),
        )
        .await
        .unwrap();
    let capture = manager
        .start_subtask_output(
            parent.id,
            Some(1),
            child,
            child_run,
            "cancel",
            "fixture task",
        )
        .await
        .unwrap()
        .unwrap();
    let reference = output_reference(&manager, parent.id, launch.part_id).await;
    let text = manager
        .store
        .append_parts(
            child,
            child_run,
            vec![NewPart {
                state: PartState::Completed,
                ..NewPart::pending(
                    "text",
                    PartRole::Assistant,
                    serde_json::json!({"text":"final queued output"}),
                )
            }],
        )
        .await
        .unwrap()
        .remove(0);
    assert!(text.part_id > child_run);
    drop(capture);
    wait_text(&manager, &reference, "[Task interrupted]").await;
    let hub = manager.store.facade.contents();
    let mut changes = hub.subscribe(reference.resource_id);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while hub.describe(reference.resource_id).await.unwrap().state == ContentState::Active {
            changes.as_mut().unwrap().changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let text = hub
        .read_text(reference.resource_id, 64 * 1024)
        .await
        .unwrap();
    assert!(text.text.contains("final queued output"));
    assert_eq!(
        hub.describe(reference.resource_id).await.unwrap().state,
        ContentState::Interrupted
    );
}

#[tokio::test]
async fn subtask_and_followups_keep_separate_durable_output_resources() {
    let manager = manager_with_provider(Arc::new(FakeProvider {
        provider_id: "fake",
        model: ModelId::new("fake-model"),
        deltas: vec!["provider live text\n".into()],
        thinking_deltas: vec![],
        finish_reason: Some(CompletionFinishReason::Stop),
    }))
    .await;
    let parent = create_with_model(&manager, "inline task parent", "fake", "fake-model").await;
    let mut references = Vec::new();
    let mut child = 0;
    // Provider call ids can repeat across turns; each run belongs to the
    // newest durable launch Part, never an earlier occurrence of that id.
    for run_in_background in [false, true, true] {
        let call_id = 1;
        let launch = launch_part(&manager, &parent, call_id).await;
        let response = manager
            .run_subtask(SessionSubtaskRequest {
                parent_session_id: parent.id,
                run_in_background,
                launch_call_id: Some(call_id),
                description: "inline fixture".into(),
                prompt: "answer".into(),
                commands: None,
                task_id: Some("resume-fixture".into()),
                requested_model_selection: agena_domain::ModelSelectionConfig::default(),
                timeout_ms: Some(5000),
                max_tokens: None,
                max_cost_microusd: None,
            })
            .await
            .unwrap();
        assert_eq!(response.status, SubtaskStatus::Completed);
        assert_eq!(response.final_text.as_deref(), Some("provider live text\n"));
        if child != 0 {
            assert_eq!(response.session.id, child);
            assert!(response.resumed);
        }
        child = response.session.id;
        if run_in_background {
            let external_id = manager
                .background_task_external_id_for_run(
                    parent.id,
                    &response.task_id,
                    child,
                    response.session.runtime.subtask.started_at_ms,
                )
                .await
                .unwrap()
                .unwrap();
            let background = manager
                .store
                .background_operation_by_external_id(
                    agena_storage::store::BackgroundOperationKind::Task,
                    &external_id,
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(background.launch_tool_part_id, Some(launch.part_id));
            assert!(background.phase.is_terminal());
        }
        let reference = output_reference(&manager, parent.id, launch.part_id).await;
        let text = manager
            .store
            .facade
            .contents()
            .read_text(reference.resource_id, 64 * 1024)
            .await
            .unwrap();
        assert_eq!(text.text.matches("provider live text").count(), 1);
        assert_eq!(
            manager
                .store
                .facade
                .contents()
                .describe(reference.resource_id)
                .await
                .unwrap()
                .state,
            ContentState::Complete
        );
        references.push(reference.resource_id);
    }
    assert_eq!(
        references
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timed_out_subtask_seals_parent_output_with_the_terminal_failure() {
    let manager = manager_with_provider(Arc::new(HangingProvider {
        model: ModelId::new("hanging-model"),
    }))
    .await;
    let parent = create_with_model(
        &manager,
        "timeout capture parent",
        "hanging",
        "hanging-model",
    )
    .await;
    let launch = launch_part(&manager, &parent, 1).await;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        manager.run_subtask(SessionSubtaskRequest {
            parent_session_id: parent.id,
            run_in_background: false,
            launch_call_id: Some(1),
            description: "timeout capture".into(),
            prompt: "wait forever".into(),
            commands: None,
            task_id: Some("timeout-capture".into()),
            requested_model_selection: agena_domain::ModelSelectionConfig::default(),
            timeout_ms: Some(50),
            max_tokens: None,
            max_cost_microusd: None,
        }),
    )
    .await
    .expect("task output cleanup must finish")
    .unwrap();
    assert_eq!(response.status, SubtaskStatus::TimedOut);
    let reference = output_reference(&manager, parent.id, launch.part_id).await;
    let hub = manager.store.facade.contents();
    assert_eq!(
        hub.describe(reference.resource_id).await.unwrap().state,
        ContentState::Interrupted
    );
    let text = hub
        .read_text(reference.resource_id, 64 * 1024)
        .await
        .unwrap()
        .text;
    assert!(text.contains("[Task timed_out]"));
    assert!(text.contains(&response.failure.unwrap().user.fallback));
    assert!(!text.contains("wait forever"));
}
