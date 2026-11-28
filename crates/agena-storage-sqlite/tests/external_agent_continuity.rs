use std::{path::PathBuf, sync::Arc};

use agena_domain::{ComposerDocument, ComposerNode, ExternalAgent};
use agena_storage::kernel::*;
use agena_storage_sqlite::open_acp_kernel;
use serde_json::json;

fn user_doc(text: &str) -> ComposerDocument {
    ComposerDocument(vec![ComposerNode::Text { text: text.into() }])
}

#[tokio::test]
async fn imported_native_session_over_8_mib_supports_anchor_and_consecutive_resumes() {
    let runtime = open_acp_kernel().await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/external-agent-continuity")
        .await
        .unwrap();

    // 40 historical tool calls * ~256 KiB output > 10 MiB, strictly exceeding
    // the 8 MiB native LLM provider input-window budget.
    let big_output = "x".repeat(256 * 1024);
    let tools: Vec<ExternalAgentHistoryPart> = (0..40)
        .map(|i| ExternalAgentHistoryPart::Tool {
            start: ToolStart {
                call_id: format!("call-{i}"),
                name: "read_file".into(),
                input: json!({ "path": format!("src/file_{i}.rs") }),
            },
            result: ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({ "content": big_output }),
            },
            blobs: Vec::new(),
        })
        .collect();

    let turns = vec![ExternalAgentHistoryTurn {
        user: user_doc("Native agy session prompt"),
        cursor: None,
        blobs: Vec::new(),
        prelude: Vec::new(),
        response: tools,
    }];

    let cwd = PathBuf::from("/external-agent-continuity");
    let session = runtime
        .ports
        .sessions
        .import_external_agent(
            RequestId::new(),
            ExternalAgentImport {
                workspace_id: workspace,
                agent: ExternalAgent::Antigravity,
                title: "Large native agy session".into(),
                link: ExternalAgentLink {
                    native_id: "native-large-1".into(),
                    cwd: cwd.clone(),
                    incarnation: None,
                    resume_at: None,
                },
                turns: Arc::from(turns),
                state: ExternalAgentState::default(),
                replace_view: None,
            },
        )
        .await
        .unwrap()
        .value
        .id;

    // Turn 1 after import: must NOT fail with "provider input facts exceed 8 MiB".
    let exec1 = runtime
        .ports
        .sessions
        .send(RequestId::new(), session, user_doc("Follow-up 1"))
        .await
        .unwrap()
        .value;
    let attempt1 = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, exec1.id)
        .await
        .unwrap()
        .value;
    let opened1 = attempt1
        .begin_external_agent_round(RequestId::new())
        .await
        .expect("external agent round must succeed even when imported history exceeds 8 MiB")
        .value;
    let continuity1 = opened1
        .continuity
        .as_ref()
        .expect("continuity must be populated for external agent rounds");
    assert!(continuity1.has_prior_turns);
    assert!(!continuity1.can_resume_previous);
    assert_eq!(
        continuity1.resume_anchor,
        opened1.link.as_ref().and_then(|l| l.resume_at.clone()),
        "Turn 1 anchor must match imported native history tail"
    );
    let facts1 = opened1.round.prompt().await.unwrap();
    assert_eq!(
        facts1.entries.len(),
        1,
        "external agent round should capture only the active turn input"
    );
    assert_eq!(facts1.entries[0].entry.id, exec1.message_id);

    let link1 = ExternalAgentLink {
        native_id: "native-large-1".into(),
        cwd: cwd.clone(),
        incarnation: Some(opened1.round.id()),
        resume_at: None,
    };
    let pending1 = ExternalAgentPendingTurn {
        message: exec1.message_id,
        round: opened1.round.id(),
        input: continuity1.pending_signature,
        delivery: ExternalAgentPromptDelivery::Unsent,
    };
    opened1
        .round
        .bind_external_agent_link(RequestId::new(), link1.clone(), pending1)
        .await
        .unwrap();
    opened1
        .round
        .finish(RequestId::new(), None, true)
        .await
        .unwrap();
    let resume1 = ExternalAgentResume {
        session,
        cwd: cwd.clone(),
        round: opened1.round.id(),
        native_id: link1.native_id.clone(),
        incarnation: link1.incarnation,
        input: continuity1.resume_signature_structured,
        message: exec1.message_id,
    };
    attempt1
        .prepare_stop(RequestId::new(), StopConditions::default())
        .await
        .unwrap();
    attempt1
        .finish_external_agent(RequestId::new(), resume1)
        .await
        .unwrap();

    // Turn 2: must verify the Turn 1 SHA-256 continuity signature and allow resume.
    let exec2 = runtime
        .ports
        .sessions
        .send(RequestId::new(), session, user_doc("Follow-up 2"))
        .await
        .unwrap()
        .value;
    let attempt2 = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, exec2.id)
        .await
        .unwrap()
        .value;
    let opened2 = attempt2
        .begin_external_agent_round(RequestId::new())
        .await
        .expect("second external agent turn over >8 MiB history must succeed")
        .value;
    let continuity2 = opened2.continuity.as_ref().unwrap();
    assert!(continuity2.has_prior_turns);
    assert!(
        continuity2.can_resume_previous,
        "Turn 2 must resume Turn 1 when no intervening history changed"
    );
    opened2
        .round
        .finish(RequestId::new(), None, true)
        .await
        .unwrap();
    let resume2 = ExternalAgentResume {
        session,
        cwd: cwd.clone(),
        round: opened2.round.id(),
        native_id: link1.native_id.clone(),
        incarnation: link1.incarnation,
        input: continuity2.resume_signature_structured,
        message: exec2.message_id,
    };
    attempt2
        .prepare_stop(RequestId::new(), StopConditions::default())
        .await
        .unwrap();
    attempt2
        .finish_external_agent(RequestId::new(), resume2)
        .await
        .unwrap();

    // Mutate history by withdrawing Turn 2's message, then send Turn 3:
    // continuity signature must reject resuming the diverged native session.
    runtime
        .ports
        .sessions
        .withdraw(RequestId::new(), session, exec2.message_id)
        .await
        .unwrap();
    let exec3 = runtime
        .ports
        .sessions
        .send(RequestId::new(), session, user_doc("Follow-up 3 after withdraw"))
        .await
        .unwrap()
        .value;
    let attempt3 = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, exec3.id)
        .await
        .unwrap()
        .value;
    let opened3 = attempt3
        .begin_external_agent_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let continuity3 = opened3.continuity.as_ref().unwrap();
    assert!(
        !continuity3.can_resume_previous,
        "withdrawing a prior turn must invalidate native resume continuity"
    );
    opened3
        .round
        .finish(RequestId::new(), None, true)
        .await
        .unwrap();
}
