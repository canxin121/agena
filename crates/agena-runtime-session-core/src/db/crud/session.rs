#[cfg(test)]
use std::collections::HashMap;

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait,
    FromQueryResult,
};
#[cfg(test)]
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder, QuerySelect};
use sea_orm::{DatabaseConnection, TransactionTrait};

use crate::db::entities;
use crate::session::{SessionRuntimeState, SubtaskRuntimeState};
#[cfg(test)]
use agena_domain::MESSAGE_CREATED_EVENT_KIND_TAGS;
use agena_domain::{SessionLifecycleState, SessionRelationKind, SubtaskStatus};

/// Materialized session aggregate. The session row owns mutable session data;
/// the optional lineage row owns immutable provenance and delegated-task
/// lifecycle. Keeping the join here prevents domain/service code from
/// accidentally treating either table as a complete session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    pub lifecycle_state: SessionLifecycleState,
    pub creation_error: Option<String>,
    pub relation_kind: SessionRelationKind,
    pub source_cutoff_seq_global: Option<i64>,
    pub source_message_id: Option<i64>,
    pub task_id: Option<String>,
    pub subtask_status: Option<String>,
    pub subtask_started_at_ms: Option<i64>,
    pub subtask_finished_at_ms: Option<i64>,
    pub subtask_error: Option<String>,
    pub runtime_state: Option<SessionRuntimeState>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLineageInput {
    pub relation_kind: SessionRelationKind,
    pub source_cutoff_seq_global: Option<i64>,
    pub source_message_id: Option<i64>,
}

impl SessionLineageInput {
    pub const CHILD: Self = Self {
        relation_kind: SessionRelationKind::Child,
        source_cutoff_seq_global: None,
        source_message_id: None,
    };

    pub const fn fork(source_cutoff_seq_global: i64, source_message_id: Option<i64>) -> Self {
        Self {
            relation_kind: SessionRelationKind::Fork,
            source_cutoff_seq_global: Some(source_cutoff_seq_global),
            source_message_id,
        }
    }

    pub const fn rewind(source_cutoff_seq_global: i64, source_message_id: i64) -> Self {
        Self {
            relation_kind: SessionRelationKind::Rewind,
            source_cutoff_seq_global: Some(source_cutoff_seq_global),
            source_message_id: Some(source_message_id),
        }
    }

    pub const fn subagent() -> Self {
        Self {
            relation_kind: SessionRelationKind::Subagent,
            source_cutoff_seq_global: None,
            source_message_id: None,
        }
    }
}

fn materialize_record(
    model: entities::session::Model,
    lineage: Option<entities::session_lineage::Model>,
) -> Result<SessionRecord, DbErr> {
    let lifecycle_state =
        SessionLifecycleState::parse(model.lifecycle_state.as_str()).ok_or_else(|| {
            DbErr::Custom(format!(
                "session {} has invalid lifecycle state `{}`",
                model.id, model.lifecycle_state
            ))
        })?;
    let (
        relation_kind,
        source_cutoff_seq_global,
        source_message_id,
        task_id,
        subtask_status,
        subtask_started_at_ms,
        subtask_finished_at_ms,
        subtask_error,
    ) = match (model.parent_id, lineage) {
        (None, None) => (
            SessionRelationKind::Root,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        (None, Some(_)) => {
            return Err(DbErr::Custom(format!(
                "root session {} must not have a lineage row",
                model.id
            )));
        }
        (Some(_), None) => {
            return Err(DbErr::Custom(format!(
                "child session {} is missing its lineage row",
                model.id
            )));
        }
        (Some(_), Some(lineage)) => {
            let kind =
                SessionRelationKind::parse(lineage.relation_kind.as_str()).ok_or_else(|| {
                    DbErr::Custom(format!(
                        "session {} has invalid lineage kind `{}`",
                        model.id, lineage.relation_kind
                    ))
                })?;
            (
                kind,
                lineage.source_cutoff_seq_global,
                lineage.source_message_id,
                lineage.task_id,
                lineage.subtask_status,
                lineage.subtask_started_at_ms,
                lineage.subtask_finished_at_ms,
                lineage.subtask_error,
            )
        }
    };
    Ok(SessionRecord {
        id: model.id,
        parent_id: model.parent_id,
        depth: model.depth,
        root_id: model.root_id,
        workspace_id: model.workspace_id,
        title: model.title,
        version: model.version,
        lifecycle_state,
        creation_error: model.creation_error,
        relation_kind,
        source_cutoff_seq_global,
        source_message_id,
        task_id,
        subtask_status,
        subtask_started_at_ms,
        subtask_finished_at_ms,
        subtask_error,
        runtime_state: model.runtime_state,
        created_at_ms: model.created_at_ms,
        updated_at_ms: model.updated_at_ms,
    })
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionMessageStats {
    pub message_count: i64,
    pub last_message_at_ms: Option<i64>,
}

#[derive(Debug, Clone, FromQueryResult)]
struct SessionTouchRow {
    id: i64,
    version: i64,
    subtask_status: Option<String>,
    subtask_started_at_ms: Option<i64>,
    subtask_finished_at_ms: Option<i64>,
    subtask_error: Option<String>,
}

impl SessionTouchRow {
    fn subtask_state(&self) -> Result<SubtaskRuntimeState, DbErr> {
        let status = match self.subtask_status.as_deref() {
            Some(value) => SubtaskStatus::parse(value).ok_or_else(|| {
                DbErr::Custom(format!(
                    "session {} has invalid subtask status `{value}`",
                    self.id
                ))
            })?,
            None => SubtaskStatus::Created,
        };
        Ok(SubtaskRuntimeState {
            status,
            started_at_ms: self.subtask_started_at_ms,
            finished_at_ms: self.subtask_finished_at_ms,
            error: self.subtask_error.clone(),
        })
    }
}

async fn get_session_touch_row<C>(db: &C, session_id: i64) -> Result<Option<SessionTouchRow>, DbErr>
where
    C: ConnectionTrait,
{
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT s.id, s.version, \
                l.subtask_status, l.subtask_started_at_ms, l.subtask_finished_at_ms, l.subtask_error \
         FROM agena_sessions s \
         LEFT JOIN agena_session_lineage l ON l.session_id = s.id \
         WHERE s.id = ?",
        [session_id.into()],
    );
    db.query_one(stmt).await?.map_or(Ok(None), |row| {
        Ok(Some(SessionTouchRow {
            id: row.try_get("", "id")?,
            version: row.try_get("", "version")?,
            subtask_status: row.try_get("", "subtask_status")?,
            subtask_started_at_ms: row.try_get("", "subtask_started_at_ms")?,
            subtask_finished_at_ms: row.try_get("", "subtask_finished_at_ms")?,
            subtask_error: row.try_get("", "subtask_error")?,
        }))
    })
}

/// Lineage info needed when materialising a child session row.
#[derive(Debug, Clone, Copy)]
pub struct ParentLineage {
    pub depth: i64,
    pub root_id: i64,
}

/// Resolve the depth and root id of `parent_id`, used to compute the
/// child's `depth = parent.depth + 1` and `root_id = parent.root_id`.
pub async fn parent_lineage<C>(
    db: &C,
    parent_id: Option<i64>,
) -> Result<Option<ParentLineage>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(parent_id) = parent_id else {
        return Ok(None);
    };
    let parent = get_session_by_id(db, parent_id)
        .await?
        .ok_or_else(|| DbErr::Custom(format!("parent session not found: {parent_id}")))?;
    if parent.lifecycle_state != SessionLifecycleState::Ready {
        return Err(DbErr::Custom(format!(
            "parent session {parent_id} is not ready"
        )));
    }
    Ok(Some(ParentLineage {
        depth: parent.depth,
        root_id: parent.root_id,
    }))
}

pub async fn create_session(
    db: &DatabaseConnection,
    workspace_id: i64,
    parent_id: Option<i64>,
    title: impl Into<String>,
) -> Result<SessionRecord, DbErr> {
    let lineage = parent_id.map(|_| SessionLineageInput::CHILD);
    create_session_with_options(
        db,
        workspace_id,
        parent_id,
        title,
        lineage,
        None,
        SessionLifecycleState::Ready,
    )
    .await
}

/// Atomically create the session row and its immutable lineage. Callers that
/// already own a transaction use [`create_session_in_transaction`] so branch
/// history and lifecycle initialization share the same commit boundary.
pub async fn create_session_with_options(
    db: &DatabaseConnection,
    workspace_id: i64,
    parent_id: Option<i64>,
    title: impl Into<String>,
    lineage_input: Option<SessionLineageInput>,
    task_id: Option<String>,
    lifecycle_state: SessionLifecycleState,
) -> Result<SessionRecord, DbErr> {
    let txn = db.begin().await?;
    let result = create_session_in_transaction(
        &txn,
        workspace_id,
        parent_id,
        title,
        lineage_input,
        task_id,
        lifecycle_state,
    )
    .await;
    match result {
        Ok(record) => {
            txn.commit().await?;
            Ok(record)
        }
        Err(err) => {
            txn.rollback().await?;
            Err(err)
        }
    }
}

/// Create the session aggregate and its immutable lineage in one caller-owned
/// transaction. Roots have no lineage; every child must supply exactly one.
pub async fn create_session_in_transaction(
    db: &DatabaseTransaction,
    workspace_id: i64,
    parent_id: Option<i64>,
    title: impl Into<String>,
    lineage_input: Option<SessionLineageInput>,
    task_id: Option<String>,
    lifecycle_state: SessionLifecycleState,
) -> Result<SessionRecord, DbErr> {
    match (parent_id, lineage_input) {
        (None, None) => {}
        (Some(_), Some(_)) => {}
        (None, Some(_)) => {
            return Err(DbErr::Custom(
                "root session cannot have lineage provenance".to_owned(),
            ));
        }
        (Some(_), None) => {
            return Err(DbErr::Custom(
                "child session requires lineage provenance".to_owned(),
            ));
        }
    }
    let is_subagent = lineage_input.is_some_and(|input| input.relation_kind.is_subagent());
    if is_subagent != task_id.is_some() {
        return Err(DbErr::Custom(
            "only subagent sessions require a task id".to_owned(),
        ));
    }
    let lineage = parent_lineage(db, parent_id).await?;
    let now_ms = Utc::now().timestamp_millis();
    let depth = lineage.map(|l| l.depth + 1).unwrap_or(0);
    let initial_root = lineage.map(|l| l.root_id).unwrap_or(0); // 0 = "self" placeholder
    let inserted = entities::session::ActiveModel {
        parent_id: Set(parent_id),
        depth: Set(depth),
        root_id: Set(initial_root),
        workspace_id: Set(workspace_id),
        title: Set(title.into()),
        version: Set(1),
        lifecycle_state: Set(lifecycle_state.as_str().to_owned()),
        creation_error: Set(None),
        runtime_state: Set(Some(SessionRuntimeState::default())),
        created_at_ms: Set(now_ms),
        updated_at_ms: Set(now_ms),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let inserted = if lineage.is_none() {
        // Root: rewrite root_id = id now that the auto-increment id is known.
        let mut active: entities::session::ActiveModel = inserted.clone().into();
        active.root_id = Set(inserted.id);
        active.update(db).await?
    } else {
        inserted
    };

    if let Some(lineage_input) = lineage_input {
        entities::session_lineage::ActiveModel {
            session_id: Set(inserted.id),
            relation_kind: Set(lineage_input.relation_kind.as_str().to_owned()),
            source_cutoff_seq_global: Set(lineage_input.source_cutoff_seq_global),
            source_message_id: Set(lineage_input.source_message_id),
            task_id: Set(task_id),
            subtask_status: Set(is_subagent.then(|| SubtaskStatus::Created.as_ref().to_owned())),
            subtask_started_at_ms: Set(None),
            subtask_finished_at_ms: Set(None),
            subtask_error: Set(None),
            created_at_ms: Set(now_ms),
        }
        .insert(db)
        .await?;
    }

    get_session_by_id(db, inserted.id)
        .await?
        .ok_or_else(|| DbErr::Custom(format!("created session disappeared: {}", inserted.id)))
}

pub async fn get_session_by_id<C>(db: &C, session_id: i64) -> Result<Option<SessionRecord>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(model) = entities::session::Entity::find_by_id(session_id)
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let lineage = entities::session_lineage::Entity::find_by_id(session_id)
        .one(db)
        .await?;
    materialize_record(model, lineage).map(Some)
}

#[cfg(test)]
pub async fn get_subagent_by_task_id<C>(
    db: &C,
    parent_session_id: i64,
    task_id: &str,
) -> Result<Option<SessionRecord>, DbErr>
where
    C: ConnectionTrait,
{
    let ids = entities::session_lineage::Entity::find()
        .select_only()
        .column(entities::session_lineage::Column::SessionId)
        .filter(entities::session_lineage::Column::RelationKind.eq("subagent"))
        .filter(entities::session_lineage::Column::TaskId.eq(task_id))
        .into_tuple::<i64>()
        .all(db)
        .await?;
    let Some(model) = entities::session::Entity::find()
        .filter(entities::session::Column::Id.is_in(ids))
        .filter(entities::session::Column::ParentId.eq(parent_session_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let lineage = entities::session_lineage::Entity::find_by_id(model.id)
        .one(db)
        .await?;
    materialize_record(model, lineage).map(Some)
}

/// Outcome of a [`touch_session_updated_at`] attempt.
#[derive(Debug)]
pub enum TouchOutcome {
    /// Session row updated; carries the post-update model.
    Updated(Box<SessionRecord>),
    /// Session id does not exist.
    NotFound,
    /// Caller passed `expected_version` and it did not match the current
    /// row — another writer raced ahead. Caller should reload + retry.
    VersionConflict,
}

pub async fn touch_session_updated_at<C>(
    db: &C,
    session_id: i64,
    runtime_state: SessionRuntimeState,
) -> Result<Option<SessionRecord>, DbErr>
where
    C: ConnectionTrait,
{
    match touch_session_with_version(db, session_id, runtime_state, None).await? {
        TouchOutcome::Updated(model) => Ok(Some(*model)),
        TouchOutcome::NotFound => Ok(None),
        TouchOutcome::VersionConflict => {
            unreachable!("expected_version=None can never produce a VersionConflict outcome")
        }
    }
}

/// Same as [`touch_session_updated_at`] but with an optional optimistic-lock
/// check. When `expected_version` is `Some(v)`, the underlying UPDATE adds
/// `WHERE version = v` and the function reports a [`TouchOutcome::VersionConflict`]
/// if zero rows change. The new row's `version` is always `existing + 1`.
pub async fn touch_session_with_version<C>(
    db: &C,
    session_id: i64,
    mut runtime_state: SessionRuntimeState,
    expected_version: Option<i64>,
) -> Result<TouchOutcome, DbErr>
where
    C: ConnectionTrait,
{
    let Some(existing) = get_session_touch_row(db, session_id).await? else {
        return Ok(TouchOutcome::NotFound);
    };
    if let Some(expected) = expected_version
        && existing.version != expected
    {
        return Ok(TouchOutcome::VersionConflict);
    }
    let next_version = existing.version + 1;
    let now_ms = Utc::now().timestamp_millis();
    runtime_state.subtask = existing.subtask_state()?;
    let runtime_value = serde_json::to_value(&runtime_state)
        .map_err(|err| DbErr::Custom(format!("serialize runtime_state: {err}")))?;

    use sea_orm::Statement;
    let backend = db.get_database_backend();
    let stmt = if let Some(expected) = expected_version {
        Statement::from_sql_and_values(
            backend,
            "UPDATE agena_sessions SET version = ?, runtime_state_json = ?, updated_at_ms = ? \
             WHERE id = ? AND version = ?",
            [
                next_version.into(),
                runtime_value.into(),
                now_ms.into(),
                session_id.into(),
                expected.into(),
            ],
        )
    } else {
        Statement::from_sql_and_values(
            backend,
            "UPDATE agena_sessions SET version = ?, runtime_state_json = ?, updated_at_ms = ? \
             WHERE id = ?",
            [
                next_version.into(),
                runtime_value.into(),
                now_ms.into(),
                session_id.into(),
            ],
        )
    };

    let exec_result = db.execute(stmt).await?;
    if exec_result.rows_affected() == 0 {
        // Reload to surface the now-current version. Could only happen with
        // `expected_version` (we just confirmed the row exists above).
        if let Some(latest) = get_session_touch_row(db, session_id).await? {
            let _ = latest;
            return Ok(TouchOutcome::VersionConflict);
        }
        return Ok(TouchOutcome::NotFound);
    }

    let updated = get_session_by_id(db, session_id)
        .await?
        .ok_or_else(|| DbErr::Custom(format!("session not found after update: {session_id}")))?;
    Ok(TouchOutcome::Updated(Box::new(updated)))
}

pub async fn update_subtask_state<C>(
    db: &C,
    session_id: i64,
    state: SubtaskRuntimeState,
) -> Result<Option<SessionRecord>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(existing) = get_session_by_id(db, session_id).await? else {
        return Ok(None);
    };
    if !existing.relation_kind.is_subagent() {
        return Err(DbErr::Custom(format!(
            "session {session_id} is not a delegated subtask"
        )));
    }
    // Lifecycle columns are authoritative and updated independently. Do not
    // rewrite runtime_state_json here: timeout/cancellation can race a final
    // provider write, and replacing the entire JSON document would discard
    // unrelated execution state. Materialization overlays these columns onto
    // runtime state on every read.
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE agena_session_lineage SET subtask_status = ?, \
         subtask_started_at_ms = ?, subtask_finished_at_ms = ?, subtask_error = ? \
         WHERE session_id = ? AND relation_kind = 'subagent'",
        [
            state.status.as_ref().to_string().into(),
            state.started_at_ms.into(),
            state.finished_at_ms.into(),
            state.error.into(),
            session_id.into(),
        ],
    );
    if db.execute(stmt).await?.rows_affected() == 0 {
        return Ok(None);
    }
    let touch = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE agena_sessions SET version = version + 1, updated_at_ms = ? WHERE id = ?",
        [Utc::now().timestamp_millis().into(), session_id.into()],
    );
    db.execute(touch).await?;
    get_session_by_id(db, session_id).await
}

pub async fn set_session_lifecycle<C>(
    db: &C,
    session_id: i64,
    lifecycle_state: SessionLifecycleState,
    creation_error: Option<String>,
) -> Result<Option<SessionRecord>, DbErr>
where
    C: ConnectionTrait,
{
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE agena_sessions SET lifecycle_state = ?, creation_error = ?, \
         version = version + 1, updated_at_ms = ? WHERE id = ?",
        [
            lifecycle_state.as_str().to_owned().into(),
            creation_error.into(),
            Utc::now().timestamp_millis().into(),
            session_id.into(),
        ],
    );
    if db.execute(stmt).await?.rows_affected() == 0 {
        return Ok(None);
    }
    get_session_by_id(db, session_id).await
}

#[cfg(test)]
pub async fn delete_session_by_id<C>(db: &C, session_id: i64) -> Result<u64, DbErr>
where
    C: ConnectionTrait,
{
    // Session hierarchy, lineage, events, permissions, projections and all
    // descendants are one foreign-key ownership graph. There is deliberately
    // no second deletion implementation and no audit-orphan special case.
    let deleted = entities::session::Entity::delete_by_id(session_id)
        .exec(db)
        .await?;
    Ok(deleted.rows_affected)
}

#[cfg(test)]
pub async fn list_session_ids_by_workspace_id<C>(
    db: &C,
    workspace_id: i64,
) -> Result<Vec<i64>, DbErr>
where
    C: ConnectionTrait,
{
    entities::session::Entity::find()
        .select_only()
        .column(entities::session::Column::Id)
        .filter(entities::session::Column::WorkspaceId.eq(workspace_id))
        .filter(entities::session::Column::LifecycleState.eq("ready"))
        .order_by_desc(entities::session::Column::UpdatedAtMs)
        .order_by_desc(entities::session::Column::Id)
        .into_tuple::<i64>()
        .all(db)
        .await
}

#[cfg(test)]
#[derive(Debug, Clone, FromQueryResult)]
struct SessionEventStatsRow {
    session_id: i64,
    message_count: i64,
    last_message_at_ms: Option<i64>,
}

/// Per-session visible-message stats computed from the unified event log in a
/// single grouped query. Only events that create a message are counted; tool
/// lifecycle events update an existing assistant message and are excluded.
#[cfg(test)]
pub async fn session_event_stats_for_ids<C>(
    db: &C,
    session_ids: &[i64],
) -> Result<HashMap<i64, SessionMessageStats>, DbErr>
where
    C: ConnectionTrait,
{
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = crate::db::event_entity::Entity::find()
        .select_only()
        .column_as(crate::db::event_entity::Column::SessionId, "session_id")
        .column_as(crate::db::event_entity::Column::Id.count(), "message_count")
        .column_as(
            crate::db::event_entity::Column::CreatedAtMs.max(),
            "last_message_at_ms",
        )
        .filter(crate::db::event_entity::Column::SessionId.is_in(session_ids.iter().copied()))
        .filter(
            crate::db::event_entity::Column::KindTag
                .is_in(MESSAGE_CREATED_EVENT_KIND_TAGS.iter().copied()),
        )
        .group_by(crate::db::event_entity::Column::SessionId)
        .into_model::<SessionEventStatsRow>()
        .all(db)
        .await?;

    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        out.insert(
            row.session_id,
            SessionMessageStats {
                message_count: row.message_count,
                last_message_at_ms: row.last_message_at_ms,
            },
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agena_storage::WorkspaceRepository;
    use sea_orm::{
        ActiveModelTrait, ActiveValue::Set, Database, EntityTrait, PaginatorTrait, Statement,
    };

    use super::*;

    async fn insert_event(
        db: &sea_orm::DatabaseConnection,
        session_id: i64,
        workspace_id: i64,
        seq: i64,
        kind_tag: &str,
        created_at_ms: i64,
    ) {
        crate::db::event_entity::ActiveModel {
            event_uuid: Set(format!("test-event-{seq}")),
            seq_global: Set(seq),
            seq_session: Set(Some(seq)),
            session_id: Set(Some(session_id)),
            workspace_id: Set(Some(workspace_id)),
            kind_tag: Set(kind_tag.to_string()),
            envelope_schema: Set(1),
            payload: Set(serde_json::json!({})),
            created_at_ms: Set(created_at_ms),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert test event");
    }

    #[tokio::test]
    async fn hierarchy_and_entity_ownership_are_database_invariants() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test")
            .await
            .expect("workspace");
        let parent = create_session(&db, workspace_id, None, "Parent")
            .await
            .expect("parent");
        let child = create_session(&db, workspace_id, Some(parent.id), "Child")
            .await
            .expect("child");
        assert_eq!(child.relation_kind, SessionRelationKind::Child);
        assert_eq!(child.root_id, parent.id);
        assert_eq!(child.depth, 1);

        let other_workspace_id =
            agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
                .ensure_id("/other-workspace")
                .await
                .expect("other workspace");
        let mismatched_event = crate::db::event_entity::ActiveModel {
            event_uuid: Set("mismatched-workspace-event".to_owned()),
            seq_global: Set(1),
            seq_session: Set(Some(1)),
            session_id: Set(Some(child.id)),
            workspace_id: Set(Some(other_workspace_id)),
            kind_tag: Set("user_message_appended".to_owned()),
            envelope_schema: Set(1),
            payload: Set(serde_json::json!({})),
            created_at_ms: Set(1),
            ..Default::default()
        }
        .insert(&db)
        .await;
        assert!(
            mismatched_event.is_err(),
            "session event workspace ownership must be enforced"
        );

        let reparent = db
            .execute(Statement::from_sql_and_values(
                db.get_database_backend(),
                "UPDATE agena_sessions SET parent_id = NULL WHERE id = ?",
                [child.id.into()],
            ))
            .await;
        assert!(reparent.is_err(), "database must reject hierarchy mutation");

        insert_event(&db, child.id, workspace_id, 1, "user_message_appended", 1).await;
        entities::activity_message::ActiveModel {
            message_id: Set(10),
            session_id: Set(child.id),
            turn_id: Set(Some(10)),
            execution_id: Set(None),
            run_id: Set(None),
            role: Set(agena_storage_sqlite::StoredRole::User),
            state: Set(agena_storage_sqlite::StoredExecutionStatus::Completed),
            created_at_ms: Set(1),
            updated_at_ms: Set(1),
            metadata: Set(crate::message::MessageMetadata {
                turn_id: Some(10),
                ..Default::default()
            }),
            provider_state: Set(None),
            usage: Set(None),
            part_count: Set(1),
            is_hidden: Set(false),
        }
        .insert(&db)
        .await
        .expect("message projection");
        entities::activity_part::ActiveModel {
            part_id: Set(11),
            message_id: Set(10),
            part_index: Set(0),
            status: Set(agena_storage_sqlite::StoredExecutionStatus::Completed),
            kind: Set(agena_storage_sqlite::StoredPartKind::Text),
            name: Set(None),
            summary: Set(None),
            has_detail: Set(false),
            operation_id: Set(None),
            created_at_ms: Set(1),
            content: Set(None),
        }
        .insert(&db)
        .await
        .expect("part projection");

        assert_eq!(
            delete_session_by_id(&db, parent.id).await.expect("delete"),
            1
        );
        assert_eq!(
            entities::session::Entity::find().count(&db).await.unwrap(),
            0
        );
        assert_eq!(
            entities::session_lineage::Entity::find()
                .count(&db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            crate::db::event_entity::Entity::find()
                .count(&db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            entities::activity_message::Entity::find()
                .count(&db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            entities::activity_part::Entity::find()
                .count(&db)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn incomplete_branches_are_hidden_until_activated() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test")
            .await
            .expect("workspace");
        let parent = create_session(&db, workspace_id, None, "Parent")
            .await
            .expect("parent");
        let txn = db.begin().await.expect("branch transaction");
        let branch = create_session_in_transaction(
            &txn,
            workspace_id,
            Some(parent.id),
            "Fork",
            Some(SessionLineageInput::fork(42, Some(7))),
            None,
            SessionLifecycleState::Creating,
        )
        .await
        .expect("creating branch");
        txn.commit().await.expect("commit branch");

        assert_eq!(branch.relation_kind, SessionRelationKind::Fork);
        assert_eq!(branch.source_cutoff_seq_global, Some(42));
        assert_eq!(branch.source_message_id, Some(7));
        assert!(
            !list_session_ids_by_workspace_id(&db, workspace_id)
                .await
                .expect("visible sessions")
                .contains(&branch.id)
        );

        assert!(
            set_session_lifecycle(&db, branch.id, SessionLifecycleState::Failed, None)
                .await
                .is_err(),
            "a failed branch must retain a non-empty creation error"
        );

        let ready = set_session_lifecycle(&db, branch.id, SessionLifecycleState::Ready, None)
            .await
            .expect("activate")
            .expect("branch exists");
        assert_eq!(ready.lifecycle_state, SessionLifecycleState::Ready);
        assert!(
            list_session_ids_by_workspace_id(&db, workspace_id)
                .await
                .expect("visible sessions")
                .contains(&branch.id)
        );
    }

    #[tokio::test]
    async fn message_stats_exclude_events_that_only_update_existing_messages() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("open test database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("initialize test schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/workspace")
            .await
            .expect("create test workspace");
        let session = create_session(&db, workspace_id, None, "Counted session")
            .await
            .expect("create test session");

        for (seq, kind_tag, created_at_ms) in [
            (1, "user_message_appended", 1_000),
            (2, "tool_call_completed", 2_000),
            (3, "assistant_message_finished", 3_000),
            (4, "run_completed", 4_000),
            (5, "system_notice_appended", 5_000),
        ] {
            insert_event(&db, session.id, workspace_id, seq, kind_tag, created_at_ms).await;
        }

        let stats = session_event_stats_for_ids(&db, &[session.id, session.id + 1])
            .await
            .expect("load message stats");

        assert_eq!(
            stats.get(&session.id),
            Some(&SessionMessageStats {
                message_count: 3,
                last_message_at_ms: Some(5_000),
            })
        );
        assert!(!stats.contains_key(&(session.id + 1)));
        assert!(
            session_event_stats_for_ids(&db, &[])
                .await
                .expect("load empty message stats")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn delegated_task_identity_is_indexed_and_unique_per_parent() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("open test database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("initialize test schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/tasks")
            .await
            .expect("create test workspace");
        let parent = create_session(&db, workspace_id, None, "Parent")
            .await
            .expect("create parent");
        let child = create_session_with_options(
            &db,
            workspace_id,
            Some(parent.id),
            "Child",
            Some(SessionLineageInput::subagent()),
            Some("stable-task".to_string()),
            SessionLifecycleState::Ready,
        )
        .await
        .expect("create child");

        let loaded = get_subagent_by_task_id(&db, parent.id, "stable-task")
            .await
            .expect("lookup child")
            .expect("child exists");
        assert_eq!(loaded.id, child.id);
        assert_eq!(loaded.task_id.as_deref(), Some("stable-task"));

        let count_before_duplicate = entities::session::Entity::find()
            .count(&db)
            .await
            .expect("count sessions before duplicate");
        let duplicate = create_session_with_options(
            &db,
            workspace_id,
            Some(parent.id),
            "Duplicate",
            Some(SessionLineageInput::subagent()),
            Some("stable-task".to_string()),
            SessionLifecycleState::Ready,
        )
        .await;
        assert!(duplicate.is_err());
        assert_eq!(
            entities::session::Entity::find()
                .count(&db)
                .await
                .expect("count sessions after duplicate"),
            count_before_duplicate,
            "a failed aggregate creation must not leave a session without lineage"
        );

        let other_parent = create_session(&db, workspace_id, None, "Other parent")
            .await
            .expect("create other parent");
        assert!(
            create_session_with_options(
                &db,
                workspace_id,
                Some(other_parent.id),
                "Same task under another parent",
                Some(SessionLineageInput::subagent()),
                Some("stable-task".to_string()),
                SessionLifecycleState::Ready,
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn late_runtime_write_cannot_revert_terminal_subtask_state() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("open test database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("initialize test schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/task-status")
            .await
            .expect("create test workspace");
        let parent = create_session(&db, workspace_id, None, "Parent")
            .await
            .expect("create parent");
        let child = create_session_with_options(
            &db,
            workspace_id,
            Some(parent.id),
            "Child",
            Some(SessionLineageInput::subagent()),
            Some("timeout-task".to_string()),
            SessionLifecycleState::Ready,
        )
        .await
        .expect("create child");
        let terminal = SubtaskRuntimeState {
            status: SubtaskStatus::TimedOut,
            started_at_ms: Some(10),
            finished_at_ms: Some(20),
            error: Some("deadline exceeded".to_string()),
        };
        assert!(
            update_subtask_state(
                &db,
                child.id,
                SubtaskRuntimeState {
                    status: SubtaskStatus::Completed,
                    started_at_ms: None,
                    finished_at_ms: None,
                    error: None,
                },
            )
            .await
            .is_err(),
            "terminal subtask state must have a valid time range"
        );
        update_subtask_state(&db, child.id, terminal.clone())
            .await
            .expect("persist timeout");

        let stale_runtime = SessionRuntimeState {
            subtask: SubtaskRuntimeState {
                status: SubtaskStatus::Running,
                started_at_ms: Some(10),
                finished_at_ms: None,
                error: None,
            },
            ..SessionRuntimeState::default()
        };
        let updated = touch_session_updated_at(&db, child.id, stale_runtime)
            .await
            .expect("persist late execution update")
            .expect("child exists");

        assert_eq!(updated.subtask_status.as_deref(), Some("timed_out"));
        assert_eq!(updated.runtime_state.expect("runtime").subtask, terminal);
    }
}
