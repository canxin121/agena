//! Tool-specific history reads never walk or deserialize unrelated transcript.
use sea_orm::sqlx::SqlitePool;

use super::*;

const MAX_PAGE_BYTES: usize = 1024 * 1024;
pub(super) const PREFLIGHT_SQL: &str = "SELECT e.id,e.body_bytes,t.input_bytes,t.result_bytes FROM agena_entries e INDEXED BY idx_agena_owned_tool_entries JOIN agena_tool_calls t INDEXED BY idx_agena_tool_bytes ON t.id=e.tool_id WHERE e.origin_session_id=? AND e.kind='tool' AND e.id>? AND e.id<=? AND EXISTS(SELECT 1 FROM agena_segment_entries m INDEXED BY idx_agena_segment_entry_reverse JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE s.session_id=e.origin_session_id AND m.entry_id=e.id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal) ORDER BY e.id LIMIT ?";

pub(super) async fn read(
    pool: &SqlitePool,
    session: SessionId,
    after: Option<OwnedToolCursor>,
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<OwnedToolPage> {
    if after.is_some_and(|cursor| cursor.session != session) {
        return Err(KernelError::Ownership);
    }
    let limit = limit.clamp(1, 128);
    let mut tx = pool.begin().await.map_err(storage)?;
    metrics::add(&diagnostics.0.owned_tool_queries, 2);
    let metadata = reads::metadata(&mut tx, session).await?;
    let control_version: i64 =
        sqlx::query_scalar("SELECT control_version FROM agena_sessions WHERE id=?")
            .bind(session.get())
            .fetch_one(&mut *tx)
            .await
            .map_err(sql_error)?;
    if let Some(cursor) = after {
        if cursor.membership > metadata.membership_generation {
            return Err(KernelError::Conflict(
                "tool history changed; restart pagination".into(),
            ));
        }
        if cursor.membership < metadata.membership_generation {
            // Appends only add positions. Continue from the previous page as
            // long as the last returned tool is still in the live membership.
            let retained: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM agena_entries e WHERE e.id=? AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE s.session_id=? AND m.entry_id=e.id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal) LIMIT 1",
            )
            .bind(cursor.after.get())
            .bind(session.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(sql_error)?;
            if retained.is_none() {
                return Err(KernelError::Conflict(
                    "tool history changed; restart pagination".into(),
                ));
            }
        }
    }
    // Pin the first page's upper identity. A busy session can keep appending
    // tools while this bounded read completes; its next read includes them.
    let through = match after.and_then(|cursor| cursor.through) {
        Some(through) => Some(through),
        None => {
            metrics::add(&diagnostics.0.owned_tool_queries, 1);
            sqlx::query_scalar::<_, i64>(
            "SELECT id FROM agena_entries INDEXED BY idx_agena_owned_tool_entries WHERE origin_session_id=? AND kind='tool' ORDER BY id DESC LIMIT 1",
        )
        .bind(session.get())
        .fetch_optional(&mut *tx)
        .await
        .map_err(sql_error)?
        .map(EntryId::try_from)
        .transpose()
        .map_err(storage)?
        }
    };
    metrics::add(&diagnostics.0.owned_tool_queries, 1);
    let candidates = sqlx::query(PREFLIGHT_SQL)
        .bind(session.get())
        .bind(after.map_or(0, |cursor| cursor.after.get()))
        .bind(through.map_or(0, EntryId::get))
        .bind((limit + 1) as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_error)?;
    metrics::add(&diagnostics.0.owned_tool_candidate_rows, candidates.len());
    let mut selected = Vec::with_capacity(limit);
    let mut admitted = 0usize;
    let mut has_more = false;
    for row in candidates {
        let mut charge = 1024usize;
        for column in ["body_bytes", "input_bytes", "result_bytes"] {
            charge = charge.saturating_add(
                usize::try_from(row.try_get::<i64, _>(column).map_err(storage)?)
                    .map_err(storage)?,
            );
        }
        if selected.len() == limit || admitted.saturating_add(charge) > MAX_PAGE_BYTES {
            if selected.is_empty() {
                return Err(KernelError::Invalid(
                    "tool fact exceeds the ordinary tool-page read budget".into(),
                ));
            }
            has_more = true;
            break;
        }
        admitted += charge;
        selected.push(row.try_get::<i64, _>("id").map_err(storage)?);
    }
    metrics::add(&diagnostics.0.owned_tool_admitted_bytes, admitted);
    let mut entries = Vec::with_capacity(selected.len());
    if !selected.is_empty() {
        metrics::add(&diagnostics.0.owned_tool_queries, 1);
        let rows = sqlx::query("SELECT e.*,(SELECT a.response_entry_id FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=e.generation) AS response_entry_id,(SELECT a.phase FROM agena_attempts a WHERE a.execution_id=e.execution_id AND a.generation=e.generation) AS execution_phase FROM agena_entries e WHERE e.id IN (SELECT value FROM json_each(?)) ORDER BY e.id")
            .bind(encode(&selected)?).fetch_all(&mut *tx).await.map_err(sql_error)?;
        metrics::add(&diagnostics.0.owned_tool_entry_rows, rows.len());
        for row in rows {
            metrics::add(
                &diagnostics.0.owned_tool_entry_body_bytes,
                row.try_get::<Option<&str>, _>("body_json")
                    .map_err(storage)?
                    .map_or(0, str::len),
            );
            entries.push(reads::entry_row(&row)?);
        }
    }
    let next = if has_more {
        Some(OwnedToolCursor {
            session,
            membership: metadata.membership_generation,
            control_version,
            after: entries.last().expect("nonempty limited tool page").id,
            through,
        })
    } else {
        None
    };
    Ok(OwnedToolPage {
        metadata,
        control_version,
        entries,
        next,
        has_more,
    })
}
