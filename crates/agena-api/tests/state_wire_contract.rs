//! Wire vocabulary of the state types shared with every client.
//!
//! `agena-domain` owns the state enums, the API re-exports them, and the web
//! client consumes a generated mirror
//! (`packages/agena-web/src/generated/agenaState.ts`, guarded by
//! `cargo test -p agena-web-types`). These assertions close the remaining gap:
//! the REST projection must spell every state exactly like the canonical domain
//! type, so a renamed variant cannot reach a client as a second vocabulary.

use agena_api::part::PartExecutionStatusResource;
use agena_api::resource::{
    ActiveExecutionResource, ExecutionAccess, ExecutionPhase, RunStatus, SessionLifecycleState,
    SessionRelationKind, SessionState, SessionStateKind, SubtaskStatus, WorkflowState,
};

#[test]
fn api_state_enums_are_the_domain_enums() {
    // Plain assignments, not conversions: a locally redeclared variant list in
    // the API crate would fail to compile here.
    let kind: agena_domain::SessionStateKind = SessionStateKind::Ready;
    let lifecycle: agena_domain::SessionLifecycleState = SessionLifecycleState::Ready;
    let relation: agena_domain::SessionRelationKind = SessionRelationKind::Root;
    let subtask: agena_domain::SubtaskStatus = SubtaskStatus::Created;
    let workflow: agena_domain::WorkflowState = WorkflowState::Quiescent;
    let phase: agena_domain::ExecutionPhase = ExecutionPhase::Starting;
    let access: agena_domain::ExecutionAccess = ExecutionAccess::Inherit;
    let status: agena_domain::ExecutionStatus = RunStatus::Pending;
    let part_status: agena_domain::ExecutionStatus = PartExecutionStatusResource::Completed;

    assert_eq!(kind.as_str(), "ready");
    assert_eq!(lifecycle.as_str(), "ready");
    assert_eq!(relation.as_str(), "root");
    let subtask_wire: &str = subtask.as_ref();
    assert_eq!(subtask_wire, "created");
    assert_eq!(workflow, agena_domain::WorkflowState::Quiescent);
    assert_eq!(phase, agena_domain::ExecutionPhase::Starting);
    assert_eq!(access, agena_domain::ExecutionAccess::Inherit);
    assert_eq!(status, agena_domain::ExecutionStatus::Pending);
    assert_eq!(part_status, agena_domain::ExecutionStatus::Completed);
}

#[test]
fn every_session_state_kind_reaches_clients_with_its_domain_spelling() {
    let mut projected = Vec::new();
    for kind in SessionStateKind::ALL {
        let state = SessionState::from(kind);
        assert_eq!(state.kind(), kind);
        assert_eq!(state.as_str(), kind.as_str());
        let value = serde_json::to_value(&state).expect("serialize session state");
        assert_eq!(value["kind"], serde_json::json!(kind.as_str()));
        assert_eq!(
            SessionState::parse(kind.as_str()).map(|parsed| parsed.kind()),
            Some(kind)
        );
        assert_eq!(
            serde_json::from_value::<SessionState>(value).expect("decode session state"),
            state
        );
        projected.push(state.kind());
    }
    assert_eq!(projected, SessionStateKind::ALL.to_vec());
    assert_eq!(SessionState::parse("succeeded"), None);
}

#[test]
fn session_state_payloads_keep_their_wire_shape() {
    assert_eq!(
        serde_json::to_value(SessionState::Creating).expect("serialize creating"),
        serde_json::json!({ "kind": "creating" })
    );
    assert_eq!(
        serde_json::to_value(SessionState::Ready {
            last_failure: Some(serde_json::json!({ "code": "boom" })),
        })
        .expect("serialize ready"),
        serde_json::json!({
            "kind": "ready",
            "data": { "last_failure": { "code": "boom" } }
        })
    );

    let running = SessionState::Running {
        execution: Some(ActiveExecutionResource {
            execution_id: uuid::Uuid::nil(),
            phase: ExecutionPhase::ExecutingTools,
        }),
        workflow: WorkflowState::ToolPending,
        requests: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(&running).expect("serialize running"),
        serde_json::json!({
            "kind": "running",
            "data": {
                "execution": {
                    "execution_id": "00000000-0000-0000-0000-000000000000",
                    "phase": "executing_tools"
                },
                "workflow": "tool_pending"
            }
        })
    );
    assert!(running.is_busy());
    assert_eq!(running.workflow_state(), WorkflowState::ToolPending);
    assert!(running.active_execution().is_some());

    let awaiting = SessionState::AwaitingInteraction {
        run_id: Some(7),
        execution: None,
        requests: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(&awaiting).expect("serialize awaiting interaction"),
        serde_json::json!({ "kind": "awaiting_interaction", "data": { "run_id": 7 } })
    );
    assert!(awaiting.is_attention());
    assert!(!awaiting.is_busy());
    assert_eq!(
        awaiting.workflow_state(),
        WorkflowState::AwaitingInteraction
    );

    let failed = SessionState::Failed {
        failure: Some(serde_json::json!({ "code": "boom" })),
    };
    assert_eq!(
        serde_json::to_value(&failed).expect("serialize failed"),
        serde_json::json!({
            "kind": "failed",
            "data": { "failure": { "code": "boom" } }
        })
    );
    assert!(failed.is_failed());
}

#[test]
fn execution_status_wire_names_match_their_rust_spelling() {
    for status in [
        RunStatus::Pending,
        RunStatus::InProgress,
        RunStatus::Completed,
        RunStatus::PolicyDenied,
        RunStatus::UserDeclined,
        RunStatus::CapabilityUnavailable,
        RunStatus::ToolUnavailable,
        RunStatus::Failed,
        RunStatus::Cancelled,
    ] {
        let wire = status.to_string();
        assert_eq!(
            serde_json::to_value(status).expect("serialize execution status"),
            serde_json::json!(wire)
        );
        assert_eq!(
            wire.parse::<RunStatus>().expect("parse execution status"),
            status
        );
    }
    assert_eq!(RunStatus::default(), RunStatus::Pending);
    assert!(RunStatus::Completed.is_terminal());
    assert!(!RunStatus::InProgress.is_terminal());
}
