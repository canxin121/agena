//! Indexed run boundaries for the current visible history.
//! Entries, attempts and branch facts supply the projection. Membership
//! edits update it in the same transaction as the transcript.

use std::sync::LazyLock;

use super::*;

const SUFFIX_MEMBERS: &str = "FROM agena_session_segments s
    CROSS JOIN agena_segment_entries m ON m.segment_id=s.segment_id
        AND m.ordinal>=CASE WHEN s.sequence=?2 THEN MAX(s.first_ordinal,?3) ELSE s.first_ordinal END
        AND m.ordinal<s.end_ordinal";
const SUFFIX: &str = "s.session_id=?1 AND s.sequence>=?2";
static REMOVE_STARTED_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "UPDATE agena_session_statistics SET started_user_message_count=started_user_message_count-(
             SELECT count(*) {SUFFIX_MEMBERS}
             CROSS JOIN agena_session_runs r ON r.session_id=s.session_id
                 AND r.owner_entry_id=m.entry_id AND r.entry_id=m.entry_id
             WHERE {SUFFIX} AND r.is_user=1
         ) WHERE session_id=?1"
    )
});
static REMOVE_RUNS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "DELETE FROM agena_session_runs WHERE (session_id,owner_entry_id,entry_id) IN (
             SELECT s.session_id,COALESCE(e.message_id,e.id),e.id {SUFFIX_MEMBERS}
             CROSS JOIN agena_entries e ON e.id=m.entry_id
             WHERE {SUFFIX}
         )"
    )
});

const RESPONSE_MEMBERS: &str = "FROM agena_session_segments s
    CROSS JOIN agena_segment_entries m ON m.segment_id=s.segment_id
        AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
    CROSS JOIN agena_entries e ON e.id=m.entry_id
    CROSS JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=e.generation
    LEFT JOIN agena_branch_entries f ON f.session_id=s.session_id AND f.entry_id=e.id
    WHERE s.session_id=?1 AND a.response_entry_id IS NOT NULL
      AND e.kind NOT IN ('user','interaction')
      AND (f.entry_id IS NULL OR f.response_entry_id=a.response_entry_id)";

static REBUILD_RESPONSE_SEGMENT_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "INSERT INTO agena_session_response_segments(session_id,response_entry_id,sequence,part_total)
         SELECT ?1,a.response_entry_id,?2,count(*)+COALESCE((
             SELECT previous.part_total FROM agena_session_response_segments previous
             WHERE previous.session_id=?1 AND previous.response_entry_id=a.response_entry_id
                AND previous.sequence<?2 ORDER BY previous.sequence DESC LIMIT 1
         ),0)
         {RESPONSE_MEMBERS} AND s.sequence=?2
         GROUP BY a.response_entry_id"
    )
});
static REBUILD_IMPORTED_RESPONSES_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "INSERT INTO agena_session_response_segments(session_id,response_entry_id,sequence,part_total)
         SELECT ?1,a.response_entry_id,s.sequence,
                sum(count(*)) OVER (PARTITION BY a.response_entry_id ORDER BY s.sequence)
         {RESPONSE_MEMBERS} GROUP BY a.response_entry_id,s.sequence"
    )
});

pub(super) async fn append(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: i64,
    sequence: i64,
) -> Result<()> {
    let anchored: Option<(String, i64)> = sqlx::query_as(
        "UPDATE agena_attempts AS a SET response_entry_id=e.id
         FROM agena_entries e
         WHERE e.id=? AND e.kind IN ('text','reasoning','tool')
           AND a.execution_id=e.execution_id AND a.generation=e.generation AND a.response_entry_id IS NULL
         RETURNING execution_id,generation",
    )
    .bind(entry)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    if let Some((execution, generation)) = anchored {
        // Notices written before the first response now belong to that run.
        // Frozen branch rows belong to a different session and keep their cuts.
        sqlx::query(
            "DELETE FROM agena_session_runs WHERE (session_id,owner_entry_id,entry_id) IN (
                 SELECT ?,COALESCE(message_id,id),id FROM agena_entries
                 WHERE execution_id=? AND generation=? AND id!=? AND kind NOT IN ('user','interaction')
             )",
        )
        .bind(session.get())
        .bind(&execution)
        .bind(generation)
        .bind(entry)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
        sqlx::query(
            "INSERT INTO agena_session_response_segments(session_id,response_entry_id,sequence,part_total)
             SELECT ?1,?2,s.sequence,sum(count(*)) OVER (ORDER BY s.sequence)
             FROM agena_entries e INDEXED BY idx_agena_entry_attempt
             CROSS JOIN agena_segment_entries m INDEXED BY idx_agena_segment_entry_reverse ON m.entry_id=e.id
             CROSS JOIN agena_session_segments s INDEXED BY idx_agena_session_segment_reverse
                 ON s.segment_id=m.segment_id AND s.session_id=?1
             WHERE e.execution_id=?3 AND e.generation=?4 AND e.kind NOT IN ('user','interaction')
               AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
             GROUP BY s.sequence",
        )
        .bind(session.get())
        .bind(entry)
        .bind(execution)
        .bind(generation)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    } else {
        // The last total is the exact response size, even when other replies
        // occupy intervening segments.
        sqlx::query(
            "INSERT INTO agena_session_response_segments(session_id,response_entry_id,sequence,part_total)
             SELECT ?1,a.response_entry_id,?2,1+COALESCE((
                 SELECT previous.part_total FROM agena_session_response_segments previous
                 WHERE previous.session_id=?1 AND previous.response_entry_id=a.response_entry_id
                 ORDER BY previous.sequence DESC LIMIT 1
             ),0)
             FROM agena_entries e
             CROSS JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=e.generation
             WHERE e.id=?3 AND e.kind NOT IN ('user','interaction') AND a.response_entry_id IS NOT NULL
             ON CONFLICT(session_id,response_entry_id,sequence) DO UPDATE SET part_total=excluded.part_total",
        )
        .bind(session.get())
        .bind(sequence)
        .bind(entry)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    }
    sqlx::query(
        "INSERT INTO agena_session_runs(session_id,owner_entry_id,entry_id,is_user)
         SELECT ?,COALESCE(e.message_id,e.id),e.id,0
         FROM agena_entries e
         CROSS JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=e.generation
         WHERE e.id=? AND e.kind NOT IN ('user','interaction')
           AND (a.response_entry_id IS NULL OR e.id=a.response_entry_id)",
    )
    .bind(session.get())
    .bind(entry)
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

pub(super) async fn start_message(
    connection: &mut SqliteConnection,
    session: SessionId,
    entry: EntryId,
) -> Result<i64> {
    let inserted = sqlx::query(
        "INSERT INTO agena_session_runs(session_id,owner_entry_id,entry_id,is_user)
         SELECT ?1,e.id,e.id,1 FROM agena_entries e
         WHERE e.id=?2 AND e.kind='user' AND EXISTS(
             SELECT 1 FROM agena_segment_entries m INDEXED BY idx_agena_segment_entry_reverse
             CROSS JOIN agena_session_segments s INDEXED BY idx_agena_session_segment_reverse
                 ON s.segment_id=m.segment_id AND s.session_id=?1
             WHERE m.entry_id=e.id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
         ) ON CONFLICT DO NOTHING",
    )
    .bind(session.get())
    .bind(entry.get())
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(i64::from(inserted.rows_affected() != 0))
}

pub(super) async fn copy_visible(
    connection: &mut SqliteConnection,
    source: SessionId,
    target: SessionId,
    cut_sequence: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO agena_session_runs(session_id,owner_entry_id,entry_id,is_user)
         SELECT s.session_id,r.owner_entry_id,r.entry_id,r.is_user
         FROM agena_session_segments s
         CROSS JOIN agena_segment_entries m ON m.segment_id=s.segment_id
             AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
         CROSS JOIN agena_entries e ON e.id=m.entry_id
         CROSS JOIN agena_session_runs r ON r.session_id=?1
             AND r.owner_entry_id=COALESCE(e.message_id,e.id) AND r.entry_id=e.id
         WHERE s.session_id=?2",
    )
    .bind(source.get())
    .bind(target.get())
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    sqlx::query(
        "INSERT INTO agena_session_response_segments(session_id,response_entry_id,sequence,part_total)
         SELECT ?2,response_entry_id,sequence,part_total FROM agena_session_response_segments
         WHERE session_id=?1 AND sequence<?3",
    )
    .bind(source.get())
    .bind(target.get())
    .bind(cut_sequence)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    rebuild_response_segment(connection, target, cut_sequence).await?;
    recount(connection, target).await
}

pub(super) async fn remove_suffix(
    connection: &mut SqliteConnection,
    session: SessionId,
    sequence: i64,
    ordinal: i64,
) -> Result<()> {
    // Use the original membership to find removed anchors. Physical history
    // order can interleave responses with newer queued messages.
    for sql in [&*REMOVE_STARTED_SQL, &*REMOVE_RUNS_SQL] {
        sqlx::query(sql)
            .bind(session.get())
            .bind(sequence)
            .bind(ordinal)
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
    sqlx::query("DELETE FROM agena_session_response_segments WHERE session_id=? AND sequence>=?")
        .bind(session.get())
        .bind(sequence)
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}

pub(super) async fn rebuild_response_segment(
    connection: &mut SqliteConnection,
    session: SessionId,
    sequence: i64,
) -> Result<()> {
    sqlx::query(&REBUILD_RESPONSE_SEGMENT_SQL)
        .bind(session.get())
        .bind(sequence)
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}

pub(super) async fn clear(connection: &mut SqliteConnection, session: SessionId) -> Result<()> {
    for sql in [
        "DELETE FROM agena_session_runs WHERE session_id=?",
        "DELETE FROM agena_session_response_segments WHERE session_id=?",
    ] {
        sqlx::query(sql)
            .bind(session.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
    recount(connection, session).await
}

pub(super) async fn rebuild_imported(
    connection: &mut SqliteConnection,
    session: SessionId,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO agena_session_runs(session_id,owner_entry_id,entry_id,is_user)
         SELECT s.session_id,COALESCE(e.message_id,e.id),e.id,e.kind='user'
         FROM agena_session_segments s
         CROSS JOIN agena_segment_entries m ON m.segment_id=s.segment_id
           AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
         CROSS JOIN agena_entries e ON e.id=m.entry_id
         CROSS JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=e.generation
         LEFT JOIN agena_branch_entries f ON f.session_id=s.session_id AND f.entry_id=e.id
         WHERE s.session_id=? AND e.kind!='interaction'
           AND CASE WHEN e.kind='user' THEN
               CASE WHEN f.entry_id IS NULL THEN EXISTS(
                   SELECT 1 FROM agena_attempts started
                   WHERE started.execution_id=e.execution_id AND started.started_at_ms IS NOT NULL
               ) ELSE f.execution_started=1 END
           ELSE e.id=COALESCE(CASE WHEN f.entry_id IS NULL THEN a.response_entry_id
                                ELSE f.response_entry_id END,e.id)
           END",
    )
    .bind(session.get())
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    sqlx::query(&REBUILD_IMPORTED_RESPONSES_SQL)
        .bind(session.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    recount(connection, session).await
}

async fn recount(connection: &mut SqliteConnection, session: SessionId) -> Result<()> {
    sqlx::query(
        "UPDATE agena_session_statistics SET started_user_message_count=(
             SELECT COUNT(*) FROM agena_session_runs WHERE session_id=?1 AND is_user=1
         ) WHERE session_id=?1",
    )
    .bind(session.get())
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}
