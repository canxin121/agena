use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};
use serde_json::{Value, json};

use super::{database, execute};

async fn insert(
    db: &DatabaseConnection,
    id: i64,
    kind: &str,
    state: &str,
    content: &str,
) -> Result<(), DbErr> {
    let finished = matches!(state, "completed" | "failed" | "cancelled").then_some(2i64);
    execute(db,
        "INSERT INTO agena_parts (part_id, kind, role, state, content, origin_session_id, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) VALUES (?, ?, 'runtime', ?, ?, 1, 1, ?, 1, 2)",
        [id.into(), kind.into(), state.into(), content.into(), finished.into()],
    ).await
}

async fn snapshot(db: &DatabaseConnection, id: i64) -> (String, i64, i64, i64) {
    let row = db.query_one(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT CAST(content AS TEXT) AS content, revision, started_at_ms, updated_at_ms FROM agena_parts WHERE part_id = ?",
        [id.into()],
    )).await.unwrap().unwrap();
    (
        row.try_get("", "content").unwrap(),
        row.try_get("", "revision").unwrap(),
        row.try_get("", "started_at_ms").unwrap(),
        row.try_get("", "updated_at_ms").unwrap(),
    )
}

#[tokio::test]
async fn content_only_updates_reject_invalid_json_and_preserve_the_row() {
    let db = database().await;
    let mut accepted = Vec::new();
    for (i, invalid) in ["", "not json", "{", "{\"text\":1,}", "\0"]
        .into_iter()
        .enumerate()
    {
        let id = i as i64 + 1;
        insert(&db, id, "text", "pending", r#"{"text":"original"}"#)
            .await
            .unwrap();
        let before = snapshot(&db, id).await;
        if execute(&db,
            "UPDATE agena_parts SET content = ?, revision = revision + 1, updated_at_ms = 3 WHERE part_id = ?",
            [invalid.into(), id.into()],
        ).await.is_ok() {
            accepted.push(invalid);
        } else {
            assert_eq!(snapshot(&db, id).await, before);
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted malformed part content: {accepted:?}"
    );
}

#[tokio::test]
async fn run_kind_is_a_string_on_insert_and_content_only_update() {
    let db = database().await;
    let mut accepted = Vec::new();
    for (i, content) in [
        json!({}),
        json!({"run_kind": null}),
        json!({"run_kind": 7}),
        json!({"run_kind": true}),
        json!({"run_kind": []}),
        json!({"run_kind": {}}),
    ]
    .into_iter()
    .enumerate()
    {
        let id = i as i64 * 2 + 1;
        let content = content.to_string();
        if insert(&db, id, "run", "pending", &content).await.is_ok() {
            accepted.push(format!("insert {content}"));
        }
        insert(
            &db,
            id + 1,
            "run",
            "pending",
            r#"{"run_kind":"future_extension"}"#,
        )
        .await
        .unwrap();
        let before = snapshot(&db, id + 1).await;
        if execute(
            &db,
            "UPDATE agena_parts SET content = ? WHERE part_id = ?",
            [content.clone().into(), (id + 1).into()],
        )
        .await
        .is_ok()
        {
            accepted.push(format!("update {content}"));
        } else {
            assert_eq!(snapshot(&db, id + 1).await, before);
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted missing/non-string run kind: {accepted:?}"
    );
}

#[tokio::test]
async fn terminal_run_content_edits_cannot_remove_or_mistype_abort_reason() {
    let db = database().await;
    let mut accepted = Vec::new();
    let mut id = 0i64;
    for state in ["completed", "failed", "cancelled"] {
        for reason in [
            None,
            Some(json!(7)),
            Some(json!(false)),
            Some(json!([])),
            Some(json!({})),
            Some(Value::Null),
        ] {
            if state == "completed" && reason == Some(Value::Null) {
                continue;
            }
            id += 2;
            let mut content = json!({"run_kind": "continue"});
            if let Some(reason) = reason {
                content["abort_reason"] = reason;
            }
            let content = content.to_string();
            if insert(&db, id, "run", state, &content).await.is_ok() {
                accepted.push(format!("insert {state} {content}"));
            }
            insert(
                &db,
                id + 1,
                "run",
                state,
                r#"{"run_kind":"continue","abort_reason":"stopped"}"#,
            )
            .await
            .unwrap();
            let before = snapshot(&db, id + 1).await;
            if execute(
                &db,
                "UPDATE agena_parts SET content = ? WHERE part_id = ?",
                [content.clone().into(), (id + 1).into()],
            )
            .await
            .is_ok()
            {
                accepted.push(format!("update {state} {content}"));
            } else {
                assert_eq!(snapshot(&db, id + 1).await, before);
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted invalid terminal run reason: {accepted:?}"
    );
}

#[tokio::test]
async fn updating_start_time_cannot_bypass_lifecycle_checks() {
    let db = database().await;
    insert(&db, 1, "text", "pending", "{}").await.unwrap();
    let before = snapshot(&db, 1).await;
    execute(
        &db,
        "UPDATE agena_parts SET started_at_ms = -1 WHERE part_id = 1",
        [],
    )
    .await
    .expect_err("negative start time must be rejected on update");
    assert_eq!(snapshot(&db, 1).await, before);
}

#[tokio::test]
async fn provider_state_requires_json_on_insert_and_update_but_allows_sql_null() {
    let db = database().await;
    insert(&db, 1, "text", "pending", "{}").await.unwrap();
    for invalid in ["", "not json", "{", "[1,]"] {
        execute(&db,
            "INSERT INTO agena_parts (part_id, kind, role, state, content, provider_state, started_at_ms, created_at_ms, updated_at_ms) VALUES (2, 'text', 'runtime', 'pending', '{}', ?, 1, 1, 1)",
            [invalid.into()],
        ).await.expect_err("invalid provider state on insert");
        execute(
            &db,
            "UPDATE agena_parts SET provider_state = ? WHERE part_id = 1",
            [invalid.into()],
        )
        .await
        .expect_err("invalid provider state on update");
    }
    let row = db.query_one(Statement::from_string(db.get_database_backend(), "SELECT CAST(provider_state AS TEXT) AS provider_state FROM agena_parts WHERE part_id = 1")).await.unwrap().unwrap();
    assert_eq!(
        row.try_get::<Option<String>>("", "provider_state").unwrap(),
        None
    );
    for content in [
        "null",
        "true",
        "7",
        "[]",
        r#""opaque continuation""#,
        r#"{"cursor":3}"#,
    ] {
        execute(
            &db,
            "UPDATE agena_parts SET content = ?, provider_state = ? WHERE part_id = 1",
            [content.into(), content.into()],
        )
        .await
        .unwrap();
    }
    execute(
        &db,
        "UPDATE agena_parts SET provider_state = NULL WHERE part_id = 1",
        [],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn duplicate_control_fields_cannot_disagree_between_sqlite_and_serde() {
    let db = database().await;
    let example = r#"{"run_kind":"continue","run_kind":null,"abort_reason":null}"#;
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT json_type(?, '$.run_kind') AS field_type",
            [example.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "field_type").unwrap(), "text");
    assert!(serde_json::from_str::<Value>(example).unwrap()["run_kind"].is_null());
    insert(
        &db,
        1,
        "run",
        "completed",
        r#"{"run_kind":"continue","abort_reason":null}"#,
    )
    .await
    .unwrap();
    let before = snapshot(&db, 1).await;
    for content in [
        example,
        r#"{"run_kind":null,"run_kind":"continue","abort_reason":null}"#,
        r#"{"run_kind":"continue","abort_reason":null,"abort_reason":[]}"#,
        r#"{"run_kind":"continue","abort_reason":[],"abort_reason":null}"#,
    ] {
        insert(&db, 2, "run", "completed", content)
            .await
            .expect_err("duplicate controls on insert");
        execute(
            &db,
            "UPDATE agena_parts SET content = ? WHERE part_id = 1",
            [content.into()],
        )
        .await
        .expect_err("duplicate controls on update");
        assert_eq!(snapshot(&db, 1).await, before);
    }
}

#[tokio::test]
async fn valid_run_metadata_remains_extensible_through_completion() {
    let db = database().await;
    insert(
        &db,
        1,
        "run",
        "pending",
        r#"{"run_kind":"future_extension","extra":{"key":7}}"#,
    )
    .await
    .unwrap();
    execute(&db, "UPDATE agena_parts SET content = json_set(content, '$.extra.next', 8), revision = revision + 1 WHERE part_id = 1", []).await.unwrap();
    execute(&db, "UPDATE agena_parts SET state = 'completed', content = json_set(content, '$.abort_reason', NULL), finished_at_ms = 3, updated_at_ms = 3, revision = revision + 1 WHERE part_id = 1", []).await.unwrap();
    let (content, revision, _, _) = snapshot(&db, 1).await;
    assert_eq!(
        serde_json::from_str::<Value>(&content).unwrap(),
        json!({"run_kind":"future_extension","extra":{"key":7,"next":8},"abort_reason":null})
    );
    assert_eq!(revision, 3);
}
