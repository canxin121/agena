use agena_domain::ExecutionId;
use agena_storage::kernel::*;
use agena_storage_sqlite::open_kernel_runtime;

#[tokio::test]
async fn command_activity_survives_archive_import_with_remapped_operation_and_output() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/command-activity")
        .await
        .unwrap();
    let ports = &runtime.ports;
    let session = ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Source".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let origin = PluginCommandActivityOrigin {
        invocation: ExecutionId::new(),
        plugin_id: "deepseek.command-test".into(),
        command_id: "jobs".into(),
    };
    let specification = BackgroundActivityStart {
        kind: BackgroundActivityKind::Process,
        label: "fixture job".into(),
    };
    let launch = RequestId::new();
    let retention = ActivityOutputRetention {
        live_bytes: 4096,
        settled_bytes: 1024,
    };
    let start = || {
        ports.sessions.begin_plugin_command_activity(
            launch,
            session,
            origin.clone(),
            specification.clone(),
            retention,
        )
    };
    let (anchor, activity) = start().await.unwrap().value;
    let (replayed_anchor, replayed_activity) = start().await.unwrap().value;
    assert_eq!(replayed_anchor, anchor);
    assert_eq!(replayed_activity.id(), activity.id());
    let entry = ports.reader.entry(session, anchor).await.unwrap();
    let EntryPayload::PluginCommand {
        plugin_id,
        command_id,
        operation,
    } = &entry.payload
    else {
        panic!("expected a plugin command");
    };
    assert_eq!(plugin_id, &origin.plugin_id);
    assert_eq!(command_id, &origin.command_id);
    let operation = *operation;
    assert_eq!(entry.message_id, anchor);
    let command = ports
        .reader
        .execution(session, origin.invocation)
        .await
        .unwrap();
    assert_eq!(command.purpose, ExecutionPurpose::PluginCommand);
    assert_eq!(command.phase, ExecutionPhase::Succeeded);
    assert!(matches!(
        ports
            .sessions
            .retry(RequestId::new(), session, origin.invocation)
            .await,
        Err(KernelError::Invalid(_))
    ));

    let capture = activity.begin_text(RequestId::new()).await.unwrap().value;
    capture.append("saved command output").await.unwrap();
    capture.finish(RequestId::new()).await.unwrap();
    let outcome = BackgroundActivityOutcome::Succeeded {
        value: serde_json::json!({"ok": true}),
    };
    activity
        .end_with_notice(
            RequestId::new(),
            outcome.clone(),
            Some(BackgroundActivityNotice {
                text: "job completed".into(),
                origin_message_id: anchor,
                wake: true,
                wake_budget: None,
                output_position: None,
            }),
        )
        .await
        .unwrap();
    let pending = ports
        .reader
        .pending_deliveries(session, None, 4)
        .await
        .unwrap();
    let [delivery] = pending.as_slice() else {
        panic!("expected one job completion");
    };
    let resume = || {
        ports.executor.resume_delivery(
            RequestId::new(),
            session,
            delivery.id,
            anchor,
            "job completed".into(),
        )
    };
    let run = resume().await.unwrap().value;
    assert_eq!(run.purpose, ExecutionPurpose::Continuation);
    assert_ne!(run.message_id, anchor);
    assert_eq!(resume().await.unwrap().value, run);
    let attempt = ports
        .executor
        .claim_next(RequestId::new(), session, run.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let prompt = round.prompt().await.unwrap();
    assert!(prompt.entries.iter().any(|entry| {
        matches!(&entry.entry.payload, EntryPayload::Notice { text } if text == "job completed")
    }));
    assert!(prompt.entries.iter().all(|entry| {
        !matches!(
            entry.entry.payload,
            EntryPayload::PluginCommand { .. }
                | EntryPayload::User { .. }
                | EntryPayload::Tool { .. }
        )
    }));
    drop(prompt);
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("command-activity.jsonl");
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    ports.reader.export_jsonl(session, &mut file).await.unwrap();
    drop(file);
    let restored = ports
        .sessions
        .import_archive(RequestId::new(), workspace, &path)
        .await
        .unwrap()
        .value
        .id;
    let history = ports.reader.history(restored, None, 16).await.unwrap();
    let (restored_entry, restored_operation) = history
        .entries
        .iter()
        .find_map(|entry| {
            let EntryPayload::PluginCommand {
                plugin_id,
                command_id,
                operation,
            } = &entry.payload
            else {
                return None;
            };
            assert_eq!(plugin_id, &origin.plugin_id);
            assert_eq!(command_id, &origin.command_id);
            Some((entry, *operation))
        })
        .unwrap();
    assert_ne!(restored_entry.id, anchor);
    assert_ne!(restored_entry.execution_id, origin.invocation);
    assert_ne!(restored_operation, operation);
    let restored_command = ports
        .reader
        .execution(restored, restored_entry.execution_id)
        .await
        .unwrap();
    assert_eq!(restored_command.purpose, ExecutionPurpose::PluginCommand);
    let operation = ports
        .reader
        .tool(restored, restored_operation)
        .await
        .unwrap();
    let [restored_activity] = operation.activities.as_slice() else {
        panic!("expected one restored activity");
    };
    assert_ne!(*restored_activity, activity.id());
    let restored_activity = ports
        .reader
        .activity(restored, *restored_activity)
        .await
        .unwrap();
    assert_eq!(restored_activity.launch_entry, restored_entry.id);
    assert_eq!(restored_activity.launch_tool, restored_operation);
    assert_eq!(restored_activity.outcome, Some(outcome));
    let [restored_capture] = operation.outputs.as_slice() else {
        panic!("expected one restored capture");
    };
    assert_ne!(restored_capture.id, capture.id());
    assert_eq!(restored_capture.activity_id, Some(restored_activity.id));
    let content = ports
        .reader
        .text_range(restored, restored_capture.id, None, 256)
        .await
        .unwrap();
    assert_eq!(content.bytes, b"saved command output");
    runtime.ports.clone().shutdown().await.unwrap();
}
