use agena_storage::store::{
    InMemoryEngine, NewPart, PartDelta, PartRole, PartState, PersistenceEngine,
};
use serde_json::json;

async fn engine(memory: bool) -> (Box<dyn PersistenceEngine>, i64) {
    if !memory {
        let (engine, session) = super::setup(super::in_memory_db().await).await;
        return (Box::new(engine), session);
    }
    let engine = InMemoryEngine::default();
    engine.set_now(1_000_000);
    let session = engine
        .create_session(super::NewSession {
            workspace_id: 1,
            parent_id: None,
            relation_kind: super::SessionRelationKind::Root,
            cutoff_part_id: None,
            title: "parts integrity".to_owned(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await
        .unwrap();
    engine
        .try_acquire_lease(session.id, "owner-a", 1_000_000)
        .await
        .unwrap();
    (Box::new(engine), session.id)
}

#[tokio::test]
async fn both_backends_reject_invalid_run_content_without_advancing_versions() {
    let mut accepted = Vec::new();
    for memory in [false, true] {
        for invalid in [
            json!(null),
            json!({}),
            json!({"run_kind": null}),
            json!({"run_kind": []}),
        ] {
            let (engine, session) = engine(memory).await;
            let submitted = engine
                .submit_user_run(
                    session,
                    "owner-a",
                    vec![super::text_part("hello")],
                    None,
                    1_000_000,
                )
                .await
                .unwrap();
            let before = engine.load_session(session).await.unwrap();
            let result = engine
                .update_part(
                    session,
                    "owner-a",
                    submitted.run_id,
                    PartDelta {
                        content: Some(invalid.clone()),
                        ..Default::default()
                    },
                    1_000_001,
                )
                .await;
            if result.is_ok() {
                accepted.push(format!("memory={memory}: {invalid}"));
            } else {
                let after = engine.load_session(session).await.unwrap();
                assert_eq!(after.parts, before.parts);
                assert_eq!(after.meta.version, before.meta.version);
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "invalid run content accepted by storage API: {accepted:?}"
    );
}

#[tokio::test]
async fn rejected_streaming_delta_preserves_part_state_and_session_version_in_both_backends() {
    for memory in [false, true] {
        let (engine, session) = engine(memory).await;
        let submitted = engine
            .submit_user_run(
                session,
                "owner-a",
                vec![NewPart::pending(
                    "extension",
                    PartRole::Assistant,
                    json!({"payload": 7}),
                )],
                None,
                1_000_000,
            )
            .await
            .unwrap();
        let part_id = submitted
            .parts
            .iter()
            .find(|part| !part.is_run_marker())
            .unwrap()
            .part_id;
        let before = engine.load_session(session).await.unwrap();
        engine
            .update_part(
                session,
                "owner-a",
                part_id,
                PartDelta {
                    state: Some(PartState::InProgress),
                    content_text_delta: Some("cannot append to this shape".to_owned()),
                    ..Default::default()
                },
                1_000_001,
            )
            .await
            .expect_err("streaming delta requires text-shaped content");
        let after = engine.load_session(session).await.unwrap();
        assert_eq!(
            after.parts, before.parts,
            "rejected update changed a part, memory={memory}"
        );
        assert_eq!(after.meta.version, before.meta.version);
    }
}

#[tokio::test]
async fn both_backends_validate_run_creation_and_completion_and_return_committed_fields() {
    use agena_storage::store::RunOutcome;

    for memory in [false, true] {
        let (engine, session) = engine(memory).await;
        for invalid in [
            json!(null),
            json!([]),
            json!(7),
            json!({"abort_reason": []}),
        ] {
            let before = engine.load_session(session).await.unwrap();
            engine
                .start_run(
                    session,
                    "owner-a",
                    "future_extension",
                    invalid,
                    None,
                    1_000_000,
                )
                .await
                .expect_err("run creation requires object content and typed controls");
            let after = engine.load_session(session).await.unwrap();
            assert_eq!(after.parts, before.parts);
            assert_eq!(after.meta.version, before.meta.version);
        }
        let run = engine
            .start_run(
                session,
                "owner-a",
                "future_extension",
                json!({"extra": 7}),
                None,
                1_000_000,
            )
            .await
            .unwrap();
        for invalid in [json!(null), json!([]), json!({}), json!({"run_kind": null})] {
            let before = engine.load_session(session).await.unwrap();
            engine
                .complete_run(
                    session,
                    "owner-a",
                    run.run_id,
                    RunOutcome {
                        status: PartState::Completed,
                        abort_reason: None,
                        content: Some(invalid),
                        provider_state: None,
                    },
                    1_000_001,
                )
                .await
                .expect_err("completion must retain valid run controls");
            let after = engine.load_session(session).await.unwrap();
            assert_eq!(after.parts, before.parts);
            assert_eq!(after.meta.version, before.meta.version);
        }
        let before = engine.load_session(session).await.unwrap();
        let completed = engine
            .complete_run(
                session,
                "owner-a",
                run.run_id,
                RunOutcome {
                    status: PartState::Completed,
                    abort_reason: None,
                    content: Some(json!({"run_kind":"future_extension","extra":8})),
                    provider_state: Some(json!({"cursor": 3})),
                },
                1_000_002,
            )
            .await
            .unwrap();
        let after = engine.load_session(session).await.unwrap();
        assert_eq!(
            &completed,
            after
                .parts
                .iter()
                .find(|part| part.part_id == run.run_id)
                .unwrap()
        );
        assert_eq!(completed.revision, 2);
        assert_eq!(completed.updated_at_ms, 1_000_002);
        assert_eq!(after.meta.version, before.meta.version + 1);
    }
}

#[tokio::test]
async fn both_backends_reject_invalid_finish_times_and_backwards_updates_without_mutation() {
    for memory in [false, true] {
        let (engine, session) = engine(memory).await;
        let submitted = engine
            .submit_user_run(
                session,
                "owner-a",
                vec![super::text_part("hello")],
                None,
                1_000_000,
            )
            .await
            .unwrap();
        let id = submitted
            .parts
            .iter()
            .find(|part| !part.is_run_marker())
            .unwrap()
            .part_id;
        let before = engine.load_session(session).await.unwrap();
        engine
            .update_part(
                session,
                "owner-a",
                id,
                PartDelta {
                    finished_at_ms: Some(1_000_001),
                    ..Default::default()
                },
                1_000_001,
            )
            .await
            .expect_err("pending part cannot have a finish time");
        assert_eq!(
            engine.load_session(session).await.unwrap().parts,
            before.parts
        );
        engine
            .update_part(
                session,
                "owner-a",
                id,
                PartDelta {
                    state: Some(PartState::InProgress),
                    ..Default::default()
                },
                1_000_002,
            )
            .await
            .unwrap();
        for (delta, now) in [
            (
                PartDelta {
                    state: Some(PartState::Completed),
                    finished_at_ms: Some(999_999),
                    ..Default::default()
                },
                1_000_003,
            ),
            (
                PartDelta {
                    content: Some(json!({"text":"old clock"})),
                    ..Default::default()
                },
                1_000_001,
            ),
        ] {
            let before = engine.load_session(session).await.unwrap();
            engine
                .update_part(session, "owner-a", id, delta, now)
                .await
                .expect_err("invalid timestamps");
            let after = engine.load_session(session).await.unwrap();
            assert_eq!(after.parts, before.parts);
            assert_eq!(after.meta.version, before.meta.version);
        }
    }
}

#[tokio::test]
async fn exhausted_part_revision_returns_an_error_without_panicking_or_wrapping() {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

    let db = super::in_memory_db().await;
    let (engine, session) = super::setup(db.clone()).await;
    let (run, _) = super::submit_hello(&engine, session).await;
    db.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "UPDATE agena_parts SET revision = ? WHERE part_id = ?",
        [i64::MAX.into(), run.into()],
    ))
    .await
    .unwrap();
    let before = engine.load_session(session).await.unwrap();
    let error = engine
        .update_part(session, "owner-a", run, PartDelta::default(), 1_000_001)
        .await
        .expect_err("revision exhaustion is an explicit error");
    assert!(error.to_string().contains("revision is exhausted"));
    let after = engine.load_session(session).await.unwrap();
    assert_eq!(after.parts, before.parts);
    assert_eq!(after.meta.version, before.meta.version);
}
