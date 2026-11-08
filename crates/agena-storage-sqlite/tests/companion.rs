use agena_storage_sqlite::{SqliteCompanionRepository, open_kernel_runtime};
use agena_superagent::*;
use chrono::{TimeZone, Utc};

fn now() -> i64 {
    Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0)
        .unwrap()
        .timestamp_millis()
}
fn spec(text: &str) -> ReminderSpec {
    ReminderSpec {
        text: text.into(),
        kind: ReminderKind::Reminder,
        after_seconds: Some(1),
        at: None,
        repeat_seconds: None,
        occurrences: 1,
    }
}
fn reminder(key: &str, source: i64, n: i64) -> Reminder {
    Reminder::new(1, source, 1, key.into(), spec(key), n).unwrap()
}
fn update(revision: i64, key: &str, patch: AssistantPatch) -> AssistantUpdate {
    AssistantUpdate {
        expected_revision: revision,
        request_key: key.into(),
        patch,
    }
}

#[tokio::test]
async fn profiles_are_workspace_scoped_revision_checked_and_replay_safe() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteCompanionRepository::new(runtime.database.clone());
    let default = repo.profile(1).await.unwrap();
    assert_eq!(default.revision, 0);
    let request = update(
        0,
        "profile-1",
        AssistantPatch {
            name: Some("小鱼".into()),
            tone: Some(AssistantTone::Warm),
            ..Default::default()
        },
    );
    let saved = repo
        .update_profile(1, request.clone(), now())
        .await
        .unwrap();
    assert_eq!(saved.preferences.name, "小鱼");
    assert_eq!(
        repo.update_profile(1, request, now() + 1).await.unwrap(),
        saved
    );
    assert_eq!(repo.profile(2).await.unwrap().preferences.name, "Agena");
    assert!(
        repo.update_profile(
            1,
            update(
                0,
                "stale",
                AssistantPatch {
                    name: Some("stale".into()),
                    ..Default::default()
                }
            ),
            now() + 2
        )
        .await
        .is_err()
    );
    assert!(
        repo.update_profile(
            1,
            update(
                1,
                "profile-1",
                AssistantPatch {
                    name: Some("changed replay".into()),
                    ..Default::default()
                }
            ),
            now() + 2
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn schedules_deduplicate_and_waiting_retries_do_not_change_the_revision() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteCompanionRepository::new(runtime.database.clone());
    let n = now();
    let first = repo.create_reminder(reminder("one", 1, n)).await.unwrap();
    assert_eq!(
        repo.create_reminder(reminder("one", 1, n + 1))
            .await
            .unwrap()
            .id,
        first.id
    );
    let mut different = reminder("one", 1, n);
    different.spec.text = "different".into();
    assert!(repo.create_reminder(different).await.is_err());
    assert!(repo.reserve_reminder(first.id, n).await.unwrap().is_none());
    let reserved = repo
        .reserve_reminder(first.id, n + 1000)
        .await
        .unwrap()
        .unwrap();
    let retry = repo
        .reserve_reminder(first.id, n + 2000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retry.revision, reserved.revision);
    assert_eq!(retry.delivery_text, reserved.delivery_text);
    repo.finish_reminder(first.id, 1, n + 3000).await.unwrap();
    repo.finish_reminder(first.id, 1, n + 4000).await.unwrap();
    let done = repo.reminder(first.id).await.unwrap();
    assert_eq!(done.delivered, 1);
    assert_eq!(done.state, ReminderState::Completed);
}

#[tokio::test]
async fn quiet_hours_mute_pause_and_cancel_are_enforced_before_contact() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteCompanionRepository::new(runtime.database.clone());
    let n = now();
    repo.update_profile(
        1,
        update(
            0,
            "quiet",
            AssistantPatch {
                quiet_enabled: Some(true),
                quiet_start_minute: Some(720),
                quiet_end_minute: Some(780),
                ..Default::default()
            },
        ),
        n,
    )
    .await
    .unwrap();
    let r = repo
        .create_reminder(reminder("quiet-one", 1, n))
        .await
        .unwrap();
    assert!(
        repo.reserve_reminder(r.id, n + 1000)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(repo.reminder(r.id).await.unwrap().retry_at_ms, n + 3600000);
    repo.update_profile(
        1,
        update(
            1,
            "mute",
            AssistantPatch {
                quiet_enabled: Some(false),
                proactive_enabled: Some(false),
                ..Default::default()
            },
        ),
        n + 2000,
    )
    .await
    .unwrap();
    assert!(
        repo.reserve_reminder(r.id, n + 2000)
            .await
            .unwrap()
            .is_none()
    );
    repo.update_profile(
        1,
        update(
            2,
            "unmute",
            AssistantPatch {
                proactive_enabled: Some(true),
                ..Default::default()
            },
        ),
        n + 3000,
    )
    .await
    .unwrap();
    let current = repo.reminder(r.id).await.unwrap();
    let paused = repo
        .control_reminder(
            r.id,
            ReminderControl {
                expected_revision: current.revision,
                action: ReminderControlAction::Pause,
            },
            n + 3000,
        )
        .await
        .unwrap();
    assert!(
        repo.reserve_reminder(r.id, n + 3600000)
            .await
            .unwrap()
            .is_none()
    );
    let cancelled = repo
        .control_reminder(
            r.id,
            ReminderControl {
                expected_revision: paused.revision,
                action: ReminderControlAction::Cancel,
            },
            n + 3600000,
        )
        .await
        .unwrap();
    assert_eq!(cancelled.state, ReminderState::Cancelled);
    assert!(repo.due_reminders(n + 86400000).await.unwrap().is_empty());
}

#[tokio::test]
async fn daily_budget_cooldown_and_old_reservations_do_not_burst_after_midnight() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteCompanionRepository::new(runtime.database.clone());
    let n = now();
    repo.update_profile(
        1,
        update(
            0,
            "limits",
            AssistantPatch {
                daily_limit: Some(2),
                cooldown_seconds: Some(60),
                ..Default::default()
            },
        ),
        n,
    )
    .await
    .unwrap();
    let a = repo.create_reminder(reminder("a", 1, n)).await.unwrap();
    let b = repo.create_reminder(reminder("b", 2, n)).await.unwrap();
    let c = repo.create_reminder(reminder("c", 3, n)).await.unwrap();
    repo.reserve_reminder(a.id, n + 1000)
        .await
        .unwrap()
        .unwrap();
    assert!(
        repo.reserve_reminder(b.id, n + 1001)
            .await
            .unwrap()
            .is_none()
    );
    repo.reserve_reminder(b.id, n + 61001)
        .await
        .unwrap()
        .unwrap();
    assert!(
        repo.reserve_reminder(c.id, n + 121001)
            .await
            .unwrap()
            .is_none()
    );
    // Two unfinished reservations survived a process/day boundary. The second
    // still obeys cooldown against the newly delivered first reservation.
    let tomorrow = n + 86400000;
    repo.reserve_reminder(a.id, tomorrow)
        .await
        .unwrap()
        .unwrap();
    repo.finish_reminder(a.id, 1, tomorrow + 1).await.unwrap();
    assert!(
        repo.reserve_reminder(b.id, tomorrow + 1000)
            .await
            .unwrap()
            .is_none()
    );
    repo.reserve_reminder(b.id, tomorrow + 61001)
        .await
        .unwrap()
        .unwrap();
    repo.finish_reminder(b.id, 1, tomorrow + 61002)
        .await
        .unwrap();
    assert!(
        repo.reserve_reminder(c.id, tomorrow + 121002)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn recurrence_is_finite_and_missed_intervals_coalesce() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let repo = SqliteCompanionRepository::new(runtime.database.clone());
    let n = now();
    let mut s = spec("每小时确认进展");
    s.repeat_seconds = Some(3600);
    s.occurrences = 2;
    let r = repo
        .create_reminder(Reminder::new(1, 1, 1, "recurring".into(), s, n).unwrap())
        .await
        .unwrap();
    let late = n + 5 * 3600000;
    repo.reserve_reminder(r.id, late).await.unwrap().unwrap();
    repo.finish_reminder(r.id, 1, late).await.unwrap();
    let next = repo.reminder(r.id).await.unwrap();
    assert!(next.next_due_ms > late);
    assert_eq!(next.occurrence, 2);
    assert_eq!(next.delivered, 1);
    let due = next.next_due_ms.max(late + 300000);
    repo.reserve_reminder(r.id, due).await.unwrap().unwrap();
    repo.finish_reminder(r.id, 2, due).await.unwrap();
    assert_eq!(
        repo.reminder(r.id).await.unwrap().state,
        ReminderState::Completed
    );
}

#[tokio::test]
async fn assistant_baseline_upgrade_preserves_existing_tasks_and_preferences_persist() {
    use sea_orm::sqlx::Connection;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("source.db");
    let url = format!("sqlite://{}?mode=rwc", file.display());
    let runtime = open_kernel_runtime(&url).await.unwrap();
    let repo = SqliteCompanionRepository::new(runtime.database.clone());
    let profile = repo
        .update_profile(
            1,
            update(
                0,
                "saved",
                AssistantPatch {
                    name: Some("持久人格".into()),
                    ..Default::default()
                },
            ),
            now(),
        )
        .await
        .unwrap();
    let image = dir.path().join("reopen.db");
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(image.to_string_lossy().as_ref())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    let reopened = open_kernel_runtime(&format!("sqlite://{}?mode=rwc", image.display()))
        .await
        .unwrap();
    let repo = SqliteCompanionRepository::new(reopened.database.clone());
    assert_eq!(repo.profile(1).await.unwrap(), profile);
    let legacy = dir.path().join("legacy.db");
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(legacy.to_string_lossy().as_ref())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    let url = format!("sqlite://{}?mode=rwc", legacy.display());
    let mut raw = sea_orm::sqlx::SqliteConnection::connect(&url)
        .await
        .unwrap();
    for table in [
        "agena_channel_access_audit",
        "agena_channel_user_budget",
        "agena_channel_sessions",
    ] {
        sea_orm::sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut raw)
            .await
            .unwrap();
    }
    for table in [
        "agena_integration_inbox",
        "agena_integration_outbox",
        "agena_integration_bindings",
    ] {
        sea_orm::sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut raw)
            .await
            .unwrap();
    }
    for table in [
        "agena_assistant_profiles",
        "agena_assistant_profile_receipts",
        "agena_assistant_reminders",
        "agena_assistant_contact_budget",
    ] {
        sea_orm::sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut raw)
            .await
            .unwrap();
    }
    raw.close().await.unwrap();
    let upgraded = open_kernel_runtime(&url).await.unwrap();
    let repo = SqliteCompanionRepository::new(upgraded.database.clone());
    assert_eq!(repo.profile(1).await.unwrap().revision, 0);
}

#[tokio::test]
async fn proactive_handoff_is_one_idempotent_control_execution() {
    use agena_domain::{ComposerDocument, ComposerNode};
    use agena_storage::kernel::*;
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let w = runtime
        .workspaces
        .ensure_id("/assistant-atomic")
        .await
        .unwrap();
    let s = runtime
        .ports
        .sessions
        .create_configured(RequestId::new(), w, "Source".into(), Default::default())
        .await
        .unwrap()
        .value
        .id;
    let input = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            s,
            ComposerDocument(vec![ComposerNode::Text {
                text: "remind me".into(),
            }]),
        )
        .await
        .unwrap()
        .value
        .message_id;
    let payload = serde_json::json!({"kind":"conversation_work","message_id":input.get(),"text":"scheduled contact"});
    let result = runtime
        .ports
        .executor
        .deliver_and_resume(
            RequestId::new(),
            s,
            "contact-key".into(),
            payload.clone(),
            input,
            "scheduled contact".into(),
        )
        .await
        .unwrap()
        .value;
    let replay = runtime
        .ports
        .executor
        .deliver_and_resume(
            RequestId::new(),
            s,
            "contact-key".into(),
            payload,
            input,
            "scheduled contact".into(),
        )
        .await
        .unwrap()
        .value;
    assert_eq!(result.id, replay.id);
    assert_eq!(result.purpose, ExecutionPurpose::Continuation);
    assert!(
        runtime
            .ports
            .reader
            .pending_deliveries(s, None, 64)
            .await
            .unwrap()
            .is_empty()
    );
}
