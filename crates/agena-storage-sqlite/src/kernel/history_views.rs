//! Message paging selects identity/size rows before loading any entry body.
use super::*;
use futures_util::TryStreamExt;
use sea_orm::sqlx::{SqliteConnection, SqlitePool};
use std::{collections::HashSet, sync::LazyLock};

const MEMBERSHIP: &str = "FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal JOIN agena_entries e ON e.id=m.entry_id WHERE s.session_id=? AND e.kind!='interaction'";
const REQUESTED: &str = "WITH requested(id) AS (SELECT DISTINCT value FROM json_each(?1))";

const ENTRY_MEMBERSHIP: &str = "CROSS JOIN agena_segment_entries m INDEXED BY idx_agena_segment_entry_reverse ON m.entry_id=picked.id
     CROSS JOIN agena_session_segments s INDEXED BY idx_agena_session_segment_reverse ON s.segment_id=m.segment_id AND s.session_id=?2
        AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal";
// A user or an unanchored part is a single-entry run. Anchored parts use
// the segment index and visit only segments that contain part of that reply.
static MATCHED_PAGE_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT picked.id,picked.body_bytes,s.sequence,m.ordinal
         FROM agena_entries picked {ENTRY_MEMBERSHIP}
         LEFT JOIN agena_branch_entries f ON f.session_id=?2 AND f.entry_id=picked.id AND picked.kind!='user'
         LEFT JOIN agena_attempts a ON f.entry_id IS NULL AND picked.kind!='user'
            AND a.execution_id=picked.execution_id AND a.generation=picked.generation
         WHERE picked.id=?1 AND picked.kind!='interaction' AND (s.sequence,m.ordinal)<(?3,?4)
           AND CASE WHEN f.entry_id IS NULL THEN a.response_entry_id ELSE f.response_entry_id END IS NULL
         UNION ALL
         SELECT * FROM (
             SELECT e.id,e.body_bytes,g.sequence,m.ordinal
             FROM agena_session_response_segments g
             CROSS JOIN agena_session_segments s ON s.session_id=g.session_id AND s.sequence=g.sequence
             CROSS JOIN agena_segment_entries m ON m.segment_id=s.segment_id
                 AND m.ordinal>=s.first_ordinal
                 AND m.ordinal<CASE WHEN g.sequence=?3 THEN MIN(s.end_ordinal,?4) ELSE s.end_ordinal END
             CROSS JOIN agena_entries e ON e.id=m.entry_id
             CROSS JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=e.generation
             LEFT JOIN agena_branch_entries f ON f.session_id=g.session_id AND f.entry_id=e.id
             WHERE g.session_id=?2 AND g.response_entry_id=?1 AND g.sequence<=?3
               AND e.kind NOT IN ('user','interaction') AND a.response_entry_id=g.response_entry_id
               AND (f.entry_id IS NULL OR f.response_entry_id=g.response_entry_id)
             ORDER BY g.sequence DESC,m.ordinal DESC LIMIT ?5
         )
         ORDER BY sequence DESC,ordinal DESC LIMIT ?5"
    )
});
static HISTORY_PAGE_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT e.id,e.body_bytes,s.sequence,m.ordinal {MEMBERSHIP}
         AND (s.sequence,m.ordinal)<(?,?) ORDER BY s.sequence DESC,m.ordinal DESC LIMIT ?"
    )
});
static SELECTED_ENTRIES_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{REQUESTED}
         SELECT {},s.sequence,m.ordinal
         FROM requested picked {ENTRY_MEMBERSHIP} CROSS JOIN agena_entries e ON e.id=picked.id",
        reads::ENTRY_COLUMNS
    )
});

pub(super) async fn execution_text_sources(
    pool: &SqlitePool,
    session: SessionId,
    execution: ExecutionId,
    generation: Option<AttemptGeneration>,
    diagnostics: &KernelDiagnostics,
) -> Result<Vec<ContentId>> {
    let mut tx = pool.begin().await.map_err(storage)?;
    reads::metadata(&mut tx, session).await?;
    let mut query = sqlx::QueryBuilder::new(
        "SELECT e.content_id
         FROM agena_entries e INDEXED BY idx_agena_entry_attempt
         CROSS JOIN agena_segment_entries m INDEXED BY idx_agena_segment_entry_reverse ON m.entry_id=e.id
         CROSS JOIN agena_session_segments s INDEXED BY idx_agena_session_segment_reverse ON s.segment_id=m.segment_id
         WHERE e.kind='text' AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal AND s.session_id=",
    );
    query
        .push_bind(session.get())
        .push(" AND e.execution_id=")
        .push_bind(execution.to_string());
    if let Some(generation) = generation {
        query.push(" AND e.generation=").push_bind(generation.get());
    }
    query.push(" ORDER BY s.sequence DESC,m.ordinal DESC LIMIT 4096");
    metrics::add(&diagnostics.0.history_queries, 2);
    let mut rows = query.build_query_scalar::<String>().fetch(&mut *tx);
    let mut sources = Vec::new();
    while let Some(id) = rows.try_next().await.map_err(sql_error)? {
        sources.push(id.parse().map_err(storage)?);
    }
    metrics::add(&diagnostics.0.history_candidate_rows, sources.len());
    Ok(sources)
}

type Candidate = (i64, i64, i64, i64);

async fn history_candidates(
    connection: &mut SqliteConnection,
    session: SessionId,
    before: Option<HistoryCursor>,
    requested: &[i64],
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<Vec<Candidate>> {
    let sequence = before.map_or(i64::MAX, |cursor| cursor.segment_sequence);
    let ordinal = before.map_or(i64::MAX, |cursor| i64::from(cursor.ordinal));
    let count = (limit + 1) as i64;
    if requested.is_empty() {
        metrics::add(&diagnostics.0.history_queries, 1);
        return sqlx::query_as(&HISTORY_PAGE_SQL)
            .bind(session.get())
            .bind(sequence)
            .bind(ordinal)
            .bind(count)
            .fetch_all(connection)
            .await
            .map_err(sql_error);
    }
    let mut seen = HashSet::with_capacity(requested.len());
    let mut rows = Vec::new();
    for &id in requested.iter().filter(|&&id| seen.insert(id)) {
        metrics::add(&diagnostics.0.history_queries, 1);
        let matches: Vec<Candidate> = sqlx::query_as(&MATCHED_PAGE_SQL)
            .bind(id)
            .bind(session.get())
            .bind(sequence)
            .bind(ordinal)
            .bind(count)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        rows.extend(matches);
    }
    rows.sort_unstable_by_key(|&(_, _, sequence, ordinal)| std::cmp::Reverse((sequence, ordinal)));
    rows.truncate(limit + 1);
    Ok(rows)
}

pub(super) async fn validate(
    connection: &mut SqliteConnection,
    session: SessionId,
    metadata: &SessionMetadata,
    before: Option<HistoryCursor>,
) -> Result<()> {
    let Some(cursor) = before else {
        return Ok(());
    };
    if cursor.session != session {
        return Err(KernelError::Conflict(
            "history membership changed; restart pagination".into(),
        ));
    }
    if cursor.membership == metadata.membership_generation {
        return Ok(());
    }
    // Appending history copies the current tail into a new segment while
    // preserving every existing (sequence, ordinal) position. A cursor from a
    // previous generation is therefore still usable until its anchor entry was
    // actually removed or moved by a rewind/withdraw/fork.
    if cursor.membership > metadata.membership_generation {
        return Err(KernelError::Conflict(
            "history membership changed; restart pagination".into(),
        ));
    }
    let Some(anchor) = cursor.anchor else {
        return Err(KernelError::Conflict(
            "history membership changed; restart pagination".into(),
        ));
    };
    let retained: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM agena_session_segments s JOIN agena_segment_entries m ON m.segment_id=s.segment_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal WHERE s.session_id=? AND s.sequence=? AND m.ordinal=? AND m.entry_id=? LIMIT 1",
    )
    .bind(session.get())
    .bind(cursor.segment_sequence)
    .bind(i64::from(cursor.ordinal))
    .bind(anchor.get())
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    if retained.is_none() {
        return Err(KernelError::Conflict(
            "history membership changed; restart pagination".into(),
        ));
    }
    Ok(())
}
fn cursor(
    session: SessionId,
    metadata: &SessionMetadata,
    &(id, _, sequence, ordinal): &Candidate,
) -> Result<HistoryCursor> {
    Ok(HistoryCursor {
        session,
        membership: metadata.membership_generation,
        segment_sequence: sequence,
        ordinal: u32::try_from(ordinal).map_err(storage)?,
        anchor: Some(EntryId::try_from(id).map_err(storage)?),
    })
}
async fn selected_entries(
    connection: &mut SqliteConnection,
    session: SessionId,
    selected: &[i64],
    diagnostics: &KernelDiagnostics,
) -> Result<Vec<((i64, i64), EntryView)>> {
    if selected.is_empty() {
        return Ok(Vec::new());
    }
    metrics::add(&diagnostics.0.history_queries, 1);
    let mut rows = sqlx::query(&SELECTED_ENTRIES_SQL)
        .bind(encode(selected)?)
        .bind(session.get())
        .fetch(connection);
    let mut entries = Vec::with_capacity(selected.len());
    while let Some(row) = rows.try_next().await.map_err(sql_error)? {
        metrics::add(
            &diagnostics.0.history_entry_body_bytes,
            row.try_get::<Option<&str>, _>("body_json")
                .map_err(storage)?
                .map_or(0, str::len),
        );
        let position = (
            row.try_get("sequence").map_err(storage)?,
            row.try_get("ordinal").map_err(storage)?,
        );
        entries.push((position, reads::entry_row(&row)?));
    }
    metrics::add(&diagnostics.0.history_entry_rows, entries.len());
    Ok(entries)
}

async fn bodies(
    connection: &mut SqliteConnection,
    session: SessionId,
    selected: &[i64],
    diagnostics: &KernelDiagnostics,
) -> Result<Vec<EntryView>> {
    let mut positioned = selected_entries(connection, session, selected, diagnostics).await?;
    positioned.sort_unstable_by_key(|(position, _)| *position);
    let mut entries: Vec<_> = positioned
        .into_iter()
        .map(|(_, mut entry)| {
            if entry.response_entry_id.is_none() {
                entry.response_entry_id = Some(entry.id);
            }
            entry
        })
        .collect();
    branches::apply_entries(connection, session, &mut entries).await?;
    for entry in &mut entries {
        if matches!(entry.payload, EntryPayload::User { .. }) {
            entry.response_entry_id = None;
        }
    }
    Ok(entries)
}

pub(super) async fn transcript_page(
    backend: &Backend,
    session: SessionId,
    position: TranscriptPosition,
    limit: usize,
) -> Result<ReadView<TranscriptPage>> {
    backend.cache.ensure_open()?;
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
    reads::metadata(&mut tx, session).await?;
    let (anchor, comparison, order) = match position {
        TranscriptPosition::Latest => (None, "<", "DESC"),
        TranscriptPosition::Before(entry) => (Some(entry), "<", "DESC"),
        TranscriptPosition::AtOrBefore(entry) => (Some(entry), "<=", "DESC"),
        TranscriptPosition::After(entry) => (Some(entry), ">", "ASC"),
    };
    let (sequence, ordinal) = match anchor {
        Some(entry) => lifecycle::boundary(&mut tx, session, entry).await?,
        None => (i64::MAX, i64::MAX),
    };
    let limit = limit.clamp(1, 64);
    let query = format!(
        "SELECT e.id,e.body_bytes {MEMBERSHIP}
         AND e.kind IN ('user','text') AND NOT EXISTS (
             SELECT 1 FROM agena_execution_purposes p
             WHERE p.execution_id=e.execution_id AND p.purpose='compaction'
         )
         AND (s.sequence,m.ordinal){comparison}(?,?)
         ORDER BY s.sequence {order},m.ordinal {order} LIMIT ?"
    );
    let rows: Vec<(i64, i64)> = sqlx::query_as(&query)
        .bind(session.get())
        .bind(sequence)
        .bind(ordinal)
        .bind((limit + 1) as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_error)?;
    let diagnostics = backend.live.diagnostics();
    metrics::add(&diagnostics.0.history_queries, 1);
    metrics::add(&diagnostics.0.history_candidate_rows, rows.len());
    let mut ids = Vec::with_capacity(limit);
    let mut bytes = 0usize;
    for &(id, body_bytes) in &rows {
        let charge = usize::try_from(body_bytes)
            .map_err(storage)?
            .saturating_add(512);
        if ids.len() == limit || (!ids.is_empty() && bytes.saturating_add(charge) > 1024 * 1024) {
            break;
        }
        bytes = bytes.saturating_add(charge);
        ids.push(id);
    }
    let value = TranscriptPage {
        has_more: rows.len() > ids.len(),
        entries: bodies(&mut tx, session, &ids, diagnostics).await?,
    };
    let bytes = modeled_read_bytes(&value)?;
    backend.wrap_read(session, value, bytes)
}

pub(super) async fn history_by_ids(
    pool: &SqlitePool,
    session: SessionId,
    ids: &[i64],
    diagnostics: &KernelDiagnostics,
) -> Result<HistoryPage> {
    if ids.len() > 256 || ids.iter().any(|id| *id <= 0) {
        return Err(invalid("invalid entry selection"));
    }
    let mut tx = pool.begin().await.map_err(storage)?;
    let metadata = reads::metadata(&mut tx, session).await?;
    let mut entries: Vec<_> = selected_entries(&mut tx, session, ids, diagnostics)
        .await?
        .into_iter()
        .map(|(_, entry)| entry)
        .collect();
    entries.sort_unstable_by_key(|entry| entry.id);
    branches::apply_entries(&mut tx, session, &mut entries).await?;
    Ok(HistoryPage {
        metadata,
        entries,
        next: None,
        has_more: false,
    })
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
    validate(&mut tx, session, &metadata, before).await?;
    let rows =
        history_candidates(&mut tx, session, before, message_ids, limit, diagnostics).await?;
    metrics::add(&diagnostics.0.history_queries, 1);
    metrics::add(&diagnostics.0.history_candidate_rows, rows.len());
    let mut selected = Vec::with_capacity(limit);
    let mut bytes = 0usize;
    let mut next = None;
    let mut has_more = false;
    for row @ &(id, body_bytes, ..) in &rows {
        let charge = usize::try_from(body_bytes)
            .map_err(storage)?
            .saturating_add(512);
        if selected.len() == limit || bytes.saturating_add(charge) > 1024 * 1024 {
            has_more = true;
            break;
        }
        bytes += charge;
        selected.push(id);
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
    validate(&mut tx, session, &metadata, before).await?;
    // No content body participates in boundary selection, counting or sorting.
    // Conversation order follows the owning input, not append time: users can
    // enqueue their next inputs before the current reply's first entry exists.
    // Keep the physical membership cursor so validation and part expansion
    // retain their existing semantics, but recover its logical position here.
    let before_key = if let Some(cursor) = before {
        let sql = format!(
            "SELECT COALESCE(e.message_id,e.id) AS owner,e.id {MEMBERSHIP} AND s.sequence=? AND m.ordinal=?"
        );
        let row = sqlx::query(&sql)
            .bind(session.get())
            .bind(cursor.segment_sequence)
            .bind(i64::from(cursor.ordinal))
            .fetch_one(&mut *tx)
            .await
            .map_err(sql_error)?;
        metrics::add(&diagnostics.0.history_queries, 1);
        (
            row.try_get::<i64, _>("owner").map_err(storage)?,
            row.try_get::<i64, _>("id").map_err(storage)?,
        )
    } else {
        (i64::MAX, i64::MAX)
    };
    let mut boundaries = sqlx::query(
        "SELECT r.entry_id AS id,r.is_user,e.body_bytes FROM agena_session_runs r
         CROSS JOIN agena_entries e INDEXED BY idx_agena_entry_preflight ON e.id=r.entry_id
         WHERE r.session_id=? AND (r.owner_entry_id,r.entry_id)<(?,?)
         ORDER BY r.owner_entry_id DESC,r.entry_id DESC LIMIT ?",
    )
    .bind(session.get())
    .bind(before_key.0)
    .bind(before_key.1)
    .bind((message_limit + 1) as i64)
    .fetch_all(&mut *tx)
    .await
    .map_err(sql_error)?;
    metrics::add(&diagnostics.0.history_queries, 2);
    metrics::add(&diagnostics.0.history_candidate_rows, boundaries.len());
    let has_more = boundaries.len() > message_limit;
    boundaries.truncate(message_limit);
    let next = if has_more && let Some(row) = boundaries.last() {
        // The projection contains only current members. Resolve physical
        // membership once, for the cursor that leaves this page.
        let anchor =
            EntryId::try_from(row.try_get::<i64, _>("id").map_err(storage)?).map_err(storage)?;
        let (segment_sequence, ordinal) = lifecycle::boundary(&mut tx, session, anchor).await?;
        metrics::add(&diagnostics.0.history_queries, 1);
        Some(HistoryCursor {
            session,
            membership: metadata.membership_generation,
            segment_sequence,
            ordinal: u32::try_from(ordinal).map_err(storage)?,
            anchor: Some(anchor),
        })
    } else {
        None
    };
    let mut selected = Vec::<i64>::new();
    let mut seen = HashSet::new();
    let mut messages = Vec::with_capacity(boundaries.len());
    let mut bytes = 0usize;
    for boundary in boundaries.iter().rev() {
        let id: i64 = boundary.try_get("id").map_err(storage)?;
        let boundary_bytes = boundary.try_get("body_bytes").map_err(storage)?;
        if boundary.try_get::<bool, _>("is_user").map_err(storage)? {
            charge_body(boundary_bytes, &mut bytes)?;
            selected.push(id);
            seen.insert(id);
            messages.push(MessageHistorySummary {
                id: EntryId::try_from(id).map_err(storage)?,
                entry_count: 1,
                loaded_count: 1,
                next: None,
            });
            continue;
        }
        let rows =
            history_candidates(&mut tx, session, None, &[id], entry_limit, diagnostics).await?;
        let total: i64 = if rows.len() <= entry_limit {
            rows.len() as i64
        } else {
            metrics::add(&diagnostics.0.history_queries, 1);
            sqlx::query_scalar(
                "SELECT part_total FROM agena_session_response_segments
                 WHERE session_id=? AND response_entry_id=? ORDER BY sequence DESC LIMIT 1",
            )
            .bind(session.get())
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(sql_error)?
        };
        metrics::add(&diagnostics.0.history_candidate_rows, rows.len());
        let mut loaded = 0u64;
        let mut child_next = None;
        for row @ &(entry, body_bytes, ..) in rows.iter().take(entry_limit) {
            charge_body(body_bytes, &mut bytes)?;
            if seen.insert(entry) {
                selected.push(entry);
                loaded += 1;
            }
            child_next = Some(cursor(session, &metadata, row)?);
        }
        // Always return the true boundary, even if the loaded suffix is far
        // newer. Clients do not invent a message while expanding a long reply.
        if seen.insert(id) {
            selected.push(id);
            charge_body(boundary_bytes, &mut bytes)?;
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
    let user_message_count: i64 = sqlx::query_scalar(
        "SELECT started_user_message_count FROM agena_session_statistics WHERE session_id=?",
    )
    .bind(session.get())
    .fetch_one(&mut *tx)
    .await
    .map_err(sql_error)?;
    metrics::add(&diagnostics.0.history_queries, 1);
    Ok(MessageHistoryPage {
        metadata,
        entries,
        messages,
        user_message_count: user_message_count as u64,
        next,
        has_more,
    })
}

fn charge_body(body_bytes: i64, bytes: &mut usize) -> Result<()> {
    let size = usize::try_from(body_bytes).map_err(storage)?;
    *bytes = bytes.saturating_add(size).saturating_add(512);
    if *bytes > 1024 * 1024 {
        return Err(KernelError::Busy);
    }
    Ok(())
}
