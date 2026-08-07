//! Activity v2 persistence (07 §8).
//!
//! The only durable shape for an activity is [`ActivityStateNode`] whose
//! `raw_output` is serialized into `agena_content_nodes.payload_json` under
//! the `activity_v2` schema marker. `ViewBlock` projections are pure
//! functions and are never persisted. Terminal writes happen once per tool;
//! live deltas are broadcast in memory only (07 §8.1 budget: 0 writes while
//! streaming, O(1) label updates, one terminal upsert).

use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

use agena_domain::ActivityState;
use crate::{activity::ActivityStateNode, AppError};

/// One-shot terminal write for a finished activity (07 §8.2
/// `upsert_content_node`). Serializes the raw output into `payload_json`
/// with an `activity_v2` marker; compact `title` and `state` are mirrored
/// into their own columns for O(1) label queries. Keyed by `activity_id`;
/// the revision guard (`excluded.revision_seq >= current`) means the v2
/// terminal write always carries the newest revision and any later legacy
/// projection upsert for the same node is ignored.
pub(crate) async fn upsert_content_node(
    db: &DatabaseConnection,
    session_id: i64,
    node: &ActivityStateNode,
) -> Result<(), AppError> {
    let backend = db.get_database_backend();
    let revision_seq = db
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT COALESCE(MAX(revision_seq), 0) + 1 AS next_revision \
             FROM agena_content_nodes WHERE node_id = ?",
            [node.activity_id.to_string().into()],
        ))
        .await?
        .and_then(|row| row.try_get("", "next_revision").ok())
        .unwrap_or(1);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let state_label = activity_state_label(node.state);
    let finished_at_ms = node.state.is_terminal().then_some(now_ms);
    let payload = serde_json::json!({
        "schema": "activity_v2",
        "kind": node.kind,
        "raw_output": node.raw_output,
        "summary": node.summary,
    });
    db.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO agena_content_nodes \
         (node_id, owner_kind, owner_id, node_type, actor, title, payload_json, text, state, \
          position, revision_seq, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) \
         VALUES (?, 'session', ?, 'activity', 'tool', ?, ?, NULL, ?, 0, ?, ?, ?, ?, ?) \
         ON CONFLICT(node_id) DO UPDATE SET \
         title = excluded.title, \
         payload_json = excluded.payload_json, \
         state = excluded.state, \
         revision_seq = excluded.revision_seq, \
         finished_at_ms = excluded.finished_at_ms, \
         updated_at_ms = excluded.updated_at_ms \
         WHERE excluded.revision_seq >= agena_content_nodes.revision_seq",
        [
            node.activity_id.to_string().into(),
            session_id.into(),
            node.title.clone().into(),
            serde_json::to_value(payload)
                .map_err(|error| AppError::Internal(format!("encode activity v2 node: {error}")))?
                .into(),
            state_label.into(),
            revision_seq.into(),
            now_ms.into(),
            finished_at_ms.into(),
            now_ms.into(),
            now_ms.into(),
        ],
    ))
    .await?;
    Ok(())
}

/// O(1) compact label update (07 §8.2 `update_activity_label`): refreshes
/// the mirrored `title` column without touching the raw output payload. Used
/// by the 2s streaming title refresh so a long stream costs only tiny
/// column updates.
pub(crate) async fn update_activity_label(
    db: &DatabaseConnection,
    activity_id: agena_domain::ActivityId,
    title: &str,
) -> Result<(), AppError> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE agena_content_nodes SET title = ?, updated_at_ms = ? WHERE node_id = ?",
        [
            title.into(),
            chrono::Utc::now().timestamp_millis().into(),
            activity_id.to_string().into(),
        ],
    ))
    .await?;
    Ok(())
}

fn activity_state_label(state: ActivityState) -> &'static str {
    match state {
        ActivityState::Pending => "pending",
        ActivityState::InProgress => "in_progress",
        ActivityState::Completed => "completed",
        ActivityState::Failed => "failed",
        ActivityState::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use agena_domain::{ActivityId, RawOutput};
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection};

    use super::*;
    use crate::activity::{ActivityKind, ActivityStateNode};

    async fn db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("open in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("initialize schema");
        // The owner trigger requires a real session row for owner_kind='session'.
        let backend = db.get_database_backend();
        db.execute(sea_orm::Statement::from_sql_and_values(
            backend,
            "INSERT INTO agena_workspaces (id, path, created_at_ms, updated_at_ms)              VALUES (1, '/tmp/activity-v2-test', 1, 1)",
            [],
        ))
        .await
        .expect("seed workspace");
        db.execute(sea_orm::Statement::from_sql_and_values(
            backend,
            "INSERT INTO agena_sessions              (id, parent_id, depth, root_id, workspace_id, title, version, lifecycle_state,               created_at_ms, updated_at_ms)              VALUES (7, NULL, 0, 0, 1, 'test', 1, 'ready', 1, 1)",
            [],
        ))
        .await
        .expect("seed session");
        db
    }

    fn node(state: ActivityState) -> ActivityStateNode {
        ActivityStateNode {
            activity_id: ActivityId::new(),
            kind: ActivityKind::Operation,
            title: "cargo test".to_owned(),
            summary: "running tests".to_owned(),
            state,
            raw_output: Some(RawOutput::text("streamed output")),
            sections: Vec::new(),
        }
    }

    #[tokio::test]
    async fn upsert_writes_raw_output_with_schema_marker() {
        let db = db().await;
        let n = node(ActivityState::Completed);
        upsert_content_node(&db, 7, &n).await.expect("upsert succeeds");

        let row = db
            .query_one(sea_orm::Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT node_id, owner_kind, owner_id, title, state, payload_json, revision_seq \
                 FROM agena_content_nodes WHERE node_id = ?",
                [n.activity_id.to_string().into()],
            ))
            .await
            .expect("query succeeds")
            .expect("row exists");
        let owner_kind: String = row.try_get("", "owner_kind").unwrap();
        let owner_id: String = row.try_get("", "owner_id").unwrap();
        let title: String = row.try_get("", "title").unwrap();
        let state: String = row.try_get("", "state").unwrap();
        let revision_seq: i64 = row.try_get("", "revision_seq").unwrap();
        let payload: serde_json::Value = row.try_get("", "payload_json").unwrap();
        assert_eq!(owner_kind, "session");
        assert_eq!(owner_id, "7");
        assert_eq!(title, "cargo test");
        assert_eq!(state, "completed");
        assert!(revision_seq >= 1);
        assert_eq!(payload["schema"], "activity_v2");
        assert_eq!(payload["raw_output"]["text"], "streamed output");
    }

    #[tokio::test]
    async fn update_activity_label_refreshes_title_column() {
        let db = db().await;
        let n = node(ActivityState::InProgress);
        upsert_content_node(&db, 7, &n).await.expect("upsert succeeds");
        update_activity_label(&db, n.activity_id, "cargo test · 5s")
            .await
            .expect("label update succeeds");
        let row = db
            .query_one(sea_orm::Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT title FROM agena_content_nodes WHERE node_id = ?",
                [n.activity_id.to_string().into()],
            ))
            .await
            .expect("query succeeds")
            .expect("row exists");
        let title: String = row.try_get("", "title").unwrap();
        assert_eq!(title, "cargo test · 5s");
    }

    #[tokio::test]
    async fn terminal_upsert_wins_over_earlier_write() {
        let db = db().await;
        let mut n = node(ActivityState::Completed);
        upsert_content_node(&db, 7, &n).await.expect("first write succeeds");
        let first_revision = current_revision(&db, n.activity_id).await;

        // A later terminal write for the same node must win via the revision
        // guard (the v2 payload is the final word for the activity).
        n.title = "cargo test".to_owned();
        upsert_content_node(&db, 7, &n).await.expect("terminal write succeeds");

        let row = db
            .query_one(sea_orm::Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT title, state, payload_json, revision_seq FROM agena_content_nodes \
                 WHERE node_id = ?",
                [n.activity_id.to_string().into()],
            ))
            .await
            .expect("query succeeds")
            .expect("row exists");
        let title: String = row.try_get("", "title").unwrap();
        let state: String = row.try_get("", "state").unwrap();
        let revision_seq: i64 = row.try_get("", "revision_seq").unwrap();
        let payload: serde_json::Value = row.try_get("", "payload_json").unwrap();
        assert_eq!(title, "cargo test", "terminal title wins");
        assert_eq!(state, "completed");
        assert!(revision_seq > first_revision, "revision moves forward");
        assert_eq!(payload["schema"], "activity_v2");
    }

    async fn current_revision(db: &DatabaseConnection, activity_id: ActivityId) -> i64 {
        db.query_one(sea_orm::Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT revision_seq FROM agena_content_nodes WHERE node_id = ?",
            [activity_id.to_string().into()],
        ))
        .await
        .expect("query succeeds")
        .expect("row exists")
        .try_get("", "revision_seq")
        .unwrap()
    }
}
