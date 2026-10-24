//! Blob preparation and pins share the kernel writer. Immutable 128 KiB SQLite
//! chunks give file/memory the same recovery protocol, with no sidecar rename
//! window. A future measured file tier can implement these same capabilities.

use std::{collections::HashMap, sync::Mutex, time::Instant};

use agena_domain::{BlobId, CommandId};
use agena_storage::backend::kernel::{BlobGc, BlobLease, BlobPin, BlobRecord};

use super::*;

const CHUNK_BYTES: usize = 128 * 1024;
const MAX_PINS: i64 = 128;
const ORPHAN_GRACE: Duration = Duration::from_secs(30);

struct PinState {
    lease: BlobLease,
    families: usize,
    claimed: bool,
    revoked: bool,
    created: Instant,
}
#[derive(Default)]
pub(super) struct Pins(Mutex<HashMap<CommandId, PinState>>);
impl Pins {
    pub fn install(&self, lease: BlobLease) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(lease.token)
            .or_insert(PinState {
                lease,
                families: 0,
                claimed: false,
                revoked: false,
                created: Instant::now(),
            });
    }
    pub fn retain(&self, lease: BlobLease) -> Result<()> {
        let mut pins = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let pin = pins
            .get_mut(&lease.token)
            .filter(|p| p.lease == lease && !p.revoked)
            .ok_or_else(|| {
                KernelError::Conflict(
                    "blob pin expired; acquire a new read/preparation capability".into(),
                )
            })?;
        pin.families += 1;
        pin.claimed = true;
        Ok(())
    }
    pub fn release(&self, lease: BlobLease) {
        if let Some(pin) = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&lease.token)
            .filter(|p| p.lease == lease)
        {
            pin.families = pin.families.saturating_sub(1);
        }
    }
    fn revoke_abandoned(&self) -> Vec<BlobLease> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values_mut()
            .filter_map(|pin| {
                if pin.families == 0 && (pin.claimed || pin.created.elapsed() >= ORPHAN_GRACE) {
                    // Revoke under the same lock used by retain. A late constructor
                    // cannot acquire a lease that the collector is about to remove.
                    pin.revoked = true;
                    Some(pin.lease)
                } else {
                    None
                }
            })
            .collect()
    }
    fn acknowledge(&self, released: &[BlobLease]) {
        let mut pins = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for lease in released {
            pins.remove(&lease.token);
        }
    }
}

pub(super) fn command_bytes(command: &Command) -> Result<Vec<u8>> {
    // Idempotency describes the bytes/reference, independent of a newly issued
    // lifetime pin. Retrying preparation after a lost response can use a new pin.
    match command {
        Command::WriteBlob {
            lease,
            offset,
            bytes,
        } => serde_json::to_vec(&("blob_chunk", lease.session, lease.id, offset, bytes)),
        Command::ReadyBlob { lease, sha256 } => {
            serde_json::to_vec(&("blob_ready", lease.session, lease.id, sha256))
        }
        Command::Send {
            session,
            document,
            blobs,
        } => serde_json::to_vec(&(
            "send",
            session,
            document,
            blobs.iter().map(|b| b.id).collect::<Vec<_>>(),
        )),
        Command::EditQueued {
            session,
            execution,
            expected_revision,
            document,
            blobs,
        } => serde_json::to_vec(&(
            "edit_queued",
            session,
            execution,
            expected_revision,
            document,
            blobs.iter().map(|b| b.id).collect::<Vec<_>>(),
        )),
        Command::CompleteTool {
            scope,
            tool,
            result,
            blobs,
        } => serde_json::to_vec(&(
            "complete_tool",
            scope,
            tool,
            result,
            blobs.iter().map(|b| b.id).collect::<Vec<_>>(),
        )),
        _ => serde_json::to_vec(command),
    }
    .map_err(invalid)
}

fn record(row: &sqlx::sqlite::SqliteRow) -> Result<BlobRecord> {
    Ok(BlobRecord {
        id: row
            .try_get::<String, _>("id")
            .map_err(storage)?
            .parse()
            .map_err(storage)?,
        byte_len: row.try_get::<i64, _>("expected_bytes").map_err(storage)? as u64,
        sha256: row
            .try_get::<Option<Vec<u8>>, _>("digest")
            .map_err(storage)?
            .map(|d| d.try_into().map_err(|_| storage("invalid blob digest")))
            .transpose()?,
    })
}

async fn check_pin(
    connection: &mut SqliteConnection,
    lease: BlobLease,
    preparation: bool,
) -> Result<sqlx::sqlite::SqliteRow> {
    let row=sqlx::query("SELECT b.* FROM agena_blobs b JOIN agena_blob_pins p ON p.blob_id=b.id WHERE b.id=? AND p.token=? AND p.owner_epoch=? AND b.state!='retired'")
        .bind(lease.id.to_string()).bind(lease.token.to_string()).bind(lease.epoch.to_string()).fetch_optional(&mut *connection).await.map_err(sql_error)?.ok_or_else(||KernelError::Conflict("blob is retired or its pin expired".into()))?;
    if preparation && row.try_get::<i64, _>("session_id").map_err(storage)? != lease.session.get() {
        return Err(KernelError::Ownership);
    }
    Ok(row)
}

async fn member(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: EntryId,
) -> Result<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE s.session_id=? AND owner.deleted_at_ms IS NULL AND m.entry_id=? AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
        .bind(session.get()).bind(entry.get()).fetch_one(connection).await.map_err(sql_error)?;
    if !exists {
        return Err(KernelError::Ownership);
    }
    Ok(())
}

pub(super) async fn apply(
    connection: &mut SqliteConnection,
    command: &Command,
) -> Result<Mutation> {
    match command {
        Command::PrepareBlob {
            session,
            expected_bytes,
        } => {
            reads::metadata(connection, *session).await?;
            if *expected_bytes > i64::MAX as u64 {
                return Err(invalid("blob length exceeds SQLite range"));
            }
            let id = BlobId::new();
            sqlx::query("INSERT INTO agena_blobs (id,session_id,state,expected_bytes,created_at_ms) VALUES (?,?,'preparing',?,?)")
                .bind(id.to_string()).bind(session.get()).bind(*expected_bytes as i64).bind(now_ms()).execute(connection).await.map_err(sql_error)?;
            Ok(Mutation::BlobRecord(BlobRecord {
                id,
                byte_len: *expected_bytes,
                sha256: None,
            }))
        }
        Command::PinBlob {
            session,
            blob,
            epoch,
            preparation,
        } => {
            reads::metadata(connection, *session).await?;
            let row = sqlx::query("SELECT * FROM agena_blobs WHERE id=? AND state!='retired'")
                .bind(blob.to_string())
                .fetch_optional(&mut *connection)
                .await
                .map_err(sql_error)?
                .ok_or_else(|| {
                    KernelError::NotFound("managed blob (preparation may have expired)".into())
                })?;
            if *preparation {
                if row.try_get::<i64, _>("session_id").map_err(storage)? != session.get() {
                    return Err(KernelError::Ownership);
                }
            } else {
                let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_blob_references r JOIN agena_segment_entries m ON m.entry_id=r.entry_id JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE r.blob_id=? AND s.session_id=? AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
                    .bind(blob.to_string()).bind(session.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
                if !allowed || row.try_get::<String, _>("state").map_err(storage)? != "ready" {
                    return Err(KernelError::Ownership);
                }
            }
            pin(connection, *session, *epoch, record(&row)?).await
        }
        Command::PinSnapshotBlob {
            snapshot,
            position,
            blob,
        } => {
            let entry = snapshots::member(connection, *snapshot, *position).await?;
            let allowed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM agena_blob_references WHERE entry_id=? AND blob_id=?)",
            )
            .bind(entry)
            .bind(blob.to_string())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
            if !allowed {
                return Err(KernelError::Ownership);
            }
            let row = sqlx::query("SELECT * FROM agena_blobs WHERE id=? AND state='ready'")
                .bind(blob.to_string())
                .fetch_optional(&mut *connection)
                .await
                .map_err(sql_error)?
                .ok_or_else(|| KernelError::NotFound("managed blob".into()))?;
            pin(connection, snapshot.session, snapshot.epoch, record(&row)?).await
        }
        Command::WriteBlob {
            lease,
            offset,
            bytes,
        } => {
            let row = check_pin(connection, *lease, true).await?;
            let received = row.try_get::<i64, _>("received_bytes").map_err(storage)? as u64;
            let expected = row.try_get::<i64, _>("expected_bytes").map_err(storage)? as u64;
            let end = offset
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| invalid("blob offset overflow"))?;
            if row.try_get::<String, _>("state").map_err(storage)? != "preparing"
                || received != *offset
                || bytes.is_empty()
                || bytes.len() > CHUNK_BYTES
                || end > expected
                || (bytes.len() != CHUNK_BYTES && end != expected)
            {
                return Err(KernelError::Conflict(
                    "blob chunk is out of order, wrong length, or already sealed".into(),
                ));
            }
            sqlx::query("INSERT INTO agena_blob_chunks VALUES (?,?,?,?)")
                .bind(lease.id.to_string())
                .bind(*offset as i64)
                .bind(Sha256::digest(bytes).to_vec())
                .bind(bytes)
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            sqlx::query("UPDATE agena_blobs SET received_bytes=? WHERE id=?")
                .bind(end as i64)
                .bind(lease.id.to_string())
                .execute(connection)
                .await
                .map_err(sql_error)?;
            Ok(Mutation::BlobOffset(end))
        }
        Command::ReadyBlob { lease, sha256 } => {
            let row = check_pin(connection, *lease, true).await?;
            if row.try_get::<String, _>("state").map_err(storage)? != "preparing"
                || row.try_get::<i64, _>("received_bytes").map_err(storage)?
                    != row.try_get::<i64, _>("expected_bytes").map_err(storage)?
            {
                return Err(KernelError::Conflict(
                    "blob preparation is incomplete or already sealed".into(),
                ));
            }
            sqlx::query("UPDATE agena_blobs SET state='ready',digest=? WHERE id=?")
                .bind(sha256.as_slice())
                .bind(lease.id.to_string())
                .execute(connection)
                .await
                .map_err(sql_error)?;
            Ok(Mutation::BlobReady(BlobDescriptor {
                id: lease.id,
                byte_len: record(&row)?.byte_len,
                sha256: *sha256,
            }))
        }
        _ => Err(invalid("incompatible blob command")),
    }
}

async fn pin(
    connection: &mut SqliteConnection,
    session: SessionId,
    epoch: KernelEpoch,
    record: BlobRecord,
) -> Result<Mutation> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_blob_pins")
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
    if count >= MAX_PINS {
        return Err(KernelError::Busy);
    }
    let lease = BlobLease {
        id: record.id,
        session,
        token: CommandId::new(),
        epoch,
    };
    // Lifetime protects active handles. Startup clears former-owner pins;
    // bounded maintenance reclaims abandoned response/reader families.
    sqlx::query("INSERT INTO agena_blob_pins VALUES (?,?,?,9223372036854775807)")
        .bind(record.id.to_string())
        .bind(lease.token.to_string())
        .bind(epoch.to_string())
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(Mutation::BlobPin(Box::new(BlobPin { lease, record })))
}

pub(super) async fn publish(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: i64,
    blobs: &[BlobLease],
) -> Result<()> {
    if blobs.len() > 16 {
        return Err(invalid("at most 16 managed blobs per entry"));
    }
    for (index, lease) in blobs.iter().enumerate() {
        if lease.session != session {
            return Err(KernelError::Ownership);
        }
        // A retained read capability can republish an immutable attachment,
        // including one inherited from a shared conversation segment. Opening
        // the pin already checked membership in this session.
        let row = check_pin(connection, *lease, false).await?;
        if row.try_get::<String, _>("state").map_err(storage)? != "ready" {
            return Err(KernelError::Conflict(
                "blob preparation is not ready".into(),
            ));
        }
        sqlx::query("INSERT INTO agena_blob_references VALUES (?,?,?)")
            .bind(entry)
            .bind(lease.id.to_string())
            .bind(index.to_string())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
    Ok(())
}

pub(super) async fn descriptors(
    connection: &mut SqliteConnection,
    entry: i64,
) -> Result<Vec<BlobDescriptor>> {
    let rows=sqlx::query("SELECT b.* FROM agena_blobs b JOIN agena_blob_references r ON r.blob_id=b.id WHERE r.entry_id=? ORDER BY CAST(r.field AS INTEGER) LIMIT 17")
        .bind(entry).fetch_all(connection).await.map_err(sql_error)?;
    if rows.len() > 16 {
        return Err(storage("blob reference count exceeds entry budget"));
    }
    rows.iter()
        .map(|row| {
            let record = record(row)?;
            Ok(BlobDescriptor {
                id: record.id,
                byte_len: record.byte_len,
                sha256: record
                    .sha256
                    .ok_or_else(|| storage("unready blob reference"))?,
            })
        })
        .collect()
}

pub(super) async fn entry_blobs(
    backend: &Backend,
    session: SessionId,
    entry: EntryId,
) -> Result<Vec<BlobDescriptor>> {
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let read_transaction = backend.live.diagnostics().read_transaction();
    member(&mut tx, session, entry).await?;
    let result = descriptors(&mut tx, entry.get()).await?;
    tx.rollback().await.map_err(storage)?;
    read_transaction.complete();
    Ok(result)
}

pub(super) async fn range(
    backend: &Backend,
    lease: BlobLease,
    offset: u64,
    limit: usize,
) -> Result<BlobRange> {
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let read_transaction = backend.live.diagnostics().read_transaction();
    let row = check_pin(&mut tx, lease, false).await?;
    if row.try_get::<String, _>("state").map_err(storage)? != "ready" {
        return Err(KernelError::Conflict("blob is not ready".into()));
    }
    let total = record(&row)?.byte_len;
    if offset > total {
        return Err(invalid("blob offset exceeds its length"));
    }
    let end = total.min(offset.saturating_add(limit.clamp(1, CHUNK_BYTES) as u64));
    let mut bytes = Vec::with_capacity((end - offset) as usize);
    if offset < end {
        let first = offset / CHUNK_BYTES as u64 * CHUNK_BYTES as u64;
        let rows=sqlx::query("SELECT logical_start,digest,body FROM agena_blob_chunks WHERE blob_id=? AND logical_start>=? AND logical_start<? ORDER BY logical_start LIMIT 2")
            .bind(lease.id.to_string()).bind(first as i64).bind(end as i64).fetch_all(&mut *tx).await.map_err(sql_error)?;
        let mut position = offset;
        for row in rows {
            let start = row.try_get::<i64, _>("logical_start").map_err(storage)? as u64;
            let body = row.try_get::<Vec<u8>, _>("body").map_err(storage)?;
            let digest = row.try_get::<Vec<u8>, _>("digest").map_err(storage)?;
            if body.len() != ((total - start).min(CHUNK_BYTES as u64) as usize)
                || Sha256::digest(&body).as_slice() != digest
                || position < start
                || position >= start + body.len() as u64
            {
                return Err(storage("blob chunk length/checksum/continuity mismatch"));
            }
            let take = end.min(start + body.len() as u64);
            bytes.extend_from_slice(&body[(position - start) as usize..(take - start) as usize]);
            position = take;
        }
        if position != end {
            return Err(storage("blob chunks have a gap"));
        }
    }
    tx.rollback().await.map_err(storage)?;
    read_transaction.complete();
    Ok(BlobRange {
        start: offset,
        next: end,
        bytes,
        eof: end == total,
    })
}

/// One bounded maintenance transaction: at most 128 released pins, 16 retired
/// identities, and 8 chunks (1 MiB). References and live pins both prevent the
/// retirement step. Retired bytes are never eligible for new publication.
pub(super) async fn collect(database: &KernelDatabase, pins: &Pins) -> Result<BlobGc> {
    let released = pins.revoke_abandoned();
    let mut tx = database
        .writer
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let result = match collect_sql(&mut tx, &released).await {
        Ok(result) => result,
        Err(error) => {
            abort_transaction(tx).await?;
            return Err(error);
        }
    };

    if let Err(error) = tx.commit().await {
        return Err(if rejected_commit(&error) {
            storage(error)
        } else {
            KernelError::CommitUnknown {
                request_id: RequestId::new(),
                diagnostic: error.to_string(),
            }
        });
    }

    pins.acknowledge(&released);

    Ok(result)
}

async fn collect_sql(tx: &mut SqliteConnection, released: &[BlobLease]) -> Result<BlobGc> {
    for lease in released {
        sqlx::query("UPDATE agena_blobs SET abandoned=1 WHERE id=?")
            .bind(lease.id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
        sqlx::query("DELETE FROM agena_blob_pins WHERE blob_id=? AND token=? AND owner_epoch=?")
            .bind(lease.id.to_string())
            .bind(lease.token.to_string())
            .bind(lease.epoch.to_string())
            .execute(&mut *tx)
            .await
            .map_err(sql_error)?;
    }
    // Scan a fixed number of identities even when a million earlier blobs
    // remain referenced. Persist the scan position so later calls make progress.
    let cursor: String = sqlx::query_scalar("SELECT after_id FROM agena_blob_gc_state WHERE id=1")
        .fetch_one(&mut *tx)
        .await
        .map_err(sql_error)?;
    let candidates: Vec<String> =
        sqlx::query_scalar("SELECT id FROM agena_blobs WHERE id>? ORDER BY id LIMIT 16")
            .bind(cursor)
            .fetch_all(&mut *tx)
            .await
            .map_err(sql_error)?;
    sqlx::query("UPDATE agena_blob_gc_state SET after_id=? WHERE id=1")
        .bind(candidates.last().map_or("", String::as_str))
        .execute(&mut *tx)
        .await
        .map_err(sql_error)?;
    let retired=sqlx::query("UPDATE agena_blobs SET state='retired' WHERE id IN (SELECT value FROM json_each(?)) AND state IN ('preparing','ready') AND (created_at_ms<=? OR abandoned=1) AND NOT EXISTS(SELECT 1 FROM agena_blob_references r WHERE r.blob_id=agena_blobs.id) AND NOT EXISTS(SELECT 1 FROM agena_blob_pins p WHERE p.blob_id=agena_blobs.id)")
        .bind(encode(&candidates)?).bind(now_ms()-ORPHAN_GRACE.as_millis() as i64).execute(&mut *tx).await.map_err(sql_error)?.rows_affected();
    let chunks=sqlx::query("DELETE FROM agena_blob_chunks WHERE (blob_id,logical_start) IN (SELECT blob_id,logical_start FROM agena_blob_chunks WHERE blob_id=(SELECT id FROM agena_blobs WHERE state='retired' ORDER BY created_at_ms,id LIMIT 1) ORDER BY logical_start LIMIT 8)")
        .execute(&mut *tx).await.map_err(sql_error)?.rows_affected();
    let blobs=sqlx::query("DELETE FROM agena_blobs WHERE id IN (SELECT id FROM agena_blobs WHERE state='retired' ORDER BY created_at_ms,id LIMIT 16) AND NOT EXISTS(SELECT 1 FROM agena_blob_chunks c WHERE c.blob_id=agena_blobs.id)")
        .execute(&mut *tx).await.map_err(sql_error)?.rows_affected();
    Ok(BlobGc {
        retired,
        chunks,
        blobs,
    })
}
