//! Bounded, frozen provider input facts. The round captures only identities,
//! small control decisions and committed heads. Large payloads remain shared
//! immutable facts; the read preflights encoded byte lengths before decoding.

use futures_util::TryStreamExt;
use sea_orm::sqlx::{SqlitePool, sqlite::SqliteRow};
use std::sync::LazyLock;

use super::*;

pub(super) const MAX_INPUTS: usize = 10_000;
pub(super) const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub(super) const INPUT_PREFLIGHT_SQL: &str = "SELECT e.body_bytes+CASE WHEN input.tool_phase='preparing' THEN 0 ELSE COALESCE(t.input_bytes,0) END+CASE WHEN input.tool_phase NOT IN ('preparing','running') THEN COALESCE(t.result_bytes,0) ELSE 0 END+COALESCE(i.request_bytes,0)+CASE WHEN input.interaction_decided=1 THEN COALESCE(i.decision_bytes,0)+COALESCE(i.remember_bytes,0) ELSE 0 END+512 AS bytes FROM agena_round_inputs input CROSS JOIN agena_entries e INDEXED BY idx_agena_entry_preflight ON e.id=input.entry_id LEFT JOIN agena_tool_calls t INDEXED BY idx_agena_tool_bytes ON t.id=e.tool_id AND input.tool_phase IS NOT NULL LEFT JOIN agena_interactions i INDEXED BY idx_agena_interaction_bytes ON i.id=e.interaction_id WHERE input.round_id=?1 LIMIT 10001";
static INPUT_BUDGET_SQL: LazyLock<String> = LazyLock::new(|| {
    let replay_budget = prompt_replays::BUDGET_SQL;
    format!(
        "SELECT length(CAST(r.input_metadata_json AS BLOB)), entries.items, entries.bytes, replays.items, replays.bytes,
                r.input_source_count,r.input_source_bytes
         FROM agena_rounds r
         CROSS JOIN (SELECT count(*) AS items,COALESCE(sum(bytes),0) AS bytes FROM ({INPUT_PREFLIGHT_SQL})) entries
         CROSS JOIN ({replay_budget}) replays
         WHERE r.id=?1"
    )
});

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

pub(super) struct InputBudget {
    pub bytes: usize,
    entries: usize,
    replays: usize,
    sources: usize,
}

fn checked_count(count: i64) -> Result<usize> {
    let count = usize::try_from(count).map_err(storage)?;
    if count > MAX_INPUTS {
        return Err(invalid("provider input identity/source budget exceeded"));
    }
    Ok(count)
}

pub(super) async fn check_input_budget(
    connection: &mut SqliteConnection,
    round: i64,
    diagnostics: Option<&KernelDiagnostics>,
) -> Result<InputBudget> {
    if let Some(diagnostics) = diagnostics {
        metrics::add(&diagnostics.0.prompt_queries, 1);
    }
    let (metadata, entries, entry_bytes, replays, replay_bytes, sources, source_bytes): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(&INPUT_BUDGET_SQL)
        .bind(round)
        .fetch_one(connection)
        .await
        .map_err(sql_error)?;
    let entries = checked_count(entries)?;
    let replays = checked_count(replays)?;
    let mut bytes = 0;
    for size in [metadata, entry_bytes, replay_bytes] {
        charge(&mut bytes, size)?;
    }
    let sources = check_source_budget(bytes, sources, source_bytes)?;
    if let Some(diagnostics) = diagnostics {
        metrics::add(
            &diagnostics.0.prompt_candidate_rows,
            entries + replays + sources,
        );
    }
    Ok(InputBudget {
        bytes,
        entries,
        replays,
        sources,
    })
}

pub(super) fn check_source_budget(
    mut input_bytes: usize,
    sources: i64,
    source_bytes: i64,
) -> Result<usize> {
    let sources = checked_count(sources)?;
    charge(&mut input_bytes, source_bytes)?;
    Ok(sources)
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
    read_owned(pool, Some(session), round, diagnostics).await
}

/// Session is optional so a caller holding only a round identity can still
/// recover the frozen input. The recorded round names its own session, and the
/// ownership/branch checks below run the same way either way.
pub(super) async fn read_owned(
    pool: &SqlitePool,
    session: Option<SessionId>,
    round: RoundId,
    diagnostics: &KernelDiagnostics,
) -> Result<PromptView> {
    let mut transaction = pool.begin().await.map_err(storage)?;
    metrics::add(&diagnostics.0.prompt_queries, 1);
    let header = sqlx::query("SELECT execution.session_id AS owner_session_id,r.execution_id,r.generation FROM agena_rounds r JOIN agena_executions execution ON execution.id=r.execution_id WHERE r.id=? AND (? IS NULL OR EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE s.session_id=? AND owner.deleted_at_ms IS NULL AND m.entry_id=execution.message_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal))")
        .bind(round.get()).bind(session.map(SessionId::get)).bind(session.map(SessionId::get)).fetch_optional(&mut *transaction).await.map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("provider input".into()))?;
    let owner_session_id = header
        .try_get::<i64, _>("owner_session_id")
        .map_err(storage)?;
    let session = match session {
        Some(session) => session,
        None => SessionId::try_from(owner_session_id).map_err(storage)?,
    };
    if owner_session_id != session.get() {
        let captured: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agena_branch_rounds WHERE session_id=? AND round_id=?)",
        )
        .bind(session.get())
        .bind(round.get())
        .fetch_one(&mut *transaction)
        .await
        .map_err(sql_error)?;
        if !captured {
            return Err(KernelError::NotFound(
                "provider input at branch boundary".into(),
            ));
        }
    }
    let budget = check_input_budget(&mut transaction, round.get(), Some(diagnostics)).await?;
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
    let mut rows = sqlx::query("SELECT e.*,(input.execution_phase!='queued') AS execution_started,e.generation AS execution_generation,input.response_entry_id,input.execution_phase,input.entry_revision AS snapshot_revision,input.tool_phase AS snapshot_tool_phase,t.call_id AS tool_call_id,t.name AS tool_name,t.execution_kind,CASE WHEN input.tool_phase='preparing' THEN 'null' ELSE t.input_json END AS tool_input_json,CASE WHEN input.tool_phase NOT IN ('preparing','running') THEN t.result_json END AS tool_result_json,i.request_json,i.created_at_ms AS interaction_created_at_ms,CASE WHEN input.interaction_decided=1 THEN i.decided_at_ms END AS interaction_decided_at_ms,CASE WHEN input.interaction_decided=1 THEN i.decision_json END AS decision_json,CASE WHEN input.interaction_decided=1 THEN i.remember_json END AS remember_json FROM agena_round_inputs input CROSS JOIN agena_entries e ON e.id=input.entry_id LEFT JOIN agena_tool_calls t ON t.id=e.tool_id AND input.tool_phase IS NOT NULL LEFT JOIN agena_interactions i ON i.id=e.interaction_id WHERE input.round_id=? ORDER BY input.start_ordinal,input.block_ordinal LIMIT 10000")
        .bind(round.get()).fetch(&mut *transaction);
    let mut entries = Vec::with_capacity(budget.entries);
    while let Some(row) = rows.try_next().await.map_err(sql_error)? {
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
    drop(rows);
    // A newly captured branch request inherits the branch's frozen facts. An
    // inherited provider round already owns its earlier immutable manifest;
    // the later fork cut must not rewrite what that original request saw.
    if header
        .try_get::<i64, _>("owner_session_id")
        .map_err(storage)?
        == session.get()
    {
        branches::apply_prompt_entries(&mut transaction, session, &mut entries).await?;
    }
    metrics::add(&diagnostics.0.prompt_queries, 1);
    let mut rows = sqlx::query(prompt_replays::READ_SQL)
        .bind(round.get())
        .fetch(&mut *transaction);
    let mut replays = Vec::with_capacity(budget.replays);
    while let Some(row) = rows.try_next().await.map_err(sql_error)? {
        replays.push(PromptReplay {
            round_id: RoundId::try_from(row.try_get::<i64, _>("source_round_id").map_err(storage)?)
                .map_err(storage)?,
            phase: decode(&encode(&row.try_get::<&str, _>("phase").map_err(storage)?)?)?,
            provider_state: body(&row, "provider_state_json", diagnostics)?
                .map(decode)
                .transpose()?,
        });
    }
    drop(rows);
    metrics::add(&diagnostics.0.prompt_queries, 1);
    let mut rows = sqlx::query(&prompt_sources::READ_SQL)
        .bind(round.get())
        .fetch(&mut *transaction);
    let mut sources = Vec::with_capacity(budget.sources);
    while let Some(row) = rows.try_next().await.map_err(sql_error)? {
        let id: &str = row.try_get("id").map_err(storage)?;
        let source = content::heads::read(id.parse().map_err(storage)?, &row)?;
        metrics::add(
            &diagnostics.0.prompt_body_bytes,
            source.capture_error.as_deref().map_or(0, str::len),
        );
        sources.push(source);
    }
    // Provider tool output keeps the prior ordering by public content ID.
    sources.sort_unstable_by_key(|head| head.id);
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
