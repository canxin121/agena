use super::*;
use agena_domain::{ComposerDocument, ComposerNode, StructuredObject};
use agena_storage::kernel::{ClearMode, ExecutionPhase, RequestId, ToolResult, ToolStart};

#[tokio::test]
async fn transcript_projects_the_visible_session_and_keeps_issued_files_alive() {
    let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let workspace_id = runtime
        .workspaces
        .ensure_id(workspace.path().to_str().unwrap())
        .await
        .unwrap();
    let kernel = &runtime.ports;
    let session = kernel
        .sessions
        .create(RequestId::new(), workspace_id, "Transcript".into())
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
                text: "visible request".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = kernel
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    output.append("first assistant text").await.unwrap();
    output.finish(RequestId::new()).await.unwrap();

    let mut tools = Vec::new();
    for command in ["printf first", "printf second"] {
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: command.into(),
                    name: ToolApiFunction::Call.function_name().into(),
                    input: json!({"tool":"shell.exec","input":{"command":command}}),
                },
            )
            .await
            .unwrap()
            .value;
        tools.push(tool);
    }
    let completed = &tools[0];
    completed
        .complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({
                    "invocation":ToolInvocation::new(
                        "shell.exec",
                        StructuredObject::try_from(json!({"command":"printf rewritten"})).unwrap(),
                    ),
                    "hook_model_output":"visible result",
                }),
            },
        )
        .await
        .unwrap();
    let transcripts = SessionTranscripts::default();
    let owner = (
        "claude.fixture".parse().unwrap(),
        workspace.path().join("plugin"),
    );
    let claude = transcripts
        .for_hook(
            &kernel.reader,
            session,
            workspace.path(),
            HookTranscriptFormat::Claude,
            owner.clone(),
        )
        .await
        .unwrap();
    let lines = read_records(&claude);
    let types = lines
        .iter()
        .map(|line| line["message"]["content"][0]["type"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        types,
        ["text", "text", "tool_use", "tool_use", "tool_result"]
    );
    assert_eq!(lines[0]["message"]["content"][0]["text"], "visible request");
    assert_eq!(
        lines[1]["message"]["content"][0]["text"],
        "first assistant text"
    );
    assert!(lines[0]["parentUuid"].is_null());
    for pair in lines.windows(2) {
        assert_eq!(pair[0]["uuid"], pair[1]["parentUuid"]);
    }
    let call = &lines[2]["message"]["content"][0];
    let result = &lines[4]["message"]["content"][0];
    assert_eq!(call["name"], "Bash");
    assert_eq!(call["input"]["command"], "printf rewritten");
    assert_eq!(call["id"], completed.id().get().to_string());
    assert_eq!(call["id"], result["tool_use_id"]);
    assert_eq!(result["content"], "visible result");

    let codex = transcripts
        .for_hook(
            &kernel.reader,
            session,
            workspace.path(),
            HookTranscriptFormat::Codex,
            owner.clone(),
        )
        .await
        .unwrap();
    let lines = read_records(&codex);
    assert_eq!(lines[0]["type"], "session_meta");
    assert_eq!(
        lines[0]["payload"]["agena_session_id"],
        session.get().to_string()
    );
    assert!(Uuid::parse_str(lines[0]["payload"]["id"].as_str().unwrap()).is_ok());
    let payloads = lines
        .iter()
        .filter(|line| line["type"] == "response_item")
        .map(|line| &line["payload"])
        .collect::<Vec<_>>();
    assert_eq!(payloads[2]["type"], "function_call");
    assert_eq!(payloads[3]["type"], "function_call");
    assert_eq!(payloads[4]["type"], "function_call_output");
    assert_eq!(payloads[2]["call_id"], payloads[4]["call_id"]);
    assert_eq!(payloads[4]["output"], "visible result");
    assert_eq!(
        transcripts
            .active_formats(|key, _| *key == owner.0)
            .await
            .len(),
        2
    );
    assert!(transcripts.active_formats(|_, _| false).await.is_empty());

    tools[1]
        .complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Failed,
                value: json!({"summary":"subsequent tool failure"}),
            },
        )
        .await
        .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let refreshed = transcripts
        .refresh(
            &kernel.reader,
            session,
            workspace.path(),
            HookTranscriptFormat::Claude,
            None,
        )
        .await
        .unwrap();
    assert_eq!(claude.path(), refreshed.path());
    let lines = read_records(&refreshed);
    assert_eq!(lines.len(), 6);
    assert_eq!(
        lines[5]["message"]["content"][0],
        json!({
            "type":"tool_result",
            "tool_use_id":tools[1].id().get().to_string(),
            "is_error":true,
            "content":"subsequent tool failure",
        })
    );
    let limited = new_file(HookTranscriptFormat::Claude).unwrap();
    let summary = kernel.reader.summary(session).await.unwrap();
    let mut document = Document {
        writer: BufWriter::new(tokio::fs::File::create(limited.path()).await.unwrap()),
        format: HookTranscriptFormat::Claude,
        session_uuid: projection_id(workspace.path(), session),
        summary: &summary,
        workspace: workspace.path(),
        previous: None,
    };
    document.project(&kernel.reader, 1_100).await.unwrap();
    document.writer.flush().await.unwrap();
    assert!(std::fs::metadata(limited.path()).unwrap().len() <= 1_100);
    let tail = read_records(&limited);
    assert!(tail.len() < lines.len());
    assert!(tail[0]["parentUuid"].is_null());
    assert_eq!(tail.last().unwrap()["uuid"], lines.last().unwrap()["uuid"]);
    assert_eq!(
        tail.last().unwrap()["message"]["content"][0]["content"],
        "subsequent tool failure"
    );
    kernel
        .sessions
        .clear(RequestId::new(), session, ClearMode::History)
        .await
        .unwrap();
    let cleared = transcripts
        .refresh(
            &kernel.reader,
            session,
            workspace.path(),
            HookTranscriptFormat::Claude,
            None,
        )
        .await
        .unwrap();
    assert_eq!(cleared.path(), claude.path());
    assert!(read_records(&cleared).is_empty());
    let path = cleared.path().to_path_buf();
    drop(transcripts);
    drop(claude);
    drop(refreshed);
    assert!(path.is_file());
    drop(cleared);
    assert!(!path.exists());
}

fn read_records(file: &ImportedHookFile) -> Vec<Value> {
    std::fs::read_to_string(file.path())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
