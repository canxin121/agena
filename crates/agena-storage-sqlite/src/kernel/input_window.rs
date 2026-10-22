//! Current model membership is selected in the same savepoint that freezes a
//! round's input facts. Only a successful compaction can change the boundary.

use agena_storage::backend::kernel::Scope;

use super::*;

const MAX_SCANNED: usize = 10_000;
pub(super) const CANDIDATES_SQL: &str = "SELECT m.ordinal,e.id,e.kind,COALESCE(e.message_id,e.id) AS message_id,a.phase AS execution_phase,EXISTS(SELECT 1 FROM agena_attempts prior WHERE prior.execution_id=e.execution_id AND prior.started_at_ms IS NOT NULL) AS ever_started,r.phase AS round_phase,c.round_id AS compaction_round FROM agena_segment_entries m JOIN agena_entries e ON e.id=m.entry_id LEFT JOIN agena_rounds r ON r.id=e.round_id LEFT JOIN agena_compaction_rounds c ON c.round_id=e.round_id LEFT JOIN agena_executions x ON x.id=e.execution_id LEFT JOIN agena_attempts a ON a.execution_id=x.id AND a.generation=x.current_generation WHERE m.segment_id=? AND m.ordinal>=? AND m.ordinal<? ORDER BY m.ordinal LIMIT ?";

pub(super) struct Selection {
    pub entries: Vec<EntryId>,
    pub checkpoint: Option<i64>,
    pub rewrite_epoch: i64,
}

pub(super) async fn select(
    connection: &mut SqliteConnection,
    session: SessionId,
    message: EntryId,
    limit: usize,
    prefix: bool,
) -> Result<Selection> {
    let state = sqlx::query("SELECT * FROM agena_prompt_windows WHERE session_id=?")
        .bind(session.get())
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("prompt window".into()))?;
    let checkpoint: Option<i64> = state.try_get("checkpoint_entry_id").map_err(storage)?;
    let through_sequence: Option<i64> = state.try_get("through_sequence").map_err(storage)?;
    let through_ordinal: Option<i64> = state.try_get("through_ordinal").map_err(storage)?;
    let mut result = Selection {
        entries: Vec::new(),
        checkpoint,
        rewrite_epoch: state.try_get("rewrite_epoch").map_err(storage)?,
    };
    if let Some(checkpoint) = checkpoint {
        result
            .entries
            .push(EntryId::try_from(checkpoint).map_err(storage)?);
    }
    let mut after = through_sequence.unwrap_or(0);
    let mut scanned = 0usize;
    let mut edges = 0usize;
    loop {
        let segments = sqlx::query("SELECT sequence,segment_id,first_ordinal,end_ordinal FROM agena_session_segments WHERE session_id=? AND sequence>=? ORDER BY sequence LIMIT 64")
            .bind(session.get()).bind(after).fetch_all(&mut *connection).await.map_err(sql_error)?;
        if segments.is_empty() {
            break;
        }
        for segment in segments {
            edges += 1;
            if edges > MAX_SCANNED {
                return Err(invalid("prompt membership edge budget exceeded"));
            }
            let sequence: i64 = segment.try_get("sequence").map_err(storage)?;
            let mut first: i64 = segment.try_get("first_ordinal").map_err(storage)?;
            if Some(sequence) == through_sequence {
                first = first.max(through_ordinal.unwrap_or(-1) + 1);
            }
            let rows = sqlx::query(CANDIDATES_SQL)
                .bind(segment.try_get::<i64, _>("segment_id").map_err(storage)?)
                .bind(first)
                .bind(segment.try_get::<i64, _>("end_ordinal").map_err(storage)?)
                .bind((MAX_SCANNED + 1 - scanned) as i64)
                .fetch_all(&mut *connection)
                .await
                .map_err(sql_error)?;
            for row in rows {
                scanned += 1;
                if scanned > MAX_SCANNED {
                    return Err(invalid(
                        "current prompt scan exceeds 10000 entries; compact an earlier prefix",
                    ));
                }
                let kind: &str = row.try_get("kind").map_err(storage)?;
                if row.try_get::<i64, _>("message_id").map_err(storage)? > message.get() {
                    // An ordinary follow-up must not see later admitted input.
                    // A compaction cannot jump across it either: doing so would
                    // advance the checkpoint past a still-queued message.
                    if prefix {
                        return Ok(result);
                    }
                    continue;
                }
                if kind == "user"
                    && row
                        .try_get::<Option<&str>, _>("execution_phase")
                        .map_err(storage)?
                        == Some("cancelled")
                    && !row.try_get::<bool, _>("ever_started").map_err(storage)?
                {
                    continue;
                }
                let phase: Option<&str> = row.try_get("round_phase").map_err(storage)?;
                let compaction: Option<i64> = row.try_get("compaction_round").map_err(storage)?;
                if kind == "compaction"
                    || (kind != "notice"
                        && (compaction.is_some()
                            || matches!(phase, Some("failed" | "interrupted"))))
                {
                    continue;
                }
                if result.entries.len() == limit {
                    return Err(invalid(
                        "current prompt exceeds input identity budget; compact a prefix",
                    ));
                }
                result.entries.push(
                    EntryId::try_from(row.try_get::<i64, _>("id").map_err(storage)?)
                        .map_err(storage)?,
                );
                if prefix && result.entries.len() == limit {
                    return Ok(result);
                }
            }
            after = sequence
                .checked_add(1)
                .ok_or_else(|| storage("prompt sequence overflow"))?;
        }
    }
    Ok(result)
}

pub(super) async fn commit(
    connection: &mut SqliteConnection,
    scope: Scope,
    round: RoundId,
    summary: ContentId,
    through: EntryId,
    provider_state: Option<&serde_json::Value>,
) -> Result<EntryId> {
    let execution = commands::check_scope(connection, scope, true).await?;
    let row = sqlx::query("SELECT c.base_checkpoint_entry_id,c.rewrite_epoch,w.checkpoint_entry_id,w.rewrite_epoch AS current_rewrite_epoch,w.through_sequence,w.through_ordinal FROM agena_compaction_rounds c JOIN agena_rounds r ON r.id=c.round_id JOIN agena_prompt_windows w ON w.session_id=r.session_id WHERE r.id=? AND r.execution_id=? AND r.generation=? AND r.session_id=? AND r.phase='running'")
        .bind(round.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(scope.session.get())
        .fetch_optional(&mut *connection).await.map_err(sql_error)?
        .ok_or_else(|| KernelError::Conflict("compaction round is not running in this attempt".into()))?;
    if row
        .try_get::<Option<i64>, _>("base_checkpoint_entry_id")
        .map_err(storage)?
        != row
            .try_get::<Option<i64>, _>("checkpoint_entry_id")
            .map_err(storage)?
        || row.try_get::<i64, _>("rewrite_epoch").map_err(storage)?
            != row
                .try_get::<i64, _>("current_rewrite_epoch")
                .map_err(storage)?
    {
        return Err(KernelError::Conflict(
            "compaction input boundary has been superseded".into(),
        ));
    }
    let position = sqlx::query("SELECT s.sequence,m.ordinal FROM agena_round_inputs input JOIN agena_entries e ON e.id=input.entry_id JOIN agena_segment_entries m ON m.entry_id=input.entry_id JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE input.round_id=? AND input.entry_id=? AND e.kind!='compaction' AND s.session_id=? AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal")
        .bind(round.get()).bind(through.get()).bind(scope.session.get()).fetch_optional(&mut *connection).await.map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("compaction input boundary".into()))?;
    let sequence: i64 = position.try_get("sequence").map_err(storage)?;
    let ordinal: i64 = position.try_get("ordinal").map_err(storage)?;
    if let Some(prior_sequence) = row
        .try_get::<Option<i64>, _>("through_sequence")
        .map_err(storage)?
    {
        let prior_ordinal = row
            .try_get::<Option<i64>, _>("through_ordinal")
            .map_err(storage)?
            .ok_or_else(|| storage("prompt checkpoint has no position"))?;
        if (sequence, ordinal) <= (prior_sequence, prior_ordinal) {
            return Err(invalid("compaction must advance its input boundary"));
        }
    }
    // execute() has checkpointed/sealed this round's pending output in this
    // same savepoint. The summary is immutable before the boundary can move.
    let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_content_streams c JOIN agena_entries e ON e.id=c.entry_id WHERE c.id=? AND c.session_id=? AND c.execution_id=? AND c.generation=? AND c.kind='text' AND c.capture_state='complete' AND c.committed_head>0 AND c.retained_start=0 AND e.kind='text' AND e.round_id=?)")
        .bind(summary.to_string()).bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(round.get())
        .fetch_one(&mut *connection).await.map_err(sql_error)?;
    if !owned {
        return Err(KernelError::Ownership);
    }
    let now = now_ms();
    sqlx::query("UPDATE agena_rounds SET phase='succeeded',provider_state_json=?,finished_at_ms=? WHERE id=?")
        .bind(provider_state.map(encode).transpose()?).bind(now).bind(round.get()).execute(&mut *connection).await.map_err(sql_error)?;
    // A committed summary has successfully observed its frozen prefix. Those
    // notices are no longer direct inputs to subsequent ordinary rounds.
    super::stop::observe_round(connection, scope, round).await?;
    let payload = EntryPayload::Compaction { summary, through };
    let entry: i64 = sqlx::query_scalar("INSERT INTO agena_entries (origin_session_id,execution_id,generation,message_id,kind,body_json,round_id,content_id,created_at_ms) VALUES (?,?,?,?,'compaction',?,?,?,?) RETURNING id")
        .bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(execution.message_id.get())
        .bind(encode(&payload)?).bind(round.get()).bind(summary.to_string()).bind(now).fetch_one(&mut *connection).await.map_err(sql_error)?;
    commands::append_history(connection, scope.session, &[entry]).await?;
    sqlx::query("UPDATE agena_prompt_windows SET checkpoint_entry_id=?,through_entry_id=?,through_sequence=?,through_ordinal=? WHERE session_id=?")
        .bind(entry).bind(through.get()).bind(sequence).bind(ordinal).bind(scope.session.get()).execute(connection).await.map_err(sql_error)?;
    EntryId::try_from(entry).map_err(storage)
}
