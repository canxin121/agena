//! Versioned transcript archives. Every SQL transaction ends before sink I/O.
//! Membership is pinned; other facts explicitly use per-record observations.
//! Content block bytes are streamed without decoding even a large PTY frame.

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::sqlite::SqliteRow;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::*;

const PIECE_BYTES: usize = 64 * 1024;
const MAX_FACT_BYTES: i64 = 4 * 1024 * 1024;
// A captured input manifest admits up to 8 MiB, including metadata. Raw JSON
// strings can double when escaped inside an archive line; leave room for the
// provider state/record envelope without rejecting a valid captured metadata.
const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Exports(std::sync::Mutex<std::collections::HashMap<SessionId, usize>>);
struct Export<'a> {
    owner: &'a Exports,
    session: SessionId,
}
impl Exports {
    fn acquire(&self, session: SessionId) -> Result<Export<'_>> {
        let mut active = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if active.values().sum::<usize>() >= 8 || active.get(&session).copied().unwrap_or(0) >= 2 {
            return Err(KernelError::Busy);
        }
        *active.entry(session).or_default() += 1;
        Ok(Export {
            owner: self,
            session,
        })
    }
}
impl Drop for Export<'_> {
    fn drop(&mut self) {
        let mut active = self.owner.0.lock().unwrap_or_else(|e| e.into_inner());
        let count = active.get_mut(&self.session).expect("active export");
        *count -= 1;
        if *count == 0 {
            active.remove(&self.session);
        }
    }
}

type Sink<'a> = dyn AsyncWrite + Unpin + Send + 'a;

/// Private capabilities are created only by fixed membership or a dependency
/// read from an already authorized record. Public callers cannot mint support.
#[derive(Clone, Copy)]
enum ExportPosition {
    Member(SnapshotCursor),
    Support(i64),
}
async fn authorized(
    connection: &mut SqliteConnection,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
) -> Result<i64> {
    match position {
        ExportPosition::Member(cursor) => snapshots::member(connection, lease, cursor).await,
        ExportPosition::Support(id) => {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_entries WHERE id=?)")
                    .bind(id)
                    .fetch_one(connection)
                    .await
                    .map_err(sql_error)?;
            if !exists {
                return Err(storage("archive dependency disappeared"));
            }
            Ok(id)
        }
    }
}
async fn enqueue(graph: &mut SqliteConnection, id: i64) -> Result<()> {
    if id <= 0 {
        return Err(storage("archive dependency identity is invalid"));
    }
    sqlx::query("INSERT INTO entries(id,member) VALUES (?,0) ON CONFLICT(id) DO NOTHING")
        .bind(id)
        .execute(graph)
        .await
        .map_err(storage)?;
    Ok(())
}
fn content_references(value: &Value, ids: &mut Vec<String>) -> Result<()> {
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                if matches!(key.as_str(), "resource_id" | "content_id") {
                    if let Some(id) = value.as_str() {
                        if ids.len() >= 4096 {
                            return Err(storage("archive reference budget exceeded"));
                        }
                        ids.push(id.to_owned());
                    }
                } else {
                    content_references(value, ids)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                content_references(value, ids)?;
            }
        }
        _ => {}
    }
    Ok(())
}
async fn discover(
    backend: &Backend,
    graph: &mut SqliteConnection,
    kind: &str,
    value: &Value,
) -> Result<()> {
    let mut entries = Vec::new();
    let mut source_ids = Vec::new();
    match kind {
        "entry" | "support_entry" => {
            let facts = &value["facts"];
            for value in [
                &facts["entry"]["message_id"],
                &facts["entry"]["response_entry_id"],
                &facts["execution"]["message_id"],
                &facts["attempt"]["response_entry_id"],
            ] {
                if let Some(id) = value.as_i64() {
                    entries.push(id);
                }
            }
            if facts["entry"]["payload"]["kind"] == "compaction" {
                if let Some(id) = facts["entry"]["payload"]["through"].as_i64() {
                    entries.push(id);
                }
                if let Some(id) = facts["entry"]["payload"]["summary"].as_str() {
                    let owner: i64 =
                        sqlx::query_scalar("SELECT entry_id FROM agena_content_streams WHERE id=?")
                            .bind(id)
                            .fetch_one(backend.reader.get_sqlite_connection_pool())
                            .await
                            .map_err(sql_error)?;
                    entries.push(owner);
                }
            }
            for raw in [&facts["tool"]["input_json"], &facts["tool"]["result_json"]] {
                if let Some(raw) = raw.as_str() {
                    let value: Value = decode(raw)?;
                    content_references(&value, &mut source_ids)?;
                }
            }
        }
        "attempt" => {
            if let Some(id) = value["response_entry_id"].as_i64() {
                entries.push(id);
            }
        }
        "round" => {
            let id = value["id"]
                .as_i64()
                .ok_or_else(|| storage("archive round identity is invalid"))?;
            sqlx::query("INSERT INTO rounds VALUES (?) ON CONFLICT(id) DO NOTHING")
                .bind(id)
                .execute(&mut *graph)
                .await
                .map_err(storage)?;
            if let Some(id) = value["base_checkpoint_entry_id"].as_i64() {
                entries.push(id);
            }
        }
        "round_input" => {
            for key in ["entry_id", "response_entry_id"] {
                if let Some(id) = value[key].as_i64() {
                    entries.push(id);
                }
            }
        }
        "round_input_source" => {
            if let Some(id) = value["entry_id"].as_i64() {
                entries.push(id);
            }
            let head: ContentStreamView = decode(
                value["head_json"]
                    .as_str()
                    .ok_or_else(|| storage("archive frozen source has no head"))?,
            )?;
            let actual =
                sqlx::query("SELECT entry_id,epoch,kind FROM agena_content_streams WHERE id=?")
                    .bind(head.id.to_string())
                    .fetch_one(backend.reader.get_sqlite_connection_pool())
                    .await
                    .map_err(sql_error)?;
            if actual.try_get::<i64, _>("entry_id").map_err(storage)? != head.entry.get()
                || actual.try_get::<String, _>("epoch").map_err(storage)? != head.epoch.to_string()
                || actual.try_get::<String, _>("kind").map_err(storage)?
                    != commands::enum_text(&head.kind)?
            {
                return Err(KernelError::ContentReset);
            }
            entries.push(head.entry.get());
        }
        "round_input_replay" => {
            let id = value["source_round_id"]
                .as_i64()
                .ok_or_else(|| storage("archive replay round identity is invalid"))?;
            let owner:i64=sqlx::query_scalar("SELECT x.message_id FROM agena_rounds r JOIN agena_executions x ON x.id=r.execution_id WHERE r.id=?").bind(id).fetch_one(backend.reader.get_sqlite_connection_pool()).await.map_err(sql_error)?;
            entries.push(owner);
        }
        _ => {}
    }
    for id in entries {
        enqueue(graph, id).await?;
    }
    // Embedded tool references may follow an existing membership/dependency,
    // or a source explicitly authorized by an exported frozen round manifest.
    for id in source_ids {
        let Some(owner) =
            sqlx::query_scalar::<_, i64>("SELECT entry_id FROM agena_content_streams WHERE id=?")
                .bind(&id)
                .fetch_optional(backend.reader.get_sqlite_connection_pool())
                .await
                .map_err(sql_error)?
        else {
            return Err(storage("archive tool source reference is missing"));
        };
        let known: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entries WHERE id=?)")
            .bind(owner)
            .fetch_one(&mut *graph)
            .await
            .map_err(storage)?;
        if !known {
            return Err(KernelError::Ownership);
        }
    }
    Ok(())
}

struct Writer<'a, 'sink> {
    backend: &'a Backend,
    session: SessionId,
    target: &'a mut Sink<'sink>,
    graph: &'a mut SqliteConnection,
    hash: Sha256,
    records: u64,
    bytes: u64,
}

#[derive(Serialize)]
struct Record<'a, T: ?Sized> {
    kind: &'static str,
    data: &'a T,
}

struct Count(usize);
impl std::io::Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_LINE_BYTES.saturating_sub(self.0) {
            return Err(std::io::Error::other("archive record exceeds byte limit"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Writer<'_, '_> {
    async fn line<T: Serialize + ?Sized>(
        &mut self,
        kind: &'static str,
        data: &T,
        hashed: bool,
    ) -> Result<()> {
        if matches!(
            kind,
            "entry"
                | "support_entry"
                | "attempt"
                | "round"
                | "round_input"
                | "round_input_source"
                | "round_input_replay"
        ) {
            discover(
                self.backend,
                self.graph,
                kind,
                &serde_json::to_value(data).map_err(storage)?,
            )
            .await?;
        }
        let record = Record { kind, data };
        let mut count = Count(0);
        serde_json::to_writer(&mut count, &record).map_err(storage)?;
        let length = count.0 + 1;
        // Reserve the exact encoded buffer before allocating it. The source
        // ReadView retains its separate lease until this write completes.
        let _encoded = self.backend.views.reserve_view(self.session, length)?;
        let mut bytes = Vec::with_capacity(length);
        serde_json::to_writer(&mut bytes, &record).map_err(storage)?;
        bytes.push(b'\n');
        self.target.write_all(&bytes).await.map_err(storage)?;
        if hashed {
            self.hash.update(&bytes);
            self.records += 1;
            self.bytes += bytes.len() as u64;
        }
        Ok(())
    }
}

fn admission(backend: &Backend) -> Result<OwnedSemaphorePermit> {
    backend.cache.ensure_open()?;
    backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)
}

/// Raw JSON columns remain strings: unknown provider fields and their stored
/// representation survive without a decode/re-encode projection.
fn raw(row: &SqliteRow, texts: &[&str], integers: &[&str]) -> Result<Value> {
    let mut value = serde_json::Map::new();
    for column in texts {
        value.insert(
            (*column).into(),
            row.try_get::<Option<String>, _>(*column)
                .map_err(storage)?
                .into(),
        );
    }
    for column in integers {
        value.insert(
            (*column).into(),
            row.try_get::<Option<i64>, _>(*column)
                .map_err(storage)?
                .into(),
        );
    }
    Ok(Value::Object(value))
}

#[derive(Serialize)]
struct Bundle {
    entry: EntryView,
    created_at_ms: i64,
    round_id: Option<i64>,
    execution: Value,
    attempt: Value,
    tool: Option<Value>,
    activities: Vec<Value>,
    interaction: Option<Value>,
    sources: Vec<ContentId>,
    blobs: Vec<BlobDescriptor>,
    round_upper: i64,
}

async fn bundle(
    backend: &Backend,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
) -> Result<ReadView<Bundle>> {
    let _read = admission(backend)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let read_transaction = backend.live.diagnostics().read_transaction();
    let id = authorized(&mut tx, lease, position).await?;
    let row = sqlx::query("SELECT e.*,a.phase AS execution_phase,a.response_entry_id FROM agena_entries e JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=e.generation WHERE e.id=?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(sql_error)?;
    let entry = reads::entry_row(&row)?;
    let execution = sqlx::query("SELECT *,(SELECT COALESCE(MAX(r.id),0) FROM agena_rounds r WHERE r.execution_id=agena_executions.id) AS round_upper FROM agena_executions WHERE id=?")
        .bind(entry.execution_id.to_string()).fetch_one(&mut *tx).await.map_err(sql_error)?;
    let attempt = sqlx::query("SELECT * FROM agena_attempts WHERE execution_id=? AND generation=?")
        .bind(entry.execution_id.to_string())
        .bind(entry.generation.get())
        .fetch_one(&mut *tx)
        .await
        .map_err(sql_error)?;
    let tool = if let EntryPayload::Tool { tool_id } = entry.payload {
        let bytes: i64 = sqlx::query_scalar("SELECT input_bytes+result_bytes FROM agena_tool_calls INDEXED BY idx_agena_tool_bytes WHERE id=?")
            .bind(tool_id.get()).fetch_one(&mut *tx).await.map_err(sql_error)?;
        if bytes > MAX_FACT_BYTES {
            return Err(storage("archive tool fact exceeds byte limit"));
        }
        let row = sqlx::query("SELECT * FROM agena_tool_calls WHERE id=?")
            .bind(tool_id.get())
            .fetch_one(&mut *tx)
            .await
            .map_err(sql_error)?;
        Some(raw(
            &row,
            &[
                "execution_id",
                "call_id",
                "name",
                "execution_kind",
                "input_json",
                "phase",
                "result_json",
            ],
            &[
                "id",
                "round_id",
                "generation",
                "created_at_ms",
                "finished_at_ms",
            ],
        )?)
    } else {
        None
    };
    let interaction = if let EntryPayload::Interaction { interaction_id } = entry.payload {
        let bytes: i64 = sqlx::query_scalar("SELECT request_bytes+decision_bytes+remember_bytes FROM agena_interactions INDEXED BY idx_agena_interaction_bytes WHERE id=?")
            .bind(interaction_id.get()).fetch_one(&mut *tx).await.map_err(sql_error)?;
        if bytes > MAX_FACT_BYTES {
            return Err(storage("archive interaction fact exceeds byte limit"));
        }
        let row = sqlx::query("SELECT *,? AS revision FROM agena_interactions WHERE id=?")
            .bind(entry.revision.get())
            .bind(interaction_id.get())
            .fetch_one(&mut *tx)
            .await
            .map_err(sql_error)?;
        Some(raw(
            &row,
            &[
                "execution_id",
                "request_kind",
                "request_json",
                "decision_json",
                "remember_json",
            ],
            &[
                "id",
                "tool_id",
                "session_id",
                "generation",
                "created_at_ms",
                "decided_at_ms",
                "revision",
            ],
        )?)
    } else {
        None
    };
    let sources: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM agena_content_streams WHERE entry_id=? ORDER BY ordinal,id LIMIT 129",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .map_err(sql_error)?;
    if sources.len() > 128 {
        return Err(storage("archive source identity budget is corrupt"));
    }
    // The per-tool identity cap bounds this observation, including completed
    // activities. Keep raw outcome JSON for one-time conversion fidelity.
    let activity_rows = sqlx::query(
        "SELECT * FROM agena_background_activities WHERE entry_id=? ORDER BY id LIMIT 17",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .map_err(sql_error)?;
    if activity_rows.len() > super::activity::MAX_ACTIVITIES_PER_TOOL as usize {
        return Err(storage("archive activity identity budget is corrupt"));
    }
    let activities = activity_rows
        .iter()
        .map(|row| {
            raw(
                row,
                &[
                    "id",
                    "execution_id",
                    "owner_epoch",
                    "kind",
                    "label",
                    "phase",
                    "outcome_json",
                ],
                &[
                    "session_id",
                    "generation",
                    "round_id",
                    "tool_id",
                    "entry_id",
                    "created_at_ms",
                    "finished_at_ms",
                ],
            )
        })
        .collect::<Result<_>>()?;
    let result = Bundle {
        entry,
        created_at_ms: row.try_get("created_at_ms").map_err(storage)?,
        round_id: row.try_get("round_id").map_err(storage)?,
        execution: raw(
            &execution,
            &["id"],
            &[
                "session_id",
                "message_id",
                "current_generation",
                "created_at_ms",
            ],
        )?,
        attempt: raw(
            &attempt,
            &["execution_id", "owner_epoch", "phase"],
            &[
                "generation",
                "session_id",
                "started_at_ms",
                "finished_at_ms",
                "response_entry_id",
            ],
        )?,
        tool,
        activities,
        interaction,
        sources: sources
            .into_iter()
            .map(|id| id.parse().map_err(storage))
            .collect::<Result<_>>()?,
        blobs: blobs::descriptors(&mut tx, id).await?,
        round_upper: execution.try_get("round_upper").map_err(storage)?,
    };
    tx.rollback().await.map_err(storage)?;
    read_transaction.complete();
    let bytes = modeled_read_bytes(&result)?;
    backend.wrap_read(lease.session, result, bytes)
}

async fn round_fact(
    backend: &Backend,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    after: i64,
    through: i64,
) -> Result<ReadView<Option<Value>>> {
    let _read = admission(backend)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let read_transaction = backend.live.diagnostics().read_transaction();
    let entry = authorized(&mut tx, lease, position).await?;
    let next = sqlx::query("SELECT id,provider_state_bytes,length(CAST(input_metadata_json AS BLOB)) AS metadata_bytes FROM agena_rounds INDEXED BY idx_agena_round_export WHERE execution_id=(SELECT execution_id FROM agena_entries WHERE id=?) AND id>? AND id<=? ORDER BY id LIMIT 1")
        .bind(entry).bind(after).bind(through).fetch_optional(&mut *tx).await.map_err(sql_error)?;
    let value = if let Some(next) = next {
        if next
            .try_get::<i64, _>("provider_state_bytes")
            .map_err(storage)?
            > MAX_FACT_BYTES
        {
            return Err(storage("archive provider state exceeds byte limit"));
        }
        if next.try_get::<i64, _>("metadata_bytes").map_err(storage)?
            > prompt::MAX_INPUT_BYTES as i64
        {
            return Err(storage("archive input metadata exceeds byte limit"));
        }
        let row = sqlx::query("SELECT r.*,CASE WHEN c.round_id IS NULL THEN 'provider' ELSE 'compaction' END AS purpose,c.base_checkpoint_entry_id,c.rewrite_epoch AS input_rewrite_epoch FROM agena_rounds r LEFT JOIN agena_compaction_rounds c ON c.round_id=r.id WHERE r.id=?")
            .bind(next.try_get::<i64, _>("id").map_err(storage)?)
            .fetch_one(&mut *tx)
            .await
            .map_err(sql_error)?;
        Some(raw(
            &row,
            &[
                "execution_id",
                "phase",
                "provider_state_json",
                "input_metadata_json",
                "purpose",
            ],
            &[
                "id",
                "generation",
                "session_id",
                "ordinal",
                "created_at_ms",
                "finished_at_ms",
                "base_checkpoint_entry_id",
                "input_rewrite_epoch",
            ],
        )?)
    } else {
        None
    };
    tx.rollback().await.map_err(storage)?;
    read_transaction.complete();
    let bytes = modeled_read_bytes(&value)?;
    backend.wrap_read(lease.session, value, bytes)
}

#[derive(Clone, Copy)]
enum Collection {
    Inputs,
    Sources,
    Replays,
}
impl Collection {
    fn specification(
        self,
    ) -> (
        &'static str,
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
        &'static str,
    ) {
        match self {
            Self::Inputs => (
                "round_input",
                "SELECT * FROM agena_round_inputs WHERE round_id=? AND ordinal>CAST(? AS INTEGER) ORDER BY ordinal LIMIT 200",
                &["tool_phase", "execution_phase"],
                &[
                    "round_id",
                    "ordinal",
                    "entry_id",
                    "entry_revision",
                    "response_entry_id",
                    "interaction_decided",
                ],
                "ordinal",
            ),
            Self::Sources => (
                "round_input_source",
                "SELECT * FROM agena_round_input_sources WHERE round_id=? AND content_id>? ORDER BY content_id LIMIT 1",
                &["content_id", "head_json"],
                &["round_id", "entry_id"],
                "content_id",
            ),
            Self::Replays => (
                "round_input_replay",
                "SELECT * FROM agena_round_input_replays WHERE round_id=? AND source_round_id>CAST(? AS INTEGER) ORDER BY source_round_id LIMIT 200",
                &["phase"],
                &["round_id", "source_round_id", "has_provider_state"],
                "source_round_id",
            ),
        }
    }
}

async fn collection(
    backend: &Backend,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    round: i64,
    kind: Collection,
    after: &str,
) -> Result<ReadView<Vec<Value>>> {
    let _read = admission(backend)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let read_transaction = backend.live.diagnostics().read_transaction();
    let entry = authorized(&mut tx, lease, position).await?;
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_rounds r JOIN agena_entries e ON e.execution_id=r.execution_id WHERE r.id=? AND e.id=?)")
        .bind(round).bind(entry).fetch_one(&mut *tx).await.map_err(sql_error)?;
    if !allowed {
        return Err(KernelError::Ownership);
    }
    let (_, sql, texts, integers, _) = kind.specification();
    let rows = sqlx::query(sql)
        .bind(round)
        .bind(after)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_error)?;
    let values = rows
        .iter()
        .map(|row| {
            let mut value = raw(row, texts, integers)?;
            if matches!(kind, Collection::Inputs) {
                value["tool_phase"] = row
                    .try_get::<Option<String>, _>("tool_phase")
                    .map_err(storage)?
                    .into();
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>>>()?;
    tx.rollback().await.map_err(storage)?;
    read_transaction.complete();
    let bytes = modeled_read_bytes(&values)?;
    backend.wrap_read(lease.session, values, bytes)
}

async fn attempts(
    writer: &mut Writer<'_, '_>,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    through: i64,
) -> Result<()> {
    let mut after = 0;
    loop {
        let values = {
            let _read = admission(writer.backend)?;
            let mut tx = writer
                .backend
                .reader
                .get_sqlite_connection_pool()
                .begin()
                .await
                .map_err(storage)?;
            let read_transaction = writer.backend.live.diagnostics().read_transaction();
            let entry = authorized(&mut tx, lease, position).await?;
            let rows = sqlx::query("SELECT * FROM agena_attempts WHERE execution_id=(SELECT execution_id FROM agena_entries WHERE id=?) AND generation>? AND generation<=? ORDER BY generation LIMIT 200")
                .bind(entry).bind(after).bind(through).fetch_all(&mut *tx).await.map_err(sql_error)?;
            let values = rows
                .iter()
                .map(|row| {
                    raw(
                        row,
                        &["execution_id", "owner_epoch", "phase"],
                        &[
                            "generation",
                            "session_id",
                            "started_at_ms",
                            "finished_at_ms",
                            "response_entry_id",
                        ],
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            tx.rollback().await.map_err(storage)?;
            read_transaction.complete();
            let bytes = modeled_read_bytes(&values)?;
            writer.backend.wrap_read(lease.session, values, bytes)?
        };
        if values.is_empty() {
            break;
        }
        for value in &*values {
            writer.line("attempt", value, true).await?;
            after = value["generation"]
                .as_i64()
                .ok_or_else(|| storage("invalid archive generation"))?;
        }
    }
    Ok(())
}

async fn rounds(
    writer: &mut Writer<'_, '_>,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    through: i64,
) -> Result<()> {
    let mut after = 0;
    loop {
        let round = round_fact(writer.backend, lease, position, after, through).await?;
        let Some(value) = &*round else {
            break;
        };
        after = value["id"]
            .as_i64()
            .ok_or_else(|| storage("invalid archive round"))?;
        writer.line("round", value, true).await?;
        for kind in [Collection::Inputs, Collection::Sources, Collection::Replays] {
            let (tag, _, _, _, key) = kind.specification();
            let mut cursor = if matches!(kind, Collection::Inputs) {
                "-1".into()
            } else {
                String::new()
            };
            // Empty text casts to zero for the positive replay identities.
            loop {
                let page =
                    collection(writer.backend, lease, position, after, kind, &cursor).await?;
                if page.is_empty() {
                    break;
                }
                for value in &*page {
                    writer.line(tag, value, true).await?;
                    cursor = if matches!(kind, Collection::Sources) {
                        value[key]
                            .as_str()
                            .ok_or_else(|| storage("invalid archive source"))?
                            .into()
                    } else {
                        value[key]
                            .as_i64()
                            .ok_or_else(|| storage("invalid archive input cursor"))?
                            .to_string()
                    };
                }
            }
        }
    }
    Ok(())
}

async fn check_source(
    connection: &mut SqliteConnection,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    head: &ContentStreamView,
) -> Result<()> {
    let entry = authorized(connection, lease, position).await?;
    let row = sqlx::query("SELECT entry_id,epoch,kind,committed_head,retained_start,lost_bytes,lost_packets FROM agena_content_streams WHERE id=?")
        .bind(head.id.to_string()).fetch_one(connection).await.map_err(sql_error)?;
    if entry != head.entry.get() || row.try_get::<i64, _>("entry_id").map_err(storage)? != entry {
        return Err(KernelError::Ownership);
    }
    if row.try_get::<String, _>("epoch").map_err(storage)? != head.epoch.to_string()
        || row.try_get::<String, _>("kind").map_err(storage)? != commands::enum_text(&head.kind)?
        || row.try_get::<i64, _>("committed_head").map_err(storage)? < head.committed as i64
    {
        return Err(KernelError::ContentReset);
    }
    if row.try_get::<i64, _>("lost_bytes").map_err(storage)?
        < head.capture_loss.committed_bytes as i64
        || row.try_get::<i64, _>("lost_packets").map_err(storage)?
            < head.capture_loss.committed_packets as i64
    {
        return Err(storage("archive capture loss summary regressed"));
    }
    let floor = row.try_get::<i64, _>("retained_start").map_err(storage)? as u64;
    if floor > head.retained_start {
        return Err(KernelError::ContentGap {
            content: head.id,
            requested: head.retained_start,
            retained_start: floor,
        });
    }
    Ok(())
}

#[derive(Serialize)]
struct Block {
    content_id: ContentId,
    logical_start: i64,
    logical_end: i64,
    codec: String,
    sha256: Vec<u8>,
    byte_len: i64,
}

async fn block(
    backend: &Backend,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    head: &ContentStreamView,
    start: i64,
) -> Result<ReadView<Option<Block>>> {
    let _read = admission(backend)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let read_transaction = backend.live.diagnostics().read_transaction();
    check_source(&mut tx, lease, position, head).await?;
    let row = sqlx::query("SELECT logical_start,logical_end,codec,digest,length(body) AS bytes FROM agena_content_blocks WHERE content_id=? AND logical_start>=? AND logical_start<? ORDER BY logical_start LIMIT 1")
        .bind(head.id.to_string()).bind(start).bind(head.committed as i64).fetch_optional(&mut *tx).await.map_err(sql_error)?;
    let value = row
        .map(|row| -> Result<Block> {
            Ok(Block {
                content_id: head.id,
                logical_start: row.try_get("logical_start").map_err(storage)?,
                logical_end: row.try_get("logical_end").map_err(storage)?,
                codec: row.try_get("codec").map_err(storage)?,
                sha256: row.try_get("digest").map_err(storage)?,
                byte_len: row.try_get("bytes").map_err(storage)?,
            })
        })
        .transpose()?;
    tx.rollback().await.map_err(storage)?;
    read_transaction.complete();
    let bytes = modeled_read_bytes(&value)?;
    backend.wrap_read(lease.session, value, bytes)
}

async fn piece(
    backend: &Backend,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    head: &ContentStreamView,
    block: &Block,
    offset: i64,
) -> Result<ReadView<Vec<u8>>> {
    let _read = admission(backend)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let read_transaction = backend.live.diagnostics().read_transaction();
    check_source(&mut tx, lease, position, head).await?;
    let bytes: Vec<u8> = sqlx::query_scalar("SELECT substr(body,?,?) FROM agena_content_blocks WHERE content_id=? AND logical_start=? AND logical_end=? AND digest=?")
        .bind(offset + 1).bind(PIECE_BYTES as i64).bind(head.id.to_string()).bind(block.logical_start).bind(block.logical_end).bind(&block.sha256)
        .fetch_optional(&mut *tx).await.map_err(sql_error)?.ok_or_else(|| storage("archive content block disappeared"))?;
    if bytes.is_empty() || offset + bytes.len() as i64 > block.byte_len {
        return Err(storage("archive content block length mismatch"));
    }
    tx.rollback().await.map_err(storage)?;
    read_transaction.complete();
    let charge = bytes.capacity() + 256;
    backend.wrap_scoped(
        lease.session,
        Some(ReadSource::Content(head.id)),
        bytes,
        charge,
    )
}

struct Hex<'a>(&'a [u8]);
impl std::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl Serialize for Hex<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Serialize)]
struct Piece<'a> {
    content_id: ContentId,
    block_start: i64,
    byte_offset: i64,
    bytes_hex: Hex<'a>,
}

async fn source_rows(
    writer: &mut Writer<'_, '_>,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    head: &ContentStreamView,
    physical_start: i64,
    checkpoints: bool,
) -> Result<()> {
    let mut after = -1i64;
    loop {
        let values = {
            let _read = admission(writer.backend)?;
            let mut tx = writer
                .backend
                .reader
                .get_sqlite_connection_pool()
                .begin()
                .await
                .map_err(storage)?;
            let read_transaction = writer.backend.live.diagnostics().read_transaction();
            check_source(&mut tx, lease, position, head).await?;
            let (sql, integers, texts): (&str, &[&str], &[&str]) = if checkpoints {
                (
                    "SELECT * FROM agena_content_checkpoints WHERE content_id=? AND logical_sequence>? AND logical_sequence<=? AND block_start>=? ORDER BY logical_sequence LIMIT 200",
                    &["logical_sequence", "block_start"],
                    &["content_id"],
                )
            } else {
                (
                    "SELECT * FROM agena_content_gaps WHERE content_id=? AND logical_start>? AND logical_start<? ORDER BY logical_start LIMIT 1",
                    &["logical_start", "logical_end"],
                    &["content_id", "reason"],
                )
            };
            let query = sqlx::query(sql)
                .bind(head.id.to_string())
                .bind(after)
                .bind(head.committed as i64);
            let rows = if checkpoints {
                query.bind(physical_start).fetch_all(&mut *tx).await
            } else {
                query.fetch_all(&mut *tx).await
            }
            .map_err(sql_error)?;
            let values = rows
                .iter()
                .map(|row| raw(row, texts, integers))
                .collect::<Result<Vec<_>>>()?;
            tx.rollback().await.map_err(storage)?;
            read_transaction.complete();
            let charge = modeled_read_bytes(&values)?;
            writer.backend.wrap_read(lease.session, values, charge)?
        };
        if values.is_empty() {
            break;
        }
        for value in &*values {
            writer
                .line(
                    if checkpoints {
                        "content_checkpoint"
                    } else {
                        "content_gap"
                    },
                    value,
                    true,
                )
                .await?;
            after = value[if checkpoints {
                "logical_sequence"
            } else {
                "logical_start"
            }]
            .as_i64()
            .ok_or_else(|| storage("invalid archive content index"))?;
        }
    }
    Ok(())
}

async fn source_export(
    writer: &mut Writer<'_, '_>,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    source: ContentId,
) -> Result<()> {
    let (head, physical_start) = {
        let _read = admission(writer.backend)?;
        let mut tx = writer
            .backend
            .reader
            .get_sqlite_connection_pool()
            .begin()
            .await
            .map_err(storage)?;
        let read_transaction = writer.backend.live.diagnostics().read_transaction();
        let entry = authorized(&mut tx, lease, position).await?;
        let head = content::stored_head(&mut tx, source).await?;
        if head.entry.get() != entry {
            return Err(KernelError::Ownership);
        }
        // A logical retention floor can fall inside an immutable block. Keep
        // that boundary block intact (including its digest/codec); the logical
        // floor and gap remain authoritative when restoring or projecting it.
        let physical_start = if head.retained_start < head.committed {
            sqlx::query_scalar::<_,i64>("SELECT logical_start FROM agena_content_blocks WHERE content_id=? AND logical_start<=? AND logical_end>? ORDER BY logical_start DESC LIMIT 1")
                .bind(source.to_string()).bind(head.retained_start as i64).bind(head.retained_start as i64)
                .fetch_optional(&mut *tx).await.map_err(sql_error)?.ok_or_else(|| storage("archive retained content has no boundary block"))?
        } else {
            head.committed as i64
        };
        tx.rollback().await.map_err(storage)?;
        read_transaction.complete();
        let bytes = modeled_read_bytes(&head)?;
        (
            writer.backend.wrap_read(lease.session, head, bytes)?,
            physical_start,
        )
    };
    writer
        .line(
            "content",
            &Source {
                stream: &head,
                physical_start,
            },
            true,
        )
        .await?;
    source_rows(writer, lease, position, &head, physical_start, false).await?;
    let mut start = physical_start;
    while start < head.committed as i64 {
        let view = block(writer.backend, lease, position, &head, start).await?;
        let block = view
            .as_ref()
            .ok_or_else(|| storage("archive committed content is missing a block"))?;
        if block.logical_start != start
            || block.logical_end > head.committed as i64
            || block.logical_end <= start
        {
            return Err(storage("archive content block range is not contiguous"));
        }
        writer.line("content_block", block, true).await?;
        let mut offset = 0i64;
        let mut hash = Sha256::new();
        while offset < block.byte_len {
            let bytes = piece(writer.backend, lease, position, &head, block, offset).await?;
            hash.update(&*bytes);
            writer
                .line(
                    "content_piece",
                    &Piece {
                        content_id: source,
                        block_start: start,
                        byte_offset: offset,
                        bytes_hex: Hex(&bytes),
                    },
                    true,
                )
                .await?;
            offset += bytes.len() as i64;
        }
        if hash.finalize().as_slice() != block.sha256 {
            return Err(storage("archive content block checksum mismatch"));
        }
        start = block.logical_end;
    }
    source_rows(writer, lease, position, &head, physical_start, true).await?;
    let mut packets = 0u64;
    let mut lost_bytes = 0u64;
    while packets < head.capture_loss.committed_packets {
        let losses = {
            let _read = admission(writer.backend)?;
            let mut tx = writer
                .backend
                .reader
                .get_sqlite_connection_pool()
                .begin()
                .await
                .map_err(storage)?;
            let read_transaction = writer.backend.live.diagnostics().read_transaction();
            check_source(&mut tx, lease, position, &head).await?;
            let losses = content::loss::records(
                &mut tx,
                head.id,
                packets,
                head.capture_loss.committed_packets,
                128,
            )
            .await?;
            tx.rollback().await.map_err(storage)?;
            read_transaction.complete();
            let bytes = modeled_read_bytes(&losses)?;
            writer.backend.wrap_scoped(
                lease.session,
                Some(ReadSource::Content(head.id)),
                losses,
                bytes,
            )?
        };
        if losses.is_empty() {
            return Err(storage("archive capture loss window is missing"));
        }
        for loss in losses.iter() {
            if loss.first_packet != packets + 1
                || loss.last_packet < loss.first_packet
                || loss.after_position > head.committed
            {
                return Err(storage("archive capture loss window is not contiguous"));
            }
            lost_bytes = lost_bytes
                .checked_add(loss.bytes)
                .ok_or_else(|| storage("archive capture loss overflow"))?;
            packets = loss.last_packet;
            writer
                .line(
                    "capture_loss",
                    &json!({"content_id":head.id,"loss":loss}),
                    true,
                )
                .await?;
        }
    }
    if lost_bytes != head.capture_loss.committed_bytes {
        return Err(storage("archive capture loss byte summary mismatch"));
    }
    // Detect retention/epoch loss even after the last sink wait of this source.
    let _read = admission(writer.backend)?;
    let mut tx = writer
        .backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let read_transaction = writer.backend.live.diagnostics().read_transaction();
    check_source(&mut tx, lease, position, &head).await?;
    tx.rollback().await.map_err(storage)?;
    read_transaction.complete();
    Ok(())
}

#[derive(Serialize)]
struct Source<'a> {
    #[serde(flatten)]
    stream: &'a ContentStreamView,
    physical_start: i64,
}

struct BlobRead<'a> {
    backend: &'a Backend,
    pin: port::BlobPin,
}
impl Drop for BlobRead<'_> {
    fn drop(&mut self) {
        self.backend.release_blob(self.pin.lease);
    }
}

#[derive(Serialize)]
struct BlobPiece<'a> {
    blob_id: agena_domain::BlobId,
    byte_offset: u64,
    bytes_hex: Hex<'a>,
}

async fn blob_export(
    writer: &mut Writer<'_, '_>,
    snapshot: port::HistorySnapshotLease,
    position: ExportPosition,
    descriptor: &BlobDescriptor,
) -> Result<()> {
    let ExportPosition::Member(position) = position else {
        return support_blob_export(writer, snapshot, position, descriptor).await;
    };
    let receipt = writer
        .backend
        .commit(
            RequestId::new(),
            Command::PinSnapshotBlob {
                snapshot,
                position,
                blob: descriptor.id,
            },
        )
        .await?;
    let Mutation::BlobPin(pin) = receipt.value else {
        return Err(storage("incompatible archive blob pin"));
    };
    writer.backend.retain_blob(pin.lease)?;
    let reader = BlobRead {
        backend: writer.backend,
        pin: *pin,
    };
    if reader.pin.record.id != descriptor.id
        || reader.pin.record.byte_len != descriptor.byte_len
        || reader.pin.record.sha256 != Some(descriptor.sha256)
    {
        return Err(storage("archive blob reference changed"));
    }
    writer
        .line(
            "blob",
            &json!({"position":position,"descriptor":descriptor}),
            true,
        )
        .await?;
    let mut offset = 0u64;
    let mut hash = Sha256::new();
    loop {
        let page = writer
            .backend
            .blob_range(reader.pin.lease, offset, PIECE_BYTES)
            .await?;
        hash.update(&page.bytes);
        writer
            .line(
                "blob_piece",
                &BlobPiece {
                    blob_id: descriptor.id,
                    byte_offset: offset,
                    bytes_hex: Hex(&page.bytes),
                },
                true,
            )
            .await?;
        offset = page.next;
        if page.eof {
            break;
        }
    }
    if offset != descriptor.byte_len || <[u8; 32]>::from(hash.finalize()) != descriptor.sha256 {
        return Err(storage("archive blob checksum mismatch"));
    }
    Ok(())
}

#[derive(Serialize)]
struct EntryRecord<'a> {
    position: SnapshotCursor,
    facts: &'a Bundle,
}

#[derive(Serialize)]
struct Header<'a> {
    format: &'static str,
    version: u32,
    scope: &'static str,
    snapshot: &'a HistorySnapshotInfo,
    configuration: &'a SessionConfiguration,
    prompt_window: &'a Value,
}

pub(super) async fn export(
    backend: &Backend,
    lease: port::HistorySnapshotLease,
    info: &ReadView<HistorySnapshotInfo>,
    target: &mut Sink<'_>,
) -> Result<ExportReport> {
    backend.cache.ensure_open()?;
    let _export = backend.exports.acquire(lease.session)?;
    let root = tempfile::tempdir().map_err(storage)?;
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(root.path().join("closure.sqlite"))
        .create_if_missing(true)
        .pragma("journal_mode", "DELETE")
        .pragma("cache_size", "-2048");
    let mut graph = SqliteConnection::connect_with(&options)
        .await
        .map_err(storage)?;
    sqlx::query("CREATE TABLE entries(id INTEGER PRIMARY KEY,member INTEGER NOT NULL,position_json TEXT,emitted INTEGER NOT NULL DEFAULT 0,segment_sequence INTEGER,ordinal INTEGER)").execute(&mut graph).await.map_err(storage)?;
    sqlx::query("CREATE TABLE rounds(id INTEGER PRIMARY KEY)")
        .execute(&mut graph)
        .await
        .map_err(storage)?;
    sqlx::query(
        "CREATE INDEX pending_entries ON entries(emitted,member DESC,segment_sequence,ordinal,id)",
    )
    .execute(&mut graph)
    .await
    .map_err(storage)?;
    // Seed all fixed membership on disk before following support references.
    let mut after = None;
    loop {
        let page = backend.snapshot_page(lease, after, 128).await?;
        for entry in &page.entries {
            sqlx::query("INSERT INTO entries(id,member,position_json,segment_sequence,ordinal) VALUES (?,1,?,?,?)").bind(entry.entry.id.get()).bind(encode(&entry.position)?).bind(entry.position.segment_sequence).bind(i64::from(entry.position.ordinal)).execute(&mut graph).await.map_err(sql_error)?;
        }
        after = page.next;
        if !page.has_more {
            break;
        }
    }
    // An export owns its temporary views. Filling the public configuration
    // cache here would retain an unrelated charge after a failed/cancelled sink.
    let configuration = {
        let _read = admission(backend)?;
        let mut tx = backend
            .reader
            .get_sqlite_connection_pool()
            .begin()
            .await
            .map_err(storage)?;
        let read_transaction = backend.live.diagnostics().read_transaction();
        let value = lifecycle::configuration(&mut tx, lease.session).await?;
        tx.rollback().await.map_err(storage)?;
        read_transaction.complete();
        let bytes = modeled_read_bytes(&value)?;
        backend.wrap_read(lease.session, value, bytes)?
    };
    let prompt_window = {
        let mut tx = backend
            .reader
            .get_sqlite_connection_pool()
            .begin()
            .await
            .map_err(storage)?;
        let read_transaction = backend.live.diagnostics().read_transaction();
        let row = sqlx::query("SELECT * FROM agena_prompt_windows WHERE session_id=?")
            .bind(lease.session.get())
            .fetch_one(&mut *tx)
            .await
            .map_err(sql_error)?;
        let value = raw(
            &row,
            &["policy_key"],
            &[
                "session_id",
                "rewrite_epoch",
                "checkpoint_entry_id",
                "through_entry_id",
                "through_sequence",
                "through_ordinal",
                "consecutive_compaction_failures",
                "auto_compaction_disabled",
                "native_compaction_unavailable",
                "last_before_tokens",
                "last_after_tokens",
            ],
        )?;
        tx.rollback().await.map_err(storage)?;
        read_transaction.complete();
        value
    };
    for key in ["checkpoint_entry_id", "through_entry_id"] {
        if let Some(id) = prompt_window[key].as_i64() {
            enqueue(&mut graph, id).await?;
        }
    }
    let mut writer = Writer {
        backend,
        session: lease.session,
        target,
        graph: &mut graph,
        hash: Sha256::new(),
        records: 0,
        bytes: 0,
    };
    writer.line("header",&Header{format:"agena.session-kernel",version:1,scope:"fixed_membership_per_record_controls_committed_sources_with_dependency_closure",snapshot:info,configuration:&configuration,prompt_window:&prompt_window},true).await?;
    let mut entries = 0u64;
    // Member order is fixed by the persisted snapshot cursor. Support facts
    // are discovered from each exported record and never count as membership.
    loop {
        let row=sqlx::query("SELECT id,member,position_json FROM entries WHERE emitted=0 ORDER BY member DESC,segment_sequence,ordinal,id LIMIT 1").fetch_optional(&mut *writer.graph).await.map_err(storage)?;
        let Some(row) = row else { break };
        let id: i64 = row.try_get("id").map_err(storage)?;
        let member = row.try_get::<i64, _>("member").map_err(storage)? == 1;
        let cursor = row
            .try_get::<Option<String>, _>("position_json")
            .map_err(storage)?
            .map(|value| decode::<SnapshotCursor>(&value))
            .transpose()?;
        let position = if let Some(cursor) = cursor {
            ExportPosition::Member(cursor)
        } else {
            ExportPosition::Support(id)
        };
        let fact = bundle(backend, lease, position).await?;
        sqlx::query("UPDATE entries SET emitted=1 WHERE id=?")
            .bind(id)
            .execute(&mut *writer.graph)
            .await
            .map_err(storage)?;
        if member {
            writer
                .line(
                    "entry",
                    &EntryRecord {
                        position: cursor.ok_or_else(|| storage("member has no cursor"))?,
                        facts: &fact,
                    },
                    true,
                )
                .await?;
            entries += 1;
        } else {
            writer
                .line("support_entry", &json!({"facts":&*fact}), true)
                .await?;
        }
        if matches!(fact.entry.payload, EntryPayload::User { .. }) {
            let through = fact.execution["current_generation"]
                .as_i64()
                .ok_or_else(|| storage("invalid archive execution"))?;
            attempts(&mut writer, lease, position, through).await?;
            rounds(&mut writer, lease, position, fact.round_upper).await?;
        }
        for source in &fact.sources {
            source_export(&mut writer, lease, position, *source).await?;
        }
        for blob in &fact.blobs {
            blob_export(&mut writer, lease, position, blob).await?;
        }
    }
    if entries != info.entry_count {
        return Err(storage("archive membership count mismatch"));
    }
    backend.cache.ensure_open()?;
    let report = ExportReport {
        entries,
        records: writer.records,
        bytes: writer.bytes,
        sha256: writer.hash.clone().finalize().into(),
    };
    writer.line("footer", &report, false).await?;
    writer.target.flush().await.map_err(storage)?;
    Ok(report)
}

async fn support_blob_export(
    writer: &mut Writer<'_, '_>,
    lease: port::HistorySnapshotLease,
    position: ExportPosition,
    descriptor: &BlobDescriptor,
) -> Result<()> {
    // A support entry was reached through the verified dependency queue. Its
    // own immutable references authorize this private range reader, while a
    // missing chunk during concurrent GC aborts the archive without a footer.
    let mut offset = 0u64;
    let mut hash = Sha256::new();
    writer
        .line("blob", &json!({"descriptor":descriptor}), true)
        .await?;
    loop {
        let bytes = {
            let _read = admission(writer.backend)?;
            let mut tx = writer
                .backend
                .reader
                .get_sqlite_connection_pool()
                .begin()
                .await
                .map_err(storage)?;
            let read_transaction = writer.backend.live.diagnostics().read_transaction();
            let entry = authorized(&mut tx, lease, position).await?;
            let reference:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_blob_references r JOIN agena_blobs b ON b.id=r.blob_id WHERE r.entry_id=? AND r.blob_id=? AND b.state='ready' AND b.expected_bytes=? AND b.digest=?)").bind(entry).bind(descriptor.id.to_string()).bind(descriptor.byte_len as i64).bind(descriptor.sha256.to_vec()).fetch_one(&mut *tx).await.map_err(sql_error)?;
            if !reference {
                return Err(KernelError::Ownership);
            }
            let bytes = if offset == descriptor.byte_len {
                Vec::new()
            } else {
                let chunk_start = offset / 131072 * 131072;
                sqlx::query_scalar::<_,Vec<u8>>("SELECT substr(body,?,?) FROM agena_blob_chunks WHERE blob_id=? AND logical_start=?").bind((offset-chunk_start+1)as i64).bind(PIECE_BYTES as i64).bind(descriptor.id.to_string()).bind(chunk_start as i64).fetch_optional(&mut *tx).await.map_err(sql_error)?.ok_or_else(||storage("archive dependency blob chunk disappeared"))?
            };
            if offset < descriptor.byte_len
                && (bytes.is_empty() || offset + bytes.len() as u64 > descriptor.byte_len)
            {
                return Err(storage("archive dependency blob range is invalid"));
            }
            tx.rollback().await.map_err(storage)?;
            read_transaction.complete();
            writer
                .backend
                .wrap_read(lease.session, bytes, PIECE_BYTES + 256)?
        };
        hash.update(&*bytes);
        writer
            .line(
                "blob_piece",
                &BlobPiece {
                    blob_id: descriptor.id,
                    byte_offset: offset,
                    bytes_hex: Hex(&bytes),
                },
                true,
            )
            .await?;
        offset += bytes.len() as u64;
        if offset == descriptor.byte_len {
            break;
        }
    }
    if offset != descriptor.byte_len || <[u8; 32]>::from(hash.finalize()) != descriptor.sha256 {
        return Err(storage("archive dependency blob checksum mismatch"));
    }
    Ok(())
}
