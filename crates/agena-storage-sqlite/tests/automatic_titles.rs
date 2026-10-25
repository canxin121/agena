use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};

async fn fixture() -> (SqliteKernel, SessionId) {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime.workspaces.ensure_id("/title-tests").await.unwrap();
    let session = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "New session".into(),
            SessionConfiguration {
                title_state: SessionTitleState::automatic(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    send(&runtime, session, "OAuth callback failure").await;
    (runtime, session)
}

async fn send(runtime: &SqliteKernel, session: SessionId, text: &str) {
    runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
        )
        .await
        .unwrap();
}

async fn update(runtime: &SqliteKernel, session: SessionId, title: Option<&str>) -> MetadataChange {
    let snapshot = runtime
        .ports
        .reader
        .control_snapshot(session, 1)
        .await
        .unwrap();
    let history = runtime
        .ports
        .reader
        .history(session, None, 32)
        .await
        .unwrap();
    let last = history
        .entries
        .iter()
        .rev()
        .find(|entry| matches!(entry.payload, EntryPayload::User { .. }))
        .unwrap();
    MetadataChange {
        automatic_title: Some(AutomaticTitleUpdate {
            expected_metadata_version: snapshot.metadata.metadata_version,
            expected_membership_generation: snapshot.metadata.membership_generation,
            user_message_count: snapshot.summary.user_message_count,
            last_user_message: last.id,
            last_user_revision: last.revision,
            title: title.map(str::to_owned),
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn naming_and_keep_results_persist_without_transcript_entries() {
    let (runtime, session) = fixture().await;
    let request = RequestId::new();
    let change = update(&runtime, session, Some("OAuth 回调配置")).await;
    let receipt = runtime
        .ports
        .sessions
        .update_metadata(request, session, change.clone())
        .await
        .unwrap();
    let replay = runtime
        .ports
        .sessions
        .update_metadata(request, session, change)
        .await
        .unwrap();
    assert_eq!(receipt.value, replay.value);
    let config = runtime.ports.reader.configuration(session).await.unwrap();
    assert_eq!(config.title_state.source, SessionTitleSource::Automatic);
    assert_eq!(config.title_state.checked_user_messages, 1);
    assert!(!config.title_state.calibrated);
    assert!(config.title_state.checked_at_ms > 0);
    send(&runtime, session, "The callback URL differs").await;
    send(&runtime, session, "Apply the same callback URL").await;
    let keep = update(&runtime, session, None).await;
    runtime
        .ports
        .sessions
        .update_metadata(RequestId::new(), session, keep)
        .await
        .unwrap();
    let snapshot = runtime
        .ports
        .reader
        .control_snapshot(session, 1)
        .await
        .unwrap();
    assert_eq!(snapshot.metadata.title, "OAuth 回调配置");
    assert_eq!(snapshot.configuration.title_state.checked_user_messages, 3);
    assert!(snapshot.configuration.title_state.calibrated);
    assert_eq!(snapshot.summary.entry_count, 3);
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn manual_same_name_locks_and_fences_background_generation() {
    let (runtime, session) = fixture().await;
    let pending = update(&runtime, session, Some("Unwanted replacement")).await;
    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                title: Some("New session".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let result = runtime
        .ports
        .sessions
        .update_metadata(RequestId::new(), session, pending)
        .await;
    assert!(matches!(result, Err(KernelError::Conflict(_))));
    // A fresh metadata version must not bypass the manual-name lock either.
    let fresh = update(&runtime, session, Some("Still unwanted")).await;
    assert!(matches!(
        runtime
            .ports
            .sessions
            .update_metadata(RequestId::new(), session, fresh)
            .await,
        Err(KernelError::Conflict(_))
    ));
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(session)
            .await
            .unwrap()
            .title_state
            .source,
        SessionTitleSource::Manual
    );
    assert_eq!(
        runtime.ports.reader.metadata(session).await.unwrap().title,
        "New session"
    );
    // An explicit opt-in is the only way to resume naming on this session.
    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                auto_title: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let fresh = update(&runtime, session, Some("Resumed automatic naming")).await;
    runtime
        .ports
        .sessions
        .update_metadata(RequestId::new(), session, fresh)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn new_input_and_other_metadata_changes_reject_stale_results() {
    let (runtime, session) = fixture().await;
    let pending = update(&runtime, session, Some("Old topic")).await;
    send(&runtime, session, "Switch to database migrations").await;
    assert!(matches!(
        runtime
            .ports
            .sessions
            .update_metadata(RequestId::new(), session, pending)
            .await,
        Err(KernelError::Conflict(_))
    ));
    let pending = update(&runtime, session, Some("Database migrations")).await;
    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                favorite: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        runtime
            .ports
            .sessions
            .update_metadata(RequestId::new(), session, pending)
            .await,
        Err(KernelError::Conflict(_))
    ));
    let fresh = update(&runtime, session, Some("Database migrations")).await;
    runtime
        .ports
        .sessions
        .update_metadata(RequestId::new(), session, fresh)
        .await
        .unwrap();
    assert!(
        runtime
            .ports
            .reader
            .metadata(session)
            .await
            .unwrap()
            .favorite
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn editing_queued_input_rejects_the_previous_title_result() {
    let (runtime, session) = fixture().await;
    let pending = update(&runtime, session, Some("OAuth callback")).await;
    let history = runtime
        .ports
        .reader
        .history(session, None, 1)
        .await
        .unwrap();
    let input = &history.entries[0];
    runtime
        .ports
        .sessions
        .edit_queued_with_blobs(
            RequestId::new(),
            session,
            input.execution_id,
            input.revision,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Investigate database migration rollback".into(),
            }]),
            &[],
        )
        .await
        .unwrap();
    assert!(matches!(
        runtime
            .ports
            .sessions
            .update_metadata(RequestId::new(), session, pending)
            .await,
        Err(KernelError::Conflict(_))
    ));
    let fresh = update(&runtime, session, Some("Database migration rollback")).await;
    runtime
        .ports
        .sessions
        .update_metadata(RequestId::new(), session, fresh)
        .await
        .unwrap();
    assert_eq!(
        runtime.ports.reader.metadata(session).await.unwrap().title,
        "Database migration rollback"
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn title_state_survives_database_reopen_and_legacy_config_stays_manual() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("titles.db").display()
    );
    let runtime = open_kernel_runtime(&url).await.unwrap();
    let workspace = runtime.workspaces.ensure_id("/title-tests").await.unwrap();
    let session = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Placeholder".into(),
            SessionConfiguration {
                title_state: SessionTitleState::automatic(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    send(&runtime, session, "Fix the connection pool").await;
    let change = update(&runtime, session, Some("Connection pool")).await;
    runtime
        .ports
        .sessions
        .update_metadata(RequestId::new(), session, change)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
    // SQLx's retained read pools own the file lock; close every pool before reopening.
    drop(runtime);
    let runtime = open_kernel_runtime(&url).await.unwrap();
    assert_eq!(
        runtime.ports.reader.metadata(session).await.unwrap().title,
        "Connection pool"
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(session)
            .await
            .unwrap()
            .title_state
            .checked_user_messages,
        1
    );
    let legacy: SessionConfiguration = serde_json::from_str("{}").unwrap();
    assert_eq!(legacy.title_state.source, SessionTitleSource::Manual);
    runtime.ports.clone().shutdown().await.unwrap();
}
