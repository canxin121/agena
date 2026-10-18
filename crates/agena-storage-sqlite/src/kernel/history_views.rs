//! Message paging selects identity/size rows before loading any entry body.
use super::*;
use sea_orm::sqlx::{SqliteConnection, SqlitePool, sqlite::SqliteRow};

// Anchors are real transcript identities. The generation predicate makes
// historical responses keep their identity after retry/continue.
const ANCHOR: &str = "CASE WHEN e.kind='user' THEN e.id ELSE COALESCE((SELECT a.response_entry_id FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=e.generation),e.id) END";
const MEMBERSHIP: &str = "FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal JOIN agena_entries e ON e.id=m.entry_id WHERE s.session_id=? AND e.kind!='interaction'";
fn validate(metadata: &SessionMetadata, before: Option<HistoryCursor>) -> Result<()> {
    if before.is_some_and(|cursor| {
        cursor.session != metadata.id || cursor.membership != metadata.membership_generation
    }) {
        return Err(KernelError::Conflict(
            "history membership changed; restart pagination".into(),
        ));
    }
    Ok(())
}
fn cursor(
    session: SessionId,
    metadata: &SessionMetadata,
    row: &SqliteRow,
) -> Result<HistoryCursor> {
    Ok(HistoryCursor {
        session,
        membership: metadata.membership_generation,
        segment_sequence: row.try_get("sequence").map_err(storage)?,
        ordinal: u32::try_from(row.try_get::<i64, _>("ordinal").map_err(storage)?)
            .map_err(storage)?,
    })
}
async fn bodies(
    connection: &mut SqliteConnection,
    session: SessionId,
    selected: &[i64],
    diagnostics: &KernelDiagnostics,
) -> Result<Vec<EntryView>> {
    if selected.is_empty() {
        return Ok(Vec::new());
    }
    metrics::add(&diagnostics.0.history_queries, 1);
    let sql = format!(
        "SELECT e.*,({ANCHOR}) AS response_entry_id,(SELECT a.phase FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=CASE WHEN e.kind='user' THEN (SELECT x.current_generation FROM agena_executions x WHERE x.id=e.execution_id) ELSE e.generation END) AS execution_phase {MEMBERSHIP} AND e.id IN (SELECT value FROM json_each(?)) ORDER BY s.sequence,m.ordinal"
    );
    let rows = sqlx::query(&sql)
        .bind(session.get())
        .bind(encode(&selected)?)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    metrics::add(&diagnostics.0.history_entry_rows, rows.len());
    rows.iter()
        .map(|row| {
            metrics::add(
                &diagnostics.0.history_entry_body_bytes,
                row.try_get::<Option<&str>, _>("body_json")
                    .map_err(storage)?
                    .map_or(0, str::len),
            );
            let mut entry = reads::entry_row(row)?;
            if matches!(entry.payload, EntryPayload::User { .. }) {
                entry.response_entry_id = None;
            }
            Ok(entry)
        })
        .collect()
}

pub(super) async fn history_window(
    pool: &SqlitePool,
    session: SessionId,
    before: Option<HistoryCursor>,
    message_ids: &[i64],
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<HistoryPage> {
    if message_ids.len() > 32 || message_ids.iter().any(|id| *id <= 0) {
        return Err(invalid("invalid message selection"));
    }
    let limit = limit.clamp(1, 256);
    let mut tx = pool.begin().await.map_err(storage)?;
    let metadata = reads::metadata(&mut tx, session).await?;
    validate(&metadata, before)?;
    let selection = if message_ids.is_empty() {
        String::new()
    } else {
        format!("AND ({ANCHOR}) IN (SELECT value FROM json_each(?))")
    };
    let sql = format!(
        "SELECT e.id,COALESCE(e.body_bytes,0) AS body_bytes,s.sequence,m.ordinal {MEMBERSHIP} {selection} AND (s.sequence,m.ordinal)<(?,?) ORDER BY s.sequence DESC,m.ordinal DESC LIMIT ?"
    );
    let mut query = sqlx::query(&sql).bind(session.get());
    if !message_ids.is_empty() {
        query = query.bind(encode(&message_ids)?);
    }
    let rows = query
        .bind(before.map_or(i64::MAX, |cursor| cursor.segment_sequence))
        .bind(before.map_or(i64::MAX, |cursor| i64::from(cursor.ordinal)))
        .bind((limit + 1) as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_error)?;
    metrics::add(&diagnostics.0.history_queries, 2);
    metrics::add(&diagnostics.0.history_candidate_rows, rows.len());
    let mut selected = Vec::with_capacity(limit);
    let mut bytes = 0usize;
    let mut next = None;
    let mut has_more = false;
    for row in &rows {
        let charge = usize::try_from(row.try_get::<i64, _>("body_bytes").map_err(storage)?)
            .map_err(storage)?
            .saturating_add(512);
        if selected.len() == limit || bytes.saturating_add(charge) > 1024 * 1024 {
            has_more = true;
            break;
        }
        bytes += charge;
        selected.push(row.try_get("id").map_err(storage)?);
        next = Some(cursor(session, &metadata, row)?);
    }
    if selected.is_empty() && !rows.is_empty() {
        return Err(KernelError::Busy);
    }
    let entries = bodies(&mut tx, session, &selected, diagnostics).await?;
    Ok(HistoryPage {
        metadata,
        entries,
        next: has_more.then_some(next).flatten(),
        has_more,
    })
}

pub(super) async fn message_history(
    pool: &SqlitePool,
    session: SessionId,
    before: Option<HistoryCursor>,
    message_limit: usize,
    entry_limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<MessageHistoryPage> {
    let message_limit = message_limit.clamp(1, 32);
    let entry_limit = entry_limit.clamp(1, 128);
    let mut tx = pool.begin().await.map_err(storage)?;
    let metadata = reads::metadata(&mut tx, session).await?;
    validate(&metadata, before)?;
    // A boundary index lives in the entry/attempt identities; no content body
    // participates in message selection, counting or sorting.
    let sql = format!(
        "SELECT e.id,s.sequence,m.ordinal {MEMBERSHIP} AND e.id=({ANCHOR}) AND e.kind!='interaction' AND (s.sequence,m.ordinal)<(?,?) ORDER BY s.sequence DESC,m.ordinal DESC LIMIT ?"
    );
    let mut boundaries = sqlx::query(&sql)
        .bind(session.get())
        .bind(before.map_or(i64::MAX, |cursor| cursor.segment_sequence))
        .bind(before.map_or(i64::MAX, |cursor| i64::from(cursor.ordinal)))
        .bind((message_limit + 1) as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_error)?;
    metrics::add(&diagnostics.0.history_queries, 2);
    metrics::add(&diagnostics.0.history_candidate_rows, boundaries.len());
    let has_more = boundaries.len() > message_limit;
    boundaries.truncate(message_limit);
    let next = if has_more {
        boundaries
            .last()
            .map(|row| cursor(session, &metadata, row))
            .transpose()?
    } else {
        None
    };
    let mut selected = Vec::<i64>::new();
    let mut messages = Vec::with_capacity(boundaries.len());
    let mut bytes = 0usize;
    for boundary in boundaries.iter().rev() {
        let id: i64 = boundary.try_get("id").map_err(storage)?;
        let sql = format!(
            "SELECT e.id,COALESCE(e.body_bytes,0) AS body_bytes,s.sequence,m.ordinal {MEMBERSHIP} AND ({ANCHOR})=? ORDER BY s.sequence DESC,m.ordinal DESC LIMIT ?"
        );
        let rows = sqlx::query(&sql)
            .bind(session.get())
            .bind(id)
            .bind((entry_limit + 1) as i64)
            .fetch_all(&mut *tx)
            .await
            .map_err(sql_error)?;
        let count_sql = format!("SELECT COUNT(*) {MEMBERSHIP} AND ({ANCHOR})=?");
        let total: i64 = sqlx::query_scalar(&count_sql)
            .bind(session.get())
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(sql_error)?;
        metrics::add(&diagnostics.0.history_queries, 2);
        metrics::add(&diagnostics.0.history_candidate_rows, rows.len());
        let mut loaded = 0u64;
        let mut child_next = None;
        for row in rows.iter().take(entry_limit) {
            let charge = usize::try_from(row.try_get::<i64, _>("body_bytes").map_err(storage)?)
                .map_err(storage)?
                .saturating_add(512);
            if bytes.saturating_add(charge) > 1024 * 1024 {
                return Err(KernelError::Busy);
            }
            bytes += charge;
            let entry: i64 = row.try_get("id").map_err(storage)?;
            if !selected.contains(&entry) {
                selected.push(entry);
                loaded += 1;
            }
            child_next = Some(cursor(session, &metadata, row)?);
        }
        // Always return the true boundary, even if the loaded suffix is far
        // newer. Clients do not invent a message while expanding a long reply.
        if !selected.contains(&id) {
            selected.push(id);
            // The boundary is a small first text/tool entry, or a bounded user
            // document. Preflight it separately before loading the body.
            let size: i64 =
                sqlx::query_scalar("SELECT COALESCE(body_bytes,0) FROM agena_entries WHERE id=?")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(sql_error)?;
            bytes = bytes.saturating_add(size as usize + 512);
            if bytes > 1024 * 1024 {
                return Err(KernelError::Busy);
            }
            loaded += 1;
        }
        messages.push(MessageHistorySummary {
            id: EntryId::try_from(id).map_err(storage)?,
            entry_count: total as u64,
            loaded_count: loaded,
            next: if loaded < (total as u64) {
                child_next
            } else {
                None
            },
        });
    }
    let entries = bodies(&mut tx, session, &selected, diagnostics).await?;
    Ok(MessageHistoryPage {
        metadata,
        entries,
        messages,
        next,
        has_more,
    })
}
