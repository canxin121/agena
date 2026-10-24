#![cfg(unix)]

use std::{collections::HashMap, sync::Arc, time::Duration};

use agena_domain::{ComposerDocument, ComposerNode, ContentKind, ProcessStatus};
use agena_runtime_tools::{
    TerminalOwner, TerminalRegistry, TerminalStartParams, tool::ToolOutputStreams,
};
use agena_storage::kernel::{
    BackgroundActivityKind, CaptureState, EventPayload, KernelError, RequestId, ToolStart,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_stream_keeps_raw_output_and_final_screen_under_observation() {
    let directory = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("terminal.db").display()
    );
    let runtime = agena_storage_sqlite::open_kernel_runtime(&database_url)
        .await
        .unwrap();
    let workspace = runtime
        .workspaces
        .create(directory.path().to_string_lossy().into_owned())
        .await
        .unwrap();
    let kernel = runtime.ports;
    let session = kernel
        .sessions
        .create(RequestId::new(), workspace.id, "terminal regression".into())
        .await
        .unwrap()
        .value
        .id;
    let execution = kernel
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "stream 1 to 10".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = kernel
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
                call_id: "terminal".into(),
                name: "shell.open".into(),
                input: serde_json::json!({"command": "seq 1 10"}),
            },
        )
        .await
        .unwrap()
        .value;
    let output = ToolOutputStreams::begin_activity(
        &tool,
        BackgroundActivityKind::Terminal,
        "terminal regression".into(),
        ContentKind::Terminal,
    )
    .await
    .unwrap();
    let terminal_id = output.reference().resource_id;
    let log_id = output
        .references()
        .iter()
        .find(|reference| reference.kind == ContentKind::Log)
        .unwrap()
        .resource_id;
    // A prompt may freeze this source between launch intent and the first PTY
    // frame. Both empty live and frozen reads must be valid observations.
    let watch = kernel
        .reader
        .watch_content(session, terminal_id)
        .await
        .unwrap();
    let empty = watch.head().unwrap();
    assert_eq!(empty.committed, 0);
    assert!(
        kernel
            .reader
            .event_snapshot(session, terminal_id)
            .await
            .unwrap()
            .terminal
            .is_none()
    );
    assert!(
        kernel
            .reader
            .event_snapshot_at(session, &empty)
            .await
            .unwrap()
            .terminal
            .is_none()
    );

    let registry = Arc::new(TerminalRegistry::default());
    let owner = TerminalOwner {
        workspace: directory.path().to_path_buf(),
        session_id: Some(session.get()),
    };
    let params = TerminalStartParams {
        output: Some(output.clone()),
        process_id: None,
        owner: owner.clone(),
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            r#"for i in $(seq 1 10); do echo "$i"; sleep 1; done"#.into(),
        ],
        display_command: "stream 1 to 10".into(),
        description: "terminal regression".into(),
        workdir: directory.path().to_path_buf(),
        env: HashMap::new(),
        rows: 24,
        cols: 80,
        timeout_ms: Some(20_000),
    };
    let worker_registry = registry.clone();
    let started = tokio::task::spawn_blocking(move || worker_registry.start(params, 10))
        .await
        .unwrap()
        .unwrap();
    let process_id = started.summary.process_id;
    let mut observers = Vec::new();
    for _ in 0..8 {
        let reader = kernel.reader.clone();
        observers.push(tokio::spawn(async move {
            let mut cursor = None;
            let mut observations = 0;
            loop {
                match reader.event_page(session, terminal_id, cursor, 64).await {
                    Ok(page) => {
                        cursor = Some(page.next);
                        observations += page.records.len();
                        assert_eq!(page.stream.capture_loss.packets, 0);
                        if page.stream.state != CaptureState::Open && !page.has_more {
                            return observations;
                        }
                    }
                    Err(KernelError::Busy) => {}
                    Err(error) => panic!("terminal observer failed: {error}"),
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }));
    }
    for observer in observers {
        assert!(
            tokio::time::timeout(Duration::from_secs(25), observer)
                .await
                .unwrap()
                .unwrap()
                >= 2
        );
    }
    let worker_registry = registry.clone();
    let result = tokio::task::spawn_blocking(move || {
        worker_registry.read(&process_id, &owner, Some(0), 0, None)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.summary.status, ProcessStatus::Exited);
    let expected = (1..=10).map(|i| format!("{i}\r\n")).collect::<String>();
    assert_eq!(result.output, expected);
    let snapshot = kernel
        .reader
        .event_snapshot(session, terminal_id)
        .await
        .unwrap();
    assert_eq!(snapshot.stream.state, CaptureState::Complete);
    assert_eq!(snapshot.stream.capture_loss.packets, 0);
    assert_eq!(
        snapshot.terminal.as_ref().unwrap().plain_text().trim_end(),
        expected.replace("\r\n", "\n").trim_end()
    );
    assert!(
        output.capture_diagnostic().is_none(),
        "{:?}",
        output.capture_diagnostic()
    );
    // The original empty boundary remains readable after the final frame.
    assert!(
        kernel
            .reader
            .event_snapshot_at(session, &empty)
            .await
            .unwrap()
            .terminal
            .is_none()
    );
    let log = kernel
        .reader
        .event_page(session, log_id, None, 64)
        .await
        .unwrap();
    let raw = log
        .records
        .iter()
        .map(|record| match &record.payload {
            EventPayload::Log { text, .. } => text.as_str(),
            other => panic!("unexpected raw output: {other:?}"),
        })
        .collect::<String>();
    assert_eq!(raw, expected);
    assert_eq!(log.stream.capture_loss.packets, 0);
    registry.shutdown();
    kernel.shutdown().await.unwrap();
}
