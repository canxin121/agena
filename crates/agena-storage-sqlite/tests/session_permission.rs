use agena_domain::{PathAccessModes, PathPermissionConfig, PathPermissionMode, PermissionConfig};
use agena_storage::kernel::*;
use agena_storage_sqlite::open_kernel_runtime;

fn paths(rules: &[(&str, PathPermissionMode)]) -> PermissionConfig {
    PermissionConfig {
        path: Some(PathPermissionConfig {
            rules: rules
                .iter()
                .map(|(path, mode)| ((*path).into(), PathAccessModes::from_mode(*mode)))
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn invalid_permission_changes_roll_back_configuration_and_metadata() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/permission-tests")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Permission storage".into(),
            SessionConfiguration {
                permission: paths(&[("output", PathPermissionMode::Readwrite)]),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let original = runtime.ports.reader.configuration(session).await.unwrap();
    assert_eq!(
        serde_json::to_value(&original.permission).unwrap()["path"]["rules"]["<workspace>/output"],
        "readwrite",
    );
    let metadata = runtime.ports.reader.metadata(session).await.unwrap();

    for permission in [
        paths(&[("<workspace>/*.key", PathPermissionMode::Readonly)]),
        paths(&[
            ("<workspace>/secret", PathPermissionMode::Deny),
            ("<workspace>/secret/key", PathPermissionMode::Readwrite),
        ]),
    ] {
        for ceiling in [false, true] {
            let change = SessionConfigurationChange {
                permission: (!ceiling).then_some(permission.clone()),
                permission_ceiling: ceiling.then_some(permission.clone()),
                // This unrelated field must not partially commit on failure.
                message_queue_paused: Some(true),
                ..Default::default()
            };
            let error = runtime
                .ports
                .sessions
                .configure(RequestId::new(), session, change)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("path"), "{error}");
            assert_eq!(
                runtime.ports.reader.configuration(session).await.unwrap(),
                original
            );
            assert_eq!(
                runtime
                    .ports
                    .reader
                    .metadata(session)
                    .await
                    .unwrap()
                    .metadata_version,
                metadata.metadata_version,
            );
        }
    }
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn writable_children_of_readonly_paths_round_trip_in_permissions_and_ceilings() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/permission-tests")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Path overrides".into())
        .await
        .unwrap()
        .value
        .id;
    // Specific writable children are supported overrides of readonly paths;
    // only explicit deny boundaries and invalid paths belong in the test above.
    let permission = paths(&[
        ("<workspace>/readonly", PathPermissionMode::Readonly),
        ("<workspace>/readonly/write", PathPermissionMode::Readwrite),
    ]);
    runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                permission: Some(permission.clone()),
                permission_ceiling: Some(permission.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let stored = runtime.ports.reader.configuration(session).await.unwrap();
    assert_eq!(stored.permission, permission);
    assert_eq!(stored.permission_ceiling, permission);
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_permission_creation_does_not_leave_a_session() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/permission-tests")
        .await
        .unwrap();
    let invalid = SessionConfiguration {
        permission: paths(&[("<workspace>/**/*.rs", PathPermissionMode::Readwrite)]),
        ..Default::default()
    };
    assert!(
        runtime
            .ports
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "Rejected project".into(),
                invalid.clone(),
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .ports
            .sessions
            .create_global(RequestId::new(), "Rejected global".into(), invalid,)
            .await
            .is_err()
    );
    assert!(
        runtime
            .ports
            .reader
            .query_summaries(SummaryQuery::default())
            .await
            .unwrap()
            .sessions
            .is_empty()
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn validated_archive_rejection_happens_before_publication() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/permission-tests")
        .await
        .unwrap();
    let permission = paths(&[("<workspace>/output", PathPermissionMode::Readwrite)]);
    let source = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Archive permissions".into(),
            SessionConfiguration {
                permission: permission.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("permission.jsonl");
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    runtime
        .ports
        .reader
        .export_jsonl(source, &mut file)
        .await
        .unwrap();
    file.sync_all().await.unwrap();
    drop(file);

    let checks = Arc::new(AtomicUsize::new(0));
    let seen = checks.clone();
    let expected = permission.clone();
    let rejected: SessionImportValidator = Arc::new(move |configuration| {
        assert_eq!(configuration.permission, expected);
        seen.fetch_add(1, Ordering::SeqCst);
        Err("host permission policy rejects this archive".into())
    });
    let request = RequestId::new();
    let error = runtime
        .ports
        .sessions
        .import_archive_validated(request, workspace, &path, rejected)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("host permission policy rejects"),
        "{error}"
    );
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    assert_eq!(
        runtime
            .ports
            .reader
            .query_summaries(SummaryQuery::default())
            .await
            .unwrap()
            .sessions
            .len(),
        1
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(source)
            .await
            .unwrap()
            .permission,
        permission
    );

    // A failed validator creates no durable request receipt or session. The
    // same request can later import exactly the configuration it validated.
    let expected = permission.clone();
    let accepted: SessionImportValidator = Arc::new(move |configuration| {
        assert_eq!(configuration.permission, expected);
        Ok(())
    });
    let imported = runtime
        .ports
        .sessions
        .import_archive_validated(request, workspace, &path, accepted)
        .await
        .unwrap()
        .value
        .id;
    assert_ne!(imported, source);
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(imported)
            .await
            .unwrap()
            .permission,
        permission
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .query_summaries(SummaryQuery::default())
            .await
            .unwrap()
            .sessions
            .len(),
        2
    );
    runtime.ports.clone().shutdown().await.unwrap();
}
