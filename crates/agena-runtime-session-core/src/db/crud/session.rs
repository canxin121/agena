use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait,
    FromQueryResult,
};
use sea_orm::{DatabaseConnection, TransactionTrait};

use crate::db::entities;
use crate::session::{SessionRuntimeState, SubtaskRuntimeState};
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
    pub creation_failure: Option<agena_failure::Failure>,
    pub relation_kind: SessionRelationKind,
    pub source_cutoff_seq_global: Option<i64>,
    pub source_message_id: Option<i64>,
    pub task_id: Option<String>,
    pub subtask_status: Option<String>,
    pub subtask_started_at_ms: Option<i64>,
    pub subtask_finished_at_ms: Option<i64>,
    pub subtask_failure_json: Option<String>,
    pub runtime_state: Option<SessionRuntimeState>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Input describing how a session relates to its parent.
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
        subtask_failure_json,
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
                lineage.subtask_failure_json,
            )
        }
    };
    let mut runtime_state = model.runtime_state;
    if relation_kind == SessionRelationKind::Subagent {
        runtime_state
            .get_or_insert_with(SessionRuntimeState::default)
            .subtask = decode_subtask_state(
            model.id,
            subtask_status.as_deref(),
            subtask_started_at_ms,
            subtask_finished_at_ms,
            subtask_failure_json.as_deref(),
        )?;
    }
    Ok(SessionRecord {
        id: model.id,
        parent_id: model.parent_id,
        depth: model.depth,
        root_id: model.root_id,
        workspace_id: model.workspace_id,
        title: model.title,
        version: model.version,
        lifecycle_state,
        creation_failure: model
            .creation_failure_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| DbErr::Custom(format!("decode session creation failure: {error}")))?,
        relation_kind,
        source_cutoff_seq_global,
        source_message_id,
        task_id,
        subtask_status,
        subtask_started_at_ms,
        subtask_finished_at_ms,
        subtask_failure_json,
        runtime_state,
        created_at_ms: model.created_at_ms,
        updated_at_ms: model.updated_at_ms,
    })
}

#[derive(Debug, Clone, FromQueryResult)]
struct SessionTouchRow {
    id: i64,
    version: i64,
    subtask_status: Option<String>,
    subtask_started_at_ms: Option<i64>,
    subtask_finished_at_ms: Option<i64>,
    subtask_failure_json: Option<String>,
}

impl SessionTouchRow {
    fn subtask_state(&self) -> Result<SubtaskRuntimeState, DbErr> {
        decode_subtask_state(
            self.id,
            self.subtask_status.as_deref(),
            self.subtask_started_at_ms,
            self.subtask_finished_at_ms,
            self.subtask_failure_json.as_deref(),
        )
    }
}

fn decode_subtask_state(
    session_id: i64,
    status: Option<&str>,
    started_at_ms: Option<i64>,
    finished_at_ms: Option<i64>,
    failure_json: Option<&str>,
) -> Result<SubtaskRuntimeState, DbErr> {
    let status = match status {
        Some(value) => SubtaskStatus::parse(value).ok_or_else(|| {
            DbErr::Custom(format!(
                "session {session_id} has invalid subtask status `{value}`"
            ))
        })?,
        None => SubtaskStatus::Created,
    };
    Ok(SubtaskRuntimeState {
        status,
        started_at_ms,
        finished_at_ms,
        failure: failure_json
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| {
                DbErr::Custom(format!(
                    "session {session_id} has invalid subtask failure JSON: {error}"
                ))
            })?,
    })
}

async fn get_session_touch_row<C>(db: &C, session_id: i64) -> Result<Option<SessionTouchRow>, DbErr>
where
    C: ConnectionTrait,
{
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT s.id, s.version, \
                l.subtask_status, l.subtask_started_at_ms, l.subtask_finished_at_ms, l.subtask_failure_json \
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
            subtask_failure_json: row.try_get("", "subtask_failure_json")?,
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
    // Acquire the SQLite write lock before the parent-lineage SELECT so a
    // concurrent writer in another process cannot make the read→write upgrade
    // fail with SQLITE_BUSY (the busy timeout only applies at transaction start).
    agena_storage_sqlite::acquire_write_lock(&txn).await?;
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
        creation_failure_json: Set(None),
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
            view_materialized_seq_global: Set(None),
            task_id: Set(task_id),
            subtask_status: Set(is_subagent.then(|| SubtaskStatus::Created.as_ref().to_owned())),
            subtask_started_at_ms: Set(None),
            subtask_finished_at_ms: Set(None),
            subtask_failure_json: Set(None),
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
    let failure_json = state
        .failure
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| DbErr::Custom(format!("encode subtask failure: {error}")))?;
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE agena_session_lineage SET subtask_status = ?, \
         subtask_started_at_ms = ?, subtask_finished_at_ms = ?, subtask_failure_json = ? \
         WHERE session_id = ? AND relation_kind = 'subagent'",
        [
            state.status.as_ref().to_string().into(),
            state.started_at_ms.into(),
            state.finished_at_ms.into(),
            failure_json.into(),
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
    creation_failure: Option<agena_failure::Failure>,
) -> Result<Option<SessionRecord>, DbErr>
where
    C: ConnectionTrait,
{
    let creation_failure_json = creation_failure
        .map(|failure| serde_json::to_string(&failure))
        .transpose()
        .map_err(|error| DbErr::Custom(format!("encode session creation failure: {error}")))?;
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE agena_sessions SET lifecycle_state = ?, creation_failure_json = ?, \
         version = version + 1, updated_at_ms = ? WHERE id = ?",
        [
            lifecycle_state.as_str().to_owned().into(),
            creation_failure_json.into(),
            Utc::now().timestamp_millis().into(),
            session_id.into(),
        ],
    );
    if db.execute(stmt).await?.rows_affected() == 0 {
        return Ok(None);
    }
    get_session_by_id(db, session_id).await
}
