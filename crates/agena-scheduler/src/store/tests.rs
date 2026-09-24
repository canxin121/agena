use chrono::{Duration, Utc};
use sea_orm::{ConnectOptions, Database};

use super::*;
use crate::schema::initialize_schema;
use crate::{ClaimDueDelivery, JobDeliveryAttempt, JobDeliveryResult, JobOutcome};

async fn scheduler_database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    initialize_schema(&db).await.unwrap();
    db
}

async fn stores() -> Vec<Box<dyn JobStore>> {
    vec![
        Box::new(InMemoryJobStore::new()),
        Box::new(SqliteJobStore::new(scheduler_database().await)),
    ]
}

async fn snapshot(store: &dyn JobStore, id: Uuid) -> JobSnapshot {
    store.get(id).await.unwrap().unwrap()
}

async fn claim(
    store: &dyn JobStore,
    expected: &JobSnapshot,
    owner: &str,
    now: chrono::DateTime<Utc>,
) -> (JobSnapshot, JobDeliveryAttempt) {
    let mut job = expected.job.clone();
    let ClaimDueDelivery::Deliver(delivery) = job.claim_due_delivery(now).unwrap() else {
        panic!("job must be due");
    };
    assert!(
        store
            .claim(expected, job, owner.into(), now.timestamp_millis())
            .await
            .unwrap()
    );
    (snapshot(store, expected.job.id).await, delivery)
}

fn transient_failure() -> JobDeliveryResult {
    JobDeliveryResult::failed(
        None,
        agena_failure::Failure::new(
            agena_failure::FailureCode::new("scheduler.delivery_failed"),
            agena_failure::FailureCategory::DependencyUnavailable,
            agena_failure::FailureResponsibility::System,
            agena_failure::RetryDirective::AfterRefresh,
            agena_failure::RecoveryDirective::Refresh,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::new("scheduler-outcome", "transient delivery failure"),
        ),
    )
}

#[tokio::test]
async fn stores_reclaim_failed_deliveries_and_reject_stale_finalization() {
    for store in stores().await {
        let now = Utc::now();
        let job = ScheduledJob::new_once(now, "retry");
        let id = job.id;
        store.put(job).await.unwrap();
        let original = snapshot(store.as_ref(), id).await;
        let (first, delivery) = claim(store.as_ref(), &original, "first", now).await;
        assert!(
            store
                .list_due(now.timestamp_millis())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !store
                .claim(
                    &original,
                    first.job.clone(),
                    "duplicate".into(),
                    now.timestamp_millis()
                )
                .await
                .unwrap()
        );
        assert!(
            !store
                .finish(
                    &first,
                    "wrong-owner",
                    first.job.clone(),
                    now.timestamp_millis()
                )
                .await
                .unwrap()
        );
        let mut failed = first.job.clone();
        assert_eq!(
            failed
                .finish_delivery(now, &delivery, transient_failure())
                .unwrap(),
            JobOutcome::RetryScheduled
        );
        let retry_at = failed.retry_at.unwrap();
        assert!(
            store
                .finish(&first, "first", failed, now.timestamp_millis())
                .await
                .unwrap()
        );
        assert!(
            store
                .list_due((retry_at - Duration::milliseconds(1)).timestamp_millis())
                .await
                .unwrap()
                .is_empty()
        );
        let due = store
            .list_due(retry_at.timestamp_millis())
            .await
            .unwrap()
            .pop()
            .unwrap();
        let (second, retry) = claim(store.as_ref(), &due, "second", retry_at).await;
        assert_eq!(retry.attempt, 2);
        assert_eq!(retry.delivery_key, delivery.delivery_key);
        assert!(
            !store
                .finish(
                    &first,
                    "first",
                    first.job.clone(),
                    retry_at.timestamp_millis()
                )
                .await
                .unwrap()
        );
        let mut finished = second.job.clone();
        assert_eq!(
            finished
                .finish_delivery(retry_at, &retry, JobDeliveryResult::submitted(None))
                .unwrap(),
            JobOutcome::Expired
        );
        assert!(
            store
                .finish(&second, "second", finished, retry_at.timestamp_millis())
                .await
                .unwrap()
        );
        assert!(snapshot(store.as_ref(), id).await.job.completed);
        assert!(
            store
                .list_due((retry_at + Duration::days(1)).timestamp_millis())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.list_history(Some(id), 10).await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn stores_reject_stale_schedule_and_prompt_only_edits() {
    for store in stores().await {
        for change_schedule in [false, true] {
            let now = Utc::now();
            let job = ScheduledJob::new_once(now, "original");
            let id = job.id;
            store.put(job).await.unwrap();
            let original = snapshot(store.as_ref(), id).await;
            let mut edited = original.job.clone();
            edited.prompt = "edited while the scheduler was polling".into();
            if change_schedule {
                edited.next_fire_at = Some(now + Duration::hours(1));
            }
            assert!(store.replace(&original, edited).await.unwrap());
            assert!(
                !store
                    .replace(&original, original.job.clone())
                    .await
                    .unwrap()
            );
            let mut stale = original.job.clone();
            stale.claim_due_delivery(now).unwrap();
            assert!(
                !store
                    .claim(
                        &original,
                        stale,
                        "stale-worker".into(),
                        now.timestamp_millis()
                    )
                    .await
                    .unwrap()
            );
        }
    }
}

#[tokio::test]
async fn duplicate_inserts_never_replace_jobs_or_active_claims() {
    for store in stores().await {
        let now = Utc::now();
        let job = ScheduledJob::new_once(now, "original");
        let id = job.id;
        store.put(job.clone()).await.unwrap();
        let original = snapshot(store.as_ref(), id).await;
        claim(store.as_ref(), &original, "owner", now).await;
        let mut duplicate = job;
        duplicate.prompt = "must not overwrite".into();
        assert!(matches!(
            store.put(duplicate).await,
            Err(SchedulerError::Conflict(_))
        ));
        let current = snapshot(store.as_ref(), id).await;
        assert_eq!(current.job.prompt, "original");
        assert_eq!(current.claim_key(), Some("owner"));
    }
}

#[tokio::test]
async fn expired_claims_are_recoverable_but_cannot_renew_or_finish() {
    for store in stores().await {
        let now = Utc::now();
        let job = ScheduledJob::new_once(now, "interrupted");
        let id = job.id;
        store.put(job).await.unwrap();
        let original = snapshot(store.as_ref(), id).await;
        let (first, delivery) = claim(store.as_ref(), &original, "first", now).await;
        let expires = now + Duration::milliseconds(CLAIM_LEASE_MILLIS);
        assert!(
            store
                .list_due(expires.timestamp_millis() - 1)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !store
                .renew(id, "first", expires.timestamp_millis())
                .await
                .unwrap()
        );
        assert!(
            !store
                .finish(
                    &first,
                    "first",
                    first.job.clone(),
                    expires.timestamp_millis()
                )
                .await
                .unwrap()
        );
        let recovered = store
            .list_due(expires.timestamp_millis())
            .await
            .unwrap()
            .pop()
            .unwrap();
        let (second, retry) = claim(store.as_ref(), &recovered, "recovered", expires).await;
        assert_eq!(retry.delivery_key, delivery.delivery_key);
        assert_eq!(retry.attempt, 2);
        assert!(
            !store
                .renew(id, "first", expires.timestamp_millis())
                .await
                .unwrap()
        );
        assert!(
            !store
                .finish(
                    &first,
                    "first",
                    first.job.clone(),
                    expires.timestamp_millis()
                )
                .await
                .unwrap()
        );
        let mut finished = second.job.clone();
        finished
            .finish_delivery(expires, &retry, JobDeliveryResult::submitted(None))
            .unwrap();
        assert!(
            store
                .finish(&second, "recovered", finished, expires.timestamp_millis())
                .await
                .unwrap()
        );
    }
}

#[tokio::test]
async fn heartbeats_exclude_recovery_without_invalidating_edits() {
    for store in stores().await {
        let now = Utc::now();
        let job = ScheduledJob::new_once(now, "slow");
        let id = job.id;
        store.put(job).await.unwrap();
        let (expected, _) = claim(
            store.as_ref(),
            &snapshot(store.as_ref(), id).await,
            "owner",
            now,
        )
        .await;
        let renewed = now + Duration::milliseconds(CLAIM_LEASE_MILLIS / 2);
        assert!(
            store
                .renew(id, "owner", renewed.timestamp_millis())
                .await
                .unwrap()
        );
        assert!(
            store
                .list_due((now + Duration::milliseconds(CLAIM_LEASE_MILLIS)).timestamp_millis())
                .await
                .unwrap()
                .is_empty()
        );
        let mut paused = expected.job.clone();
        paused.pause();
        assert!(
            store.replace(&expected, paused).await.unwrap(),
            "a heartbeat must not conflict with a pause"
        );
        assert_eq!(
            snapshot(store.as_ref(), id).await.renewed_at_ms,
            Some(renewed.timestamp_millis())
        );
    }
}

#[tokio::test]
async fn sqlite_store_survives_store_reconstruction() {
    let db = scheduler_database().await;
    let first = SqliteJobStore::new(db.clone());
    let job = ScheduledJob::new_once(Utc::now() + Duration::minutes(5), "verify");
    let id = job.id;
    first.put(job).await.unwrap();
    let reconstructed = SqliteJobStore::new(db);
    let original = snapshot(&reconstructed, id).await;
    assert_eq!(original.job.prompt, "verify");
    assert_eq!(reconstructed.list().await.unwrap().len(), 1);
    let mut updated = original.job.clone();
    updated.prompt = "verify again".into();
    assert!(reconstructed.replace(&original, updated).await.unwrap());
    assert_eq!(
        snapshot(&reconstructed, id).await.job.prompt,
        "verify again"
    );
    assert!(reconstructed.remove(id).await.unwrap());
    assert!(reconstructed.list().await.unwrap().is_empty());
}

async fn file_database(path: &std::path::Path) -> DatabaseConnection {
    let mut options = ConnectOptions::new(format!("sqlite://{}?mode=rwc", path.display()));
    options.max_connections(1);
    let db = Database::connect(options).await.unwrap();
    initialize_schema(&db).await.unwrap();
    db
}

#[tokio::test]
async fn sqlite_claim_is_exclusive_across_independent_connections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scheduler.db");
    let a = SqliteJobStore::new(file_database(&path).await);
    let b = SqliteJobStore::new(file_database(&path).await);
    let now = Utc::now();
    let job = ScheduledJob::new_once(now, "exclusive");
    let id = job.id;
    a.put(job).await.unwrap();
    let a_snapshot = snapshot(&a, id).await;
    let b_snapshot = snapshot(&b, id).await;
    let mut pending = a_snapshot.job.clone();
    pending.claim_due_delivery(now).unwrap();
    let (left, right) = tokio::join!(
        a.claim(
            &a_snapshot,
            pending.clone(),
            "a".into(),
            now.timestamp_millis()
        ),
        b.claim(&b_snapshot, pending, "b".into(), now.timestamp_millis()),
    );
    assert!(left.unwrap() ^ right.unwrap());
}

#[tokio::test]
async fn scheduler_wide_history_survives_job_deletion_and_store_reconstruction() {
    let db = scheduler_database().await;
    let store = SqliteJobStore::new(db.clone());
    let now = Utc::now();
    let job = ScheduledJob::new_once(now, "audit me");
    let id = job.id;
    store.put(job).await.unwrap();
    let (expected, delivery) = claim(&store, &snapshot(&store, id).await, "owner", now).await;
    let mut completed = expected.job.clone();
    completed
        .finish_delivery(now, &delivery, JobDeliveryResult::submitted(Some(7)))
        .unwrap();
    assert!(
        store
            .finish(&expected, "owner", completed, now.timestamp_millis())
            .await
            .unwrap()
    );
    assert!(store.remove(id).await.unwrap());
    let history = SqliteJobStore::new(db)
        .list_history(Some(id), 10)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].record.delivery_key.as_deref(),
        Some(delivery.delivery_key.as_str())
    );
}

#[tokio::test]
async fn history_retention_is_bounded_and_ordered_by_completion_time() {
    for store in stores().await {
        let now = Utc::now();
        let mut job = ScheduledJob::new_once(now, "history");
        job.record_delivery(now, JobDeliveryResult::submitted(None));
        let record = job.last_run.unwrap();
        // Insert newest first so insertion order alone cannot pass the test.
        for index in (0..=MAX_RETAINED_HISTORY_ENTRIES).rev() {
            let mut record = record.clone();
            record.finished_at = now + Duration::milliseconds(index as i64);
            record.delivery_key = Some(index.to_string());
            store
                .append_history(SchedulerHistoryEntry {
                    owner_workspace: None,
                    owner_session_id: None,
                    job_id: job.id,
                    record,
                })
                .await
                .unwrap();
        }
        let history = store
            .list_history(Some(job.id), MAX_RETAINED_HISTORY_ENTRIES + 10)
            .await
            .unwrap();
        assert_eq!(history.len(), MAX_RETAINED_HISTORY_ENTRIES);
        assert_eq!(
            history.last().unwrap().record.delivery_key.as_deref(),
            Some("1")
        );
    }
}

#[tokio::test]
async fn history_prune_failure_must_roll_back_the_insert() {
    let db = scheduler_database().await;
    let store = SqliteJobStore::new(db.clone());
    let mut job = ScheduledJob::new_once(Utc::now(), "history");
    job.record_delivery(Utc::now(), JobDeliveryResult::submitted(None));
    let record = job.last_run.unwrap();
    db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
        "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i < ?) \
         INSERT INTO agena_scheduler_history (job_id, run_json, finished_at_ms) SELECT ?, ?, 0 FROM n",
        [(MAX_RETAINED_HISTORY_ENTRIES as i64).into(), job.id.to_string().into(), serde_json::to_string(&record).unwrap().into()],
    )).await.unwrap();
    db.execute(Statement::from_string(DatabaseBackend::Sqlite,
        "CREATE TRIGGER reject_history_prune BEFORE DELETE ON agena_scheduler_history BEGIN SELECT RAISE(ABORT, 'injected prune failure'); END",
    )).await.unwrap();
    assert!(
        store
            .append_history(SchedulerHistoryEntry {
                owner_workspace: None,
                owner_session_id: None,
                job_id: job.id,
                record
            })
            .await
            .is_err()
    );
    let count: i64 = db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM agena_scheduler_history",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "count")
        .unwrap();
    assert_eq!(count, MAX_RETAINED_HISTORY_ENTRIES as i64);
}

#[tokio::test]
async fn failed_history_writes_roll_back_completion_and_misfire_transitions() {
    let db = scheduler_database().await;
    let store = SqliteJobStore::new(db.clone());
    let now = Utc::now();
    db.execute(Statement::from_string(DatabaseBackend::Sqlite,
        "CREATE TRIGGER reject_history BEFORE INSERT ON agena_scheduler_history BEGIN SELECT RAISE(ABORT, 'injected history failure'); END",
    )).await.unwrap();
    let job = ScheduledJob::new_once(now, "completion");
    let id = job.id;
    store.put(job).await.unwrap();
    let (expected, delivery) = claim(&store, &snapshot(&store, id).await, "owner", now).await;
    let mut completed = expected.job.clone();
    completed
        .finish_delivery(now, &delivery, JobDeliveryResult::submitted(None))
        .unwrap();
    assert!(
        store
            .finish(
                &expected,
                "owner",
                completed.clone(),
                now.timestamp_millis()
            )
            .await
            .is_err()
    );
    let current = snapshot(&store, id).await;
    assert!(!current.job.completed);
    assert_eq!(current.claim_key(), Some("owner"));
    assert!(store.list_history(Some(id), 10).await.unwrap().is_empty());
    let mut skip = ScheduledJob::new_once(now - Duration::minutes(5), "misfire");
    skip.misfire_policy = crate::MisfirePolicy::Skip;
    let skip_id = skip.id;
    store.put(skip).await.unwrap();
    let original = snapshot(&store, skip_id).await;
    let mut skipped = original.job.clone();
    assert_eq!(
        skipped.claim_due_delivery(now).unwrap(),
        ClaimDueDelivery::StateUpdated
    );
    assert!(store.replace(&original, skipped).await.is_err());
    assert!(!snapshot(&store, skip_id).await.job.completed);
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "DROP TRIGGER reject_history",
    ))
    .await
    .unwrap();
    assert!(
        store
            .finish(&expected, "owner", completed, now.timestamp_millis())
            .await
            .unwrap()
    );
    assert!(
        !store
            .finish(
                &expected,
                "owner",
                expected.job.clone(),
                now.timestamp_millis()
            )
            .await
            .unwrap()
    );
    assert_eq!(store.list_history(Some(id), 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn persistence_and_decode_errors_are_not_empty_or_missing_jobs() {
    let db = scheduler_database().await;
    let store = SqliteJobStore::new(db.clone());
    let now = Utc::now();
    let job = ScheduledJob::new_once(now, "corrupt me");
    let id = job.id;
    store.put(job).await.unwrap();
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "UPDATE agena_scheduler_jobs SET job_json = '{broken'",
    ))
    .await
    .unwrap();
    assert!(store.get(id).await.is_err());
    assert!(store.list().await.is_err());
    assert!(store.list_due(now.timestamp_millis()).await.is_err());
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "DROP TABLE agena_scheduler_jobs",
    ))
    .await
    .unwrap();
    assert!(
        store
            .put(ScheduledJob::new_once(now, "must fail"))
            .await
            .is_err()
    );
    assert!(store.remove(id).await.is_err());
    assert!(store.get(id).await.is_err());
    assert!(store.list_due(now.timestamp_millis()).await.is_err());
}

#[tokio::test]
async fn list_due_filters_paused_completed_future_and_claimed_jobs() {
    for store in stores().await {
        let now = Utc::now();
        let mut paused = ScheduledJob::new_once(now, "paused");
        paused.pause();
        let mut completed = ScheduledJob::new_once(now, "completed");
        completed.advance(now).unwrap();
        let future = ScheduledJob::new_once(now + Duration::days(1), "future");
        let mut retry = ScheduledJob::new_once(now, "retry");
        let ClaimDueDelivery::Deliver(delivery) = retry.claim_due_delivery(now).unwrap() else {
            panic!("due");
        };
        retry
            .finish_delivery(now - Duration::minutes(1), &delivery, transient_failure())
            .unwrap();
        for job in [
            paused,
            completed,
            future,
            retry,
            ScheduledJob::new_once(now, "due"),
        ] {
            store.put(job).await.unwrap();
        }
        let claimed = ScheduledJob::new_once(now, "claimed");
        let id = claimed.id;
        store.put(claimed).await.unwrap();
        claim(
            store.as_ref(),
            &snapshot(store.as_ref(), id).await,
            "owner",
            now,
        )
        .await;
        let mut found: Vec<_> = store
            .list_due(now.timestamp_millis())
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.job.prompt)
            .collect();
        found.sort();
        assert_eq!(found, ["due", "retry"]);
    }
}

#[tokio::test]
async fn owned_history_survives_job_deletion_and_hides_unowned_host_rows() {
    for store in stores().await {
        let now = Utc::now();
        let mut job = ScheduledJob::new_once(now, "owned");
        job.owner_workspace = Some("workspace-a".into());
        job.set_owner(41);
        store.put(job.clone()).await.unwrap();
        let before = snapshot(store.as_ref(), job.id).await;
        job.record_delivery(now, JobDeliveryResult::submitted(Some(91)));
        assert!(store.replace(&before, job.clone()).await.unwrap());
        assert_eq!(
            store
                .list_history_owned("workspace-a", Some(41), None, 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .list_history_owned("workspace-a", Some(42), None, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_history_owned("workspace-b", Some(41), None, 10)
                .await
                .unwrap()
                .is_empty()
        );
        let current = snapshot(store.as_ref(), job.id).await;
        assert!(
            !store.remove_checked(&before).await.unwrap(),
            "stale authorized snapshot must not delete a new version"
        );
        assert!(store.remove_checked(&current).await.unwrap());
        assert_eq!(
            store
                .list_history_owned("workspace-a", Some(41), Some(job.id), 10)
                .await
                .unwrap()
                .len(),
            1
        );
        store
            .append_history(SchedulerHistoryEntry {
                job_id: Uuid::new_v4(),
                owner_workspace: None,
                owner_session_id: Some(41),
                record: job.last_run.clone().unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(
            store
                .list_history_owned("workspace-a", Some(41), None, 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn owner_cannot_be_changed_by_any_store_update() {
    for store in stores().await {
        let mut job = ScheduledJob::new_once(Utc::now(), "owned");
        job.owner_workspace = Some("workspace-a".into());
        job.set_owner(41);
        store.put(job.clone()).await.unwrap();
        let before = snapshot(store.as_ref(), job.id).await;
        job.set_owner(42);
        assert!(store.replace(&before, job).await.is_err());
        assert_eq!(
            snapshot(store.as_ref(), before.job.id)
                .await
                .job
                .owner_session_id,
            Some(41)
        );
    }
}
