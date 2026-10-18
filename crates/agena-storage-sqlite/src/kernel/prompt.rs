//! Bounded, frozen provider input facts. The round captures only identities,
//! small control decisions and committed heads. Large payloads remain shared
//! immutable facts; the read preflights encoded byte lengths before decoding.

use sea_orm::sqlx::{SqlitePool, sqlite::SqliteRow};

use super::*;

const MAX_INPUTS: usize = 10_000;
pub(super) const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub(super) const INPUT_PREFLIGHT_SQL: &str = "SELECT e.body_bytes+CASE WHEN input.tool_phase='preparing' THEN 0 ELSE COALESCE(t.input_bytes,0) END+CASE WHEN input.tool_phase NOT IN ('preparing','running') THEN COALESCE(t.result_bytes,0) ELSE 0 END+COALESCE(i.request_bytes,0)+CASE WHEN input.interaction_decided=1 THEN COALESCE(i.decision_bytes,0)+COALESCE(i.remember_bytes,0) ELSE 0 END+512 AS bytes FROM agena_round_inputs input JOIN agena_entries e INDEXED BY idx_agena_entry_prompt ON e.id=input.entry_id LEFT JOIN agena_tool_calls t INDEXED BY idx_agena_tool_bytes ON t.id=e.tool_id AND input.tool_phase IS NOT NULL LEFT JOIN agena_interactions i INDEXED BY idx_agena_interaction_bytes ON i.id=e.interaction_id WHERE input.round_id=? ORDER BY input.ordinal LIMIT 10001";
const REPLAY_PREFLIGHT_SQL: &str = "SELECT CASE WHEN input.has_provider_state=1 THEN r.provider_state_bytes ELSE 0 END+256 AS bytes FROM agena_round_input_replays input JOIN agena_rounds r INDEXED BY idx_agena_round_bytes ON r.id=input.source_round_id WHERE input.round_id=? ORDER BY input.source_round_id LIMIT 10001";
const SOURCE_PREFLIGHT_SQL: &str = "SELECT head_bytes+256 AS bytes FROM agena_round_input_sources INDEXED BY idx_agena_round_source_bytes WHERE round_id=? ORDER BY content_id LIMIT 10001";

fn charge(total: &mut usize, bytes: i64) -> Result<()> {
    let bytes = usize::try_from(bytes).map_err(storage)?;
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| invalid("provider input size overflow"))?;
    if *total > MAX_INPUT_BYTES {
        return Err(invalid(
            "provider input facts exceed 8 MiB; compact or select a smaller input window",
        ));
    }
    Ok(())
}

pub(super) async fn check_budget(
    connection: &mut SqliteConnection,
    round: i64,
    diagnostics: Option<&KernelDiagnostics>,
) -> Result<usize> {
    let mut total = 0;
    if let Some(diagnostics) = diagnostics {
        metrics::add(&diagnostics.0.prompt_queries, 1);
    }
    let metadata: i64 = sqlx::query_scalar(
        "SELECT length(CAST(input_metadata_json AS BLOB)) FROM agena_rounds WHERE id=?",
    )
    .bind(round)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    charge(&mut total, metadata)?;
    for sql in [
        INPUT_PREFLIGHT_SQL,
        REPLAY_PREFLIGHT_SQL,
        SOURCE_PREFLIGHT_SQL,
    ] {
        if let Some(diagnostics) = diagnostics {
            metrics::add(&diagnostics.0.prompt_queries, 1);
        }
        let rows = sqlx::query(sql)
            .bind(round)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        if let Some(diagnostics) = diagnostics {
            metrics::add(&diagnostics.0.prompt_candidate_rows, rows.len());
        }
        if rows.len() > MAX_INPUTS {
            return Err(invalid("provider input identity/source budget exceeded"));
        }
        for row in rows {
            charge(&mut total, row.try_get("bytes").map_err(storage)?)?;
        }
    }
    Ok(total)
}

fn body<'a>(
    row: &'a SqliteRow,
    name: &str,
    diagnostics: &KernelDiagnostics,
) -> Result<Option<&'a str>> {
    let value = row.try_get::<Option<&str>, _>(name).map_err(storage)?;
    metrics::add(&diagnostics.0.prompt_body_bytes, value.map_or(0, str::len));
    Ok(value)
}

pub(super) async fn read(
    pool: &SqlitePool,
    session: SessionId,
    round: RoundId,
    diagnostics: &KernelDiagnostics,
) -> Result<PromptView> {
    let mut transaction = pool.begin().await.map_err(storage)?;
    metrics::add(&diagnostics.0.prompt_queries, 1);
    let header = sqlx::query("SELECT r.execution_id,r.generation FROM agena_rounds r JOIN agena_executions execution ON execution.id=r.execution_id WHERE r.id=? AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE s.session_id=? AND owner.deleted_at_ms IS NULL AND m.entry_id=execution.message_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
        .bind(round.get()).bind(session.get()).fetch_optional(&mut *transaction).await.map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("provider input".into()))?;
    check_budget(&mut transaction, round.get(), Some(diagnostics)).await?;
    metrics::add(&diagnostics.0.prompt_queries, 1);
    let metadata: String =
        sqlx::query_scalar("SELECT input_metadata_json FROM agena_rounds WHERE id=?")
            .bind(round.get())
            .fetch_one(&mut *transaction)
            .await
            .map_err(sql_error)?;
    metrics::add(&diagnostics.0.prompt_body_bytes, metadata.len());
    let metadata = decode(&metadata)?;
    metrics::add(&diagnostics.0.prompt_queries, 1);
    let rows = sqlx::query("SELECT e.*,input.response_entry_id,input.execution_phase,input.entry_revision AS snapshot_revision,input.tool_phase AS snapshot_tool_phase,t.call_id AS tool_call_id,t.name AS tool_name,t.execution_kind,CASE WHEN input.tool_phase='preparing' THEN 'null' ELSE t.input_json END AS tool_input_json,CASE WHEN input.tool_phase NOT IN ('preparing','running') THEN t.result_json END AS tool_result_json,i.request_json,i.created_at_ms AS interaction_created_at_ms,CASE WHEN input.interaction_decided=1 THEN i.decided_at_ms END AS interaction_decided_at_ms,CASE WHEN input.interaction_decided=1 THEN i.decision_json END AS decision_json,CASE WHEN input.interaction_decided=1 THEN i.remember_json END AS remember_json FROM agena_round_inputs input JOIN agena_entries e ON e.id=input.entry_id LEFT JOIN agena_tool_calls t ON t.id=e.tool_id AND input.tool_phase IS NOT NULL LEFT JOIN agena_interactions i ON i.id=e.interaction_id WHERE input.round_id=? ORDER BY input.ordinal LIMIT 10000")
        .bind(round.get()).fetch_all(&mut *transaction).await.map_err(sql_error)?;
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        body(&row, "body_json", diagnostics)?;
        let mut entry = reads::entry_row(&row)?;
        entry.revision = EntryRevision::try_from(
            row.try_get::<i64, _>("snapshot_revision")
                .map_err(storage)?,
        )
        .map_err(storage)?;
        let tool = match row
            .try_get::<Option<&str>, _>("snapshot_tool_phase")
            .map_err(storage)?
        {
            Some(phase) => Some(PromptTool {
                execution_kind: decode(&encode(
                    &row.try_get::<&str, _>("execution_kind").map_err(storage)?,
                )?)?,
                id: ToolId::try_from(row.try_get::<i64, _>("tool_id").map_err(storage)?)
                    .map_err(storage)?,
                call: ToolStart {
                    call_id: row.try_get("tool_call_id").map_err(storage)?,
                    name: row.try_get("tool_name").map_err(storage)?,
                    input: decode(
                        body(&row, "tool_input_json", diagnostics)?
                            .ok_or_else(|| storage("tool input is absent"))?,
                    )?,
                },
                phase: decode(&encode(&phase)?)?,
                result: body(&row, "tool_result_json", diagnostics)?
                    .map(decode)
                    .transpose()?,
            }),
            None => None,
        };
        let interaction = if let EntryPayload::Interaction { interaction_id } = entry.payload {
            for name in ["request_json", "decision_json", "remember_json"] {
                body(&row, name, diagnostics)?;
            }
            let mut view = reads::interaction_row(interaction_id, &row)?;
            view.revision = entry.revision;
            Some(view)
        } else {
            None
        };
        entries.push(PromptEntry {
            entry,
            tool,
            interaction,
            round_id: row
                .try_get::<Option<i64>, _>("round_id")
                .map_err(storage)?
                .map(RoundId::try_from)
                .transpose()
                .map_err(storage)?,
        });
    }
    metrics::add(&diagnostics.0.prompt_queries, 1);
    let replays = sqlx::query("SELECT input.source_round_id,input.phase,CASE WHEN input.has_provider_state=1 THEN r.provider_state_json END AS provider_state_json FROM agena_round_input_replays input JOIN agena_rounds r ON r.id=input.source_round_id WHERE input.round_id=? ORDER BY input.source_round_id LIMIT 10000")
        .bind(round.get()).fetch_all(&mut *transaction).await.map_err(sql_error)?;
    let replays = replays
        .iter()
        .map(|row| {
            Ok(PromptReplay {
                round_id: RoundId::try_from(
                    row.try_get::<i64, _>("source_round_id").map_err(storage)?,
                )
                .map_err(storage)?,
                phase: decode(&encode(&row.try_get::<&str, _>("phase").map_err(storage)?)?)?,
                provider_state: body(row, "provider_state_json", diagnostics)?
                    .map(decode)
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    metrics::add(&diagnostics.0.prompt_queries, 1);
    let sources = sqlx::query("SELECT head_json FROM agena_round_input_sources WHERE round_id=? ORDER BY content_id LIMIT 10000")
        .bind(round.get()).fetch_all(&mut *transaction).await.map_err(sql_error)?;
    let sources = sources
        .iter()
        .map(|row| {
            decode(
                body(row, "head_json", diagnostics)?
                    .ok_or_else(|| storage("input source boundary is absent"))?,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(PromptView {
        round_id: round,
        execution_id: header
            .try_get::<String, _>("execution_id")
            .map_err(storage)?
            .parse()
            .map_err(storage)?,
        generation: AttemptGeneration::try_from(
            header.try_get::<i64, _>("generation").map_err(storage)?,
        )
        .map_err(storage)?,
        metadata,
        entries,
        replays,
        sources,
    })
}
