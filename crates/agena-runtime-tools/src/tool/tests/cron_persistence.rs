use std::{sync::Arc, time::Duration};

use agena_scheduler::{
    JobDeliveryAttempt, JobDeliveryResult, JobSink, ScheduledJob, Scheduler, SqliteJobStore,
};
use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};

use super::{
    ExecutionPrincipal, PermissionPolicy, ToolExecutor, ToolPermissionPolicy,
    empty_test_plugin_host,
};
use crate::{
    part::{CronCreateToolInput, CronDeleteToolInput, CronHistoryToolInput, CronListToolInput},
    tool::{ToolRuntimeContext, cron},
};

struct UnusedSink;

#[agena_plugin_host::sdk::async_trait]
impl JobSink for UnusedSink {
    async fn deliver(&self, _: &ScheduledJob, _: &JobDeliveryAttempt) -> JobDeliveryResult {
        panic!("this test must never start the scheduler");
    }
}

#[tokio::test]
async fn cron_tools_report_database_failures_instead_of_success_or_empty_results() {
    let workspace = tempfile::tempdir().unwrap();
    let db = Database::connect("sqlite::memory:").await.unwrap();
    agena_scheduler::schema::initialize_schema(&db)
        .await
        .unwrap();
    let scheduler = Scheduler::new(
        Arc::new(SqliteJobStore::new(db.clone())),
        Arc::new(UnusedSink),
        Duration::from_secs(60),
    );
    let executor = ToolExecutor::new(
        workspace.path(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        empty_test_plugin_host(workspace.path()).await,
        None,
        Some(scheduler.clone()),
        None,
    );
    db.execute(Statement::from_string(DatabaseBackend::Sqlite,
        "CREATE TRIGGER reject_insert BEFORE INSERT ON agena_scheduler_jobs BEGIN SELECT RAISE(ABORT, 'injected persistence failure'); END",
    )).await.unwrap();
    let input: CronCreateToolInput = serde_json::from_value(serde_json::json!({
        "expression": "0 * * * * *", "prompt": "must be durable", "timezone": "UTC"
    }))
    .unwrap();
    assert!(
        cron::execute_create_async(&executor, &input, &ToolRuntimeContext::default())
            .await
            .is_err()
    );
    assert!(scheduler.list().await.unwrap().is_empty());
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "DROP TRIGGER reject_insert",
    ))
    .await
    .unwrap();
    assert!(
        cron::execute_create_async(&executor, &input, &ToolRuntimeContext::default())
            .await
            .is_ok()
    );
    let id = scheduler.list().await.unwrap()[0].id;
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "DROP TABLE agena_scheduler_jobs",
    ))
    .await
    .unwrap();
    assert!(
        cron::execute_list_async(
            &executor,
            &CronListToolInput::default(),
            &ToolRuntimeContext::default()
        )
        .await
        .is_err()
    );
    assert!(
        cron::execute_delete_async(
            &executor,
            &CronDeleteToolInput { id: id.to_string() },
            &ToolRuntimeContext::default()
        )
        .await
        .is_err()
    );
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "DROP TABLE agena_scheduler_history",
    ))
    .await
    .unwrap();
    let history: CronHistoryToolInput = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(
        cron::execute_history_async(&executor, &history, &ToolRuntimeContext::default())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn all_cron_tool_controls_are_bound_to_the_launching_session() {
    let workspace = tempfile::tempdir().unwrap();
    let db = Database::connect("sqlite::memory:").await.unwrap();
    agena_scheduler::schema::initialize_schema(&db)
        .await
        .unwrap();
    let scheduler = Scheduler::new(
        Arc::new(SqliteJobStore::new(db)),
        Arc::new(UnusedSink),
        Duration::from_secs(60),
    );
    let executor = ToolExecutor::new(
        workspace.path(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        empty_test_plugin_host(workspace.path()).await,
        None,
        Some(scheduler.clone()),
        None,
    );
    let a = ToolRuntimeContext {
        session_id: Some(41),
        ..Default::default()
    };
    let b = ToolRuntimeContext {
        session_id: Some(42),
        ..Default::default()
    };
    let create = serde_json::from_value(
        serde_json::json!({"expression":"0 * * * * *","prompt":"private","timezone":"UTC"}),
    )
    .unwrap();
    cron::execute_create_async(&executor, &create, &a)
        .await
        .unwrap();
    let id = scheduler.list().await.unwrap()[0].id.to_string();
    let control = crate::part::CronJobControlToolInput { id: id.clone() };
    let update =
        serde_json::from_value(serde_json::json!({"id":id,"prompt":"must not replace"})).unwrap();
    assert!(
        cron::execute_pause_async(&executor, &control, &b)
            .await
            .is_err()
    );
    assert!(
        cron::execute_resume_async(&executor, &control, &b)
            .await
            .is_err()
    );
    assert!(
        cron::execute_update_async(&executor, &update, &b)
            .await
            .is_err()
    );
    let deleted =
        cron::execute_delete_async(&executor, &CronDeleteToolInput { id: id.clone() }, &b)
            .await
            .unwrap();
    assert!(
        !serde_json::to_value(deleted.output).unwrap()["removed"]
            .as_bool()
            .unwrap()
    );
    let listed = cron::execute_list_async(&executor, &CronListToolInput::default(), &b)
        .await
        .unwrap();
    assert!(
        serde_json::to_value(listed.output).unwrap()["jobs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(scheduler.list().await.unwrap()[0].prompt, "private");
    cron::execute_pause_async(&executor, &control, &a)
        .await
        .unwrap();
    cron::execute_resume_async(&executor, &control, &a)
        .await
        .unwrap();
    cron::execute_delete_async(&executor, &CronDeleteToolInput { id }, &a)
        .await
        .unwrap();
    assert!(scheduler.list().await.unwrap().is_empty());
}
