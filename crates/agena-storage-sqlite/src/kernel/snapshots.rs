//! Export roots pin membership generations, not a live SQLite transaction or
//! an in-memory list of every entry/segment. Only a replaced boundary edge is
//! archived for roots that actually need that older edge.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use port::{HistorySnapshotGc, HistorySnapshotLease, HistorySnapshotOpened};

use super::*;

const MAX_ROOTS: i64 = 8;
const MAX_SESSION_ROOTS: i64 = 2;
const GC_BATCH: i64 = 64;
const ORPHAN_GRACE: Duration = Duration::from_secs(30);

struct Family {
    lease: HistorySnapshotLease,
    retained: usize,
    claimed: bool,
    revoked: bool,
    created: Instant,
}

pub(super) struct Pins {
    roots: Mutex<HashMap<RequestId, Family>>,
    dirty: AtomicBool,
}
impl Default for Pins {
    fn default() -> Self {
        Self {
            roots: Mutex::default(),
            dirty: AtomicBool::new(true),
        }
    }
}
impl Pins {
    fn install(&self, lease: HistorySnapshotLease) {
        self.roots.lock().unwrap_or_else(|e| e.into_inner()).insert(
            lease.token,
            Family {
                lease,
                retained: 0,
                claimed: false,
                revoked: false,
                created: Instant::now(),
            },
        );
    }
    pub fn retain(&self, lease: HistorySnapshotLease) -> Result<()> {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        let root = roots
            .get_mut(&lease.token)
            .filter(|root| root.lease == lease && !root.revoked)
            .ok_or_else(|| KernelError::Conflict("history snapshot expired".into()))?;
        root.claimed = true;
        root.retained += 1;
        Ok(())
    }
    pub fn release(&self, lease: HistorySnapshotLease) {
        if let Some(root) = self
            .roots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&lease.token)
            .filter(|root| root.lease == lease)
        {
            root.retained = root.retained.saturating_sub(1);
            self.dirty.store(true, Ordering::Release);
        }
    }
    pub fn needs_collection(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
            || self
                .roots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .any(|root| {
                    root.retained == 0 && (root.claimed || root.created.elapsed() >= ORPHAN_GRACE)
                })
    }
    fn revoke(&self) -> Vec<HistorySnapshotLease> {
        self.roots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values_mut()
            .filter_map(|root| {
                if root.retained == 0 && (root.claimed || root.created.elapsed() >= ORPHAN_GRACE) {
                    root.revoked = true;
                    Some(root.lease)
                } else {
                    None
                }
            })
            .collect()
    }
    fn acknowledge(&self, released: &[HistorySnapshotLease]) {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        for root in released {
            roots.remove(&root.token);
        }
    }
}

async fn commit(transaction: sqlx::Transaction<'_, sqlx::Sqlite>, token: RequestId) -> Result<()> {
    transaction.commit().await.map_err(|error| {
        if rejected_commit(&error) {
            storage(error)
        } else {
            KernelError::CommitUnknown {
                request_id: token,
                diagnostic: error.to_string(),
            }
        }
    })
}

pub(super) async fn open(
    database: &KernelDatabase,
    session: SessionId,
    epoch: KernelEpoch,
    pins: &Pins,
    views: &ReadBudget,
    cache: &read_cache::ReadCache,
) -> Result<HistorySnapshotOpened> {
    let mut transaction = database
        .writer
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let result = async {
        let metadata = reads::metadata(&mut transaction, session).await?;
        let counts = sqlx::query("SELECT count(*) AS total,COALESCE(sum(session_id=?),0) AS owned FROM agena_history_snapshots")
            .bind(session.get()).fetch_one(&mut *transaction).await.map_err(sql_error)?;
        if counts.try_get::<i64,_>("total").map_err(storage)? >= MAX_ROOTS
            || counts.try_get::<i64,_>("owned").map_err(storage)? >= MAX_SESSION_ROOTS {
            return Err(KernelError::Busy);
        }
        let entry_count: i64 = sqlx::query_scalar("SELECT entry_count FROM agena_session_statistics WHERE session_id=?")
            .bind(session.get()).fetch_one(&mut *transaction).await.map_err(sql_error)?;
        let info = HistorySnapshotInfo { metadata, entry_count: u64::try_from(entry_count).map_err(storage)? };
        let bytes = modeled_read_bytes(&info)?.saturating_add(4096);
        let reserve = || views.reserve_view(session, bytes);
        let lease = reserve().or_else(|error| {
            if error != KernelError::Busy { return Err(error); }
            cache.evict(); reserve()
        })?;
        let info = ReadView::from_lease(info, lease);
        let lease = HistorySnapshotLease {
            token: RequestId::new(), session, epoch, generation: info.metadata.membership_generation,
        };
        sqlx::query("INSERT INTO agena_history_snapshots (token,session_id,generation,owner_epoch,last_sequence) SELECT ?,?,?,?,(SELECT sequence FROM agena_session_segments WHERE session_id=? ORDER BY sequence DESC LIMIT 1)")
            .bind(lease.token.to_string()).bind(session.get()).bind(lease.generation.get()).bind(epoch.to_string()).bind(session.get())
            .execute(&mut *transaction).await.map_err(sql_error)?;
        Ok(HistorySnapshotOpened { lease, info })
    }.await;
    let opened = match result {
        Ok(value) => value,
        Err(error) => {
            let error = abort_transaction(transaction).await.err().unwrap_or(error);
            return Err(error);
        }
    };
    commit(transaction, opened.lease.token).await?;
    pins.install(opened.lease);
    Ok(opened)
}

pub(super) async fn collect(database: &KernelDatabase, pins: &Pins) -> Result<HistorySnapshotGc> {
    pins.dirty.swap(false, Ordering::AcqRel);
    let released = pins.revoke();
    let mut transaction = match database.writer.get_sqlite_connection_pool().begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            pins.dirty.store(true, Ordering::Release);
            return Err(storage(error));
        }
    };
    let result = async {
        let mut result = HistorySnapshotGc::default();
        for lease in &released {
            result.roots += sqlx::query("DELETE FROM agena_history_snapshots WHERE token=? AND owner_epoch=?")
                .bind(lease.token.to_string()).bind(lease.epoch.to_string()).execute(&mut *transaction).await.map_err(sql_error)?.rows_affected() as usize;
        }
        let after: i64 = sqlx::query_scalar("SELECT after_id FROM agena_history_snapshot_gc_state WHERE id=1")
            .fetch_one(&mut *transaction).await.map_err(sql_error)?;
        let rows = sqlx::query("SELECT id,segment_id FROM agena_retired_session_segments WHERE id>? ORDER BY id LIMIT ?")
            .bind(after).bind(GC_BATCH).fetch_all(&mut *transaction).await.map_err(sql_error)?;
        let next = if rows.len() == GC_BATCH as usize {
            pins.dirty.store(true, Ordering::Release);
            rows.last().unwrap().try_get::<i64,_>("id").map_err(storage)?
        } else { 0 };
        for row in rows {
            let id: i64 = row.try_get("id").map_err(storage)?;
            let segment: i64 = row.try_get("segment_id").map_err(storage)?;
            let removed = sqlx::query("DELETE FROM agena_retired_session_segments WHERE id=? AND NOT EXISTS(SELECT 1 FROM agena_history_snapshots root WHERE root.session_id=agena_retired_session_segments.session_id AND root.generation>=agena_retired_session_segments.since_generation AND root.generation<agena_retired_session_segments.until_generation)")
                .bind(id).execute(&mut *transaction).await.map_err(sql_error)?.rows_affected();
            if removed > 0 {
                result.references += 1;
                result.segments += sqlx::query("DELETE FROM agena_history_segments WHERE id=? AND NOT EXISTS(SELECT 1 FROM agena_session_segments WHERE segment_id=?) AND NOT EXISTS(SELECT 1 FROM agena_retired_session_segments WHERE segment_id=?)")
                    .bind(segment).bind(segment).bind(segment).execute(&mut *transaction).await.map_err(sql_error)?.rows_affected() as usize;
            }
        }
        if after != next {
            sqlx::query("UPDATE agena_history_snapshot_gc_state SET after_id=? WHERE id=1")
                .bind(next).execute(&mut *transaction).await.map_err(sql_error)?;
        }
        Ok(result)
    }.await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            pins.dirty.store(true, Ordering::Release);
            let error = abort_transaction(transaction).await.err().unwrap_or(error);
            return Err(error);
        }
    };
    if let Err(error) = commit(transaction, RequestId::new()).await {
        pins.dirty.store(true, Ordering::Release);
        return Err(error);
    }
    pins.acknowledge(&released);
    Ok(result)
}

// Each arm selects a bounded indexed window before the union's small merge.
// A newer current edge is invisible to the old generation; its archived edge
// remains pinned until the last applicable snapshot family has been released.
pub(super) const SEGMENTS_SQL: &str = "SELECT * FROM (SELECT * FROM (SELECT sequence,segment_id,first_ordinal,end_ordinal FROM agena_session_segments WHERE session_id=? AND sequence>=? AND sequence<=? AND since_generation<=? ORDER BY sequence LIMIT ?) UNION ALL SELECT * FROM (SELECT sequence,segment_id,first_ordinal,end_ordinal FROM agena_retired_session_segments WHERE session_id=? AND sequence>=? AND sequence<=? AND since_generation<=? AND until_generation>? ORDER BY sequence LIMIT ?)) ORDER BY sequence LIMIT ?";
pub(super) const MEMBERS_SQL: &str = "SELECT m.entry_id,m.ordinal,e.body_bytes FROM agena_segment_entries m JOIN agena_entries e INDEXED BY idx_agena_entry_bytes ON e.id=m.entry_id WHERE m.segment_id=? AND m.ordinal>=? AND m.ordinal<? ORDER BY m.ordinal LIMIT ?";

/// Authorize a position through the pinned generation, including retired
/// membership. Export details never rely on the session's current edges.
pub(super) async fn member(
    connection: &mut SqliteConnection,
    lease: HistorySnapshotLease,
    position: SnapshotCursor,
) -> Result<i64> {
    if position.token != lease.token || position.segment_sequence < 0 || position.ordinal >= 128 {
        return Err(KernelError::Ownership);
    }
    let last: Option<Option<i64>> = sqlx::query_scalar("SELECT last_sequence FROM agena_history_snapshots WHERE token=? AND session_id=? AND generation=? AND owner_epoch=?")
        .bind(lease.token.to_string()).bind(lease.session.get()).bind(lease.generation.get()).bind(lease.epoch.to_string())
        .fetch_optional(&mut *connection).await.map_err(sql_error)?;
    let last = last.ok_or_else(|| KernelError::Conflict("history snapshot expired".into()))?;
    if !last.is_some_and(|last| position.segment_sequence <= last) {
        return Err(KernelError::NotFound("snapshot member".into()));
    }
    let sql = "SELECT m.entry_id FROM agena_segment_entries m JOIN (SELECT segment_id,first_ordinal,end_ordinal FROM agena_session_segments WHERE session_id=? AND sequence=? AND since_generation<=? UNION ALL SELECT segment_id,first_ordinal,end_ordinal FROM agena_retired_session_segments WHERE session_id=? AND sequence=? AND since_generation<=? AND until_generation>?) s ON s.segment_id=m.segment_id WHERE m.ordinal=? AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal";
    sqlx::query_scalar(sql)
        .bind(lease.session.get())
        .bind(position.segment_sequence)
        .bind(lease.generation.get())
        .bind(lease.session.get())
        .bind(position.segment_sequence)
        .bind(lease.generation.get())
        .bind(lease.generation.get())
        .bind(i64::from(position.ordinal))
        .fetch_optional(connection)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("snapshot member".into()))
}

pub(super) async fn page(
    pool: &sqlx::SqlitePool,
    lease: HistorySnapshotLease,
    after: Option<SnapshotCursor>,
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<SnapshotPage> {
    if after.is_some_and(|cursor| cursor.token != lease.token) {
        return Err(KernelError::Ownership);
    }
    if after.is_some_and(|cursor| cursor.segment_sequence < 0 || cursor.ordinal >= 128) {
        return Err(invalid("invalid snapshot cursor"));
    }
    let limit = limit.clamp(1, 200);
    let mut transaction = pool.begin().await.map_err(storage)?;
    let read_transaction = diagnostics.read_transaction();
    metrics::add(&diagnostics.0.snapshot_queries, 1);
    let root = sqlx::query("SELECT last_sequence FROM agena_history_snapshots WHERE token=? AND session_id=? AND generation=? AND owner_epoch=?")
        .bind(lease.token.to_string()).bind(lease.session.get()).bind(lease.generation.get()).bind(lease.epoch.to_string())
        .fetch_optional(&mut *transaction).await.map_err(sql_error)?.ok_or_else(||KernelError::Conflict("history snapshot expired".into()))?;
    let last: Option<i64> = root.try_get("last_sequence").map_err(storage)?;
    let Some(last) = last else {
        transaction.rollback().await.map_err(storage)?;
        read_transaction.complete();
        return Ok(SnapshotPage {
            entries: Vec::new(),
            next: None,
            has_more: false,
        });
    };
    let start = after.map_or(0, |cursor| cursor.segment_sequence);
    if start > last {
        return Err(invalid("snapshot cursor exceeds its history root"));
    }
    metrics::add(&diagnostics.0.snapshot_queries, 1);
    let segments = sqlx::query(SEGMENTS_SQL)
        .bind(lease.session.get())
        .bind(start)
        .bind(last)
        .bind(lease.generation.get())
        .bind((limit + 2) as i64)
        .bind(lease.session.get())
        .bind(start)
        .bind(last)
        .bind(lease.generation.get())
        .bind(lease.generation.get())
        .bind((limit + 2) as i64)
        .bind((limit + 2) as i64)
        .fetch_all(&mut *transaction)
        .await
        .map_err(sql_error)?;
    let mut selected = Vec::with_capacity(limit);
    let mut bytes = 0usize;
    let mut has_more = false;
    'segments: for segment in segments {
        let sequence: i64 = segment.try_get("sequence").map_err(storage)?;
        let mut first: i64 = segment.try_get("first_ordinal").map_err(storage)?;
        if let Some(cursor) = after
            && cursor.segment_sequence == sequence
        {
            first = first.max(i64::from(cursor.ordinal) + 1);
        }
        metrics::add(&diagnostics.0.snapshot_queries, 1);
        let rows = sqlx::query(MEMBERS_SQL)
            .bind(segment.try_get::<i64, _>("segment_id").map_err(storage)?)
            .bind(first)
            .bind(segment.try_get::<i64, _>("end_ordinal").map_err(storage)?)
            .bind((limit + 1 - selected.len()) as i64)
            .fetch_all(&mut *transaction)
            .await
            .map_err(sql_error)?;
        metrics::add(&diagnostics.0.snapshot_candidate_rows, rows.len());
        for row in rows {
            let charge = usize::try_from(row.try_get::<i64, _>("body_bytes").map_err(storage)?)
                .map_err(storage)?
                .saturating_add(512);
            if selected.len() == limit || bytes.saturating_add(charge) > 1024 * 1024 {
                if selected.is_empty() {
                    return Err(storage("entry exceeds snapshot page byte budget"));
                }
                has_more = true;
                break 'segments;
            }
            bytes += charge;
            selected.push((
                row.try_get::<i64, _>("entry_id").map_err(storage)?,
                SnapshotCursor {
                    token: lease.token,
                    segment_sequence: sequence,
                    ordinal: u32::try_from(row.try_get::<i64, _>("ordinal").map_err(storage)?)
                        .map_err(storage)?,
                },
            ));
        }
    }
    let mut entries = Vec::with_capacity(selected.len());
    if !selected.is_empty() {
        metrics::add(&diagnostics.0.snapshot_queries, 1);
        let ids = selected.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let rows =
            sqlx::query("SELECT e.*,EXISTS(SELECT 1 FROM agena_attempts started WHERE started.execution_id=e.execution_id AND started.started_at_ms IS NOT NULL) AS execution_started,CASE WHEN e.kind='user' THEN (SELECT x.current_generation FROM agena_executions x WHERE x.id=e.execution_id) ELSE e.generation END AS execution_generation,(SELECT a.response_entry_id FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=e.generation) AS response_entry_id,(SELECT a.phase FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=CASE WHEN e.kind='user' THEN (SELECT x.current_generation FROM agena_executions x WHERE x.id=e.execution_id) ELSE e.generation END) AS execution_phase FROM agena_entries e WHERE e.id IN (SELECT value FROM json_each(?))")
                .bind(encode(&ids)?)
                .fetch_all(&mut *transaction)
                .await
                .map_err(sql_error)?;
        let mut rows = rows
            .into_iter()
            .map(|row| Ok((row.try_get::<i64, _>("id").map_err(storage)?, row)))
            .collect::<Result<HashMap<_, _>>>()?;
        for (id, position) in selected {
            let row = rows
                .remove(&id)
                .ok_or_else(|| storage("snapshot member is missing"))?;
            metrics::add(
                &diagnostics.0.snapshot_body_bytes,
                row.try_get::<Option<&str>, _>("body_json")
                    .map_err(storage)?
                    .map_or(0, str::len),
            );
            entries.push(SnapshotEntry {
                position,
                entry: reads::entry_row(&row)?,
            });
        }
    }
    let next = if has_more {
        entries.last().map(|entry| entry.position)
    } else {
        None
    };
    transaction.rollback().await.map_err(storage)?;
    read_transaction.complete();
    Ok(SnapshotPage {
        entries,
        next,
        has_more,
    })
}
