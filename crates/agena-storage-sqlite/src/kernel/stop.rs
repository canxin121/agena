//! The stop boundary shares the delivery writer transaction. Historical
//! deliveries leave this decision's working set once a successful round sees
//! their frozen input; failed rounds cannot acknowledge business input.
use port::Scope;

use super::*;

pub(super) async fn observe_round(
    connection: &mut SqliteConnection,
    scope: Scope,
    round: RoundId,
) -> Result<()> {
    sqlx::query("UPDATE agena_deliveries SET observed_round_id=? WHERE session_id=? AND state='consumed' AND observed_round_id IS NULL AND (target_execution_id IS NOT NULL OR COALESCE(json_extract(payload_json,'$.wake'),1)!=0 OR json_extract(payload_json,'$.kind')='agent_message') AND (target_execution_id IS NULL OR target_execution_id=?) AND result_entry_id IN (SELECT entry_id FROM agena_round_inputs WHERE round_id=?)")
        .bind(round.get()).bind(scope.session.get()).bind(scope.execution.to_string()).bind(round.get())
        .execute(&mut *connection).await.map_err(sql_error)?;
    Ok(())
}

pub(super) async fn prepare(
    connection: &mut SqliteConnection,
    scope: Scope,
    conditions: &StopConditions,
) -> Result<Mutation> {
    commands::check_scope(connection, scope, true).await?;
    if conditions.pending_once_delivery_keys.len() > 128
        || conditions
            .pending_once_delivery_keys
            .iter()
            .any(|key| key.is_empty() || key.len() > 1024)
    {
        return Err(invalid("stop scheduler facts exceed their identity budget"));
    }
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_deliveries INDEXED BY idx_agena_delivery_session WHERE session_id=? AND state='pending' AND (target_execution_id IS NULL OR target_execution_id=?))")
        .bind(scope.session.get()).bind(scope.execution.to_string())
        .fetch_one(&mut *connection).await.map_err(sql_error)?;
    let unobserved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_deliveries d INDEXED BY idx_agena_delivery_unobserved WHERE d.session_id=? AND d.state='consumed' AND d.observed_round_id IS NULL AND (d.target_execution_id IS NULL OR d.target_execution_id=?) AND (d.target_execution_id IS NOT NULL OR COALESCE(json_extract(d.payload_json,'$.wake'),1)!=0 OR json_extract(d.payload_json,'$.kind')='agent_message') AND EXISTS(SELECT 1 FROM agena_segment_entries m JOIN agena_session_segments s ON s.segment_id=m.segment_id WHERE s.session_id=d.session_id AND m.entry_id=d.result_entry_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal))")
        .bind(scope.session.get()).bind(scope.execution.to_string())
        .fetch_one(&mut *connection).await.map_err(sql_error)?;
    if pending || unobserved {
        return Ok(Mutation::StopDecision(StopDecision::Continue));
    }
    let queued: bool = sqlx::query_scalar("SELECT queued_executions>0 OR running_executions>1 FROM agena_session_statistics WHERE session_id=?")
        .bind(scope.session.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    let activity: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_background_activities INDEXED BY idx_agena_background_active WHERE session_id=? AND phase='running')")
        .bind(scope.session.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
    if queued || activity || conditions.scheduler_waiting {
        return Ok(Mutation::StopDecision(StopDecision::AwaitingBackground));
    }
    for key in &conditions.pending_once_delivery_keys {
        let observed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_deliveries WHERE business_key=? AND session_id=? AND state='consumed' AND observed_round_id IS NOT NULL)")
            .bind(key).bind(scope.session.get()).fetch_one(&mut *connection).await.map_err(sql_error)?;
        if !observed {
            return Ok(Mutation::StopDecision(StopDecision::AwaitingBackground));
        }
    }
    sqlx::query("UPDATE agena_attempts SET stop_requested=1 WHERE execution_id=? AND generation=?")
        .bind(scope.execution.to_string())
        .bind(scope.generation.get())
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    Ok(Mutation::StopDecision(StopDecision::Ready))
}
