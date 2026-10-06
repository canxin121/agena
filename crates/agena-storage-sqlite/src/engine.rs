//! SQLite implementation of the persistence engine.
//!
//! `SqliteEngine` is the production backend behind the sealed `SessionStore`
//! facade (design sections 14-15). It is the ONLY component that imports
//! SeaORM or holds a `DatabaseConnection`; every raw SQL statement in the
//! codebase lives in this module. The in-memory engine
//! (`agena_storage::store::InMemoryEngine`) shares the exact same contract,
//! invariants, and state derivation, so callers cannot distinguish the two.
//!
//! Write operations run in a transaction that acquires the SQLite write lock
//! up front (the `__agena_write_lock__` fence), so each mutation is atomic
//! with respect to every other writer. Part ids come from the
//! `agena_sequences` row, matching the in-memory allocator (first part
//! id = 1).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use agena_domain::{SessionLifecycleState, SessionRelationKind};
use agena_storage::store::{
    BackgroundDelivery, BackgroundDeliveryPhase, BackgroundEventRequest, BackgroundOperation,
    BackgroundOperationKind, BackgroundOperationPhase, BackgroundOperationTransition,
    BackgroundSettleOutcome, InFlightRun, MaintenanceOutcome, NewBackgroundOperation, NewPart,
    NewSession, Part, PartCursor, PartDelta, PartRole, PartState, PartVisibility,
    PersistenceEngine, ReconcileOutcome, RunOutcome, SessionListQuery, SessionMeta,
    SessionMetadataPatch, SessionPartPage, SessionState, SessionSummary, SessionView, StoreError,
    SubmitOutcome, UsageGroup, UsageQuery, UsageRecord, UsageStats, prepare_part_update,
    prepare_run_completion, validate_run_content,
};
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, DbErr, Statement,
    Value,
};

use crate::{acquire_write_permit, is_sqlite_busy, transaction::begin_with_write_lock};

/// Build a `Value::String` (which is `Option<Box<String>>` in SeaORM 1.1).
fn text_value(option: Option<String>) -> Value {
    Value::String(option.map(Box::new))
}

/// Every part column, aliased so the row mapper reads them by field name.
const PART_COLS: &str = "\
    p.part_id, p.kind, p.role, p.state, CAST(p.content AS TEXT) AS content, p.summary, \
    p.visibility, p.parent_part_id, p.run_id, p.origin_session_id, \
    p.revision, p.started_at_ms, p.finished_at_ms, p.created_at_ms, p.updated_at_ms, \
    CAST(p.provider_state AS TEXT) AS provider_state";

/// Every session column, aliased to the `SessionMeta` field names.
const SESSION_COLS: &str = "\
    s.id, s.parent_id, s.depth, s.root_id, s.workspace_id, s.relation_kind, s.cutoff_part_id, \
    s.title, s.favorite, s.pinned, s.version, s.lifecycle_state, \
    CAST(s.creation_failure_json AS TEXT) AS creation_failure, \
    s.task_id, s.subtask_status, s.subtask_started_at_ms, s.subtask_finished_at_ms, \
    CAST(s.subtask_failure_json AS TEXT) AS subtask_failure, CAST(s.config_json AS TEXT) AS config_json, \
    CAST(s.provider_anchors_json AS TEXT) AS provider_anchors_json, s.created_at_ms, s.updated_at_ms";

const BACKGROUND_OPERATION_COLS: &str = "\
    operation_id, session_id, launch_run_id, launch_tool_part_id, kind, external_id, phase, \
    CAST(outcome_json AS TEXT) AS outcome_json, CAST(failure_json AS TEXT) AS failure_json, \
    last_event_seq, owner_id, lease_until_ms, revision, created_at_ms, updated_at_ms, finished_at_ms";

const BACKGROUND_DELIVERY_COLS: &str = "\
    delivery_id, operation_id, session_id, event_key, \
    CAST(payload_json AS TEXT) AS payload_json, phase, claim_owner, claim_until_ms, attempts, \
    notification_part_id, CAST(last_error_json AS TEXT) AS last_error_json, \
    created_at_ms, updated_at_ms, consumed_at_ms, next_attempt_at_ms";

/// The production [`PersistenceEngine`]: raw SQL over the current schema.
#[derive(Clone)]
pub struct SqliteEngine {
    db: Arc<DatabaseConnection>,
}

impl SqliteEngine {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }

    fn db(&self) -> &DatabaseConnection {
        &self.db
    }
}

/// Run a write transaction that already holds the SQLite write lock.
///
/// The closure runs once, inside the transaction; on success the commit
/// happens before any read is returned. `DbErr` from begin/commit is mapped
/// through [`map_db_err`].
async fn run_write<T>(
    db: &DatabaseConnection,
    op: impl for<'a> FnOnce(
        &'a DatabaseTransaction,
    ) -> Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>,
) -> Result<T, StoreError> {
    let (txn, _permit) = begin_with_write_lock(db).await.map_err(map_db_err)?;
    match op(&txn).await {
        Ok(value) => {
            txn.commit().await.map_err(map_db_err)?;
            Ok(value)
        }
        Err(error) => match txn.rollback().await {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(StoreError::Database(format!(
                "{}; additionally, {}",
                agena_failure::diagnostic::format_error_chain(&error),
                agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to roll back the SQLite write transaction",
                    &rollback_error,
                )
            ))),
        },
    }
}

/// Map a SeaORM error into the backend-neutral [`StoreError`], preserving the
/// transient busy signal for dependency-unavailable classification and backoff.
fn map_db_err(error: DbErr) -> StoreError {
    if is_sqlite_busy(&error) || crate::is_write_admission_busy(&error) {
        StoreError::Busy
    } else {
        StoreError::Database(agena_failure::diagnostic::format_error_chain(&error))
    }
}

/// The next part id from the `agena_sequences` allocator. Safe inside a write
/// transaction: the write lock serializes every other allocator, and the
/// `RETURNING` update is atomic.
async fn next_part_id_tx(txn: &DatabaseTransaction) -> Result<i64, DbErr> {
    reserve_part_ids_tx(txn, 1).await
}

/// Reserve a whole batch with one allocator UPDATE, rather than one database
/// round trip per attachment/imported part. The range stays transactional.
async fn reserve_part_ids_tx(txn: &DatabaseTransaction, count: usize) -> Result<i64, DbErr> {
    if count == 0 {
        return Ok(0);
    }
    let count = i64::try_from(count)
        .map_err(|_| DbErr::Custom("part id allocation is too large".into()))?;
    let row = txn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE agena_sequences SET next_val = next_val + ? \
         WHERE seq_name = 'part_id' AND next_val <= ? RETURNING next_val - ? AS next_id",
            [count.into(), (i64::MAX - count).into(), count.into()],
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("part id allocator is missing or exhausted".into()))?;
    row.try_get("", "next_id")
}

/// Insert one part row.
async fn insert_part_tx(
    txn: &DatabaseTransaction,
    part: &Part,
    content: String,
    provider_state: Option<String>,
) -> Result<(), DbErr> {
    txn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO agena_parts \
         (part_id, kind, role, state, content, summary, visibility, parent_part_id, run_id, \
          origin_session_id, revision, started_at_ms, finished_at_ms, created_at_ms, \
          updated_at_ms, provider_state) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        [
            part.part_id.into(),
            part.kind.clone().into(),
            part.role.as_str().into(),
            part.state.as_str().into(),
            text_value(Some(content)),
            text_value(part.summary.clone()),
            part.visibility.as_str().into(),
            Value::BigInt(part.parent_part_id),
            Value::BigInt(part.run_id),
            part.origin_session_id.into(),
            part.revision.into(),
            part.started_at_ms.into(),
            Value::BigInt(part.finished_at_ms),
            part.created_at_ms.into(),
            part.updated_at_ms.into(),
            text_value(provider_state),
        ],
    ))
    .await
    .map(|_| ())
}

/// Insert one membership edge.
async fn insert_membership_tx(
    txn: &DatabaseTransaction,
    session_id: i64,
    part_id: i64,
    added_at_ms: i64,
) -> Result<(), DbErr> {
    txn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO agena_session_parts (session_id, part_id, added_at_ms) VALUES (?, ?, ?)",
        [session_id.into(), part_id.into(), added_at_ms.into()],
    ))
    .await
    .map(|_| ())
}

/// Advance the content version for a committed mutation. Navigation recency
/// advances only when this mutation finishes a user/assistant message.
async fn bump_session_version_tx(
    txn: &DatabaseTransaction,
    session_id: i64,
    message_ended_at_ms: Option<i64>,
) -> Result<(), StoreError> {
    let result = txn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE agena_sessions \
             SET version = version + 1, \
                 updated_at_ms = CASE WHEN ? IS NULL THEN updated_at_ms \
                                      ELSE MAX(?, updated_at_ms + 1) END WHERE id = ?",
            [
                Value::BigInt(message_ended_at_ms),
                Value::BigInt(message_ended_at_ms),
                session_id.into(),
            ],
        ))
        .await
        .map_err(map_db_err)?;
    if result.rows_affected() != 1 {
        return Err(StoreError::not_found(format!("session {session_id}")));
    }
    Ok(())
}

/// Advance every session whose view includes any changed part. Shared prefix
/// parts therefore invalidate fork/rewind caches as well as their origin.
async fn bump_member_session_versions_for_parts_tx(
    txn: &DatabaseTransaction,
    part_ids: &[i64],
    message_ended: Option<(i64, i64)>,
) -> Result<(), StoreError> {
    if part_ids.is_empty() {
        return Ok(());
    }
    let part_list = comma_list(part_ids);
    let result = txn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "UPDATE agena_sessions \
                 SET version = version + 1, \
                     updated_at_ms = CASE WHEN id = ? THEN MAX(?, updated_at_ms + 1) \
                                          ELSE updated_at_ms END \
                 WHERE id IN ( \
                     SELECT DISTINCT session_id FROM agena_session_parts \
                     WHERE part_id IN ({part_list}) \
                 )"
            ),
            [
                Value::BigInt(message_ended.map(|(id, _)| id)),
                Value::BigInt(message_ended.map(|(_, at)| at)),
            ],
        ))
        .await
        .map_err(map_db_err)?;
    if result.rows_affected() == 0 {
        return Err(StoreError::InvalidState(
            "changed part has no session membership".to_owned(),
        ));
    }
    Ok(())
}

/// In-flight run markers of a session (state `pending` | `in_progress`),
/// newest first.
async fn in_flight_runs_tx<C: ConnectionTrait>(
    txn: &C,
    session_id: i64,
) -> Result<Vec<InFlightRun>, StoreError> {
    let rows = txn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT part_id, created_at_ms FROM agena_parts \
             WHERE kind = 'run' AND state IN ('pending', 'in_progress') \
               AND origin_session_id = ? \
             ORDER BY created_at_ms DESC, part_id DESC",
            [session_id.into()],
        ))
        .await
        .map_err(map_db_err)?;
    let mut runs = Vec::with_capacity(rows.len());
    for row in rows {
        runs.push(InFlightRun {
            part_id: row.try_get("", "part_id").map_err(map_db_err)?,
            created_at_ms: row.try_get("", "created_at_ms").map_err(map_db_err)?,
        });
    }
    Ok(runs)
}

/// Terminalize a set of run markers (`cancelled` for user cancel, otherwise
/// `failed`; `abort_reason` always set) and cancel their non-terminal children
/// in one transaction. Returns every changed row so the facade can emit
/// commit-derived patches. Used by user cancel and reconcile.
#[derive(Default)]
struct ReconcileRows {
    aborted_runs: Vec<i64>,
    cancelled_parts: usize,
    rows: Vec<sea_orm::QueryResult>,
}

impl ReconcileRows {
    async fn finish(self) -> Result<ReconcileOutcome, StoreError> {
        Ok(ReconcileOutcome {
            aborted_runs: self.aborted_runs,
            cancelled_parts: self.cancelled_parts,
            updated_parts: decode_part_rows(self.rows).await?,
        })
    }
}

async fn abort_runs_tx(
    txn: &DatabaseTransaction,
    session_id: i64,
    run_ids: &[i64],
    reason: &str,
    now_ms: i64,
) -> Result<ReconcileRows, StoreError> {
    if run_ids.is_empty() {
        return Ok(ReconcileRows::default());
    }
    let marker_state = if reason == "user_cancelled" {
        PartState::Cancelled
    } else {
        PartState::Failed
    };
    let run_list = comma_list(run_ids);
    let aborted: Vec<i64> = {
        let rows = txn
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "UPDATE agena_parts \
                     SET state = ?, finished_at_ms = ?, revision = revision + 1, \
                         updated_at_ms = ?, \
                         content = json_set(content, '$.abort_reason', ?) \
                     WHERE kind = 'run' AND origin_session_id = ? \
                       AND part_id IN ({run_list}) AND state IN ('pending', 'in_progress') \
                     RETURNING part_id"
                ),
                [
                    marker_state.as_str().into(),
                    now_ms.into(),
                    now_ms.into(),
                    reason.into(),
                    session_id.into(),
                ],
            ))
            .await
            .map_err(map_db_err)?;
        let mut ids = Vec::with_capacity(rows.len());
        for row in rows {
            ids.push(row.try_get("", "part_id").map_err(map_db_err)?);
        }
        ids
    };
    let mut cancelled_ids: Vec<i64> = Vec::new();
    if !aborted.is_empty() {
        let run_list = comma_list(&aborted);
        let rows = txn
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "UPDATE agena_parts \
                     SET state = 'cancelled', finished_at_ms = ?, revision = revision + 1, \
                         updated_at_ms = ? \
                     WHERE origin_session_id = ? AND run_id IN ({run_list}) \
                       AND state IN ('pending', 'in_progress') \
                     RETURNING part_id"
                ),
                [now_ms.into(), now_ms.into(), session_id.into()],
            ))
            .await
            .map_err(map_db_err)?;
        for row in rows {
            cancelled_ids.push(row.try_get("", "part_id").map_err(map_db_err)?);
        }
    }
    let mut changed_ids = aborted.clone();
    changed_ids.extend(cancelled_ids.iter().copied());
    let rows = if changed_ids.is_empty() {
        Vec::new()
    } else {
        txn.query_all(Statement::from_string(DatabaseBackend::Sqlite,
            format!("SELECT {PART_COLS} FROM agena_parts p WHERE p.part_id IN ({}) ORDER BY p.created_at_ms, p.part_id", comma_list(&changed_ids)),
        )).await.map_err(map_db_err)?
    };
    let mut message_ended = None;
    for row in &rows {
        let kind: String = row.try_get("", "kind").map_err(map_db_err)?;
        let role: String = row.try_get("", "role").map_err(map_db_err)?;
        if kind == "run" && matches!(role.as_str(), "user" | "assistant") {
            message_ended = Some((session_id, now_ms));
            break;
        }
    }
    bump_member_session_versions_for_parts_tx(txn, &changed_ids, message_ended).await?;
    Ok(ReconcileRows {
        aborted_runs: aborted,
        cancelled_parts: cancelled_ids.len(),
        rows,
    })
}

fn comma_list(ids: &[i64]) -> String {
    ids.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

// --- row mappers ---

fn part_from_row(row: sea_orm::QueryResult) -> Result<Part, DbErr> {
    let kind: String = row.try_get("", "kind")?;
    let role_raw: String = row.try_get("", "role")?;
    let state_raw: String = row.try_get("", "state")?;
    let visibility_raw: String = row.try_get("", "visibility")?;
    let role = PartRole::parse(&role_raw)
        .ok_or_else(|| DbErr::Custom("invalid part role in row".to_owned()))?;
    let state = PartState::parse(&state_raw)
        .ok_or_else(|| DbErr::Custom("invalid part state in row".to_owned()))?;
    let visibility = PartVisibility::parse(&visibility_raw)
        .ok_or_else(|| DbErr::Custom("invalid part visibility in row".to_owned()))?;
    let content: String = row.try_get("", "content")?;
    let content = serde_json::from_str(&content)
        .map_err(|error| DbErr::Custom(format!("decode part content: {error}")))?;
    let provider_state: Option<String> = row.try_get("", "provider_state")?;
    let provider_state = provider_state
        .map(|s| {
            serde_json::from_str(&s)
                .map_err(|error| DbErr::Custom(format!("decode provider state: {error}")))
        })
        .transpose()?;
    Ok(Part {
        part_id: row.try_get("", "part_id")?,
        kind,
        role,
        state,
        content,
        summary: row.try_get("", "summary")?,
        visibility,
        parent_part_id: row.try_get("", "parent_part_id")?,
        run_id: row.try_get("", "run_id")?,
        origin_session_id: row.try_get("", "origin_session_id")?,
        revision: row.try_get("", "revision")?,
        started_at_ms: row.try_get("", "started_at_ms")?,
        finished_at_ms: row.try_get("", "finished_at_ms")?,
        created_at_ms: row.try_get("", "created_at_ms")?,
        updated_at_ms: row.try_get("", "updated_at_ms")?,
        provider_state,
    })
}

fn meta_from_row(row: sea_orm::QueryResult) -> Result<SessionMeta, DbErr> {
    let relation_kind_raw: String = row.try_get("", "relation_kind")?;
    let lifecycle_state_raw: String = row.try_get("", "lifecycle_state")?;
    let relation_kind = SessionRelationKind::parse(&relation_kind_raw)
        .ok_or_else(|| DbErr::Custom("invalid relation_kind in row".to_owned()))?;
    let lifecycle_state = SessionLifecycleState::parse(&lifecycle_state_raw)
        .ok_or_else(|| DbErr::Custom("invalid lifecycle_state in row".to_owned()))?;
    let json_col = |name: &str| -> Result<Option<serde_json::Value>, DbErr> {
        let raw: Option<String> = row.try_get("", name)?;
        raw.map(|s| {
            serde_json::from_str(&s)
                .map_err(|error| DbErr::Custom(format!("decode {name}: {error}")))
        })
        .transpose()
    };
    Ok(SessionMeta {
        id: row.try_get("", "id")?,
        parent_id: row.try_get("", "parent_id")?,
        depth: row.try_get("", "depth")?,
        root_id: row.try_get("", "root_id")?,
        workspace_id: row.try_get("", "workspace_id")?,
        relation_kind,
        cutoff_part_id: row.try_get("", "cutoff_part_id")?,
        title: row.try_get("", "title")?,
        favorite: row.try_get("", "favorite")?,
        pinned: row.try_get("", "pinned")?,
        version: row.try_get("", "version")?,
        lifecycle_state,
        creation_failure: json_col("creation_failure")?,
        task_id: row.try_get("", "task_id")?,
        subtask_status: row.try_get("", "subtask_status")?,
        subtask_started_at_ms: row.try_get("", "subtask_started_at_ms")?,
        subtask_finished_at_ms: row.try_get("", "subtask_finished_at_ms")?,
        subtask_failure: json_col("subtask_failure")?,
        config_json: json_col("config_json")?,
        provider_anchors_json: json_col("provider_anchors_json")?,
        created_at_ms: row.try_get("", "created_at_ms")?,
        updated_at_ms: row.try_get("", "updated_at_ms")?,
    })
}

fn background_operation_from_row(row: sea_orm::QueryResult) -> Result<BackgroundOperation, DbErr> {
    let kind_raw: String = row.try_get("", "kind")?;
    let phase_raw: String = row.try_get("", "phase")?;
    let kind = BackgroundOperationKind::parse(&kind_raw)
        .ok_or_else(|| DbErr::Custom(format!("invalid background kind {kind_raw}")))?;
    let phase = BackgroundOperationPhase::parse(&phase_raw)
        .ok_or_else(|| DbErr::Custom(format!("invalid background phase {phase_raw}")))?;
    let json_col = |name: &str| -> Result<Option<serde_json::Value>, DbErr> {
        let raw: Option<String> = row.try_get("", name)?;
        raw.map(|value| {
            serde_json::from_str(&value)
                .map_err(|error| DbErr::Custom(format!("decode {name}: {error}")))
        })
        .transpose()
    };
    let last_event_seq: i64 = row.try_get("", "last_event_seq")?;
    Ok(BackgroundOperation {
        operation_id: row.try_get("", "operation_id")?,
        session_id: row.try_get("", "session_id")?,
        launch_run_id: row.try_get("", "launch_run_id")?,
        launch_tool_part_id: row.try_get("", "launch_tool_part_id")?,
        kind,
        external_id: row.try_get("", "external_id")?,
        phase,
        outcome: json_col("outcome_json")?,
        failure: json_col("failure_json")?,
        last_event_seq: u64::try_from(last_event_seq)
            .map_err(|_| DbErr::Custom("negative background event sequence".to_owned()))?,
        owner_id: row.try_get("", "owner_id")?,
        lease_until_ms: row.try_get("", "lease_until_ms")?,
        revision: row.try_get("", "revision")?,
        created_at_ms: row.try_get("", "created_at_ms")?,
        updated_at_ms: row.try_get("", "updated_at_ms")?,
        finished_at_ms: row.try_get("", "finished_at_ms")?,
    })
}

fn background_delivery_from_row(row: sea_orm::QueryResult) -> Result<BackgroundDelivery, DbErr> {
    let phase_raw: String = row.try_get("", "phase")?;
    let phase = BackgroundDeliveryPhase::parse(&phase_raw)
        .ok_or_else(|| DbErr::Custom(format!("invalid delivery phase {phase_raw}")))?;
    let payload_raw: String = row.try_get("", "payload_json")?;
    let payload = serde_json::from_str(&payload_raw)
        .map_err(|error| DbErr::Custom(format!("decode delivery payload: {error}")))?;
    let last_error_raw: Option<String> = row.try_get("", "last_error_json")?;
    let last_error = last_error_raw
        .map(|value| {
            serde_json::from_str(&value)
                .map_err(|error| DbErr::Custom(format!("decode delivery error: {error}")))
        })
        .transpose()?;
    let attempts: i64 = row.try_get("", "attempts")?;
    Ok(BackgroundDelivery {
        delivery_id: row.try_get("", "delivery_id")?,
        operation_id: row.try_get("", "operation_id")?,
        session_id: row.try_get("", "session_id")?,
        event_key: row.try_get("", "event_key")?,
        payload,
        phase,
        claim_owner: row.try_get("", "claim_owner")?,
        claim_until_ms: row.try_get("", "claim_until_ms")?,
        attempts: u32::try_from(attempts)
            .map_err(|_| DbErr::Custom("invalid delivery attempt count".to_owned()))?,
        notification_part_id: row.try_get("", "notification_part_id")?,
        last_error,
        created_at_ms: row.try_get("", "created_at_ms")?,
        updated_at_ms: row.try_get("", "updated_at_ms")?,
        consumed_at_ms: row.try_get("", "consumed_at_ms")?,
        next_attempt_at_ms: row.try_get("", "next_attempt_at_ms")?,
    })
}

fn summary_from_row(row: sea_orm::QueryResult) -> Result<SessionSummary, DbErr> {
    let relation_kind_raw: String = row.try_get("", "relation_kind")?;
    let lifecycle_state_raw: String = row.try_get("", "lifecycle_state")?;
    let relation_kind = SessionRelationKind::parse(&relation_kind_raw)
        .ok_or_else(|| DbErr::Custom("invalid relation_kind in row".to_owned()))?;
    let lifecycle_state = SessionLifecycleState::parse(&lifecycle_state_raw)
        .ok_or_else(|| DbErr::Custom("invalid lifecycle_state in row".to_owned()))?;
    Ok(SessionSummary {
        id: row.try_get("", "id")?,
        workspace_id: row.try_get("", "workspace_id")?,
        parent_id: row.try_get("", "parent_id")?,
        depth: row.try_get("", "depth")?,
        root_id: row.try_get("", "root_id")?,
        title: row.try_get("", "title")?,
        favorite: row.try_get("", "favorite")?,
        pinned: row.try_get("", "pinned")?,
        relation_kind,
        lifecycle_state,
        version: row.try_get("", "version")?,
        task_id: row.try_get("", "task_id")?,
        subtask_status: row.try_get("", "subtask_status")?,
        message_count: row.try_get("", "message_count")?,
        child_session_count: row.try_get("", "child_session_count")?,
        last_message_at_ms: row.try_get("", "last_message_at_ms")?,
        created_at_ms: row.try_get("", "created_at_ms")?,
        updated_at_ms: row.try_get("", "updated_at_ms")?,
    })
}

/// SQL projection of the same precedence implemented by
/// `agena_storage::store::derive_session_state`, scoped to the outer
/// `agena_sessions s` row. Keeping this as one correlated batch query avoids
/// loading every transcript separately for a session overview.
fn session_state_projection_sql() -> String {
    String::from(
        "CASE \
         WHEN s.lifecycle_state = 'creating' THEN 'creating' \
         WHEN s.lifecycle_state = 'failed' THEN 'failed' \
         WHEN EXISTS ( \
           SELECT 1 FROM agena_session_parts spi \
           JOIN agena_parts pi ON pi.part_id = spi.part_id \
           WHERE spi.session_id = s.id \
             AND pi.origin_session_id = s.id \
             AND pi.state IN ('pending', 'in_progress') \
             AND pi.kind = 'tool_call' AND EXISTS ( \
                 SELECT 1 \
                 FROM json_each(pi.content, '$.user_input.requests') request \
                 WHERE json_type(request.value, '$.reply') IS NULL \
             ) \
         ) THEN 'awaiting_interaction' \
         WHEN EXISTS ( \
           SELECT 1 FROM agena_session_parts spr \
           JOIN agena_parts pr ON pr.part_id = spr.part_id \
           WHERE spr.session_id = s.id \
             AND pr.origin_session_id = s.id \
             AND pr.kind = 'run' \
             AND pr.state IN ('pending', 'in_progress') \
         ) THEN 'running' \
         ELSE 'ready' END",
    )
}

fn session_list_filter(query: &SessionListQuery) -> (String, Vec<Value>) {
    let mut where_clauses = Vec::new();
    where_clauses.push(
        if query.temporary_only {
            "json_extract(s.config_json, '$.conversation.mode') = 'btw'"
        } else {
            "json_extract(s.config_json, '$.conversation.mode') IS NOT 'btw'"
        }
        .to_owned(),
    );
    let mut values: Vec<Value> = Vec::new();
    if let Some(workspace_id) = query.workspace_id {
        where_clauses.push("s.workspace_id = ?".to_owned());
        values.push(workspace_id.into());
    }
    if let Some(parent_id) = query.parent_id {
        where_clauses.push("s.parent_id = ?".to_owned());
        values.push(parent_id.into());
    } else if query.roots_only {
        where_clauses.push("s.parent_id IS NULL".to_owned());
    }
    if query.exclude_subagents {
        where_clauses.push("s.is_subagent = 0".to_owned());
    }
    if let Some(search) = query.search.as_deref()
        && !search.trim().is_empty()
    {
        where_clauses.push("s.title LIKE ? ESCAPE '\\'".to_owned());
        // Escape `%`/`_` so user input is a literal substring match.
        let escaped = search
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        values.push(Value::String(Some(Box::new(format!("%{escaped}%")))));
    }
    if let Some(before) = query.before.as_ref() {
        where_clauses.push("(s.updated_at_ms, s.id) < (?, ?)".to_owned());
        values.push(before.updated_at_ms.into());
        values.push(before.id.into());
    }
    for (column, flag) in [("favorite", query.favorite), ("pinned", query.pinned)] {
        if let Some(flag) = flag {
            where_clauses.push(format!("s.{column} = ?"));
            values.push(i64::from(flag).into());
        }
    }
    if !query.states.is_empty() {
        where_clauses.push(format!(
            "({}) IN ({})",
            session_state_projection_sql(),
            vec!["?"; query.states.len()].join(", ")
        ));
        values.extend(query.states.iter().map(|state| Value::from(state.as_str())));
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };
    (where_sql, values)
}

static PART_DECODERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
// Mutation work cannot queue behind large read-only history requests while
// holding a SQLite transaction. Codec closures never perform database I/O.
static PART_MUTATION_CODECS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
// Required transactional payload reads never queue behind history decoding or
// pre-encoding a large new message while holding SQLite's write lock.
static TRANSACTION_CODECS: agena_async::BlockingPool = agena_async::BlockingPool::new(1);

fn encode_part_payload(part: &Part) -> Result<(String, Option<String>), StoreError> {
    let content = serde_json::to_string(&part.content)
        .map_err(|error| StoreError::Serialization(format!("encode part content: {error}")))?;
    let provider_state = part
        .provider_state
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| StoreError::Serialization(format!("encode provider state: {error}")))?;
    Ok((content, provider_state))
}

struct PreparedNewPart {
    part: NewPart,
    content: String,
}

async fn prepare_new_parts(parts: Vec<NewPart>) -> Result<Vec<PreparedNewPart>, StoreError> {
    if parts.is_empty() {
        return Ok(Vec::new());
    }
    PART_MUTATION_CODECS
        .run(move || {
            parts
                .into_iter()
                .map(|part| {
                    let content = serde_json::to_string(&part.content).map_err(|error| {
                        StoreError::Serialization(format!("encode part content: {error}"))
                    })?;
                    Ok(PreparedNewPart { part, content })
                })
                .collect()
        })
        .await
        .map_err(|error| StoreError::Database(format!("part encode worker failed: {error}")))?
}

async fn prepare_marker(
    content: serde_json::Value,
) -> Result<(serde_json::Value, String), StoreError> {
    PART_MUTATION_CODECS
        .run(move || {
            let encoded = serde_json::to_string(&content).map_err(|error| {
                StoreError::Serialization(format!("encode run marker: {error}"))
            })?;
            Ok((content, encoded))
        })
        .await
        .map_err(|error| StoreError::Database(format!("run encode worker failed: {error}")))?
}

#[derive(Clone)]
enum PartMutation {
    Delta(Arc<PartDelta>),
    Complete(Arc<RunOutcome>),
}

/// Decode/compare/encode outside the writer transaction. A revision predicate
/// protects against other processes; only a successful CAS bumps member
/// sessions, in the same transaction. A failed CAS reloads and reapplies the
/// pure mutation, never publishing duplicate events or losing text deltas.
async fn mutate_part(
    db: &DatabaseConnection,
    session_id: i64,
    part_id: i64,
    mutation: PartMutation,
    now_ms: i64,
) -> Result<Part, StoreError> {
    for _ in 0..8 {
        let part = load_part_by_id(db, part_id)
            .await?
            .ok_or_else(|| StoreError::not_found(format!("part {part_id}")))?;
        if part.origin_session_id != session_id {
            return Err(StoreError::InvalidState(format!(
                "part {part_id} is shared; only its origin session may update it in place"
            )));
        }
        let revision = part.revision;
        let previous_state = part.state;
        let operation = mutation.clone();
        let (part, payload) = PART_MUTATION_CODECS
            .run(move || {
                let part = match operation {
                    PartMutation::Delta(delta) => {
                        prepare_part_update(part, (*delta).clone(), now_ms)?
                    }
                    PartMutation::Complete(outcome) => {
                        prepare_run_completion(part, (*outcome).clone(), now_ms)?
                    }
                };
                let payload = (part.revision != revision)
                    .then(|| encode_part_payload(&part))
                    .transpose()?;
                Ok::<_, StoreError>((part, payload))
            })
            .await
            .map_err(|error| {
                StoreError::Database(format!("part mutation worker failed: {error}"))
            })??;
        let Some((content, provider_state)) = payload else {
            return Ok(part);
        };
        let committed = run_write(db, move |txn| Box::pin(async move {
            let result = txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE agena_parts SET state = ?, content = ?, summary = ?, provider_state = ?, \
                 finished_at_ms = ?, revision = revision + 1, updated_at_ms = ? \
                 WHERE part_id = ? AND revision = ?",
                [part.state.as_str().into(), text_value(Some(content)), text_value(part.summary.clone()),
                 text_value(provider_state), Value::BigInt(part.finished_at_ms), part.updated_at_ms.into(),
                 part_id.into(), revision.into()],
            )).await.map_err(map_db_err)?;
            if result.rows_affected() == 0 {
                return Ok(None);
            }
            let message_ended = (!previous_state.is_terminal() && part.is_terminal_message())
                .then_some((session_id, part.updated_at_ms));
            bump_member_session_versions_for_parts_tx(txn, &[part_id], message_ended).await?;
            Ok(Some(part))
        })).await?;
        if let Some(part) = committed {
            return Ok(part);
        }
        tokio::task::yield_now().await;
    }
    Err(StoreError::Conflict(format!(
        "part {part_id} changed during eight mutation attempts"
    )))
}

/// History rows can contain large tool results and inline media. Database I/O
/// is asynchronous, but parsing all their JSON on the caller is CPU work.
async fn decode_part_rows(rows: Vec<sea_orm::QueryResult>) -> Result<Vec<Part>, StoreError> {
    decode_part_rows_in(&PART_DECODERS, rows).await
}

async fn decode_part_rows_in(
    pool: &'static agena_async::BlockingPool,
    rows: Vec<sea_orm::QueryResult>,
) -> Result<Vec<Part>, StoreError> {
    decode_rows(pool, rows, part_from_row).await
}

async fn decode_rows<T: Send + 'static>(
    pool: &'static agena_async::BlockingPool,
    rows: Vec<sea_orm::QueryResult>,
    decode: fn(sea_orm::QueryResult) -> Result<T, DbErr>,
) -> Result<Vec<T>, StoreError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    pool.run(move || rows.into_iter().map(decode).collect::<Result<Vec<_>, _>>())
        .await
        .map_err(|error| StoreError::Database(format!("SQLite row decode worker failed: {error}")))?
        .map_err(map_db_err)
}

async fn encode_optional_json(
    value: Option<serde_json::Value>,
) -> Result<(Option<serde_json::Value>, Option<String>), StoreError> {
    if value.is_none() {
        return Ok((None, None));
    }
    PART_MUTATION_CODECS
        .run(move || {
            let encoded = value
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|error| {
                    StoreError::Serialization(format!("encode SQLite JSON: {error}"))
                })?;
            Ok((value, encoded))
        })
        .await
        .map_err(|error| StoreError::Database(format!("SQLite encode worker failed: {error}")))?
}

/// Validate relationships/state without loading a tool result or provider
/// continuation payload into the writer transaction.
struct PartControl {
    part_id: i64,
    kind: String,
    role: PartRole,
    state: PartState,
    origin_session_id: i64,
    run_id: Option<i64>,
    created_at_ms: i64,
    finished_at_ms: Option<i64>,
}

impl PartControl {
    fn is_run_marker(&self) -> bool {
        self.kind == "run"
    }
    fn is_terminal_message(&self) -> bool {
        self.is_run_marker()
            && matches!(self.role, PartRole::User | PartRole::Assistant)
            && self.state.is_terminal()
    }
}

async fn load_part_control(
    txn: &DatabaseTransaction,
    part_id: i64,
) -> Result<Option<PartControl>, StoreError> {
    let row = txn.query_one(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
        "SELECT part_id, kind, role, state, origin_session_id, run_id, created_at_ms, finished_at_ms FROM agena_parts WHERE part_id = ?",
        [part_id.into()],
    )).await.map_err(map_db_err)?;
    row.map(|row| {
        let role: String = row.try_get("", "role").map_err(map_db_err)?;
        let state: String = row.try_get("", "state").map_err(map_db_err)?;
        Ok(PartControl {
            part_id: row.try_get("", "part_id").map_err(map_db_err)?,
            kind: row.try_get("", "kind").map_err(map_db_err)?,
            role: PartRole::parse(&role)
                .ok_or_else(|| StoreError::Database("invalid part role".into()))?,
            state: PartState::parse(&state)
                .ok_or_else(|| StoreError::Database("invalid part state".into()))?,
            origin_session_id: row.try_get("", "origin_session_id").map_err(map_db_err)?,
            run_id: row.try_get("", "run_id").map_err(map_db_err)?,
            created_at_ms: row.try_get("", "created_at_ms").map_err(map_db_err)?,
            finished_at_ms: row.try_get("", "finished_at_ms").map_err(map_db_err)?,
        })
    })
    .transpose()
}

/// Load a single part by id through any connection (db or transaction).
async fn load_part_by_id<C: ConnectionTrait>(
    connection: &C,
    part_id: i64,
) -> Result<Option<Part>, StoreError> {
    load_part_by_id_in(connection, part_id, &PART_DECODERS).await
}

async fn load_part_by_id_tx(
    txn: &DatabaseTransaction,
    part_id: i64,
) -> Result<Option<Part>, StoreError> {
    load_part_by_id_in(txn, part_id, &TRANSACTION_CODECS).await
}

async fn load_part_by_id_in<C: ConnectionTrait>(
    connection: &C,
    part_id: i64,
    pool: &'static agena_async::BlockingPool,
) -> Result<Option<Part>, StoreError> {
    let row = connection
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("SELECT {PART_COLS} FROM agena_parts p WHERE p.part_id = ?"),
            [part_id.into()],
        ))
        .await
        .map_err(map_db_err)?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(decode_part_rows_in(pool, vec![row]).await?.pop())
}

async fn load_background_operation<C: ConnectionTrait>(
    connection: &C,
    operation_id: &str,
) -> Result<Option<BackgroundOperation>, StoreError> {
    load_background_operation_in(connection, operation_id, &PART_DECODERS).await
}

async fn load_background_operation_tx(
    txn: &DatabaseTransaction,
    operation_id: &str,
) -> Result<Option<BackgroundOperation>, StoreError> {
    load_background_operation_in(txn, operation_id, &TRANSACTION_CODECS).await
}

async fn load_background_operation_in<C: ConnectionTrait>(
    connection: &C,
    operation_id: &str,
    pool: &'static agena_async::BlockingPool,
) -> Result<Option<BackgroundOperation>, StoreError> {
    let row = connection
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT {BACKGROUND_OPERATION_COLS} FROM agena_background_operations \
                 WHERE operation_id = ?"
            ),
            [operation_id.into()],
        ))
        .await
        .map_err(map_db_err)?;
    Ok(decode_rows(
        pool,
        row.into_iter().collect(),
        background_operation_from_row,
    )
    .await?
    .pop())
}

async fn load_background_delivery_tx(
    txn: &DatabaseTransaction,
    delivery_id: &str,
) -> Result<Option<BackgroundDelivery>, StoreError> {
    load_background_delivery_in(txn, delivery_id, &TRANSACTION_CODECS).await
}

async fn load_background_delivery_in<C: ConnectionTrait>(
    connection: &C,
    delivery_id: &str,
    pool: &'static agena_async::BlockingPool,
) -> Result<Option<BackgroundDelivery>, StoreError> {
    let row = connection
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT {BACKGROUND_DELIVERY_COLS} FROM agena_background_deliveries \
                 WHERE delivery_id = ?"
            ),
            [delivery_id.into()],
        ))
        .await
        .map_err(map_db_err)?;
    Ok(decode_rows(
        pool,
        row.into_iter().collect(),
        background_delivery_from_row,
    )
    .await?
    .pop())
}

/// Build a run marker `Part` ready for insertion (batch root).
///
/// `finished_at_ms` mirrors the schema lifecycle invariant (a terminal state
/// must carry a finish time), exactly like `content_part`. `submit_batch_tx`
/// normally runs with a `Pending` marker, but a terminal-state marker (e.g. a
/// marker created by a batch that is committed already-terminal) would violate
/// the schema CHECK
/// `(state IN ('completed','failed','cancelled') AND finished_at_ms IS NOT NULL)`
/// if this stayed `None`.
fn marker_part(
    marker_id: i64,
    session_id: i64,
    role: PartRole,
    state: PartState,
    content: serde_json::Value,
    now_ms: i64,
) -> Part {
    Part {
        part_id: marker_id,
        kind: "run".to_owned(),
        role,
        state,
        content,
        summary: None,
        visibility: PartVisibility::Both,
        parent_part_id: None,
        run_id: None,
        origin_session_id: session_id,
        revision: 1,
        started_at_ms: now_ms,
        finished_at_ms: state.is_terminal().then_some(now_ms),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        provider_state: None,
    }
}

/// Build a content `Part` bound to `run_id`, ready for insertion.
fn content_part(id: i64, session_id: i64, run_id: i64, new_part: NewPart, now_ms: i64) -> Part {
    Part {
        part_id: id,
        kind: new_part.kind,
        role: new_part.role,
        state: new_part.state,
        content: new_part.content,
        summary: new_part.summary,
        visibility: new_part.visibility,
        parent_part_id: new_part.parent_part_id,
        run_id: Some(run_id),
        origin_session_id: session_id,
        revision: 1,
        started_at_ms: now_ms,
        finished_at_ms: new_part.state.is_terminal().then_some(now_ms),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        provider_state: None,
    }
}

/// The user-send run marker content.
fn user_send_marker_content(execution_id: Option<&str>) -> serde_json::Value {
    let mut content = serde_json::json!({ "run_kind": "user_send", "abort_reason": null });
    if let Some(execution_id) = execution_id {
        content["execution_id"] = serde_json::Value::String(execution_id.to_owned());
    }
    content
}

#[async_trait]
impl PersistenceEngine for SqliteEngine {
    async fn create_session(&self, new_session: NewSession) -> Result<SessionMeta, StoreError> {
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {
                let id = create_session_tx(txn, new_session).await?;
                session_meta_tx(txn, id).await
            })
        })
        .await
    }

    async fn session_meta(&self, session_id: i64) -> Result<SessionMeta, StoreError> {
        session_meta_in(self.db(), session_id, &PART_DECODERS).await
    }

    async fn user_message_count(&self, session_id: i64) -> Result<u64, StoreError> {
        let row = self
            .db()
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS user_message_count \
                 FROM agena_session_parts sp \
                 JOIN agena_parts p ON p.part_id = sp.part_id \
                 WHERE sp.session_id = ? AND p.kind = 'run' AND p.role = 'user'",
                [session_id.into()],
            ))
            .await
            .map_err(map_db_err)?;
        let Some(row) = row else {
            return Err(StoreError::not_found(format!("session {session_id}")));
        };
        let count: i64 = row.try_get("", "user_message_count").map_err(map_db_err)?;
        u64::try_from(count).map_err(|_| {
            StoreError::InvalidState(format!(
                "negative user message count for session {session_id}"
            ))
        })
    }

    async fn user_message_ordinal(
        &self,
        session_id: i64,
        part_id: i64,
    ) -> Result<Option<u64>, StoreError> {
        // One statement both proves the anchor is a user-send run marker of
        // this session and ranks it in durable `(created_at_ms, part_id)`
        // order. No row means the part is not a user message at all.
        let row = self
            .db()
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT ( \
                     SELECT COUNT(*) AS ordinal \
                     FROM agena_session_parts sp \
                     JOIN agena_parts p ON p.part_id = sp.part_id \
                     WHERE sp.session_id = ? AND p.kind = 'run' AND p.role = 'user' \
                       AND (p.created_at_ms < anchor.created_at_ms \
                            OR (p.created_at_ms = anchor.created_at_ms \
                                AND p.part_id <= anchor.part_id)) \
                 ) AS ordinal \
                 FROM agena_session_parts asp \
                 JOIN agena_parts anchor ON anchor.part_id = asp.part_id \
                 WHERE asp.session_id = ? AND anchor.part_id = ? \
                   AND anchor.kind = 'run' AND anchor.role = 'user'",
                [session_id.into(), session_id.into(), part_id.into()],
            ))
            .await
            .map_err(map_db_err)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let ordinal: i64 = row.try_get("", "ordinal").map_err(map_db_err)?;
        u64::try_from(ordinal).map(Some).map_err(|_| {
            StoreError::InvalidState(format!("negative user message ordinal for part {part_id}"))
        })
    }

    async fn load_session(&self, session_id: i64) -> Result<SessionView, StoreError> {
        let meta = self.session_meta(session_id).await?;
        let parts = self
            .db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT {PART_COLS} FROM agena_parts p \
                     JOIN agena_session_parts sp ON sp.part_id = p.part_id \
                     WHERE sp.session_id = ? \
                     ORDER BY p.created_at_ms, p.part_id"
                ),
                [session_id.into()],
            ))
            .await
            .map_err(map_db_err)?;
        let parts = decode_part_rows(parts).await?;
        Ok(SessionView { meta, parts })
    }

    async fn load_part_ids(&self, session_id: i64, ids: &[i64]) -> Result<SessionView, StoreError> {
        let meta = self.session_meta(session_id).await?;
        let mut parts = Vec::new();
        for chunk in ids.chunks(256) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let values = std::iter::once(Value::from(session_id))
                .chain(chunk.iter().copied().map(Value::from))
                .collect::<Vec<_>>();
            parts.extend(decode_part_rows(self.db().query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("SELECT {PART_COLS} FROM agena_parts p JOIN agena_session_parts sp ON sp.part_id = p.part_id WHERE sp.session_id = ? AND p.part_id IN ({placeholders})"),
                values,
            )).await.map_err(map_db_err)?).await?);
        }
        parts.sort_by_key(|part| (part.created_at_ms, part.part_id));
        parts.dedup_by_key(|part| part.part_id);
        Ok(SessionView { meta, parts })
    }

    async fn load_owned_parts_by_kind(
        &self,
        session_id: i64,
        kind: &str,
    ) -> Result<SessionView, StoreError> {
        let meta = self.session_meta(session_id).await?;
        let parts = self.db().query_all(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("SELECT {PART_COLS} FROM agena_parts p JOIN agena_session_parts sp ON sp.part_id = p.part_id WHERE sp.session_id = ? AND p.origin_session_id = ? AND p.kind = ? ORDER BY p.created_at_ms, p.part_id"),
            [session_id.into(), session_id.into(), kind.to_owned().into()],
        )).await.map_err(map_db_err)?;
        let parts = decode_part_rows(parts).await?;
        Ok(SessionView { meta, parts })
    }

    async fn load_session_page(
        &self,
        session_id: i64,
        before: Option<PartCursor>,
        limit: i64,
    ) -> Result<SessionPartPage, StoreError> {
        let meta = self.session_meta(session_id).await?;
        let fetch_limit = limit.max(1).saturating_add(1);
        let mut values = vec![session_id.into()];
        let position_clause = if let Some(before) = before {
            values.push(before.created_at_ms.into());
            values.push(before.created_at_ms.into());
            values.push(before.part_id.into());
            " AND (p.created_at_ms < ? OR (p.created_at_ms = ? AND p.part_id < ?))"
        } else {
            ""
        };
        let rows = self
            .db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT {PART_COLS} FROM agena_parts p \
                     JOIN agena_session_parts sp ON sp.part_id = p.part_id \
                     WHERE sp.session_id = ?{position_clause} \
                     ORDER BY p.created_at_ms DESC, p.part_id DESC LIMIT {fetch_limit}"
                ),
                values,
            ))
            .await
            .map_err(map_db_err)?;
        let rows = decode_part_rows(rows).await?;
        let has_more = rows.len() > usize::try_from(limit.max(1)).unwrap_or(usize::MAX);
        let mut parts = rows;
        parts.truncate(usize::try_from(limit.max(1)).unwrap_or(usize::MAX));
        Ok(SessionPartPage {
            meta,
            parts,
            has_more,
        })
    }

    async fn load_run_page(
        &self,
        session_id: i64,
        run_id: i64,
        before: Option<PartCursor>,
        limit: i64,
    ) -> Result<SessionPartPage, StoreError> {
        let meta = self.session_meta(session_id).await?;
        let fetch_limit = limit.max(1).saturating_add(1);
        let mut values = vec![session_id.into(), run_id.into()];
        let position_clause = if let Some(before) = before {
            values.push(before.created_at_ms.into());
            values.push(before.created_at_ms.into());
            values.push(before.part_id.into());
            " AND (p.created_at_ms < ? OR (p.created_at_ms = ? AND p.part_id < ?))"
        } else {
            ""
        };
        let rows = self
            .db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT {PART_COLS} FROM agena_parts p \
                     JOIN agena_session_parts sp ON sp.part_id = p.part_id \
                     WHERE sp.session_id = ? AND p.run_id = ?{position_clause} \
                     ORDER BY p.created_at_ms DESC, p.part_id DESC LIMIT {fetch_limit}"
                ),
                values,
            ))
            .await
            .map_err(map_db_err)?;
        let rows = decode_part_rows(rows).await?;
        let has_more = rows.len() > usize::try_from(limit.max(1)).unwrap_or(usize::MAX);
        let mut parts = rows;
        parts.truncate(usize::try_from(limit.max(1)).unwrap_or(usize::MAX));
        Ok(SessionPartPage {
            meta,
            parts,
            has_more,
        })
    }

    async fn newest_member_cursor(
        &self,
        session_id: i64,
    ) -> Result<Option<(i64, i64)>, StoreError> {
        self.session_meta(session_id).await?;
        self.db()
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT p.created_at_ms, p.part_id FROM agena_parts p \
                 JOIN agena_session_parts sp ON sp.part_id = p.part_id \
                 WHERE sp.session_id = ? \
                 ORDER BY p.created_at_ms DESC, p.part_id DESC LIMIT 1",
                [session_id.into()],
            ))
            .await
            .map_err(map_db_err)?
            .map(|row| {
                let created_at_ms: i64 = row.try_get("", "created_at_ms").map_err(map_db_err)?;
                let part_id: i64 = row.try_get("", "part_id").map_err(map_db_err)?;
                Ok((created_at_ms, part_id))
            })
            .transpose()
    }

    async fn rename_session(
        &self,
        session_id: i64,
        title: String,
    ) -> Result<SessionMeta, StoreError> {
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {
                let previous = session_meta_tx(txn, session_id).await?;
                if previous.title == title {
                    return Ok(previous);
                }
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "UPDATE agena_sessions \
                     SET title = ?, version = version + 1 WHERE id = ?",
                    [title.into(), session_id.into()],
                ))
                .await
                .map_err(map_db_err)?;
                session_meta_tx(txn, session_id).await
            })
        })
        .await
    }

    async fn update_session_metadata(
        &self,
        session_id: i64,
        patch: SessionMetadataPatch,
    ) -> Result<SessionMeta, StoreError> {
        if patch.is_empty() {
            return Err(StoreError::InvalidState(
                "session metadata patch cannot be empty".to_owned(),
            ));
        }
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {
                let previous = session_meta_tx(txn, session_id).await?;
                if patch
                    .title
                    .as_ref()
                    .is_none_or(|title| *title == previous.title)
                    && patch
                        .favorite
                        .is_none_or(|favorite| favorite == previous.favorite)
                    && patch.pinned.is_none_or(|pinned| pinned == previous.pinned)
                {
                    return Ok(previous);
                }
                let has_title = patch.title.is_some();
                let has_favorite = patch.favorite.is_some();
                let has_pinned = patch.pinned.is_some();
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "UPDATE agena_sessions SET \
                     title = CASE WHEN ? THEN ? ELSE title END, \
                     favorite = CASE WHEN ? THEN ? ELSE favorite END, \
                     pinned = CASE WHEN ? THEN ? ELSE pinned END, \
                     version = version + 1 WHERE id = ?",
                    [
                        has_title.into(),
                        text_value(patch.title),
                        has_favorite.into(),
                        patch.favorite.unwrap_or(false).into(),
                        has_pinned.into(),
                        patch.pinned.unwrap_or(false).into(),
                        session_id.into(),
                    ],
                ))
                .await
                .map_err(map_db_err)?;
                session_meta_tx(txn, session_id).await
            })
        })
        .await
    }

    async fn set_provider_anchors(
        &self,
        session_id: i64,
        anchors: Option<serde_json::Value>,
    ) -> Result<SessionMeta, StoreError> {
        let db = self.db();
        let (anchors, anchors_json) = encode_optional_json(anchors).await?;
        run_write(db, move |txn| {
            Box::pin(async move {
                let previous = session_meta_tx(txn, session_id).await?;
                if previous.provider_anchors_json == anchors {
                    return Ok(previous);
                }
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "UPDATE agena_sessions \
                     SET provider_anchors_json = ?, version = version + 1 \
                     WHERE id = ?",
                    [text_value(anchors_json), session_id.into()],
                ))
                .await
                .map_err(map_db_err)?;
                session_meta_tx(txn, session_id).await
            })
        })
        .await
    }

    async fn set_config_json(
        &self,
        session_id: i64,
        config: Option<serde_json::Value>,
    ) -> Result<SessionMeta, StoreError> {
        let db = self.db();
        let (config, config_json) = encode_optional_json(config).await?;
        run_write(db, move |txn| {
            Box::pin(async move {
                let previous = session_meta_tx(txn, session_id).await?;
                if previous.config_json == config {
                    return Ok(previous);
                }
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "UPDATE agena_sessions \
                     SET config_json = ?, version = version + 1 WHERE id = ?",
                    [text_value(config_json), session_id.into()],
                ))
                .await
                .map_err(map_db_err)?;
                session_meta_tx(txn, session_id).await
            })
        })
        .await
    }

    async fn find_subagent_by_task_id(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Option<SessionMeta>, StoreError> {
        let rows = self
            .db()
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT {SESSION_COLS} FROM agena_sessions s \
                     WHERE s.parent_id = ? AND s.task_id = ?"
                ),
                [parent_session_id.into(), task_id.into()],
            ))
            .await
            .map_err(map_db_err)?;
        Ok(
            decode_rows(&PART_DECODERS, rows.into_iter().collect(), meta_from_row)
                .await?
                .pop(),
        )
    }

    async fn create_subagent_session(
        &self,
        parent_session_id: i64,
        task_id: String,
        title: String,
        now_ms: i64,
    ) -> Result<SessionMeta, StoreError> {
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {
                let (depth, root_id, workspace_id) = {
                    let parent = session_meta_tx(txn, parent_session_id).await?;
                    if parent.lifecycle_state != SessionLifecycleState::Ready {
                        return Err(StoreError::InvalidState(format!(
                            "parent session {parent_session_id} is not ready"
                        )));
                    }
                    (parent.depth + 1, parent.root_id, parent.workspace_id)
                };
                let now = now_ms;
                let result = txn
                    .execute(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "INSERT INTO agena_sessions \
                         (parent_id, depth, root_id, workspace_id, relation_kind, cutoff_part_id, title, \
                          version, lifecycle_state, task_id, subtask_status, config_json, provider_anchors_json, \
                          created_at_ms, updated_at_ms) \
                         VALUES (?, ?, ?, ?, 'subagent', NULL, ?, 1, 'creating', ?, 'created', NULL, NULL, ?, ?)",
                        [
                            parent_session_id.into(),
                            depth.into(),
                            root_id.into(),
                            workspace_id.into(),
                            title.into(),
                            task_id.into(),
                            now.into(),
                            now.into(),
                        ],
                    ))
                    .await
                    .map_err(map_db_err)?;
                let id = i64::try_from(result.last_insert_id()).map_err(|_| {
                    StoreError::Database("session identifier exceeds i64 range".to_owned())
                })?;
                session_meta_tx(txn, id).await
            })
        })
        .await
    }

    async fn update_subtask_state(
        &self,
        session_id: i64,
        status: Option<String>,
        started_at_ms: Option<i64>,
        finished_at_ms: Option<i64>,
        failure: Option<serde_json::Value>,
    ) -> Result<SessionMeta, StoreError> {
        let db = self.db();
        let (failure, failure_json) = encode_optional_json(failure).await?;
        run_write(db, move |txn| {
            Box::pin(async move {
                let previous = session_meta_tx(txn, session_id).await?;
                if previous.subtask_status == status
                    && previous.subtask_started_at_ms == started_at_ms
                    && previous.subtask_finished_at_ms == finished_at_ms
                    && previous.subtask_failure == failure
                {
                    return Ok(previous);
                }
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "UPDATE agena_sessions \
                     SET subtask_status = ?, subtask_started_at_ms = ?, \
                         subtask_finished_at_ms = ?, subtask_failure_json = ?, \
                         version = version + 1 \
                     WHERE id = ?",
                    [
                        text_value(status),
                        Value::BigInt(started_at_ms),
                        Value::BigInt(finished_at_ms),
                        text_value(failure_json),
                        session_id.into(),
                    ],
                ))
                .await
                .map_err(map_db_err)?;
                session_meta_tx(txn, session_id).await
            })
        })
        .await
    }

    async fn session_revision_rows(&self) -> Result<Vec<(i64, i64, i64)>, StoreError> {
        self.db().query_all(Statement::from_string(DatabaseBackend::Sqlite,
            "SELECT id, workspace_id, version FROM agena_sessions WHERE json_extract(config_json, '$.conversation.mode') IS NOT 'btw'"))
            .await.map_err(map_db_err)?.into_iter().map(|row| {
                Ok((row.try_get("", "id").map_err(map_db_err)?,
                    row.try_get("", "workspace_id").map_err(map_db_err)?,
                    row.try_get("", "version").map_err(map_db_err)?))
            }).collect()
    }

    async fn list_session_summaries(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        let (where_sql, values) = session_list_filter(&query);
        let mut sql = format!(
            "SELECT s.id, s.workspace_id, s.parent_id, s.depth, s.root_id, s.title, \
                    s.favorite, s.pinned, \
                    s.relation_kind, s.lifecycle_state, s.version, s.task_id, s.subtask_status, \
                    s.created_at_ms, s.updated_at_ms, \
             (SELECT COUNT(*) FROM agena_session_parts sp \
               JOIN agena_parts p ON p.part_id = sp.part_id \
               WHERE sp.session_id = s.id AND p.kind = 'run') AS message_count, \
             (SELECT COUNT(*) FROM agena_sessions c WHERE c.parent_id = s.id AND json_extract(c.config_json, '$.conversation.mode') IS NOT 'btw') \
               AS child_session_count, \
             (SELECT MAX(p.finished_at_ms) FROM agena_session_parts sp \
               JOIN agena_parts p ON p.part_id = sp.part_id \
               WHERE sp.session_id = s.id AND p.kind = 'run' \
                 AND p.role IN ('user', 'assistant') \
                 AND p.state IN ('completed', 'failed', 'cancelled')) AS last_message_at_ms \
             FROM agena_sessions s {where_sql} \
             ORDER BY s.updated_at_ms DESC, s.id DESC"
        );
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit.max(0)));
        }
        if query.offset > 0 {
            if query.limit.is_none() {
                sql.push_str(" LIMIT -1");
            }
            sql.push_str(&format!(" OFFSET {}", query.offset.min(i64::MAX as u64)));
        }
        self.db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await
            .map_err(map_db_err)?
            .into_iter()
            .map(summary_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_db_err)
    }

    async fn workspace_session_stats(
        &self,
        workspace_ids: &[i64],
    ) -> Result<HashMap<i64, agena_storage::store::WorkspaceSessionStats>, StoreError> {
        if workspace_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = vec!["?"; workspace_ids.len()].join(", ");
        let projection = session_state_projection_sql();
        let rows = self.db().query_all(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            format!("SELECT workspace_id, COUNT(*) AS total, SUM(parent_id IS NULL) AS roots, SUM(pinned) AS pinned, SUM(state IN ('running', 'creating')) AS running, SUM(state IN ('awaiting_interaction', 'failed')) AS attention FROM (SELECT s.workspace_id, s.parent_id, s.pinned, {projection} AS state FROM agena_sessions s WHERE s.is_subagent = 0 AND json_extract(s.config_json, '$.conversation.mode') IS NOT 'btw' AND s.workspace_id IN ({placeholders})) GROUP BY workspace_id"),
            workspace_ids.iter().copied().map(Value::from),
        )).await.map_err(map_db_err)?;
        let mut result = workspace_ids
            .iter()
            .map(|id| (*id, agena_storage::store::WorkspaceSessionStats::default()))
            .collect::<HashMap<_, _>>();
        for row in rows {
            let id: i64 = row.try_get("", "workspace_id").map_err(map_db_err)?;
            let count = |name: &str| -> Result<u64, StoreError> {
                Ok(row.try_get::<i64>("", name).map_err(map_db_err)?.max(0) as u64)
            };
            result.insert(
                id,
                agena_storage::store::WorkspaceSessionStats {
                    total: count("total")?,
                    roots: count("roots")?,
                    pinned: count("pinned")?,
                    running: count("running")?,
                    attention: count("attention")?,
                },
            );
        }
        Ok(result)
    }

    async fn count_session_summaries(
        &self,
        mut query: SessionListQuery,
    ) -> Result<u64, StoreError> {
        query.before = None;
        let (where_sql, values) = session_list_filter(&query);
        let row = self
            .db()
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!("SELECT COUNT(*) AS count FROM agena_sessions s {where_sql}"),
                values,
            ))
            .await
            .map_err(map_db_err)?
            .ok_or_else(|| StoreError::InvalidState("session count missing".into()))?;
        let count: i64 = row.try_get("", "count").map_err(map_db_err)?;
        Ok(count.max(0) as u64)
    }

    async fn session_states(
        &self,
        session_ids: &[i64],
        _now_ms: i64,
    ) -> Result<HashMap<i64, SessionState>, StoreError> {
        if session_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = vec!["?"; session_ids.len()].join(", ");
        let values = session_ids
            .iter()
            .copied()
            .map(Value::from)
            .collect::<Vec<_>>();
        let state_projection = session_state_projection_sql();
        let rows = self
            .db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT s.id, {state_projection} AS session_state \
                     FROM agena_sessions s WHERE s.id IN ({placeholders})"
                ),
                values,
            ))
            .await
            .map_err(map_db_err)?;
        let mut states = HashMap::with_capacity(rows.len());
        for row in rows {
            let session_id: i64 = row.try_get("", "id").map_err(map_db_err)?;
            let state: String = row.try_get("", "session_state").map_err(map_db_err)?;
            let state = SessionState::parse(state.as_str()).ok_or_else(|| {
                StoreError::InvalidState(format!(
                    "invalid derived session state '{state}' for session {session_id}"
                ))
            })?;
            states.insert(session_id, state);
        }
        Ok(states)
    }

    async fn get_session_summary(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionSummary>, StoreError> {
        let rows = self
            .db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT s.id, s.workspace_id, s.parent_id, s.depth, s.root_id, s.title, \
                        s.favorite, s.pinned, \
                        s.relation_kind, s.lifecycle_state, s.version, s.task_id, s.subtask_status, \
                        s.created_at_ms, s.updated_at_ms, \
                 (SELECT COUNT(*) FROM agena_session_parts sp \
                   JOIN agena_parts p ON p.part_id = sp.part_id \
                   WHERE sp.session_id = s.id AND p.kind = 'run') AS message_count, \
                 (SELECT COUNT(*) FROM agena_sessions c WHERE c.parent_id = s.id AND json_extract(c.config_json, '$.conversation.mode') IS NOT 'btw') \
                   AS child_session_count, \
                 (SELECT MAX(p.finished_at_ms) FROM agena_session_parts sp \
                   JOIN agena_parts p ON p.part_id = sp.part_id \
                   WHERE sp.session_id = s.id AND p.kind = 'run' \
                     AND p.role IN ('user', 'assistant') \
                     AND p.state IN ('completed', 'failed', 'cancelled')) AS last_message_at_ms \
                 FROM agena_sessions s WHERE s.id = ?",
                [session_id.into()],
            ))
            .await
            .map_err(map_db_err)?;
        let mut iter = rows.into_iter();
        let Some(row) = iter.next() else {
            return Ok(None);
        };
        Ok(Some(summary_from_row(row).map_err(map_db_err)?))
    }

    async fn session_counts_by_workspace(
        &self,
        workspace_ids: &[i64],
    ) -> Result<HashMap<i64, i64>, StoreError> {
        if workspace_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = vec!["?"; workspace_ids.len()].join(", ");
        let values: Vec<Value> = workspace_ids.iter().map(|id| Value::from(*id)).collect();
        let rows = self
            .db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT workspace_id, COUNT(*) AS count FROM agena_sessions \
                     WHERE json_extract(config_json, '$.conversation.mode') IS NOT 'btw' AND workspace_id IN ({placeholders}) GROUP BY workspace_id"
                ),
                values,
            ))
            .await
            .map_err(map_db_err)?;
        let mut counts = HashMap::with_capacity(workspace_ids.len());
        for row in rows {
            let workspace_id: i64 = row.try_get("", "workspace_id").map_err(map_db_err)?;
            let count: i64 = row.try_get("", "count").map_err(map_db_err)?;
            counts.insert(workspace_id, count);
        }
        Ok(counts)
    }

    async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, StoreError> {
        self.db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT s.id, s.workspace_id, s.parent_id, s.depth, s.root_id, s.title, \
                        s.favorite, s.pinned, \
                        s.relation_kind, s.lifecycle_state, s.version, s.task_id, s.subtask_status, \
                        s.created_at_ms, s.updated_at_ms, \
                 (SELECT COUNT(*) FROM agena_session_parts sp \
                   JOIN agena_parts p ON p.part_id = sp.part_id \
                   WHERE sp.session_id = s.id AND p.kind = 'run') AS message_count, \
                     (SELECT COUNT(*) FROM agena_sessions c WHERE c.parent_id = s.id AND json_extract(c.config_json, '$.conversation.mode') IS NOT 'btw') \
                       AS child_session_count, \
                     (SELECT MAX(p.finished_at_ms) FROM agena_session_parts sp \
                       JOIN agena_parts p ON p.part_id = sp.part_id \
                       WHERE sp.session_id = s.id AND p.kind = 'run' \
                         AND p.role IN ('user', 'assistant') \
                         AND p.state IN ('completed', 'failed', 'cancelled')) AS last_message_at_ms \
                     FROM agena_sessions s WHERE s.root_id = ? \
                     ORDER BY s.updated_at_ms DESC, s.id DESC",
                [root_id.into()],
            ))
            .await
            .map_err(map_db_err)?
            .into_iter()
            .map(summary_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_db_err)
    }

    async fn delete_session(&self, session_id: i64) -> Result<(), StoreError> {
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {
                // Descendant sessions and membership edges cascade via FKs.
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "DELETE FROM agena_sessions WHERE id = ?",
                    [session_id.into()],
                ))
                .await
                .map_err(map_db_err)?;
                Ok(())
            })
        })
        .await
    }

    async fn create_background_operation(
        &self,
        new: NewBackgroundOperation,
        now_ms: i64,
    ) -> Result<BackgroundOperation, StoreError> {
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {
                if let Some(existing) = load_background_operation_tx(txn, &new.operation_id).await? {
                    if existing.session_id == new.session_id
                        && existing.launch_run_id == new.launch_run_id
                        && existing.launch_tool_part_id == new.launch_tool_part_id
                        && existing.kind == new.kind
                    {
                        return Ok(existing);
                    }
                    return Err(StoreError::InvalidState(format!(
                        "background operation {} already identifies a different launch",
                        new.operation_id
                    )));
                }
                match (new.launch_run_id, new.launch_tool_part_id) {
                    (None, None) if new.kind == BackgroundOperationKind::ScheduledDelivery => {
                        session_meta_tx(txn, new.session_id).await?;
                    }
                    (Some(run_id), Some(tool_part_id)) => {
                        if new.kind != BackgroundOperationKind::ScheduledDelivery
                            && let Some(row) = txn
                                .query_one(Statement::from_sql_and_values(
                                    DatabaseBackend::Sqlite,
                                    format!(
                                        "SELECT {BACKGROUND_OPERATION_COLS} FROM agena_background_operations \
                                         WHERE session_id = ? AND launch_tool_part_id = ? \
                                           AND kind != 'scheduled_delivery'"
                                    ),
                                    [new.session_id.into(), tool_part_id.into()],
                                ))
                                .await
                                .map_err(map_db_err)?
                        {
                            let existing = decode_rows(&TRANSACTION_CODECS, vec![row], background_operation_from_row)
                                .await?.pop().expect("one background operation row");
                            if existing.operation_id == new.operation_id
                                && existing.kind == new.kind
                            {
                                return Ok(existing);
                            }
                            return Err(StoreError::InvalidState(format!(
                                "tool part {tool_part_id} already owns background operation {}",
                                existing.operation_id
                            )));
                        }
                        let run = load_part_control(txn, run_id).await?.ok_or_else(|| {
                            StoreError::not_found(format!("run marker {run_id}"))
                        })?;
                        if !run.is_run_marker() || run.origin_session_id != new.session_id {
                            return Err(StoreError::InvalidState(format!(
                                "background launch run {run_id} does not belong to session {}",
                                new.session_id
                            )));
                        }
                        let tool = load_part_control(txn, tool_part_id).await?.ok_or_else(|| {
                            StoreError::not_found(format!("tool part {tool_part_id}"))
                        })?;
                        if tool.kind != "tool_call"
                            || tool.origin_session_id != new.session_id
                            || tool.run_id != Some(run_id)
                        {
                            return Err(StoreError::InvalidState(format!(
                                "background launch tool {tool_part_id} is not owned by run {run_id} in session {}",
                                new.session_id
                            )));
                        }
                    }
                    _ => {
                        return Err(StoreError::InvalidState(
                            "background launch ids must be paired, and non-scheduled operations require them"
                                .to_owned(),
                        ));
                    }
                }
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "INSERT INTO agena_background_operations \
                     (operation_id, session_id, launch_run_id, launch_tool_part_id, kind, phase, \
                      last_event_seq, revision, created_at_ms, updated_at_ms) \
                     VALUES (?, ?, ?, ?, ?, 'launch_requested', 0, 1, ?, ?)",
                    [
                        new.operation_id.as_str().into(),
                        new.session_id.into(),
                        Value::BigInt(new.launch_run_id),
                        Value::BigInt(new.launch_tool_part_id),
                        new.kind.as_str().into(),
                        now_ms.into(),
                        now_ms.into(),
                    ],
                ))
                .await
                .map_err(map_db_err)?;
                load_background_operation_tx(txn, &new.operation_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::not_found(format!(
                            "background operation {} after insert",
                            new.operation_id
                        ))
                    })
            })
        })
        .await
    }

    async fn background_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<BackgroundOperation>, StoreError> {
        load_background_operation(self.db(), operation_id).await
    }

    async fn background_operation_by_external_id(
        &self,
        kind: BackgroundOperationKind,
        external_id: &str,
    ) -> Result<Option<BackgroundOperation>, StoreError> {
        let rows = self
            .db()
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT {BACKGROUND_OPERATION_COLS} FROM agena_background_operations \
                     WHERE kind = ? AND external_id = ?"
                ),
                [kind.as_str().into(), external_id.into()],
            ))
            .await
            .map_err(map_db_err)?;
        Ok(decode_rows(
            &PART_DECODERS,
            rows.into_iter().collect(),
            background_operation_from_row,
        )
        .await?
        .pop())
    }

    async fn active_background_operations(
        &self,
        kind: Option<BackgroundOperationKind>,
        limit: usize,
    ) -> Result<Vec<BackgroundOperation>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let (sql, values) = match kind {
            Some(kind) => (
                format!(
                    "SELECT {BACKGROUND_OPERATION_COLS} FROM agena_background_operations \
                     WHERE phase IN ('launch_requested','launching','running') AND kind = ? \
                     ORDER BY created_at_ms ASC, operation_id ASC LIMIT ?"
                ),
                vec![kind.as_str().into(), limit.into()],
            ),
            None => (
                format!(
                    "SELECT {BACKGROUND_OPERATION_COLS} FROM agena_background_operations \
                     WHERE phase IN ('launch_requested','launching','running') \
                     ORDER BY created_at_ms ASC, operation_id ASC LIMIT ?"
                ),
                vec![limit.into()],
            ),
        };
        let rows = self
            .db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                sql,
                values,
            ))
            .await
            .map_err(map_db_err)?;
        decode_rows(&PART_DECODERS, rows, background_operation_from_row).await
    }

    async fn session_has_active_background_operations(
        &self,
        session_id: i64,
    ) -> Result<bool, StoreError> {
        Ok(self.db().query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT 1 FROM agena_background_operations WHERE session_id = ? AND phase IN ('launch_requested','launching','running') LIMIT 1",
            [session_id.into()],
        )).await.map_err(map_db_err)?.is_some())
    }

    async fn transition_background_operation(
        &self,
        transition: BackgroundOperationTransition,
        now_ms: i64,
    ) -> Result<BackgroundOperation, StoreError> {
        let db = self.db();
        let mut transition = transition;
        let (_, outcome_json) = encode_optional_json(transition.outcome.take()).await?;
        let (_, failure_json) = encode_optional_json(transition.failure.take()).await?;
        run_write(db, move |txn| {
            Box::pin(async move {
                let current = load_background_operation_tx(txn, &transition.operation_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::not_found(format!(
                            "background operation {}",
                            transition.operation_id
                        ))
                    })?;
                if current.revision != transition.expected_revision {
                    return Err(StoreError::InvalidState(format!(
                        "background operation {} revision changed: expected {}, found {}",
                        current.operation_id, transition.expected_revision, current.revision
                    )));
                }
                if !current.phase.can_transition(transition.next_phase) {
                    return Err(StoreError::InvalidState(format!(
                        "invalid background transition {} -> {} for {}",
                        current.phase.as_str(),
                        transition.next_phase.as_str(),
                        current.operation_id
                    )));
                }
                let external_id = transition.external_id.or(current.external_id);
                if transition.next_phase == BackgroundOperationPhase::Running
                    && external_id.is_none()
                {
                    return Err(StoreError::InvalidState(format!(
                        "background operation {} cannot enter running without an external id",
                        current.operation_id
                    )));
                }
                let finished_at_ms = transition.next_phase.is_terminal().then_some(now_ms);
                let result = txn
                    .execute(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "UPDATE agena_background_operations \
                         SET external_id = ?, phase = ?, outcome_json = COALESCE(?, outcome_json), failure_json = COALESCE(?, failure_json), \
                             owner_id = ?, lease_until_ms = ?, revision = revision + 1, \
                             updated_at_ms = ?, finished_at_ms = ? \
                         WHERE operation_id = ? AND revision = ?",
                        [
                            text_value(external_id),
                            transition.next_phase.as_str().into(),
                            text_value(outcome_json),
                            text_value(failure_json),
                            text_value(transition.owner_id),
                            Value::BigInt(transition.lease_until_ms),
                            now_ms.into(),
                            Value::BigInt(finished_at_ms),
                            transition.operation_id.as_str().into(),
                            transition.expected_revision.into(),
                        ],
                    ))
                    .await
                    .map_err(map_db_err)?;
                if result.rows_affected() != 1 {
                    return Err(StoreError::InvalidState(format!(
                        "background operation {} changed concurrently",
                        transition.operation_id
                    )));
                }
                load_background_operation_tx(txn, &transition.operation_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::not_found(format!(
                            "background operation {} after transition",
                            transition.operation_id
                        ))
                    })
            })
        })
        .await
    }

    async fn record_background_event(
        &self,
        request: BackgroundEventRequest,
        now_ms: i64,
    ) -> Result<BackgroundSettleOutcome, StoreError> {
        let db = self.db();
        let (request, notification_json, delivery_json) = PART_MUTATION_CODECS
            .run(move || {
                let mut request = request;
                request.notification.state = PartState::Completed;
                let content = request
                    .notification
                    .content
                    .as_object_mut()
                    .ok_or_else(|| {
                        StoreError::InvalidState(
                            "background notification content must be a JSON object".into(),
                        )
                    })?;
                content.insert("delivery_protocol".into(), "provider_round".into());
                let encoded =
                    serde_json::to_string(&request.notification.content).map_err(|error| {
                        StoreError::Serialization(format!("encode background delivery: {error}"))
                    })?;
                let delivery_json = encoded.clone();
                Ok::<_, StoreError>((request, encoded, delivery_json))
            })
            .await
            .map_err(|error| {
                StoreError::Database(format!("background encode worker failed: {error}"))
            })??;
        let mut request = request;
        let (_, outcome_json) = encode_optional_json(request.outcome.take()).await?;
        let (_, failure_json) = encode_optional_json(request.failure.take()).await?;
        run_write(db, move |txn| {
            Box::pin(async move {
                let delivery_id = format!("{}:{}", request.operation_id, request.event_key);
                if let Some(delivery) = load_background_delivery_tx(txn, &delivery_id).await? {
                    let operation = load_background_operation_tx(txn, &request.operation_id)
                        .await?
                        .ok_or_else(|| {
                            StoreError::not_found(format!(
                                "background operation {}",
                                request.operation_id
                            ))
                        })?;
                    let notification_part_id = delivery.notification_part_id.ok_or_else(|| {
                        StoreError::InvalidState(format!(
                            "background delivery {} has no notification part",
                            delivery.delivery_id
                        ))
                    })?;
                    let notification_part = load_part_by_id_tx(txn, notification_part_id)
                        .await?
                        .ok_or_else(|| {
                            StoreError::not_found(format!("part {notification_part_id}"))
                        })?;
                    return Ok(BackgroundSettleOutcome {
                        operation,
                        delivery,
                        notification_part,
                        created: false,
                    });
                }
                let current = load_background_operation_tx(txn, &request.operation_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::not_found(format!(
                            "background operation {}",
                            request.operation_id
                        ))
                    })?;
                if let Some(next_phase) = request.next_phase
                    && !current.phase.can_transition(next_phase)
                {
                    return Err(StoreError::InvalidState(format!(
                        "invalid background transition {} -> {} for {}",
                        current.phase.as_str(),
                        next_phase.as_str(),
                        current.operation_id
                    )));
                }
                let mut notification = request.notification;
                let notification_part = if let Some(run_id) = current.launch_run_id {
                    let run = load_part_control(txn, run_id)
                        .await?
                        .ok_or_else(|| StoreError::not_found(format!("run marker {run_id}")))?;
                    if !run.is_run_marker()
                        || run.origin_session_id != current.session_id
                        || run.role != PartRole::Assistant
                    {
                        return Err(StoreError::InvalidState(format!(
                            "background operation {} launch run {run_id} is not an assistant run owned by session {}",
                            current.operation_id, current.session_id
                        )));
                    }
                    notification.role = PartRole::Assistant;
                    let id = next_part_id_tx(txn).await.map_err(map_db_err)?;
                    let part = content_part(id, current.session_id, run_id, notification, now_ms);
                    insert_part_tx(txn, &part, notification_json, None).await.map_err(map_db_err)?;
                    insert_membership_tx(txn, current.session_id, id, now_ms)
                        .await
                        .map_err(map_db_err)?;
                    bump_session_version_tx(txn, current.session_id, None).await?;
                    part
                } else {
                    notification.role = PartRole::Runtime;
                    let marker = serde_json::json!({
                            "run_kind": "runtime_ingress",
                            "source": "background_operation",
                            "operation_id": current.operation_id,
                            "abort_reason": null,
                        });
                    let marker_json = serde_json::to_string(&marker)
                        .map_err(|error| StoreError::Serialization(format!("encode ingress marker: {error}")))?;
                    let ingress = submit_batch_tx(
                        txn,
                        current.session_id,
                        PartRole::Runtime,
                        PartState::Completed,
                        (marker, marker_json),
                        vec![PreparedNewPart { part: notification, content: notification_json }],
                        Some(delivery_id.clone()),
                        now_ms,
                    )
                    .await?;
                    ingress.parts.into_iter().nth(1).ok_or_else(|| {
                        StoreError::InvalidState(
                            "runtime ingress omitted notification".to_owned(),
                        )
                    })?
                };

                let next_phase = request.next_phase.unwrap_or(current.phase);
                let last_event_seq = request.event_seq.map_or(current.last_event_seq, |seq| {
                    current.last_event_seq.max(seq)
                });
                let finished_at_ms = if next_phase.is_terminal() {
                    current.finished_at_ms.or(Some(now_ms))
                } else {
                    None
                };
                let owner_id = (!next_phase.is_terminal())
                    .then(|| current.owner_id.clone())
                    .flatten();
                let lease_until_ms = (!next_phase.is_terminal())
                    .then_some(current.lease_until_ms)
                    .flatten();
                let result = txn
                    .execute(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "UPDATE agena_background_operations \
                         SET phase = ?, outcome_json = COALESCE(?, outcome_json), failure_json = COALESCE(?, failure_json), last_event_seq = ?, \
                             owner_id = ?, lease_until_ms = ?, revision = revision + 1, \
                             updated_at_ms = ?, finished_at_ms = ? \
                         WHERE operation_id = ? AND revision = ?",
                        [
                            next_phase.as_str().into(),
                            text_value(outcome_json),
                            text_value(failure_json),
                            i64::try_from(last_event_seq)
                                .map_err(|_| {
                                    StoreError::InvalidState(
                                        "background event sequence exceeds SQLite range".to_owned(),
                                    )
                                })?
                                .into(),
                            text_value(owner_id),
                            Value::BigInt(lease_until_ms),
                            now_ms.into(),
                            Value::BigInt(finished_at_ms),
                            request.operation_id.as_str().into(),
                            current.revision.into(),
                        ],
                    ))
                    .await
                    .map_err(map_db_err)?;
                if result.rows_affected() != 1 {
                    return Err(StoreError::InvalidState(format!(
                        "background operation {} changed concurrently",
                        request.operation_id
                    )));
                }
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "INSERT INTO agena_background_deliveries \
                     (delivery_id, operation_id, session_id, event_key, payload_json, phase, \
                      attempts, notification_part_id, created_at_ms, updated_at_ms) \
                     VALUES (?, ?, ?, ?, ?, 'pending', 0, ?, ?, ?)",
                    [
                        delivery_id.as_str().into(),
                        request.operation_id.as_str().into(),
                        current.session_id.into(),
                        request.event_key.as_str().into(),
                        delivery_json.into(),
                        notification_part.part_id.into(),
                        now_ms.into(),
                        now_ms.into(),
                    ],
                ))
                .await
                .map_err(map_db_err)?;
                let operation = load_background_operation_tx(txn, &request.operation_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::not_found(format!(
                            "background operation {} after event",
                            request.operation_id
                        ))
                    })?;
                let delivery = load_background_delivery_tx(txn, &delivery_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::not_found(format!("background delivery {delivery_id}"))
                    })?;
                Ok(BackgroundSettleOutcome {
                    operation,
                    delivery,
                    notification_part,
                    created: true,
                })
            })
        })
        .await
    }

    async fn claim_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        claim_until_ms: i64,
        now_ms: i64,
    ) -> Result<Option<BackgroundDelivery>, StoreError> {
        let owner_id = owner_id.to_owned();
        let db = self.db();
        let delivery_id = delivery_id.to_owned();
        run_write(db, move |txn| {
            Box::pin(async move {
                let result = txn
                    .execute(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "UPDATE agena_background_deliveries \
                         SET phase = 'claimed', claim_owner = ?, claim_until_ms = ?, \
                             attempts = attempts + 1, updated_at_ms = ? \
                         WHERE delivery_id = ? AND ((phase = 'pending' AND next_attempt_at_ms <= ?) OR \
                               (phase = 'claimed' AND claim_until_ms <= ?))",
                        [
                            owner_id.as_str().into(),
                            claim_until_ms.into(),
                            now_ms.into(),
                            delivery_id.as_str().into(),
                            now_ms.into(),
                            now_ms.into(),
                        ],
                    ))
                    .await
                    .map_err(map_db_err)?;
                if result.rows_affected() == 0 {
                    return Ok(None);
                }
                load_background_delivery_tx(txn, &delivery_id).await
            })
        })
        .await
    }

    async fn consume_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        now_ms: i64,
    ) -> Result<BackgroundDelivery, StoreError> {
        let owner_id = owner_id.to_owned();
        let db = self.db();
        let delivery_id = delivery_id.to_owned();
        run_write(db, move |txn| {
            Box::pin(async move {
                if let Some(existing) = load_background_delivery_tx(txn, &delivery_id).await?
                    && matches!(
                        existing.phase,
                        BackgroundDeliveryPhase::Consumed | BackgroundDeliveryPhase::Failed
                    )
                {
                    return Ok(existing);
                }
                let result = txn
                    .execute(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "UPDATE agena_background_deliveries \
                         SET phase = 'consumed', claim_owner = NULL, claim_until_ms = NULL, \
                             updated_at_ms = ?, consumed_at_ms = ?, next_attempt_at_ms = ? \
                         WHERE delivery_id = ? AND phase = 'claimed' AND claim_owner = ?",
                        [
                            now_ms.into(),
                            now_ms.into(),
                            now_ms.into(),
                            delivery_id.as_str().into(),
                            owner_id.as_str().into(),
                        ],
                    ))
                    .await
                    .map_err(map_db_err)?;
                if result.rows_affected() != 1 {
                    return Err(StoreError::InvalidState(format!(
                        "background delivery {delivery_id} is not claimed by {owner_id}"
                    )));
                }
                load_background_delivery_tx(txn, &delivery_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::not_found(format!("background delivery {delivery_id}"))
                    })
            })
        })
        .await
    }

    async fn retry_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        error: serde_json::Value,
        next_attempt_at_ms: i64,
        now_ms: i64,
    ) -> Result<BackgroundDelivery, StoreError> {
        let owner_id = owner_id.to_owned();
        let db = self.db();
        let delivery_id = delivery_id.to_owned();
        let error = prepare_marker(error).await?.1;
        run_write(db, move |txn| {
            Box::pin(async move {
                let result = txn
                    .execute(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "UPDATE agena_background_deliveries \
                         SET phase = 'pending', claim_owner = NULL, claim_until_ms = NULL, \
                             last_error_json = ?, updated_at_ms = ?, next_attempt_at_ms = ? \
                         WHERE delivery_id = ? AND phase = 'claimed' AND claim_owner = ?",
                        [
                            error.into(),
                            now_ms.into(),
                            next_attempt_at_ms.into(),
                            delivery_id.as_str().into(),
                            owner_id.as_str().into(),
                        ],
                    ))
                    .await
                    .map_err(map_db_err)?;
                if result.rows_affected() != 1 {
                    return Err(StoreError::InvalidState(format!(
                        "background delivery {delivery_id} is not claimed by {owner_id}"
                    )));
                }
                load_background_delivery_tx(txn, &delivery_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::not_found(format!("background delivery {delivery_id}"))
                    })
            })
        })
        .await
    }

    async fn fail_background_delivery(
        &self,
        delivery_id: &str,
        owner_id: &str,
        error: serde_json::Value,
        now_ms: i64,
    ) -> Result<BackgroundDelivery, StoreError> {
        let owner_id = owner_id.to_owned();
        let db = self.db();
        let delivery_id = delivery_id.to_owned();
        let error = prepare_marker(error).await?.1;
        run_write(db, move |txn| {
            Box::pin(async move {
                if let Some(existing) = load_background_delivery_tx(txn, &delivery_id).await?
                    && matches!(
                        existing.phase,
                        BackgroundDeliveryPhase::Consumed | BackgroundDeliveryPhase::Failed
                    )
                {
                    return Ok(existing);
                }
                let result = txn
                    .execute(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "UPDATE agena_background_deliveries \
                         SET phase = 'failed', claim_owner = NULL, claim_until_ms = NULL, \
                             last_error_json = ?, updated_at_ms = ?, next_attempt_at_ms = ? \
                         WHERE delivery_id = ? AND phase = 'claimed' AND claim_owner = ?",
                        [
                            error.into(),
                            now_ms.into(),
                            now_ms.into(),
                            delivery_id.as_str().into(),
                            owner_id.as_str().into(),
                        ],
                    ))
                    .await
                    .map_err(map_db_err)?;
                if result.rows_affected() != 1 {
                    return Err(StoreError::InvalidState(format!(
                        "background delivery {delivery_id} is not claimed by {owner_id}"
                    )));
                }
                load_background_delivery_tx(txn, &delivery_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::not_found(format!("background delivery {delivery_id}"))
                    })
            })
        })
        .await
    }

    async fn fail_pending_background_deliveries(
        &self,
        session_id: i64,
        error: serde_json::Value,
        now_ms: i64,
    ) -> Result<usize, StoreError> {
        let error = prepare_marker(error).await?.1;
        let _permit = acquire_write_permit(self.db()).await.map_err(map_db_err)?;
        let result = self
            .db()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE agena_background_deliveries \
                 SET phase = 'failed', claim_owner = NULL, claim_until_ms = NULL, \
                     last_error_json = ?, updated_at_ms = ?, next_attempt_at_ms = ? \
                 WHERE session_id = ? AND phase IN ('pending', 'claimed')",
                [
                    error.into(),
                    now_ms.into(),
                    now_ms.into(),
                    session_id.into(),
                ],
            ))
            .await
            .map_err(map_db_err)?;
        Ok(result.rows_affected() as usize)
    }

    async fn pending_background_deliveries(
        &self,
        limit: usize,
        now_ms: i64,
    ) -> Result<Vec<BackgroundDelivery>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = self
            .db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT {BACKGROUND_DELIVERY_COLS} FROM agena_background_deliveries \
                     WHERE (phase = 'pending' AND next_attempt_at_ms <= ?) OR \
                           (phase = 'claimed' AND claim_until_ms <= ?) \
                     ORDER BY created_at_ms, delivery_id LIMIT ?"
                ),
                [now_ms.into(), now_ms.into(), limit.into()],
            ))
            .await
            .map_err(map_db_err)?;
        decode_rows(&PART_DECODERS, rows, background_delivery_from_row).await
    }

    async fn submit_user_run(
        &self,
        session_id: i64,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError> {
        let db = self.db();
        let marker_state = if parts.iter().all(|part| part.state.is_terminal()) {
            PartState::Completed
        } else {
            PartState::Pending
        };
        let parts = prepare_new_parts(parts).await?;
        let marker = prepare_marker(user_send_marker_content(None)).await?;
        run_write(db, move |txn| {
            Box::pin(async move {
                submit_batch_tx(
                    txn,
                    session_id,
                    PartRole::User,
                    marker_state,
                    marker,
                    parts,
                    idempotency_key,
                    now_ms,
                )
                .await
            })
        })
        .await
    }

    async fn submit_user_run_for_execution(
        &self,
        session_id: i64,
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        execution_id: &str,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError> {
        let db = self.db();
        let execution_id = execution_id.to_owned();
        let marker_state = if parts.iter().all(|part| part.state.is_terminal()) {
            PartState::Completed
        } else {
            PartState::Pending
        };
        let parts = prepare_new_parts(parts).await?;
        let marker = prepare_marker(user_send_marker_content(Some(&execution_id))).await?;
        run_write(db, move |txn| {
            Box::pin(async move {
                submit_batch_tx(
                    txn,
                    session_id,
                    PartRole::User,
                    marker_state,
                    marker,
                    parts,
                    idempotency_key,
                    now_ms,
                )
                .await
            })
        })
        .await
    }

    async fn settle_background_run(
        &self,
        session_id: i64,
        run_id: i64,
        tool_part: Option<(i64, PartState, serde_json::Value)>,
        new_parts: Vec<NewPart>,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError> {
        let db = self.db();
        let new_parts = prepare_new_parts(new_parts).await?;
        let tool_part = PART_MUTATION_CODECS
            .run(move || {
                tool_part
                    .map(|(id, state, content)| {
                        let encoded = serde_json::to_string(&content).map_err(|error| {
                            StoreError::Serialization(format!(
                                "encode background tool result: {error}"
                            ))
                        })?;
                        Ok::<_, StoreError>((id, state, encoded))
                    })
                    .transpose()
            })
            .await
            .map_err(|error| {
                StoreError::Database(format!("background encode worker failed: {error}"))
            })??;
        run_write(db, move |txn| {
            Box::pin(async move {
                // The launching run marker must exist; it may already be
                // terminal (e.g. aborted before the operation settled), in
                // which case the result parts are still appended onto it.
                let run = load_part_control(txn, run_id)
                    .await?
                    .ok_or_else(|| StoreError::not_found(format!("run marker {run_id}")))?;
                if !run.is_run_marker() {
                    return Err(StoreError::InvalidState(format!(
                        "part {run_id} is not a run marker"
                    )));
                }
                if run.origin_session_id != session_id {
                    return Err(StoreError::InvalidState(format!(
                        "run marker {run_id} is shared; only its origin session may settle it"
                    )));
                }

                // Transition the launching tool part (the operation's own
                // part) when supplied. Background launch uses an InProgress
                // transition to commit the durable correlation marker in the
                // same transaction as its guard result; completion uses a
                // terminal transition.
                let tool_part_id = tool_part.as_ref().map(|(id, _, _)| *id);
                if let Some((part_id, next_state, content)) = tool_part {
                    let mut part = load_part_control(txn, part_id)
                        .await?
                        .ok_or_else(|| StoreError::not_found(format!("part {part_id}")))?;
                    if part.origin_session_id != session_id {
                        return Err(StoreError::InvalidState(format!(
                            "part {part_id} is shared; only its origin session may settle it"
                        )));
                    }
                    part.state = next_state;
                    part.finished_at_ms = next_state.is_terminal().then_some(now_ms);
                    txn.execute(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "UPDATE agena_parts \
                         SET state = ?, content = ?, finished_at_ms = ?, \
                             revision = revision + 1, updated_at_ms = ? \
                         WHERE part_id = ?",
                        [
                            part.state.as_str().into(),
                            text_value(Some(content)),
                            Value::BigInt(part.finished_at_ms),
                            now_ms.into(),
                            part_id.into(),
                        ],
                    ))
                    .await
                    .map_err(map_db_err)?;
                }

                // Append companion parts under the launching run — the launch
                // guard or settled notifications, with their supplied roles —
                // and create no new run marker.
                let mut created = Vec::with_capacity(new_parts.len());
                let first_id = reserve_part_ids_tx(txn, new_parts.len()).await.map_err(map_db_err)?;
                for (offset, new_part) in new_parts.into_iter().enumerate() {
                    let id = first_id + offset as i64;
                    let part = content_part(id, session_id, run_id, new_part.part, now_ms);
                    insert_part_tx(txn, &part, new_part.content, None).await.map_err(map_db_err)?;
                    insert_membership_tx(txn, session_id, id, now_ms)
                        .await
                        .map_err(map_db_err)?;
                    created.push(part);
                }

                // Terminalize the launching run marker (Completed) once no
                // in-flight child remains, so the session returns to Ready
                // instead of lingering in Running/Interrupted.
                let mut message_ended = None;
                let mut run_ended = false;
                if run.state.is_in_flight() {
                    let remaining_row = txn
                        .query_one(Statement::from_sql_and_values(
                            DatabaseBackend::Sqlite,
                            "SELECT part_id FROM agena_parts \
                             WHERE origin_session_id = ? AND run_id = ? AND part_id != ? \
                               AND state IN ('pending', 'in_progress') \
                             LIMIT 1",
                            [session_id.into(), run_id.into(), run_id.into()],
                        ))
                        .await
                        .map_err(map_db_err)?;
                    let remaining: Option<i64> = match remaining_row {
                        Some(row) => Some(row.try_get("", "part_id").map_err(map_db_err)?),
                        None => None,
                    };
                    if remaining.is_none() {
                        let mut marker = run;
                        marker.state = PartState::Completed;
                        run_ended = true;
                        message_ended =
                            marker.is_terminal_message().then_some((session_id, now_ms));
                        marker.finished_at_ms = Some(now_ms);
                        txn.execute(Statement::from_sql_and_values(
                            DatabaseBackend::Sqlite,
                            "UPDATE agena_parts \
                             SET state = ?, content = json_set(content, '$.abort_reason', NULL), finished_at_ms = ?, \
                                 revision = revision + 1, updated_at_ms = ? \
                             WHERE part_id = ?",
                            [
                                marker.state.as_str().into(),
                                Value::BigInt(marker.finished_at_ms),
                                now_ms.into(),
                                run_id.into(),
                            ],
                        ))
                        .await
                        .map_err(map_db_err)?;
                    }
                }

                let changed_ids = tool_part_id
                    .into_iter()
                    .chain(created.iter().map(|part| part.part_id))
                    .chain(run_ended.then_some(run_id))
                    .collect::<Vec<_>>();
                bump_member_session_versions_for_parts_tx(txn, &changed_ids, message_ended).await?;
                Ok(created)
            })
        })
        .await
    }

    async fn append_parts(
        &self,
        session_id: i64,
        run_id: i64,
        parts: Vec<NewPart>,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError> {
        let db = self.db();
        let parts = prepare_new_parts(parts).await?;
        run_write(db, move |txn| {
            Box::pin(async move {
                let run = load_part_control(txn, run_id)
                    .await?
                    .ok_or_else(|| StoreError::not_found(format!("run marker {run_id}")))?;
                if !run.is_run_marker() || !run.state.is_in_flight() {
                    return Err(StoreError::InvalidState(format!(
                        "run {run_id} is not an in-flight run marker"
                    )));
                }
                let mut created = Vec::with_capacity(parts.len());
                let first_id = reserve_part_ids_tx(txn, parts.len())
                    .await
                    .map_err(map_db_err)?;
                for (offset, new_part) in parts.into_iter().enumerate() {
                    let id = first_id + offset as i64;
                    let part = content_part(id, session_id, run_id, new_part.part, now_ms);
                    insert_part_tx(txn, &part, new_part.content, None)
                        .await
                        .map_err(map_db_err)?;
                    insert_membership_tx(txn, session_id, id, now_ms)
                        .await
                        .map_err(map_db_err)?;
                    created.push(part);
                }
                if !created.is_empty() {
                    bump_session_version_tx(txn, session_id, None).await?;
                }
                Ok(created)
            })
        })
        .await
    }

    async fn update_part(
        &self,
        session_id: i64,
        part_id: i64,
        delta: PartDelta,
        now_ms: i64,
    ) -> Result<Part, StoreError> {
        mutate_part(
            self.db(),
            session_id,
            part_id,
            PartMutation::Delta(Arc::new(delta)),
            now_ms,
        )
        .await
    }

    async fn complete_run(
        &self,
        session_id: i64,
        run_id: i64,
        outcome: RunOutcome,
        now_ms: i64,
    ) -> Result<Part, StoreError> {
        mutate_part(
            self.db(),
            session_id,
            run_id,
            PartMutation::Complete(Arc::new(outcome)),
            now_ms,
        )
        .await
    }

    async fn start_run(
        &self,
        session_id: i64,
        run_kind: &str,
        content: serde_json::Value,
        idempotency_key: Option<String>,
        now_ms: i64,
    ) -> Result<SubmitOutcome, StoreError> {
        let db = self.db();
        let run_kind = run_kind.to_owned();
        let role = match run_kind.as_str() {
            "user_send" => PartRole::User,
            "continue" | "compaction" | "steer" | "execution" => PartRole::Assistant,
            _ => PartRole::Runtime,
        };
        let mut marker_content = content;
        if let serde_json::Value::Object(map) = &mut marker_content {
            map.insert(
                "run_kind".to_owned(),
                serde_json::Value::String(run_kind.clone()),
            );
        }
        let marker = prepare_marker(marker_content).await?;
        run_write(db, move |txn| {
            Box::pin(async move {
                submit_batch_tx(
                    txn,
                    session_id,
                    role,
                    PartState::Pending,
                    marker,
                    Vec::new(),
                    idempotency_key,
                    now_ms,
                )
                .await
            })
        })
        .await
    }

    async fn cancel_run(
        &self,
        session_id: i64,
        run_id: i64,
        now_ms: i64,
    ) -> Result<Vec<Part>, StoreError> {
        let rows = run_write(self.db(), move |txn| {
            Box::pin(async move {
                abort_runs_tx(txn, session_id, &[run_id], "user_cancelled", now_ms).await
            })
        })
        .await?;
        Ok(rows.finish().await?.updated_parts)
    }

    async fn withdraw_user_run(
        &self,
        session_id: i64,
        run_id: i64,
        _now_ms: i64,
    ) -> Result<Vec<Part>, StoreError> {
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {

                let Some(marker) = load_part_by_id_tx(txn, run_id).await? else {
                    return Ok(Vec::new());
                };
                if !marker.is_run_marker()
                    || marker.role != PartRole::User
                    || marker.origin_session_id != session_id
                    || marker
                        .content
                        .get("run_kind")
                        .and_then(serde_json::Value::as_str)
                        != Some("user_send")
                {
                    return Err(StoreError::InvalidState(format!(
                        "part {run_id} is not a user_send run owned by session {session_id}"
                    )));
                }

                let rows = txn
                    .query_all(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        format!(
                            "SELECT {PART_COLS} FROM agena_parts p \
                             JOIN agena_session_parts sp ON sp.part_id = p.part_id \
                             WHERE sp.session_id = ? AND (p.part_id = ? OR p.run_id = ?) \
                             ORDER BY p.created_at_ms, p.part_id"
                        ),
                        [session_id.into(), run_id.into(), run_id.into()],
                    ))
                    .await
                    .map_err(map_db_err)?;
                if rows.is_empty() {
                    return Ok(Vec::new());
                }
                let removed = decode_part_rows_in(&TRANSACTION_CODECS, rows).await?;
                let part_ids = removed
                    .iter()
                    .map(|part| part.part_id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    format!(
                        "DELETE FROM agena_session_parts WHERE session_id = ? AND part_id IN ({part_ids})"
                    ),
                    [session_id.into()],
                ))
                .await
                .map_err(map_db_err)?;
                txn.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "DELETE FROM agena_idempotency WHERE session_id = ? AND run_id = ?",
                    [session_id.into(), run_id.into()],
                ))
                .await
                .map_err(map_db_err)?;
                bump_session_version_tx(txn, session_id, None).await?;
                Ok(removed)
            })
        })
        .await
    }

    async fn fork_session_with_config(
        &self,
        session_id: i64,
        at_part_id: i64,
        title: String,
        rewind: bool,
        config_json: Option<serde_json::Value>,
        now_ms: i64,
    ) -> Result<SessionMeta, StoreError> {
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {
                let parent = session_meta_tx(txn, session_id).await?;
                if parent.lifecycle_state != SessionLifecycleState::Ready {
                    return Err(StoreError::InvalidState(format!(
                        "parent session {session_id} is not ready"
                    )));
                }
                let cutoff = load_part_control(txn, at_part_id)
                    .await?
                    .ok_or_else(|| StoreError::not_found(format!("cutoff part {at_part_id}")))?;
                let member = txn
                    .query_one(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "SELECT part_id FROM agena_session_parts WHERE session_id = ? AND part_id = ?",
                        [session_id.into(), at_part_id.into()],
                    ))
                    .await
                    .map_err(map_db_err)?;
                if member.is_none() {
                    return Err(StoreError::InvalidState(format!(
                        "cutoff part {at_part_id} is not a member of session {session_id}"
                    )));
                }
                let relation_kind = if rewind {
                    SessionRelationKind::Rewind
                } else {
                    SessionRelationKind::Fork
                };
                let child = NewSession {
                    workspace_id: parent.workspace_id,
                    parent_id: Some(session_id),
                    relation_kind,
                    cutoff_part_id: Some(at_part_id),
                    title,
                    task_id: None,
                    config_json: config_json.or(parent.config_json.clone()),
                    provider_anchors_json: None,
                };
                let child_id = create_session_tx(txn, child).await?;
                // Eager edge copy up to the cutoff (7.3).
                let (cutoff_created, cutoff_id) = (cutoff.created_at_ms, cutoff.part_id);
                let comparison = if rewind { "<" } else { "<=" };
                txn.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
                    format!("INSERT INTO agena_session_parts (session_id, part_id, added_at_ms) \
                             SELECT ?, p.part_id, ? FROM agena_parts p JOIN agena_session_parts sp ON sp.part_id = p.part_id \
                             WHERE sp.session_id = ? AND (p.created_at_ms < ? OR (p.created_at_ms = ? AND p.part_id {comparison} ?))"),
                    [child_id.into(), now_ms.into(), session_id.into(), cutoff_created.into(), cutoff_created.into(), cutoff_id.into()],
                )).await.map_err(map_db_err)?;
                session_meta_tx(txn, child_id).await
            })
        })
        .await
    }

    async fn in_flight_session_ids(&self) -> Result<Vec<i64>, StoreError> {
        let rows = self
            .db()
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT DISTINCT origin_session_id FROM agena_parts \
                 WHERE kind = 'run' AND state IN ('pending', 'in_progress') \
                 ORDER BY origin_session_id",
            ))
            .await
            .map_err(map_db_err)?;
        rows.into_iter()
            .map(|row| row.try_get("", "origin_session_id").map_err(map_db_err))
            .collect()
    }

    async fn in_flight_run_ids(&self, session_id: i64) -> Result<Vec<i64>, StoreError> {
        Ok(in_flight_runs_tx(self.db(), session_id)
            .await?
            .into_iter()
            .map(|run| run.part_id)
            .collect())
    }

    async fn reconcile(
        &self,
        session_id: i64,
        run_ids: &[i64],
        now_ms: i64,
    ) -> Result<ReconcileOutcome, StoreError> {
        if run_ids.is_empty() {
            return Ok(ReconcileOutcome::default());
        }
        let run_ids = run_ids.to_vec();
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {
                abort_runs_tx(txn, session_id, &run_ids, "process_restart", now_ms).await
            })
        })
        .await?
        .finish()
        .await
    }

    async fn maintenance(&self, _now_ms: i64) -> Result<MaintenanceOutcome, StoreError> {
        let mut gc_deleted_parts = 0;
        // Separate commits let interactive writes enter between batches. Empty
        // maintenance ticks never acquire a write connection or a write lock.
        for _ in 0..4 {
            let candidates = self
                .db()
                .query_all(Statement::from_string(
                    DatabaseBackend::Sqlite,
                    format!(
                        "SELECT p.part_id FROM agena_parts p WHERE {} ORDER BY p.part_id LIMIT 256",
                        gc_leaf_predicate()
                    ),
                ))
                .await
                .map_err(map_db_err)?;
            let ids = candidates
                .into_iter()
                .map(|row| row.try_get("", "part_id"))
                .collect::<Result<Vec<i64>, _>>()
                .map_err(map_db_err)?;
            if ids.is_empty() {
                break;
            }
            let deleted = run_write(self.db(), move |txn| Box::pin(async move {
                txn.execute(Statement::from_string(DatabaseBackend::Sqlite,
                    format!("DELETE FROM agena_parts WHERE part_id IN (SELECT p.part_id FROM agena_parts p WHERE p.part_id IN ({}) AND {})",
                        comma_list(&ids), gc_leaf_predicate()),
                )).await.map(|result| result.rows_affected() as usize).map_err(map_db_err)
            })).await?;
            gc_deleted_parts += deleted;
            if deleted == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        Ok(MaintenanceOutcome { gc_deleted_parts })
    }

    async fn record_usage(&self, record: UsageRecord) -> Result<(), StoreError> {
        let mut record = record;
        let (_, detail_json) = encode_optional_json(record.detail_json.take()).await?;
        let _permit = acquire_write_permit(self.db()).await.map_err(map_db_err)?;
        self.db()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO agena_usage \
                 (workspace_id, session_id, run_id, provider_id, model_id, created_at_ms, \
                  input_tokens, output_tokens, reasoning_tokens, cache_write_tokens, \
                  cache_read_tokens, tool_use_tokens, other_tokens, total_cost_micros, \
                  recorded_cost_micros, cost_estimate_incomplete, detail_json) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                [
                    record.workspace_id.into(),
                    record.session_id.into(),
                    Value::BigInt(record.run_id),
                    record.provider_id.into(),
                    record.model_id.into(),
                    record.created_at_ms.into(),
                    record.input_tokens.into(),
                    record.output_tokens.into(),
                    record.reasoning_tokens.into(),
                    record.cache_write_tokens.into(),
                    record.cache_read_tokens.into(),
                    record.tool_use_tokens.into(),
                    record.other_tokens.into(),
                    record.total_cost_micros.into(),
                    Value::BigInt(record.recorded_cost_micros),
                    (record.cost_estimate_incomplete as i64).into(),
                    text_value(detail_json),
                ],
            ))
            .await
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn usage_stats(&self, query: UsageQuery) -> Result<UsageStats, StoreError> {
        let mut where_clauses = Vec::new();
        let mut values: Vec<Value> = Vec::new();
        if let Some(session_id) = query.session_id {
            where_clauses.push("session_id = ?".to_owned());
            values.push(session_id.into());
        }
        if let Some(workspace_id) = query.workspace_id {
            where_clauses.push("workspace_id = ?".to_owned());
            values.push(workspace_id.into());
        }
        if let Some(provider_id) = query.provider_id.as_deref() {
            where_clauses.push("provider_id = ?".to_owned());
            values.push(provider_id.into());
        }
        if let Some(model_id) = query.model_id.as_deref() {
            where_clauses.push("model_id = ?".to_owned());
            values.push(model_id.into());
        }
        if let Some(after_ms) = query.after_ms {
            where_clauses.push("created_at_ms >= ?".to_owned());
            values.push(after_ms.into());
        }
        if let Some(before_ms) = query.before_ms {
            where_clauses.push("created_at_ms < ?".to_owned());
            values.push(before_ms.into());
        }
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };
        let rows = self
            .db()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT provider_id, model_id, COUNT(*) AS calls, \
                            SUM(input_tokens) AS input_tokens, SUM(output_tokens) AS output_tokens, \
                            SUM(reasoning_tokens) AS reasoning_tokens, \
                            SUM(cache_write_tokens) AS cache_write_tokens, \
                            SUM(cache_read_tokens) AS cache_read_tokens, \
                            SUM(total_cost_micros) AS total_cost_micros \
                     FROM agena_usage {where_sql} \
                     GROUP BY provider_id, model_id \
                     ORDER BY provider_id, model_id"
                ),
                values,
            ))
            .await
            .map_err(map_db_err)?;
        let mut stats = UsageStats::default();
        for row in rows {
            let group = UsageGroup {
                provider_id: row.try_get("", "provider_id").map_err(map_db_err)?,
                model_id: row.try_get("", "model_id").map_err(map_db_err)?,
                calls: row.try_get("", "calls").map_err(map_db_err)?,
                input_tokens: row.try_get("", "input_tokens").map_err(map_db_err)?,
                output_tokens: row.try_get("", "output_tokens").map_err(map_db_err)?,
                reasoning_tokens: row.try_get("", "reasoning_tokens").map_err(map_db_err)?,
                cache_write_tokens: row.try_get("", "cache_write_tokens").map_err(map_db_err)?,
                cache_read_tokens: row.try_get("", "cache_read_tokens").map_err(map_db_err)?,
                total_cost_micros: row.try_get("", "total_cost_micros").map_err(map_db_err)?,
            };
            stats.total_calls += group.calls;
            stats.total_input_tokens += group.input_tokens;
            stats.total_output_tokens += group.output_tokens;
            stats.total_cost_micros += group.total_cost_micros;
            stats.groups.push(group);
        }
        Ok(stats)
    }

    async fn export_session_jsonl(&self, session_id: i64) -> Result<String, StoreError> {
        let view = self.load_session(session_id).await?;
        PART_DECODERS
            .run(move || agena_storage::store::serialize(&view))
            .await
            .map_err(|error| StoreError::Database(format!("export worker failed: {error}")))?
    }

    async fn import_session_jsonl(
        &self,
        workspace_id: i64,
        bundle: &str,
        now_ms: i64,
    ) -> Result<i64, StoreError> {
        let bundle = bundle.to_owned();
        let (parsed, parts) = PART_MUTATION_CODECS
            .run(move || {
                let mut parsed = agena_storage::store::parse(&bundle)?;
                let parts = std::mem::take(&mut parsed.parts)
                    .into_iter()
                    .map(|part| {
                        let (content, provider_state) = encode_part_payload(&part)?;
                        Ok::<_, StoreError>((part, content, provider_state))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<_, StoreError>((parsed, parts))
            })
            .await
            .map_err(|error| StoreError::Database(format!("import worker failed: {error}")))??;
        let db = self.db();
        run_write(db, move |txn| {
            Box::pin(async move {
                let session_id = create_root_session_tx(
                    txn,
                    workspace_id,
                    parsed.title.clone(),
                    parsed.task_id.clone(),
                    parsed.config_json.clone(),
                    parsed.provider_anchors_json.clone(),
                    now_ms,
                )
                .await?;
                // Remap part ids so run_id/parent_part_id references stay
                // valid even if the exported ids collide with existing parts.
                let mut id_map: std::collections::HashMap<i64, i64> = Default::default();
                let first_id = reserve_part_ids_tx(txn, parts.len())
                    .await
                    .map_err(map_db_err)?;
                for (offset, (part, _, _)) in parts.iter().enumerate() {
                    id_map.insert(part.part_id, first_id + offset as i64);
                }
                let had_parts = !parts.is_empty();
                for (mut remapped, content, provider_state) in parts {
                    let new_id = id_map[&remapped.part_id];
                    remapped.part_id = new_id;
                    remapped.run_id = remapped.run_id.map(|run| id_map[&run]);
                    remapped.parent_part_id = remapped.parent_part_id.map(|parent| id_map[&parent]);
                    remapped.origin_session_id = session_id;
                    insert_part_tx(txn, &remapped, content, provider_state)
                        .await
                        .map_err(map_db_err)?;
                    insert_membership_tx(txn, session_id, new_id, now_ms)
                        .await
                        .map_err(map_db_err)?;
                }
                if had_parts {
                    bump_session_version_tx(txn, session_id, None).await?;
                }
                Ok(session_id)
            })
        })
        .await
    }
}

// --- shared transactional helpers ---

/// Validate a new session, insert it, and return its id. The root-finalize
/// trigger fills in `root_id` for roots after insert.
async fn create_session_tx(
    txn: &DatabaseTransaction,
    new_session: NewSession,
) -> Result<i64, StoreError> {
    let NewSession {
        workspace_id,
        parent_id,
        relation_kind,
        cutoff_part_id,
        title,
        task_id,
        config_json,
        provider_anchors_json,
    } = new_session;
    if parent_id.is_none() && relation_kind != SessionRelationKind::Root {
        return Err(StoreError::InvalidState(
            "root session must have relation_kind = root".to_owned(),
        ));
    }
    if parent_id.is_some() && relation_kind == SessionRelationKind::Root {
        return Err(StoreError::InvalidState(
            "child session cannot have relation_kind = root".to_owned(),
        ));
    }
    let is_branch = matches!(
        relation_kind,
        SessionRelationKind::Fork | SessionRelationKind::Rewind
    );
    if is_branch != cutoff_part_id.is_some() {
        return Err(StoreError::InvalidState(
            "fork/rewind sessions require a cutoff_part_id".to_owned(),
        ));
    }
    if cutoff_part_id.is_some()
        && load_part_control(txn, cutoff_part_id.unwrap())
            .await?
            .is_none()
    {
        return Err(StoreError::not_found("cutoff part"));
    }
    let (depth, root_id) = match parent_id {
        None => (0i64, 0i64),
        Some(parent_id) => {
            let parent = session_meta_tx(txn, parent_id).await?;
            if parent.lifecycle_state != SessionLifecycleState::Ready {
                return Err(StoreError::InvalidState(format!(
                    "parent session {parent_id} is not ready"
                )));
            }
            (parent.depth + 1, parent.root_id)
        }
    };
    let now = wall_clock_ms();
    let config_json = config_json
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| StoreError::Serialization(format!("encode config: {error}")))?;
    let provider_anchors_json = provider_anchors_json
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| StoreError::Serialization(format!("encode anchors: {error}")))?;
    let result = txn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agena_sessions \
             (parent_id, depth, root_id, workspace_id, relation_kind, cutoff_part_id, title, \
              version, lifecycle_state, task_id, config_json, provider_anchors_json, \
              created_at_ms, updated_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 1, 'ready', ?, ?, ?, ?, ?)",
            [
                Value::BigInt(parent_id),
                depth.into(),
                root_id.into(),
                workspace_id.into(),
                relation_kind.as_str().into(),
                Value::BigInt(cutoff_part_id),
                title.into(),
                text_value(task_id),
                text_value(config_json),
                text_value(provider_anchors_json),
                now.into(),
                now.into(),
            ],
        ))
        .await
        .map_err(map_db_err)?;
    let id = i64::try_from(result.last_insert_id())
        .map_err(|_| StoreError::Database("session identifier exceeds i64 range".to_owned()))?;
    Ok(id)
}

/// Create a root session for an import (no parent, no validation beyond root).
async fn create_root_session_tx(
    txn: &DatabaseTransaction,
    workspace_id: i64,
    title: String,
    task_id: Option<String>,
    config_json: Option<serde_json::Value>,
    provider_anchors_json: Option<serde_json::Value>,
    now_ms: i64,
) -> Result<i64, StoreError> {
    let config_json = config_json
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| StoreError::Serialization(format!("encode config: {error}")))?;
    let provider_anchors_json = provider_anchors_json
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| StoreError::Serialization(format!("encode anchors: {error}")))?;
    let result = txn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agena_sessions \
             (parent_id, depth, root_id, workspace_id, relation_kind, cutoff_part_id, title, \
              version, lifecycle_state, task_id, config_json, provider_anchors_json, \
              created_at_ms, updated_at_ms) \
             VALUES (NULL, 0, 0, ?, 'root', NULL, ?, 1, 'ready', ?, ?, ?, ?, ?)",
            [
                workspace_id.into(),
                title.into(),
                text_value(task_id),
                text_value(config_json),
                text_value(provider_anchors_json),
                now_ms.into(),
                now_ms.into(),
            ],
        ))
        .await
        .map_err(map_db_err)?;
    i64::try_from(result.last_insert_id())
        .map_err(|_| StoreError::Database("session identifier exceeds i64 range".to_owned()))
}

async fn session_meta_tx(
    txn: &DatabaseTransaction,
    session_id: i64,
) -> Result<SessionMeta, StoreError> {
    session_meta_in(txn, session_id, &TRANSACTION_CODECS).await
}

async fn session_meta_in<C: ConnectionTrait>(
    connection: &C,
    session_id: i64,
    pool: &'static agena_async::BlockingPool,
) -> Result<SessionMeta, StoreError> {
    let row = connection
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("SELECT {SESSION_COLS} FROM agena_sessions s WHERE s.id = ?"),
            [session_id.into()],
        ))
        .await
        .map_err(map_db_err)?;
    decode_rows(pool, row.into_iter().collect(), meta_from_row)
        .await?
        .pop()
        .ok_or_else(|| StoreError::not_found(format!("session {session_id}")))
}

/// The shared batch creator for user send and start_run: marker + content
/// parts + membership + optional idempotency row, one transaction.
async fn submit_batch_tx(
    txn: &DatabaseTransaction,
    session_id: i64,
    marker_role: PartRole,
    marker_state: PartState,
    marker_content: (serde_json::Value, String),
    content_parts: Vec<PreparedNewPart>,
    idempotency_key: Option<String>,
    now_ms: i64,
) -> Result<SubmitOutcome, StoreError> {
    // Idempotency: a replay of the same key returns the prior run.
    if let Some(key) = idempotency_key.as_deref() {
        let existing_row = txn
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT run_id FROM agena_idempotency \
                 WHERE session_id = ? AND idempotency_key = ?",
                [session_id.into(), key.into()],
            ))
            .await
            .map_err(map_db_err)?;
        let existing: Option<i64> = match existing_row {
            Some(row) => Some(row.try_get("", "run_id").map_err(map_db_err)?),
            None => None,
        };
        if let Some(run_id) = existing {
            let parts = run_parts_tx(txn, run_id).await?;
            return Ok(SubmitOutcome {
                run_id,
                created: false,
                parts,
            });
        }
    }

    let (marker_content, marker_json) = marker_content;
    validate_run_content(marker_state, &marker_content)?;
    let marker_id = reserve_part_ids_tx(txn, content_parts.len() + 1)
        .await
        .map_err(map_db_err)?;
    let marker = marker_part(
        marker_id,
        session_id,
        marker_role,
        marker_state,
        marker_content,
        now_ms,
    );
    insert_part_tx(txn, &marker, marker_json, None)
        .await
        .map_err(map_db_err)?;
    insert_membership_tx(txn, session_id, marker_id, now_ms)
        .await
        .map_err(map_db_err)?;

    let message_ended = marker.is_terminal_message().then_some(now_ms);
    let mut created = vec![marker];
    for (offset, new_part) in content_parts.into_iter().enumerate() {
        let id = marker_id + 1 + offset as i64;
        let part = content_part(id, session_id, marker_id, new_part.part, now_ms);
        insert_part_tx(txn, &part, new_part.content, None)
            .await
            .map_err(map_db_err)?;
        insert_membership_tx(txn, session_id, id, now_ms)
            .await
            .map_err(map_db_err)?;
        created.push(part);
    }

    if let Some(key) = idempotency_key {
        txn.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agena_idempotency (session_id, idempotency_key, run_id, created_at_ms) \
             VALUES (?, ?, ?, ?)",
            [
                session_id.into(),
                key.into(),
                marker_id.into(),
                now_ms.into(),
            ],
        ))
        .await
        .map_err(map_db_err)?;
    }
    bump_session_version_tx(txn, session_id, message_ended).await?;
    Ok(SubmitOutcome {
        run_id: marker_id,
        created: true,
        parts: created,
    })
}

/// Parts of a run, ordered by `(created_at_ms, part_id)`.
async fn run_parts_tx(txn: &DatabaseTransaction, run_id: i64) -> Result<Vec<Part>, StoreError> {
    let rows = txn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!(
                "SELECT {PART_COLS} FROM agena_parts p WHERE p.run_id = ? \
             ORDER BY p.created_at_ms, p.part_id"
            ),
            [run_id.into()],
        ))
        .await
        .map_err(map_db_err)?;
    decode_part_rows_in(&TRANSACTION_CODECS, rows).await
}

/// Delete only unreferenced leaves. Parent/run foreign keys and durable
/// background references must remain valid, including across batch boundaries.
fn gc_leaf_predicate() -> &'static str {
    "NOT EXISTS (SELECT 1 FROM agena_session_parts sp WHERE sp.part_id = p.part_id) \
     AND NOT (p.kind = 'run' AND p.state IN ('pending', 'in_progress')) \
     AND (p.run_id IS NULL OR NOT EXISTS (SELECT 1 FROM agena_parts run \
         WHERE run.part_id = p.run_id AND run.state IN ('pending', 'in_progress'))) \
     AND NOT EXISTS (SELECT 1 FROM agena_parts child WHERE child.parent_part_id = p.part_id) \
     AND NOT EXISTS (SELECT 1 FROM agena_parts child WHERE child.run_id = p.part_id) \
     AND p.part_id NOT IN (SELECT launch_run_id FROM agena_background_operations WHERE launch_run_id IS NOT NULL) \
     AND p.part_id NOT IN (SELECT launch_tool_part_id FROM agena_background_operations WHERE launch_tool_part_id IS NOT NULL) \
     AND p.part_id NOT IN (SELECT notification_part_id FROM agena_background_deliveries WHERE notification_part_id IS NOT NULL)"
}

/// Current wall-clock time in milliseconds (the engine's default clock).
fn wall_clock_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
