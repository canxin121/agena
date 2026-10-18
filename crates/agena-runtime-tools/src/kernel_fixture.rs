//! Each capture fixture owns an isolated new-schema database and workspace.

use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

pub(crate) struct Fixture {
    pub runtime: agena_storage_sqlite::SqliteKernel,
    pub session: SessionId,
    pub _round: RoundHandle,
    pub tool: ToolHandle,
    pub _root: tempfile::TempDir,
}

pub(crate) async fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    runtime
        .database
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agena_workspaces (id,path,created_at_ms,updated_at_ms) VALUES (1,?,0,0)",
            [root.path().to_string_lossy().into_owned().into()],
        ))
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), 1, "capture fixture".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "capture".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = runtime
        .ports
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "capture".into(),
                name: "fixture".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    Fixture {
        runtime,
        session,
        _round: round,
        tool,
        _root: root,
    }
}

pub(crate) async fn complete(tool: &ToolHandle) {
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({}),
        },
    )
    .await
    .unwrap();
}
