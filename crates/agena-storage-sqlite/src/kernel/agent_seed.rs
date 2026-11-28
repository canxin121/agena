//! The parent's completed turns become immutable history in a delegated session.

use super::*;

// User entries can be admitted while an earlier turn is still writing its
// response. Select by execution ownership before grouping adjacent ordinals
// back into shared segment slices. A turn can finish without assistant text.
const SHARE_COMPLETED_TURNS_SQL: &str = "
    WITH visible AS MATERIALIZED (
        SELECT e.id,x.message_id,e.kind,e.created_at_ms,s.sequence,s.segment_id,m.ordinal,
               e.kind='compaction' AND e.round_id IS NULL AS context_cleared,
               CASE WHEN f.entry_id IS NOT NULL THEN
                        f.execution_started=1 AND f.execution_phase NOT IN ('queued','running')
                    WHEN e.kind='user' THEN EXISTS(
                        SELECT 1 FROM agena_attempts done WHERE done.execution_id=e.execution_id
                        AND done.started_at_ms IS NOT NULL AND done.finished_at_ms IS NOT NULL
                    )
                    ELSE a.started_at_ms IS NOT NULL AND a.finished_at_ms IS NOT NULL
               END AS completed
        FROM agena_session_segments s
        CROSS JOIN agena_segment_entries m ON m.segment_id=s.segment_id
            AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
        CROSS JOIN agena_entries e ON e.id=m.entry_id
        CROSS JOIN agena_executions x ON x.id=e.execution_id
        CROSS JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=e.generation
        LEFT JOIN agena_branch_entries f ON f.session_id=s.session_id AND f.entry_id=e.id
        WHERE s.session_id=?1
    ),
    retained AS (
        SELECT *,ordinal-row_number() OVER (PARTITION BY sequence ORDER BY ordinal) AS slice
        FROM visible WHERE completed AND message_id IN (
            SELECT id FROM visible WHERE id=message_id AND completed
            AND id>COALESCE((SELECT MAX(id) FROM visible WHERE context_cleared),0)
        )
    ),
    ranges AS (
        SELECT sequence,segment_id,MIN(ordinal) AS first_ordinal,MAX(ordinal)+1 AS end_ordinal,
               COUNT(*) FILTER (WHERE kind='user') AS users,MAX(created_at_ms) AS latest
        FROM retained GROUP BY sequence,segment_id,slice
    )
    INSERT INTO agena_session_segments (
        session_id,sequence,segment_id,first_ordinal,end_ordinal,user_message_count,max_entry_created_at_ms
    )
    SELECT ?2,row_number() OVER (ORDER BY sequence,first_ordinal)-1,
           segment_id,first_ordinal,end_ordinal,users,latest
    FROM ranges";

pub(super) async fn capture(
    connection: &mut SqliteConnection,
    parent: SessionId,
    child: SessionId,
) -> Result<()> {
    sqlx::query(SHARE_COMPLETED_TURNS_SQL)
        .bind(parent.get())
        .bind(child.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    branches::capture(connection, parent, child, true).await?;
    history_runs::rebuild_imported(connection, child).await?;
    lifecycle::recount(connection, child).await?;
    // A summary is usable only if its own entry and its coverage boundary
    // survived the selection. Find the boundary in the child's slices.
    sqlx::query(
        "UPDATE agena_prompt_windows SET (checkpoint_entry_id,through_entry_id,through_sequence,through_ordinal)=(
             SELECT w.checkpoint_entry_id,w.through_entry_id,s.sequence,m.ordinal
             FROM agena_prompt_windows w
             CROSS JOIN agena_segment_entries m INDEXED BY idx_agena_segment_entry_reverse
                 ON m.entry_id=w.through_entry_id
             CROSS JOIN agena_session_segments s INDEXED BY idx_agena_session_segment_reverse
                 ON s.segment_id=m.segment_id AND s.session_id=?2
             WHERE w.session_id=?1 AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
               AND EXISTS(SELECT 1 FROM agena_branch_entries f
                          WHERE f.session_id=?2 AND f.entry_id=w.checkpoint_entry_id)
         ) WHERE session_id=?2",
    )
    .bind(parent.get())
    .bind(child.get())
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}
