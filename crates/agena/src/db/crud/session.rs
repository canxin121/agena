use std::collections::HashMap;

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    FromQueryResult, QueryFilter, QueryOrder, QuerySelect,
};

use crate::db::entities;
use crate::session::{SessionListRequest, SessionRuntimeState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionMessageStats {
    pub message_count: i64,
    pub last_message_at_ms: Option<i64>,
}

#[derive(Debug, Clone, FromQueryResult)]
struct SessionChildCountRow {
    session_id: i64,
    child_session_count: i64,
}

#[derive(Debug, Clone, FromQueryResult)]
struct SessionTouchRow {
    id: i64,
    parent_id: Option<i64>,
    depth: i64,
    root_id: i64,
    workspace_id: i64,
    title: String,
    version: i64,
    is_subagent: bool,
    created_at_ms: i64,
}

impl SessionTouchRow {
    fn into_model(
        self,
        version: i64,
        updated_at_ms: i64,
        runtime_state: SessionRuntimeState,
    ) -> entities::session::Model {
        entities::session::Model {
            id: self.id,
            parent_id: self.parent_id,
            depth: self.depth,
            root_id: self.root_id,
            workspace_id: self.workspace_id,
            title: self.title,
            version,
            is_subagent: self.is_subagent,
            runtime_state: Some(runtime_state),
            created_at_ms: self.created_at_ms,
            updated_at_ms,
        }
    }
}

async fn get_session_touch_row<C>(db: &C, session_id: i64) -> Result<Option<SessionTouchRow>, DbErr>
where
    C: ConnectionTrait,
{
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT id, parent_id, depth, root_id, workspace_id, title, version, is_subagent, created_at_ms \
         FROM agena_sessions WHERE id = ?",
        [session_id.into()],
    );
    db.query_one(stmt).await?.map_or(Ok(None), |row| {
        Ok(Some(SessionTouchRow {
            id: row.try_get("", "id")?,
            parent_id: row.try_get("", "parent_id")?,
            depth: row.try_get("", "depth")?,
            root_id: row.try_get("", "root_id")?,
            workspace_id: row.try_get("", "workspace_id")?,
            title: row.try_get("", "title")?,
            version: row.try_get("", "version")?,
            is_subagent: row.try_get("", "is_subagent")?,
            created_at_ms: row.try_get("", "created_at_ms")?,
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
    Ok(Some(ParentLineage {
        depth: parent.depth,
        root_id: parent.root_id,
    }))
}

pub async fn create_session<C>(
    db: &C,
    workspace_id: i64,
    parent_id: Option<i64>,
    title: impl Into<String>,
) -> Result<entities::session::Model, DbErr>
where
    C: ConnectionTrait,
{
    create_session_with_options(db, workspace_id, parent_id, title, false).await
}

/// Same as [`create_session`] but lets callers mark the row as a subagent
/// session (see `agena_sessions.is_subagent`). Used by the subtask spawner
/// so user-facing listings can hide implementation-detail children.
pub async fn create_session_with_options<C>(
    db: &C,
    workspace_id: i64,
    parent_id: Option<i64>,
    title: impl Into<String>,
    is_subagent: bool,
) -> Result<entities::session::Model, DbErr>
where
    C: ConnectionTrait,
{
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
        is_subagent: Set(is_subagent),
        runtime_state: Set(Some(SessionRuntimeState::default())),
        created_at_ms: Set(now_ms),
        updated_at_ms: Set(now_ms),
        ..Default::default()
    }
    .insert(db)
    .await?;

    if lineage.is_none() {
        // Root: rewrite root_id = id now that the auto-increment id is known.
        let mut active: entities::session::ActiveModel = inserted.clone().into();
        active.root_id = Set(inserted.id);
        active.update(db).await
    } else {
        Ok(inserted)
    }
}

pub async fn get_session_by_id<C>(
    db: &C,
    session_id: i64,
) -> Result<Option<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    entities::session::Entity::find_by_id(session_id)
        .one(db)
        .await
}

pub async fn rename_session<C>(
    db: &C,
    session_id: i64,
    title: impl Into<String>,
) -> Result<Option<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let Some(existing) = get_session_by_id(db, session_id).await? else {
        return Ok(None);
    };
    let next_version = existing.version + 1;
    let mut active: entities::session::ActiveModel = existing.into();
    active.title = Set(title.into());
    active.version = Set(next_version);
    active.updated_at_ms = Set(Utc::now().timestamp_millis());
    active.update(db).await.map(Some)
}

/// Outcome of a [`touch_session_updated_at`] attempt.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum TouchOutcome {
    /// Session row updated; carries the post-update model.
    Updated(entities::session::Model),
    /// Session id does not exist.
    NotFound,
    /// Caller passed `expected_version` and it did not match the current
    /// row — another writer raced ahead. Caller should reload + retry.
    VersionConflict {
        current_version: i64,
        expected_version: i64,
    },
}

pub async fn touch_session_updated_at<C>(
    db: &C,
    session_id: i64,
    runtime_state: SessionRuntimeState,
) -> Result<Option<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    match touch_session_with_version(db, session_id, runtime_state, None).await? {
        TouchOutcome::Updated(model) => Ok(Some(model)),
        TouchOutcome::NotFound => Ok(None),
        TouchOutcome::VersionConflict { .. } => {
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
    runtime_state: SessionRuntimeState,
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
        return Ok(TouchOutcome::VersionConflict {
            current_version: existing.version,
            expected_version: expected,
        });
    }
    let next_version = existing.version + 1;
    let now_ms = Utc::now().timestamp_millis();
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
            return Ok(TouchOutcome::VersionConflict {
                current_version: latest.version,
                expected_version: expected_version.unwrap_or(existing.version),
            });
        }
        return Ok(TouchOutcome::NotFound);
    }

    Ok(TouchOutcome::Updated(existing.into_model(
        next_version,
        now_ms,
        runtime_state,
    )))
}

pub async fn delete_session_by_id<C>(db: &C, session_id: i64) -> Result<u64, DbErr>
where
    C: ConnectionTrait,
{
    // The schema has `parent_id ON DELETE CASCADE` so the SQL DELETE will
    // sweep the whole descendant subtree. The unified event log and snapshot
    // table are *not* foreign-keyed to `agena_sessions` (events outlive
    // sessions in audit scenarios), so we must clean them up explicitly for
    // every node we are about to drop. Walk the subtree first, prune events
    // and snapshots for each node, then DELETE the root and let CASCADE
    // remove the descendant rows.
    use sea_orm::QueryFilter;
    let descendants: Vec<i64> = entities::session::Entity::find()
        .select_only()
        .column(entities::session::Column::Id)
        .filter(
            entities::session::Column::RootId.eq(entities::session::Entity::find_by_id(session_id)
                .one(db)
                .await?
                .map(|m| m.root_id)
                .unwrap_or(session_id)),
        )
        .into_tuple::<i64>()
        .all(db)
        .await?;

    // Restrict to the actual subtree rooted at `session_id` (not the entire
    // tree — `root_id` groups every session that shares a root, including
    // siblings). Use the parent chain to keep only the descendants.
    let mut subtree_ids = vec![session_id];
    if descendants.len() > 1 {
        // Pull each row's parent_id and BFS from session_id.
        let rows: Vec<(i64, Option<i64>)> = entities::session::Entity::find()
            .select_only()
            .column(entities::session::Column::Id)
            .column(entities::session::Column::ParentId)
            .filter(entities::session::Column::Id.is_in(descendants.iter().copied()))
            .into_tuple::<(i64, Option<i64>)>()
            .all(db)
            .await?;
        let mut frontier = vec![session_id];
        while let Some(node) = frontier.pop() {
            for (id, parent) in &rows {
                if *parent == Some(node) && !subtree_ids.contains(id) {
                    subtree_ids.push(*id);
                    frontier.push(*id);
                }
            }
        }
    }

    if !subtree_ids.is_empty() {
        crate::db::event_entity::Entity::delete_many()
            .filter(crate::db::event_entity::Column::SessionId.is_in(subtree_ids.iter().copied()))
            .exec(db)
            .await?;
    }

    let deleted = entities::session::Entity::delete_by_id(session_id)
        .exec(db)
        .await?;
    Ok(deleted.rows_affected)
}

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
        .order_by_desc(entities::session::Column::UpdatedAtMs)
        .order_by_desc(entities::session::Column::Id)
        .into_tuple::<i64>()
        .all(db)
        .await
}

pub async fn list_all_session_ids<C>(db: &C) -> Result<Vec<i64>, DbErr>
where
    C: ConnectionTrait,
{
    entities::session::Entity::find()
        .select_only()
        .column(entities::session::Column::Id)
        .into_tuple::<i64>()
        .all(db)
        .await
}

pub async fn list_sessions_by_workspace_id_with_request<C>(
    db: &C,
    workspace_id: i64,
    request: SessionListRequest,
) -> Result<Vec<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    let mut query = entities::session::Entity::find()
        .filter(entities::session::Column::WorkspaceId.eq(workspace_id))
        .order_by_desc(entities::session::Column::UpdatedAtMs)
        .order_by_desc(entities::session::Column::Id);
    if !request.include_subagents {
        query = query.filter(entities::session::Column::IsSubagent.eq(false));
    }

    if let Some(limit) = request.limit {
        if request.offset > 0 {
            query = query.offset(request.offset);
        }
        query = query.limit(limit);
        return query.all(db).await;
    }

    let sessions = query.all(db).await?;
    let offset = usize::try_from(request.offset)
        .map_err(|_| DbErr::Custom(format!("session list offset too large: {}", request.offset)))?;
    Ok(sessions.into_iter().skip(offset).collect())
}

pub async fn child_session_counts_by_parent_ids<C>(
    db: &C,
    parent_ids: &[i64],
) -> Result<HashMap<i64, i64>, DbErr>
where
    C: ConnectionTrait,
{
    if parent_ids.is_empty() {
        return Ok(HashMap::new());
    }

    entities::session::Entity::find()
        .select_only()
        .column_as(entities::session::Column::ParentId, "session_id")
        .column_as(entities::session::Column::Id.count(), "child_session_count")
        .filter(entities::session::Column::ParentId.is_in(parent_ids.iter().copied()))
        .group_by(entities::session::Column::ParentId)
        .into_model::<SessionChildCountRow>()
        .all(db)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| (row.session_id, row.child_session_count))
                .collect()
        })
}

/// Pull every session that shares `root_id`, ordered by `(depth, id)` so the
/// caller can render a tree without a recursive walk.
pub async fn list_session_tree<C>(
    db: &C,
    root_id: i64,
) -> Result<Vec<entities::session::Model>, DbErr>
where
    C: ConnectionTrait,
{
    entities::session::Entity::find()
        .filter(entities::session::Column::RootId.eq(root_id))
        .order_by_asc(entities::session::Column::Depth)
        .order_by_asc(entities::session::Column::Id)
        .all(db)
        .await
}

#[derive(Debug, Clone, FromQueryResult)]
struct SessionEventStatsRow {
    session_id: i64,
    message_count: i64,
    last_message_at_ms: Option<i64>,
}

/// Approximate per-session message stats computed from the unified event log
/// in a single grouped query. `message_count` counts message-emitting event
/// kinds; use the projection for exact visible-message counts.
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
    let kinds = [
        "user_message_appended",
        "assistant_message_completed",
        "tool_call_completed",
        "system_notice_appended",
    ];
    let rows = crate::db::event_entity::Entity::find()
        .select_only()
        .column_as(crate::db::event_entity::Column::SessionId, "session_id")
        .column_as(crate::db::event_entity::Column::Id.count(), "message_count")
        .column_as(
            crate::db::event_entity::Column::CreatedAtMs.max(),
            "last_message_at_ms",
        )
        .filter(crate::db::event_entity::Column::SessionId.is_in(session_ids.iter().copied()))
        .filter(crate::db::event_entity::Column::KindTag.is_in(kinds.iter().copied()))
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
