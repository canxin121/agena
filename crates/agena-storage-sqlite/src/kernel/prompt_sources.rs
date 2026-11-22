//! Round manifests store the frozen boundary. The stream owns the static
//! identity and shape, and the caller reconstructs the wire head when read.

use std::sync::LazyLock;

use super::*;
use content::heads;

const SOURCE_CANDIDATES: &str = "FROM agena_round_inputs input
    CROSS JOIN agena_rounds request ON request.id=input.round_id
    CROSS JOIN agena_entries e ON e.id=input.entry_id
    CROSS JOIN agena_content_streams c ON c.entry_id=input.entry_id OR (e.kind='compaction' AND c.id=e.content_id)
    LEFT JOIN agena_branch_entries inherited ON inherited.session_id=request.session_id AND inherited.entry_id=c.entry_id
    LEFT JOIN agena_branch_sources scope ON scope.session_id=request.session_id AND scope.source_key=c.source_key
    LEFT JOIN agena_branch_sources h ON h.session_id=COALESCE(scope.head_owner_session_id,scope.session_id)
       AND h.source_key=scope.source_key
    WHERE input.round_id=?1 AND (inherited.entry_id IS NULL OR h.source_key IS NOT NULL)
    LIMIT 10001";

static SOURCE_ESTIMATE_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT count(*),COALESCE(sum(bytes),0) FROM (
             SELECT CASE WHEN h.source_key IS NOT NULL THEN h.head_bytes+256
                         ELSE 1024+6*c.capture_error_bytes END AS bytes {SOURCE_CANDIDATES}
         )"
    )
});

static CAPTURE_SOURCES_SQL: LazyLock<String> = LazyLock::new(|| {
    let columns = heads::COLUMNS;
    let values = &*heads::CAPTURE_VALUES;
    format!(
        "INSERT INTO agena_round_input_sources(round_id,block_id,entry_id,source_key,{columns})
         SELECT ?1,input.block_id,input.entry_id,c.source_key,{values}
         {SOURCE_CANDIDATES} ON CONFLICT(round_id,source_key) DO NOTHING"
    )
});

// The source rows are frozen at round creation or archive publication.
// Measure their encoded bytes once in the same transaction as the insert.
pub(super) const SOURCE_BUDGET_ASSIGNMENT: &str = "
    SET (input_source_count,input_source_bytes)=(
        SELECT count(*),COALESCE(sum(head_bytes+256),0)
        FROM agena_round_input_sources WHERE round_id=r.id
    )";

static WRITE_SOURCE_BUDGET_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "UPDATE agena_rounds AS r {SOURCE_BUDGET_ASSIGNMENT}
         WHERE r.id=? RETURNING input_source_count,input_source_bytes"
    )
});

const SOURCE_HEADS: &str = "FROM agena_round_input_sources h
    CROSS JOIN agena_content_streams c ON c.source_key=h.source_key";

pub(super) static READ_SQL: LazyLock<String> = LazyLock::new(|| {
    let columns = heads::READ_COLUMNS;
    format!("SELECT {columns} {SOURCE_HEADS} WHERE h.round_id=? ORDER BY h.source_key LIMIT 10000")
});

// Measure the page before materializing its headers. Retain the first source
// when its header fills a page on its own so the export cursor can advance.
pub(super) async fn export_page_end(
    connection: &mut SqliteConnection,
    round: i64,
    after: i64,
) -> Result<i64> {
    let candidates: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT source_key,head_bytes FROM agena_round_input_sources
         WHERE round_id=? AND source_key>? ORDER BY source_key LIMIT 64",
    )
    .bind(round)
    .bind(after)
    .fetch_all(connection)
    .await
    .map_err(sql_error)?;
    let mut bytes = 0i64;
    let mut end = after;
    for (source, head_bytes) in candidates {
        bytes = bytes.saturating_add(head_bytes).saturating_add(256);
        if end != after && bytes > 256 * 1024 {
            break;
        }
        end = source;
    }
    Ok(end)
}

pub(super) static EXPORT_SQL: LazyLock<String> = LazyLock::new(|| {
    let head_json = &*heads::WIRE_JSON;
    format!(
        "SELECT h.round_id,h.entry_id,h.source_key,c.id AS content_id,{head_json} AS head_json,
                c.entry_id AS source_entry_id,
                c.committed_head>=h.committed_head AND
                  CASE WHEN c.epoch=h.head_epoch THEN 1 ELSE EXISTS(
                      SELECT 1 FROM agena_branch_source_heads b
                      WHERE b.session_id=?4 AND b.source_key=h.source_key
                        AND b.head_epoch=h.head_epoch AND b.committed_head>=h.committed_head
                  ) END AS boundary_matches
         {SOURCE_HEADS} WHERE h.round_id=?1 AND h.source_key>?2 AND h.source_key<=?3
         ORDER BY h.source_key"
    )
});

pub(super) async fn capture(
    connection: &mut SqliteConnection,
    round: i64,
    input_bytes: usize,
) -> Result<()> {
    let (sources, source_bytes): (i64, i64) = sqlx::query_as(&SOURCE_ESTIMATE_SQL)
        .bind(round)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
    let source_bytes = usize::try_from(source_bytes).map_err(storage)?;
    if sources > prompt::MAX_INPUTS as i64
        || input_bytes.saturating_add(source_bytes) > prompt::MAX_INPUT_BYTES
    {
        return Err(invalid("provider input source/byte budget exceeded"));
    }
    if sources == 0 {
        return Ok(());
    }
    sqlx::query(&CAPTURE_SOURCES_SQL)
        .bind(round)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    let (count, bytes): (i64, i64) = sqlx::query_as(&WRITE_SOURCE_BUDGET_SQL)
        .bind(round)
        .fetch_one(connection)
        .await
        .map_err(sql_error)?;
    prompt::check_source_budget(input_bytes, count, bytes)?;
    Ok(())
}
