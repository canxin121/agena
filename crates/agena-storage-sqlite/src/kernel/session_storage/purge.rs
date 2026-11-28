use sea_orm::sqlx::Sqlite;

use super::*;

const BLOCKS_PER_BATCH: i64 = 16;
const BYTES_PER_BATCH: u64 = 8 * 1024 * 1024;

/// Reclaim payload blocks in short batches. The final transaction drops the
/// session's own membership and circularly linked metadata together. A
/// tombstone remains addressable as an ancestor or canonical Bot target.
pub(super) async fn reclaim(
    mut transaction: sqlx::Transaction<'_, Sqlite>,
    session: SessionId,
    shared: bool,
) -> Result<(u64, bool)> {
    let result = reclaim_batch(&mut transaction, session, shared).await;
    match result {
        Ok(result) => {
            transaction.commit().await.map_err(sql_error)?;
            Ok(result)
        }
        Err(error) => {
            transaction.rollback().await.map_err(sql_error)?;
            Err(error)
        }
    }
}

async fn reclaim_batch(
    connection: &mut SqliteConnection,
    session: SessionId,
    shared: bool,
) -> Result<(u64, bool)> {
    let deleted: bool =
        sqlx::query_scalar("SELECT deleted_at_ms IS NOT NULL FROM agena_sessions WHERE id=?")
            .bind(session.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if !deleted {
        return Err(KernelError::Conflict(
            "only a deleted session can be reclaimed".into(),
        ));
    }
    if !shared {
        let mut removed = 0u64;
        let blocks = sqlx::query_as::<_, (i64, i64, i64)>(
            r#"
            SELECT b.source_key,b.logical_start,b.body_bytes
            FROM agena_entries e CROSS JOIN agena_content_streams c ON c.entry_id=e.id
            CROSS JOIN agena_content_blocks b ON b.source_key=c.source_key
            WHERE e.origin_session_id=? LIMIT ?
        "#,
        )
        .bind(session.get())
        .bind(BLOCKS_PER_BATCH)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        for (source_key, start, bytes) in &blocks {
            let bytes = accounting::unsigned(*bytes);
            if removed > 0 && removed + bytes > BYTES_PER_BATCH {
                break;
            }
            sqlx::query(
                "DELETE FROM agena_content_checkpoints WHERE source_key=? AND block_start=?",
            )
            .bind(source_key)
            .bind(start)
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
            sqlx::query("DELETE FROM agena_content_blocks WHERE source_key=? AND logical_start=?")
                .bind(source_key)
                .bind(start)
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
            removed += bytes;
        }
        if !blocks.is_empty() {
            return Ok((removed, true));
        }
        let chunks = sqlx::query_scalar::<_, i64>(
            r#"
            DELETE FROM agena_blob_chunks WHERE (blob_id,logical_start) IN (
                SELECT c.blob_id,c.logical_start FROM agena_blobs b
                CROSS JOIN agena_blob_chunks c ON c.blob_id=b.id WHERE b.session_id=?
                ORDER BY b.id,c.logical_start LIMIT ?
            ) RETURNING length(body)
        "#,
        )
        .bind(session.get())
        .bind(BLOCKS_PER_BATCH)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        if !chunks.is_empty() {
            return Ok((chunks.into_iter().map(accounting::unsigned).sum(), true));
        }
    }
    let before = session_state(connection, session, true, &[])
        .await?
        .summary
        .bytes
        .total();
    detach(connection, session).await?;
    if !shared {
        delete_owned_rows(connection, session).await?;
    }
    let after = session_state(connection, session, true, &[])
        .await?
        .summary
        .bytes
        .total();
    Ok((before.saturating_sub(after), false))
}

async fn detach(connection: &mut SqliteConnection, session: SessionId) -> Result<()> {
    let segments: Vec<i64> = sqlx::query_scalar(r#"
        SELECT segment_id FROM agena_session_segments WHERE session_id=?1
        UNION SELECT segment_id FROM agena_retired_session_segments WHERE session_id=?1
        UNION SELECT m.segment_id FROM agena_segment_entries m JOIN agena_entries e ON e.id=m.entry_id
            WHERE e.origin_session_id=?1
    "#).bind(session.get()).fetch_all(&mut *connection).await.map_err(sql_error)?;
    for sql in [
        "DELETE FROM agena_prompt_windows WHERE session_id=?",
        "DELETE FROM agena_branch_entries WHERE session_id=?",
        "DELETE FROM agena_branch_activities WHERE session_id=?",
        "DELETE FROM agena_branch_rounds WHERE session_id=?",
        "DELETE FROM agena_session_segments WHERE session_id=?",
        "DELETE FROM agena_retired_session_segments WHERE session_id=?",
    ] {
        sqlx::query(sql)
            .bind(session.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
    history_runs::clear(connection, session).await?;
    detach_sources(connection, session).await?;
    sqlx::query(r#"
        DELETE FROM agena_history_segments WHERE id IN (SELECT value FROM json_each(?))
          AND NOT EXISTS(SELECT 1 FROM agena_session_segments s WHERE s.segment_id=agena_history_segments.id)
          AND NOT EXISTS(SELECT 1 FROM agena_retired_session_segments s WHERE s.segment_id=agena_history_segments.id)
    "#).bind(encode(&segments)?).execute(connection).await.map_err(sql_error)?;
    Ok(())
}

async fn detach_sources(connection: &mut SqliteConnection, session: SessionId) -> Result<()> {
    // Keep canonical heads while another branch points to them. Removing
    // the last link can also release the head of an already detached branch.
    sqlx::query(
        r#"
        WITH released(session_id,source_key) AS MATERIALIZED (
            SELECT h.session_id,h.source_key
            FROM agena_branch_sources h WHERE h.session_id=?1
            AND NOT EXISTS(
                SELECT 1 FROM agena_branch_sources link
                WHERE link.head_owner_session_id=h.session_id AND link.source_key=h.source_key
            )
            UNION ALL
            SELECT owner.id,h.source_key
            FROM agena_branch_sources h
            CROSS JOIN agena_sessions owner ON owner.id=h.head_owner_session_id
            CROSS JOIN agena_content_streams c ON c.source_key=h.source_key
            WHERE h.session_id=?1 AND owner.deleted_at_ms IS NOT NULL
            AND NOT EXISTS(
                SELECT 1 FROM agena_branch_entries e WHERE e.session_id=owner.id AND e.entry_id=c.entry_id
            )
            AND NOT EXISTS(
                SELECT 1 FROM agena_branch_sources link WHERE link.head_owner_session_id=owner.id
                AND link.source_key=h.source_key AND link.session_id!=?1
            )
        )
        DELETE FROM agena_branch_sources WHERE (session_id,source_key) IN (SELECT * FROM released)
        "#,
    )
    .bind(session.get())
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

async fn delete_owned_rows(connection: &mut SqliteConnection, session: SessionId) -> Result<()> {
    // Entries, attempts and executions contain intentional circular foreign
    // keys. Keep checks enabled and defer them across the coordinated delete.
    sqlx::query("PRAGMA defer_foreign_keys=ON")
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    for sql in [
        "DELETE FROM agena_round_input_sources WHERE round_id IN (SELECT id FROM agena_rounds WHERE session_id=?)",
        // Blocks own the frozen facts. Only remove a block once all its
        // remaining links belong to the rounds being removed.
        "DELETE FROM agena_prompt_input_blocks WHERE id IN (
            SELECT DISTINCT l.block_id FROM agena_rounds r
            CROSS JOIN agena_round_input_blocks l ON l.round_id=r.id WHERE r.session_id=?1
         ) AND NOT EXISTS(
            SELECT 1 FROM agena_round_input_blocks l CROSS JOIN agena_rounds r ON r.id=l.round_id
            WHERE l.block_id=agena_prompt_input_blocks.id AND r.session_id!=?1
         )",
        "DELETE FROM agena_prompt_replay_blocks WHERE id IN (
            SELECT DISTINCT l.block_id FROM agena_rounds r
            CROSS JOIN agena_round_replay_blocks l ON l.round_id=r.id WHERE r.session_id=?1
         ) AND NOT EXISTS(
            SELECT 1 FROM agena_round_replay_blocks l CROSS JOIN agena_rounds r ON r.id=l.round_id
            WHERE l.block_id=agena_prompt_replay_blocks.id AND r.session_id!=?1
         )",
        "DELETE FROM agena_round_input_blocks WHERE round_id IN (SELECT id FROM agena_rounds WHERE session_id=?)",
        "DELETE FROM agena_round_replay_blocks WHERE round_id IN (SELECT id FROM agena_rounds WHERE session_id=?)",
        "DELETE FROM agena_compaction_rounds WHERE round_id IN (SELECT id FROM agena_rounds WHERE session_id=?)",
        "DELETE FROM agena_content_capture_losses WHERE source_key IN (
            SELECT c.source_key FROM agena_entries e CROSS JOIN agena_content_streams c ON c.entry_id=e.id
            WHERE e.origin_session_id=?
         )",
        "DELETE FROM agena_content_gaps WHERE source_key IN (
            SELECT c.source_key FROM agena_entries e CROSS JOIN agena_content_streams c ON c.entry_id=e.id
            WHERE e.origin_session_id=?
         )",
        "DELETE FROM agena_content_streams WHERE source_key IN (
            SELECT c.source_key FROM agena_entries e CROSS JOIN agena_content_streams c ON c.entry_id=e.id
            WHERE e.origin_session_id=?
         )",
        "DELETE FROM agena_blob_references WHERE entry_id IN (SELECT id FROM agena_entries WHERE origin_session_id=?)",
        "DELETE FROM agena_blob_pins WHERE blob_id IN (SELECT id FROM agena_blobs WHERE session_id=?)",
        "DELETE FROM agena_blobs WHERE session_id=?",
        "DELETE FROM agena_deliveries WHERE session_id=?",
        "DELETE FROM agena_server_state WHERE key GLOB 'activity-output/' || ? || '/*'",
        "DELETE FROM agena_background_activities WHERE session_id=?",
        "DELETE FROM agena_interactions WHERE session_id=?",
        "DELETE FROM agena_tool_calls WHERE round_id IN (SELECT id FROM agena_rounds WHERE session_id=?)",
        "DELETE FROM agena_entries WHERE origin_session_id=?",
        "DELETE FROM agena_rounds WHERE session_id=?",
        "DELETE FROM agena_channel_sessions WHERE session_id=?",
        "DELETE FROM agena_execution_purposes WHERE execution_id IN (SELECT id FROM agena_executions WHERE session_id=?)",
        "DELETE FROM agena_attempts WHERE session_id=?",
        "DELETE FROM agena_executions WHERE session_id=?",
        "DELETE FROM agena_permission_rules WHERE session_id=?",
        "DELETE FROM agena_scheduler_history WHERE owner_session_id=?",
        "DELETE FROM agena_assistant_reminders WHERE source_session_id=? AND state IN ('completed','cancelled')",
        "DELETE FROM agena_session_statistics WHERE session_id=?",
    ] {
        sqlx::query(sql)
            .bind(session.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
    let invalid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)")
        .fetch_one(connection)
        .await
        .map_err(sql_error)?;
    if invalid {
        return Err(KernelError::Conflict(
            "an existing record still refers to this history".into(),
        ));
    }
    Ok(())
}
