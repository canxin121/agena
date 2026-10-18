//! A short, coherent read of durable control facts, independent of transcript I/O.

use super::*;

pub(super) async fn read(
    backend: &Backend,
    session: SessionId,
    limit: usize,
) -> Result<ReadView<SessionControlSnapshot>> {
    let limit = limit.clamp(1, 128);
    backend
        .cached_read(
            &backend.control_reads,
            read_cache::Key::ControlSnapshot(session, limit),
            async {
                let mut transaction = backend
                    .reader
                    .get_sqlite_connection_pool()
                    .begin()
                    .await
                    .map_err(storage)?;
                let read_transaction = backend.live.diagnostics().read_transaction();
                let snapshot =
                    read_on(&mut transaction, session, limit, backend.live.diagnostics()).await?;
                transaction.rollback().await.map_err(sql_error)?;
                read_transaction.complete();
                Ok(snapshot)
            },
        )
        .await
}

/// The caller owns the read transaction. No helper acquires another connection
/// or uses independently cached values, so every field shares the first SELECT.
pub(super) async fn read_on(
    connection: &mut SqliteConnection,
    session: SessionId,
    limit: usize,
    diagnostics: &KernelDiagnostics,
) -> Result<SessionControlSnapshot> {
    let limit = limit.clamp(1, 128);
    let control_version = sqlx::query_scalar(
        "SELECT control_version FROM agena_sessions WHERE id=? AND deleted_at_ms IS NULL",
    )
    .bind(session.get())
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?
    .ok_or_else(|| KernelError::NotFound("session".into()))?;
    metrics::add(&diagnostics.0.metadata_queries, 1);
    let metadata = reads::metadata(connection, session).await?;
    metrics::add(&diagnostics.0.summary_queries, 1);
    let summary = reads::summary(connection, session).await?;
    let configuration = lifecycle::configuration(connection, session).await?;
    let pending_interactions =
        reads::pending_interactions_on(connection, session, None, limit, diagnostics).await?;
    let active_executions =
        reads::active_executions_on(connection, session, None, limit, diagnostics).await?;
    // Execution IDs are random, so the first ID-ordered page can contain only
    // queued work even while a later execution is running. Select the running
    // attempt independently without changing the public pagination contract.
    metrics::add(&diagnostics.0.activity_queries, 1);
    let running = sqlx::query(
        "SELECT e.id,e.current_generation FROM agena_attempts a INDEXED BY idx_agena_attempt_active JOIN agena_executions e ON e.id=a.execution_id AND e.current_generation=a.generation WHERE a.session_id=? AND a.phase IN ('queued','running') AND a.phase='running' ORDER BY a.execution_id LIMIT 1",
    )
    .bind(session.get())
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    metrics::add(&diagnostics.0.activity_rows, usize::from(running.is_some()));
    let selected: Option<(ExecutionId, AttemptGeneration, bool)> = match running {
        Some(row) => Some((
            row.try_get::<String, _>("id")
                .map_err(storage)?
                .parse()
                .map_err(storage)?,
            AttemptGeneration::try_from(
                row.try_get::<i64, _>("current_generation")
                    .map_err(storage)?,
            )
            .map_err(storage)?,
            false,
        )),
        None => active_executions
            .executions
            .first()
            .map(|run| (run.id, run.generation, run.phase == ExecutionPhase::Queued)),
    };
    let active_execution = match selected {
        Some((execution_id, generation, queued)) => {
            let phase = if queued {
                agena_domain::ExecutionPhase::Starting
            } else if !pending_interactions.interactions.is_empty() {
                agena_domain::ExecutionPhase::AwaitingInteraction
            } else if sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM agena_tool_calls INDEXED BY idx_agena_tool_active WHERE execution_id=? AND generation=? AND phase IN ('preparing','running'))",
            )
            .bind(execution_id.to_string())
            .bind(generation.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?
            {
                agena_domain::ExecutionPhase::ExecutingTools
            } else if sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM agena_content_streams c INDEXED BY idx_agena_content_active JOIN agena_entries e ON e.id=c.entry_id WHERE c.execution_id=? AND c.generation=? AND c.capture_state='open' AND e.kind IN ('text','reasoning'))",
            )
            .bind(execution_id.to_string())
            .bind(generation.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?
            {
                agena_domain::ExecutionPhase::StreamingModel
            } else {
                agena_domain::ExecutionPhase::PreparingModel
            };
            Some(agena_domain::ExecutionLifecycle::Active {
                execution_id,
                phase,
            })
        }
        None => None,
    };
    Ok(SessionControlSnapshot {
        control_version,
        metadata,
        summary,
        configuration,
        pending_interactions,
        active_executions,
        active_execution,
    })
}
