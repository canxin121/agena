//! Prefix retention preserves logical positions, declares the gap first, and
//! reclaims immutable blocks in bounded writer transactions. It never prunes
//! an open producer or leaves a document/terminal without a checkpoint base.

use agena_domain::ContentKind;

use super::*;

pub(in crate::kernel) async fn prune(
    connection: &mut SqliteConnection,
    session: SessionId,
    content: ContentId,
    before: u64,
) -> Result<ContentRetention> {
    reads::metadata(connection, session).await?;
    let owner: Option<i64> =
        sqlx::query_scalar("SELECT session_id FROM agena_content_streams WHERE id=?")
            .bind(content.to_string())
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
    if owner != Some(session.get()) {
        return Err(KernelError::Ownership);
    }
    let mut head = stored_head(connection, content).await?;
    let checkpoint: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_entries e JOIN agena_prompt_windows w ON w.checkpoint_entry_id=e.id WHERE e.content_id=?)")
        .bind(content.to_string()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    if checkpoint && before > head.retained_start {
        return Err(KernelError::Conflict(
            "the current prompt checkpoint pins this summary".into(),
        ));
    }
    if head.state == CaptureState::Open {
        return Err(KernelError::Conflict(
            "retention requires a sealed capture".into(),
        ));
    }
    if before > head.committed {
        return Err(invalid("retention boundary exceeds committed content"));
    }
    let mut floor = before.max(head.retained_start);
    if head.kind == ContentKind::Text && floor > head.retained_start && floor < head.committed {
        // A declared prefix gap must not leave a retained suffix starting in
        // the middle of a UTF-8 scalar. Only one indexed immutable block is
        // loaded; canonical byte positions after that scalar stay unchanged.
        let row=sqlx::query("SELECT logical_start,logical_end,codec,digest,body FROM agena_content_blocks WHERE content_id=? AND logical_start<=? ORDER BY logical_start DESC LIMIT 1")
            .bind(content.to_string()).bind(floor as i64).fetch_optional(&mut *connection).await.map_err(sql_error)?.ok_or_else(||storage("retention boundary has no text block"))?;
        if row.try_get::<&str, _>("codec").map_err(storage)? != "utf8" {
            return Err(storage("retention text codec mismatch"));
        }
        let start = u64::try_from(row.try_get::<i64, _>("logical_start").map_err(storage)?)
            .map_err(storage)?;
        let end = u64::try_from(row.try_get::<i64, _>("logical_end").map_err(storage)?)
            .map_err(storage)?;
        let digest: Vec<u8> = row.try_get("digest").map_err(storage)?;
        let block = ImmutableTextBlock::from_stored(
            start,
            end,
            row.try_get("body").map_err(storage)?,
            digest
                .try_into()
                .map_err(|_| storage("invalid text digest"))?,
        )
        .map_err(storage)?;
        let text = std::str::from_utf8(block.bytes()).map_err(storage)?;
        let mut offset = usize::try_from(floor - start).map_err(storage)?;
        if offset > text.len() {
            return Err(storage("retention boundary outside text block"));
        }
        while !text.is_char_boundary(offset) {
            offset += 1;
        }
        floor = start + offset as u64;
    }
    if matches!(
        head.kind,
        ContentKind::Structured | ContentKind::Terminal | ContentKind::Document
    ) && floor > head.retained_start
    {
        let checkpoint=sqlx::query("SELECT logical_sequence,block_start FROM agena_content_checkpoints WHERE content_id=? AND logical_sequence<=? ORDER BY logical_sequence DESC LIMIT 1")
            .bind(content.to_string()).bind(floor.saturating_add(1).min(head.committed) as i64).fetch_optional(&mut *connection).await.map_err(sql_error)?
            .ok_or_else(||storage("cannot retain a projection without a valid checkpoint"))?;
        let sequence = checkpoint
            .try_get::<i64, _>("logical_sequence")
            .map_err(storage)? as u64;
        let block = events::stored_block(
            connection,
            &head,
            checkpoint.try_get("block_start").map_err(storage)?,
        )
        .await?;
        let mut found = false;
        for record in block.iter() {
            let record = record.map_err(storage)?;
            if record.sequence == sequence {
                let kind = if head.kind == ContentKind::Structured {
                    EventKind::Structured
                } else if head.kind == ContentKind::Document {
                    EventKind::Mixed
                } else {
                    EventKind::Terminal
                };
                if !record.payload.is_checkpoint() || record.payload.kind() != kind {
                    return Err(storage("retention checkpoint index points to a patch"));
                }
                EventProjection::from_checkpoint(&record)?;
                found = true;
                break;
            }
        }
        if !found {
            return Err(storage("retention checkpoint is outside its indexed block"));
        }
        floor = (sequence - 1).max(head.retained_start);
    }
    if floor > head.retained_start {
        sqlx::query("UPDATE agena_content_streams SET retained_start=? WHERE id=?")
            .bind(floor as i64)
            .bind(content.to_string())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
        sqlx::query("INSERT INTO agena_content_gaps (content_id,logical_start,logical_end,reason) VALUES (?,0,?,'retention') ON CONFLICT(content_id,logical_start) DO UPDATE SET logical_end=MAX(logical_end,excluded.logical_end),reason=excluded.reason")
            .bind(content.to_string()).bind(floor as i64).execute(&mut *connection).await.map_err(sql_error)?;
        sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE id=?")
            .bind(head.entry.get())
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
        head.retained_start = floor;
    }
    let rows=sqlx::query("SELECT logical_start,length(body) AS bytes FROM agena_content_blocks WHERE content_id=? AND logical_start<? AND logical_end<=? ORDER BY logical_start LIMIT 8")
        .bind(content.to_string()).bind(floor as i64).bind(floor as i64).fetch_all(&mut *connection).await.map_err(sql_error)?;
    let mut starts = Vec::new();
    let mut bytes = 0;
    for row in rows {
        let size = row.try_get::<i64, _>("bytes").map_err(storage)? as usize;
        // Preserve the existing allowance for one large atomic terminal block.
        if !starts.is_empty() && bytes + size > MAX_CHECKPOINT_BYTES {
            break;
        }
        starts.push(row.try_get::<i64, _>("logical_start").map_err(storage)?);
        bytes += size;
    }
    if !starts.is_empty() {
        let keys = encode(&starts)?;
        sqlx::query("DELETE FROM agena_content_checkpoints WHERE content_id=? AND block_start IN (SELECT value FROM json_each(?))")
            .bind(content.to_string()).bind(&keys).execute(&mut *connection).await.map_err(sql_error)?;
        sqlx::query("DELETE FROM agena_content_blocks WHERE content_id=? AND logical_start IN (SELECT value FROM json_each(?))")
            .bind(content.to_string()).bind(keys).execute(&mut *connection).await.map_err(sql_error)?;
    }
    let pending_reclamation=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_content_blocks WHERE content_id=? AND logical_start<? AND logical_end<=?)")
        .bind(content.to_string()).bind(floor as i64).bind(floor as i64).fetch_one(connection).await.map_err(sql_error)?;
    Ok(ContentRetention {
        stream: head,
        blocks_removed: starts.len() as u32,
        pending_reclamation,
    })
}
