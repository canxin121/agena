use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbErr, Statement, Value};

mod parts;

async fn database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("database");
    crate::initialize_schema(&db).await.expect("schema");
    execute(
        &db,
        "INSERT INTO agena_workspaces (id, path, created_at_ms, updated_at_ms) \
         VALUES (1, '/invariants', 1, 1)",
        [],
    )
    .await
    .expect("workspace");
    execute(
        &db,
        "INSERT INTO agena_sessions \
         (id, workspace_id, title, version, lifecycle_state, created_at_ms, updated_at_ms) \
         VALUES (1, 1, 'parent', 1, 'ready', 1, 1), (2, 1, 'creating', 1, 'creating', 1, 1)",
        [],
    )
    .await
    .expect("sessions");
    db
}

async fn execute(
    db: &DatabaseConnection,
    sql: &str,
    values: impl IntoIterator<Item = Value>,
) -> Result<(), DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        values,
    ))
    .await
    .map(|_| ())
}

const VALID_FAILURE: &str =
    r#"{"id":"failure-1","code":"execution_failed","user":{"fallback":"The task failed."}}"#;

// These are the minimum database-level failure fields, not a substitute for
// deserializing the full application Failure contract.
const INVALID_FAILURES: &[Option<&str>] = &[
    None,
    Some(""),
    Some("   "),
    Some("not json"),
    Some("null"),
    Some("[]"),
    Some("1"),
    Some("true"),
    Some("\"failure\""),
    Some("{}"),
    Some(r#"{"id":"failure-1"}"#),
    Some(r#"{"code":"execution_failed","user":{"fallback":"failed"}}"#),
    Some(r#"{"id":"failure-1","user":{"fallback":"failed"}}"#),
    Some(r#"{"id":"failure-1","code":"execution_failed"}"#),
    Some(r#"{"id":"failure-1","code":"execution_failed","user":{}}"#),
    Some(r#"{"id":null,"code":"execution_failed","user":{"fallback":"failed"}}"#),
    Some(r#"{"id":"failure-1","code":7,"user":{"fallback":"failed"}}"#),
    Some(r#"{"id":"failure-1","code":"execution_failed","user":{"fallback":[]}}"#),
];

#[tokio::test]
async fn creation_failures_reject_missing_fields_and_preserve_the_previous_row() {
    let db = database().await;
    let mut accepted = Vec::new();
    for payload in INVALID_FAILURES {
        let result = execute(
            &db,
            "UPDATE agena_sessions SET lifecycle_state = 'failed', creation_failure_json = ? \
             WHERE id = 2",
            [(*payload).into()],
        )
        .await;
        if result.is_ok() {
            accepted.push(*payload);
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted invalid failure records: {accepted:?}"
    );
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "SELECT lifecycle_state, CAST(creation_failure_json AS TEXT) AS failure \
             FROM agena_sessions WHERE id = 2",
        ))
        .await
        .expect("session")
        .expect("row");
    assert_eq!(
        row.try_get::<String>("", "lifecycle_state").unwrap(),
        "creating"
    );
    assert_eq!(row.try_get::<Option<String>>("", "failure").unwrap(), None);

    execute(
        &db,
        "UPDATE agena_sessions SET lifecycle_state = 'failed', creation_failure_json = ? \
         WHERE id = 2",
        [VALID_FAILURE.into()],
    )
    .await
    .expect("valid failure record");
    execute(
        &db,
        "UPDATE agena_sessions SET creation_failure_json = '{}' WHERE id = 2",
        [],
    )
    .await
    .expect_err("failure-only updates also enforce the shape");
}

#[tokio::test]
async fn subtask_insert_enforces_terminal_failure_and_timestamp_shapes() {
    let db = database().await;
    let mut accepted = Vec::new();
    let insert = "INSERT INTO agena_sessions \
        (parent_id, depth, root_id, workspace_id, relation_kind, title, version, \
         task_id, subtask_status, subtask_started_at_ms, subtask_finished_at_ms, \
         subtask_failure_json, created_at_ms, updated_at_ms) \
        VALUES (1, 1, 1, 1, 'subagent', 'child', 1, ?, ?, ?, ?, ?, 1, 1)";
    for status in ["failed", "timed_out", "interrupted"] {
        for (i, payload) in INVALID_FAILURES.iter().enumerate() {
            let task_id = format!("{status}-{i}");
            let result = execute(
                &db,
                insert,
                [
                    task_id.into(),
                    status.into(),
                    1.into(),
                    2.into(),
                    (*payload).into(),
                ],
            )
            .await;
            if result.is_ok() {
                accepted.push(format!("{status}: {payload:?}"));
            }
        }
    }
    for (status, start, finish, failure) in [
        ("created", Some(1), None, None),
        ("running", None, None, None),
        ("completed", Some(1), None, None),
        ("cancelled", Some(1), Some(2), Some(VALID_FAILURE)),
        ("failed", Some(-1), Some(2), Some(VALID_FAILURE)),
        ("timed_out", Some(2), Some(1), Some(VALID_FAILURE)),
    ] {
        let result = execute(
            &db,
            insert,
            [
                format!("shape-{status}").into(),
                status.into(),
                start.into(),
                finish.into(),
                failure.into(),
            ],
        )
        .await;
        if result.is_ok() {
            accepted.push(format!("invalid timestamps/failure for {status}"));
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted invalid subtasks: {accepted:?}"
    );

    for (status, start, finish, failure) in [
        ("created", None, None, None),
        ("running", Some(1), None, None),
        ("completed", Some(1), Some(2), None),
        ("cancelled", Some(1), Some(2), None),
        ("failed", Some(1), Some(2), Some(VALID_FAILURE)),
        ("timed_out", Some(1), Some(2), Some(VALID_FAILURE)),
        ("interrupted", Some(1), Some(2), Some(VALID_FAILURE)),
    ] {
        execute(
            &db,
            insert,
            [
                format!("valid-{status}").into(),
                status.into(),
                start.into(),
                finish.into(),
                failure.into(),
            ],
        )
        .await
        .expect("valid subtask shape");
    }
}

#[tokio::test]
async fn reopening_current_schema_refreshes_historical_failure_triggers_without_changing_data() {
    let db = database().await;
    execute(
        &db,
        "DROP TRIGGER agena_sessions_subtask_lifecycle_insert",
        [],
    )
    .await
    .expect("historical schema has no insert lifecycle trigger");
    for (name, historical_sql) in [
        (
            "agena_sessions_creation_failure_shape",
            include_str!("fixtures/v13_creation_failure.sql"),
        ),
        (
            "agena_sessions_subtask_lifecycle",
            include_str!("fixtures/v13_subtask_lifecycle.sql"),
        ),
    ] {
        execute(&db, &format!("DROP TRIGGER {name}"), [])
            .await
            .expect("remove fixed trigger");
        execute(&db, historical_sql, [])
            .await
            .expect("historical trigger");
    }
    execute(
        &db,
        "INSERT INTO agena_sessions \
         (id, parent_id, depth, root_id, workspace_id, relation_kind, title, version, \
          task_id, subtask_status, created_at_ms, updated_at_ms) \
         VALUES (3, 1, 1, 1, 1, 'subagent', 'child', 1, 'historical', 'created', 1, 1)",
        [],
    )
    .await
    .expect("child");
    crate::initialize_schema(&db)
        .await
        .expect("refresh current schema");
    execute(
        &db,
        "UPDATE agena_sessions SET lifecycle_state = 'failed', creation_failure_json = '{}' WHERE id = 2",
        [],
    )
    .await
    .expect_err("reopened database enforces creation failure fields");
    execute(
        &db,
        "UPDATE agena_sessions SET subtask_status = 'failed', subtask_started_at_ms = 1, \
         subtask_finished_at_ms = 2, subtask_failure_json = '{}' WHERE id = 3",
        [],
    )
    .await
    .expect_err("reopened database enforces delegated failure fields");
    execute(
        &db,
        "INSERT INTO agena_sessions \
         (parent_id, depth, root_id, workspace_id, relation_kind, title, version, \
          task_id, subtask_status, created_at_ms, updated_at_ms) \
         VALUES (1, 1, 1, 1, 'subagent', 'invalid child', 1, 'invalid', 'failed', 1, 1)",
        [],
    )
    .await
    .expect_err("reopened database also installs missing insert lifecycle checks");
    let states = db.query_all(Statement::from_string(
        db.get_database_backend(),
        "SELECT id, title, lifecycle_state, subtask_status, version FROM agena_sessions ORDER BY id",
    )).await.expect("sessions");
    assert_eq!(states.len(), 3);
    for (row, title, lifecycle, subtask) in [
        (&states[0], "parent", "ready", None),
        (&states[1], "creating", "creating", None),
        (&states[2], "child", "creating", Some("created")),
    ] {
        assert_eq!(row.try_get::<String>("", "title").unwrap(), title);
        assert_eq!(
            row.try_get::<String>("", "lifecycle_state").unwrap(),
            lifecycle
        );
        assert_eq!(
            row.try_get::<Option<String>>("", "subtask_status")
                .unwrap()
                .as_deref(),
            subtask
        );
        assert_eq!(row.try_get::<i64>("", "version").unwrap(), 1);
    }
    async fn pragma(db: &DatabaseConnection, name: &str) -> i64 {
        db.query_one(Statement::from_string(
            db.get_database_backend(),
            format!("PRAGMA {name}"),
        ))
        .await
        .expect("pragma")
        .expect("row")
        .try_get("", name)
        .expect("value")
    }
    assert_eq!(
        pragma(&db, "user_version").await,
        crate::CURRENT_SCHEMA_VERSION
    );
    let schema_cookie = pragma(&db, "schema_version").await;
    crate::initialize_schema(&db).await.expect("reopen again");
    assert_eq!(
        pragma(&db, "schema_version").await,
        schema_cookie,
        "unchanged triggers must not be rewritten on every open"
    );
}
