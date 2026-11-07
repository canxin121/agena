//! Real kernel state transitions behind the compact notifications read model.
//! Only timestamps are seeded directly, so ordering and millisecond ties are
//! deterministic without sleeps; phases and visibility use public commands.

use agena_domain::{
    AgentStatus, ComposerDocument, ComposerNode, ConversationMode, SessionConversation,
    SessionRelationKind,
};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use sea_orm::sqlx;

async fn session(runtime: &SqliteKernel, title: &str, config: SessionConfiguration) -> SessionId {
    let workspace = runtime
        .workspaces
        .ensure_id("/activity-summary")
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .create_configured(RequestId::new(), workspace, title.into(), config)
        .await
        .unwrap()
        .value
        .id
}

async fn queue(runtime: &SqliteKernel, session: SessionId) -> ExecutionView {
    runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Exercise the activity read model".into(),
            }]),
        )
        .await
        .unwrap()
        .value
}

async fn start(runtime: &SqliteKernel, session: SessionId) -> (ExecutionView, AttemptHandle) {
    let execution = queue(runtime, session).await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    (execution, attempt)
}

async fn await_input(attempt: &AttemptHandle, session: SessionId) {
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "activity-question".into(),
                name: "ask".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.request_interaction(
        RequestId::new(),
        InteractionRequest::Input {
            request: agena_domain::UserInputRequest {
                request_id: "activity-question".into(),
                session_id: Some(session.get()),
                title: "Choose an option".into(),
                body_markdown: String::new(),
                kind: agena_domain::UserInputKind::AskUser,
                source: agena_domain::UserInputSource::Host,
                auto_resolution_ms: None,
                presented_at: None,
                questions: Vec::new(),
                created_at: chrono::Utc::now(),
            },
        },
    )
    .await
    .unwrap();
}

async fn finish_at(
    runtime: &SqliteKernel,
    attempt: &AttemptHandle,
    phase: ExecutionPhase,
    at_ms: i64,
) {
    attempt.finish(RequestId::new(), phase).await.unwrap();
    let result = sqlx::query(
        "UPDATE agena_attempts SET finished_at_ms=? WHERE execution_id=? AND generation=?",
    )
    .bind(at_ms)
    .bind(attempt.execution_id().to_string())
    .bind(attempt.generation().get())
    .execute(runtime.database.get_sqlite_connection_pool())
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1);
}

async fn assert_summary(
    runtime: &SqliteKernel,
    running: u64,
    awaiting: u64,
    latest_at_ms: Option<i64>,
    latest_failed: bool,
) {
    let summary = runtime.ports.reader.activity_summary().await.unwrap();
    assert_eq!(
        (
            summary.running_sessions,
            summary.awaiting_interaction_sessions,
            summary.latest_terminal_at_ms,
            summary.latest_terminal_failed,
        ),
        (running, awaiting, latest_at_ms, latest_failed),
    );
}

#[tokio::test]
async fn counts_sessions_with_parallel_runs_queued_work_and_pending_interactions() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    assert_summary(&runtime, 0, 0, None, false).await;
    let first = session(&runtime, "First worker", Default::default()).await;
    let second = session(&runtime, "Second worker", Default::default()).await;
    let waiting = session(&runtime, "User question", Default::default()).await;
    let (_, first_attempt) = start(&runtime, first).await;
    let queued_followup = queue(&runtime, first).await;
    let (_, second_attempt) = start(&runtime, second).await;
    let (_, waiting_attempt) = start(&runtime, waiting).await;
    await_input(&waiting_attempt, waiting).await;

    // The waiting session has a running attempt, but occupies the awaiting
    // category. Two executions in the first session still count as one session.
    assert_summary(&runtime, 2, 1, None, false).await;
    let base = chrono::Utc::now().timestamp_millis() + 60_000;
    finish_at(&runtime, &first_attempt, ExecutionPhase::Succeeded, base).await;
    assert_summary(&runtime, 2, 1, Some(base), false).await;
    runtime
        .ports
        .sessions
        .cancel_queued(RequestId::new(), first, queued_followup.id)
        .await
        .unwrap();
    assert_summary(&runtime, 1, 1, Some(base), false).await;
    finish_at(
        &runtime,
        &waiting_attempt,
        ExecutionPhase::Cancelled,
        base + 1,
    )
    .await;
    assert_summary(&runtime, 1, 0, Some(base + 1), false).await;
    finish_at(
        &runtime,
        &second_attempt,
        ExecutionPhase::Succeeded,
        base + 2,
    )
    .await;
    assert_summary(&runtime, 0, 0, Some(base + 2), false).await;
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn terminal_order_uses_finish_time_and_survives_renaming_an_older_failure() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let older = session(&runtime, "Older failure", Default::default()).await;
    let newer = session(&runtime, "Newer success", Default::default()).await;
    let (_, failed) = start(&runtime, older).await;
    let (_, succeeded) = start(&runtime, newer).await;
    let base = chrono::Utc::now().timestamp_millis() + 60_000;
    finish_at(&runtime, &failed, ExecutionPhase::Failed, base).await;
    finish_at(&runtime, &succeeded, ExecutionPhase::Succeeded, base + 1).await;
    assert_summary(&runtime, 0, 0, Some(base + 1), false).await;

    // Equal controlled metadata clocks let the real rename deterministically
    // move the old failed session ahead in the normal session listing.
    sqlx::query("UPDATE agena_sessions SET updated_at_ms=? WHERE id IN (?,?)")
        .bind(base + 100)
        .bind(older.get())
        .bind(newer.get())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            older,
            MetadataChange {
                title: Some("Renamed after the successful run".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let listing = runtime
        .ports
        .reader
        .query_summaries(SummaryQuery::default())
        .await
        .unwrap();
    assert_eq!(listing.sessions[0].id, older);
    assert!(listing.sessions[0].updated_at_ms > listing.sessions[1].updated_at_ms);
    drop(listing);
    assert_summary(&runtime, 0, 0, Some(base + 1), false).await;
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn excludes_deleted_temporary_and_subagent_activity_but_keeps_regular_children() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let parent = session(&runtime, "Parent", Default::default()).await;
    let child_config = SessionConfiguration {
        parent_session_id: Some(parent),
        relation_kind: SessionRelationKind::Child,
        ..Default::default()
    };
    let child = session(&runtime, "Visible child", child_config.clone()).await;
    let temporary = session(
        &runtime,
        "Temporary conversation",
        SessionConfiguration {
            conversation: Some(SessionConversation {
                mode: ConversationMode::Btw,
                parent_session_id: parent.get(),
            }),
            ..child_config
        },
    )
    .await;
    let subagent = session(
        &runtime,
        "Delegated worker",
        SessionConfiguration {
            parent_session_id: Some(parent),
            relation_kind: SessionRelationKind::Subagent,
            agent: Some(SessionAgentConfiguration {
                name: "worker".into(),
                status: AgentStatus::Running,
                started_at_ms: Some(1),
                finished_at_ms: None,
                failure: None,
                limits: Default::default(),
            }),
            ..Default::default()
        },
    )
    .await;
    let (_, child_attempt) = start(&runtime, child).await;
    let (_, temporary_attempt) = start(&runtime, temporary).await;
    let (_, subagent_attempt) = start(&runtime, subagent).await;
    assert_summary(&runtime, 1, 0, None, false).await;
    await_input(&temporary_attempt, temporary).await;
    await_input(&subagent_attempt, subagent).await;
    assert_summary(&runtime, 1, 0, None, false).await;

    let base = chrono::Utc::now().timestamp_millis() + 60_000;
    finish_at(&runtime, &child_attempt, ExecutionPhase::Succeeded, base).await;
    finish_at(
        &runtime,
        &temporary_attempt,
        ExecutionPhase::Failed,
        base + 10,
    )
    .await;
    finish_at(
        &runtime,
        &subagent_attempt,
        ExecutionPhase::Failed,
        base + 20,
    )
    .await;
    assert_summary(&runtime, 0, 0, Some(base), false).await;

    let deleted = session(&runtime, "Deleted failure", Default::default()).await;
    let (_, deleted_attempt) = start(&runtime, deleted).await;
    finish_at(
        &runtime,
        &deleted_attempt,
        ExecutionPhase::Failed,
        base + 30,
    )
    .await;
    assert_summary(&runtime, 0, 0, Some(base + 30), true).await;
    runtime
        .ports
        .sessions
        .delete(RequestId::new(), deleted)
        .await
        .unwrap();
    assert_summary(&runtime, 0, 0, Some(base), false).await;
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn latest_timestamp_ties_preserve_failure_and_superseded_attempts_do_not() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let failed_session = session(&runtime, "Retry failure", Default::default()).await;
    let succeeded_session = session(&runtime, "Concurrent success", Default::default()).await;
    let (failed_execution, failed_attempt) = start(&runtime, failed_session).await;
    let (_, succeeded_attempt) = start(&runtime, succeeded_session).await;
    let base = chrono::Utc::now().timestamp_millis() + 60_000;
    finish_at(&runtime, &failed_attempt, ExecutionPhase::Failed, base).await;
    finish_at(
        &runtime,
        &succeeded_attempt,
        ExecutionPhase::Succeeded,
        base,
    )
    .await;
    // A later command or lexicographically greater execution id cannot erase
    // a failure at the same latest millisecond: the timestamp is a tie group.
    assert_summary(&runtime, 0, 0, Some(base), true).await;
    let retry = runtime
        .ports
        .sessions
        .continue_execution(
            RequestId::new(),
            failed_session,
            failed_execution.message_id,
        )
        .await
        .unwrap()
        .value;
    assert_eq!(retry.id, failed_execution.id);
    assert_eq!(
        retry.generation,
        failed_execution.generation.next().unwrap()
    );
    let historical = runtime
        .ports
        .reader
        .execution_attempt(
            failed_session,
            failed_execution.id,
            failed_execution.generation,
        )
        .await
        .unwrap();
    assert_eq!(historical.phase, ExecutionPhase::Failed);
    drop(historical);
    assert_summary(&runtime, 1, 0, Some(base), false).await;
    let retry_attempt = runtime
        .ports
        .executor
        .claim_next_generation(RequestId::new(), failed_session, retry.id, retry.generation)
        .await
        .unwrap()
        .value;
    finish_at(
        &runtime,
        &retry_attempt,
        ExecutionPhase::Succeeded,
        base + 1,
    )
    .await;
    assert_summary(&runtime, 0, 0, Some(base + 1), false).await;

    // Interrupted is terminal, but is not the failed-outcome indicator.
    let (_, interrupted) = start(&runtime, succeeded_session).await;
    finish_at(
        &runtime,
        &interrupted,
        ExecutionPhase::Interrupted,
        base + 2,
    )
    .await;
    assert_summary(&runtime, 0, 0, Some(base + 2), false).await;
    runtime.ports.clone().shutdown().await.unwrap();
}
