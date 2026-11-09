use agena_domain::{ComposerDocument, ComposerNode, SessionRelationKind};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use sea_orm::{ConnectionTrait, Statement};

async fn sql(rt: &SqliteKernel, query: &str) {
    rt.database
        .execute(Statement::from_string(
            rt.database.get_database_backend(),
            query.to_owned(),
        ))
        .await
        .unwrap();
}
async fn number(rt: &SqliteKernel, query: &str) -> i64 {
    rt.database
        .query_one(Statement::from_string(
            rt.database.get_database_backend(),
            query.to_owned(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "n")
        .unwrap()
}
async fn guard(rt: &SqliteKernel, id: SessionId) -> DetachGlobalGuard {
    let row = rt
        .database
        .query_one(Statement::from_string(
            rt.database.get_database_backend(),
            format!(
                "SELECT metadata_version,control_version FROM agena_sessions WHERE id={}",
                id.get()
            ),
        ))
        .await
        .unwrap()
        .unwrap();
    DetachGlobalGuard {
        metadata_version: row.try_get("", "metadata_version").unwrap(),
        control_version: row.try_get("", "control_version").unwrap(),
    }
}
#[tokio::test]
async fn global_creation_uses_no_project() {
    let rt = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let global = rt
        .ports
        .sessions
        .create_global(RequestId::new(), "Bot".into(), Default::default())
        .await
        .unwrap()
        .value;
    assert_eq!(global.workspace_id, None);
    let project_list = rt
        .ports
        .reader
        .query_summaries(SummaryQuery::default())
        .await
        .unwrap();
    assert!(project_list.sessions.is_empty());
    let all = rt
        .ports
        .reader
        .query_summaries(SummaryQuery {
            include_global: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all.workspace_id, None);
    assert_eq!(all.sessions.len(), 1);
    assert_eq!(all.sessions[0].workspace_id, None);
    rt.ports
        .sessions
        .send(
            RequestId::new(),
            global.id,
            ComposerDocument(vec![ComposerNode::Text {
                text: "global request".into(),
            }]),
        )
        .await
        .unwrap();
    assert_eq!(
        number(&rt, "SELECT count(*) AS n FROM agena_workspaces").await,
        0
    );
}
#[tokio::test]
async fn detach_guards_all_descendants_and_preserves_source_scope() {
    let rt = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = rt.workspaces.ensure_id("/project").await.unwrap();
    let root = rt
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Bot".into(),
            SessionConfiguration {
                effective_workspace_root: Some("/project".into()),
                message_queue_paused: true,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let child = rt
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "child".into(),
            SessionConfiguration {
                parent_session_id: Some(root),
                relation_kind: SessionRelationKind::Child,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let other = rt
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "normal project".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    sql(&rt, &format!("INSERT INTO agena_assistant_reminders VALUES('ours',{workspace},{},'one','delivering',1,10,0,'{{\"workspace_id\":{workspace}}}'),('other',{workspace},{},'two','active',1,10,0,'{{\"workspace_id\":{workspace}}}')", child.get(), other.get())).await;
    let result = rt
        .ports
        .sessions
        .detach_global(RequestId::new(), root, guard(&rt, root).await)
        .await;
    assert!(result.is_err());
    assert_eq!(
        number(
            &rt,
            "SELECT count(*) AS n FROM agena_sessions WHERE workspace_id IS NULL"
        )
        .await,
        0
    );
    sql(
        &rt,
        "UPDATE agena_assistant_reminders SET state='active' WHERE id='ours'",
    )
    .await;
    let request = RequestId::new();
    let expected_guard = guard(&rt, root).await;
    let receipt = rt
        .ports
        .sessions
        .detach_global(request, root, expected_guard.clone())
        .await
        .unwrap()
        .value;
    let replay = rt
        .ports
        .sessions
        .detach_global(request, root, expected_guard)
        .await
        .unwrap()
        .value;
    assert_eq!(receipt, replay);
    assert_eq!(number(&rt, &format!("SELECT count(*) AS n FROM agena_sessions WHERE id={} AND json_type(configuration_json,'$.effective_workspace_root')='null' AND json_extract(configuration_json,'$.message_queue_paused')=1", root.get())).await, 1);
    assert_eq!(
        (
            receipt.affected_sessions,
            receipt.changed_sessions,
            receipt.changed_reminders
        ),
        (2, 2, 1)
    );
    assert_eq!(
        number(
            &rt,
            &format!(
                "SELECT count(*) AS n FROM agena_sessions WHERE id={} AND workspace_id={workspace}",
                other.get()
            )
        )
        .await,
        1
    );
    assert_eq!(number(&rt, "SELECT count(*) AS n FROM agena_assistant_reminders WHERE id='ours' AND workspace_id IS NULL AND json_type(reminder_json,'$.workspace_id')='null'").await, 1);
    assert_eq!(number(&rt, "SELECT count(*) AS n FROM agena_assistant_reminders WHERE id='other' AND workspace_id IS NOT NULL").await, 1);
}
#[tokio::test]
async fn queued_descendant_rejects_whole_tree() {
    let rt = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = rt.workspaces.ensure_id("/project").await.unwrap();
    let root = rt
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Bot".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let child = rt
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "child".into(),
            SessionConfiguration {
                parent_session_id: Some(root),
                relation_kind: SessionRelationKind::Child,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    rt.ports
        .sessions
        .send(
            RequestId::new(),
            child,
            ComposerDocument(vec![ComposerNode::Text {
                text: "pending".into(),
            }]),
        )
        .await
        .unwrap();
    assert!(
        rt.ports
            .sessions
            .detach_global(RequestId::new(), root, guard(&rt, root).await)
            .await
            .is_err()
    );
    assert_eq!(
        number(
            &rt,
            "SELECT count(*) AS n FROM agena_sessions WHERE workspace_id IS NULL"
        )
        .await,
        0
    );
}
#[tokio::test]
async fn deleted_registry_root_still_detaches_its_live_descendant() {
    let rt = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = rt.workspaces.ensure_id("/project").await.unwrap();
    let root = rt
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "retired Bot".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let child = rt
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "child".into(),
            SessionConfiguration {
                parent_session_id: Some(root),
                relation_kind: SessionRelationKind::Child,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    assert!(
        rt.ports
            .sessions
            .detach_global(RequestId::new(), root, DetachGlobalGuard::tombstone())
            .await
            .is_err()
    );
    sql(
        &rt,
        &format!(
            "UPDATE agena_sessions SET deleted_at_ms=updated_at_ms WHERE id={}",
            root.get()
        ),
    )
    .await;
    let receipt = rt
        .ports
        .sessions
        .detach_global(RequestId::new(), root, DetachGlobalGuard::tombstone())
        .await
        .unwrap()
        .value;
    assert_eq!(receipt.affected_sessions, 2);
    assert_eq!(
        number(
            &rt,
            &format!(
                "SELECT count(*) AS n FROM agena_sessions WHERE id={} AND workspace_id IS NULL",
                child.get()
            )
        )
        .await,
        1
    );
}

#[tokio::test]
async fn global_session_and_queued_history_survive_restart_without_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("global.db").display()
    );
    let rt = open_kernel_runtime(&url).await.unwrap();
    let global = rt
        .ports
        .sessions
        .create_global(RequestId::new(), "Bot".into(), Default::default())
        .await
        .unwrap()
        .value
        .id;
    rt.ports
        .sessions
        .send(
            RequestId::new(),
            global,
            ComposerDocument(vec![ComposerNode::Text {
                text: "preserved".into(),
            }]),
        )
        .await
        .unwrap();
    rt.ports.clone().shutdown().await.unwrap();
    drop(rt);
    let rt = open_kernel_runtime(&url).await.unwrap();
    assert_eq!(
        number(&rt, "SELECT count(*) AS n FROM agena_workspaces").await,
        0
    );
    assert_eq!(
        number(
            &rt,
            &format!(
                "SELECT count(*) AS n FROM agena_sessions WHERE id={} AND workspace_id IS NULL",
                global.get()
            )
        )
        .await,
        1
    );
    assert_eq!(
        number(
            &rt,
            &format!(
                "SELECT count(*) AS n FROM agena_entries WHERE origin_session_id={}",
                global.get()
            )
        )
        .await,
        1
    );
}

#[tokio::test]
async fn pending_interaction_keeps_the_entire_tree_in_project_scope() {
    let rt = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = rt.workspaces.ensure_id("/project").await.unwrap();
    let root = rt
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Bot".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let child = rt
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "child".into(),
            SessionConfiguration {
                parent_session_id: Some(root),
                relation_kind: SessionRelationKind::Child,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let execution = rt
        .ports
        .sessions
        .send(
            RequestId::new(),
            child,
            ComposerDocument(vec![ComposerNode::Text {
                text: "question".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = rt
        .ports
        .executor
        .claim_next(RequestId::new(), child, execution.id)
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
                call_id: "ask".into(),
                name: "ask".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.request_interaction(
        RequestId::new(),
        InteractionRequest::Input {
            request: agena_domain::UserInputRequest {
                request_id: "ask".into(),
                session_id: Some(child.get()),
                title: "Question".into(),
                body_markdown: String::new(),
                kind: agena_domain::UserInputKind::AskUser,
                source: agena_domain::UserInputSource::Host,
                auto_resolution_ms: None,
                presented_at: None,
                questions: Vec::new(),
                created_at: chrono::Utc::now(),
            },
        },
    )
    .await
    .unwrap();
    assert!(
        rt.ports
            .sessions
            .detach_global(RequestId::new(), root, guard(&rt, root).await)
            .await
            .is_err()
    );
    assert_eq!(
        number(
            &rt,
            "SELECT count(*) AS n FROM agena_sessions WHERE workspace_id IS NULL"
        )
        .await,
        0
    );
}
