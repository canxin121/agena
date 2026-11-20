//! A profile reference is durable session configuration, guarded like policy changes.
use agena_domain::{
    ComposerDocument, ComposerNode, ExecutionSelection, PathAccessModes, PathPermissionConfig,
    PathPermissionMode, PermissionConfig,
};
use agena_prompt_layers::{FragmentId, PromptFragment, PromptLayer};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};

fn configuration() -> SessionConfiguration {
    let fragment = FragmentId::parse("session-note").unwrap();
    SessionConfiguration {
        profile: Some("reviewer".into()),
        selection_defaults: ExecutionSelection {
            provider: Some("profile-parent".into()),
            model: Some("parent-model".into()),
            thinking_mode: Some("high".into()),
            ..Default::default()
        },
        prompts: PromptLayer {
            order: vec![fragment.clone()],
            inline: vec![PromptFragment::new(
                fragment,
                "Keep the explicit session instruction.",
            )],
            ..Default::default()
        },
        permission: serde_json::from_value::<PermissionConfig>(serde_json::json!({
            "path":{"external":"deny"}
        }))
        .unwrap(),
        permission_ceiling: serde_json::from_value::<PermissionConfig>(serde_json::json!({
            "path":{"workspace":"readonly"}
        }))
        .unwrap(),
        permission_ceiling_layers: vec![
            serde_json::from_value::<PermissionConfig>(serde_json::json!({
                "path":{"external":"deny"}
            }))
            .unwrap(),
        ],
        ..Default::default()
    }
}

async fn create(runtime: &SqliteKernel) -> SessionId {
    let workspace = runtime
        .workspaces
        .ensure_id("/profile-tests")
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Profile session".into(),
            configuration(),
        )
        .await
        .unwrap()
        .value
        .id
}

async fn send(runtime: &SqliteKernel, session: SessionId) -> ExecutionView {
    runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Inspect the code".into(),
            }]),
        )
        .await
        .unwrap()
        .value
}

#[tokio::test]
async fn profile_survives_restart_and_legacy_configuration_has_no_profile() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("profiles.db").display()
    );
    let runtime = open_kernel_runtime(&url).await.unwrap();
    let session = create(&runtime).await;
    runtime.ports.clone().shutdown().await.unwrap();
    drop(runtime);
    let runtime = open_kernel_runtime(&url).await.unwrap();
    let stored = runtime.ports.reader.configuration(session).await.unwrap();
    assert_eq!(stored.profile.as_deref(), Some("reviewer"));
    assert_eq!(
        stored.selection_defaults,
        configuration().selection_defaults
    );
    assert_eq!(stored.prompts, configuration().prompts);
    assert_eq!(
        stored.permission_ceiling,
        configuration().permission_ceiling
    );
    assert_eq!(
        stored.permission_ceiling_layers,
        configuration().permission_ceiling_layers
    );
    let legacy: SessionConfiguration = serde_json::from_str("{}").unwrap();
    assert!(legacy.profile.is_none());
    assert!(legacy.selection_defaults.is_empty());
    assert!(legacy.permission_ceiling_layers.is_empty());
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn switching_and_clearing_profiles_preserves_explicit_session_settings() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let session = create(&runtime).await;
    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                profile: Some("developer".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let changed = runtime.ports.reader.configuration(session).await.unwrap();
    assert_eq!(changed.profile.as_deref(), Some("developer"));
    assert_eq!(
        changed.selection_defaults,
        configuration().selection_defaults
    );
    assert_eq!(changed.prompts, configuration().prompts);
    assert_eq!(changed.permission, configuration().permission);
    assert_eq!(
        changed.permission_ceiling,
        configuration().permission_ceiling
    );
    assert_eq!(
        changed.permission_ceiling_layers,
        configuration().permission_ceiling_layers
    );

    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                title: Some("Still developer".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(session)
            .await
            .unwrap()
            .profile
            .as_deref(),
        Some("developer")
    );

    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                profile: Some(String::new()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        runtime
            .ports
            .reader
            .configuration(session)
            .await
            .unwrap()
            .profile
            .is_none()
    );

    runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                profile: Some(Some("reviewer".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                profile: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let cleared = runtime.ports.reader.configuration(session).await.unwrap();
    assert!(cleared.profile.is_none());
    assert_eq!(
        cleared.selection_defaults,
        configuration().selection_defaults
    );
    assert_eq!(
        cleared.permission_ceiling,
        configuration().permission_ceiling
    );
    assert_eq!(
        cleared.permission_ceiling_layers,
        configuration().permission_ceiling_layers
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn inherited_permission_layers_can_only_be_extended_with_valid_policies() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let session = create(&runtime).await;
    let original = runtime.ports.reader.configuration(session).await.unwrap();
    let metadata = runtime.ports.reader.metadata(session).await.unwrap();
    // Typed callers must pass storage validation even when they bypass serde.
    let invalid = PermissionConfig {
        path: Some(PathPermissionConfig {
            rules: [(
                "<workspace>/*.key".into(),
                PathAccessModes::from_mode(PathPermissionMode::Readonly),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let broader: PermissionConfig = serde_json::from_value(serde_json::json!({
        "path":{"external":"readwrite"}
    }))
    .unwrap();
    let mut invalid_extension = original.permission_ceiling_layers.clone();
    invalid_extension.push(invalid);
    for layers in [Vec::new(), vec![broader], invalid_extension] {
        let result = runtime
            .ports
            .sessions
            .configure(
                RequestId::new(),
                session,
                SessionConfigurationChange {
                    profile: Some(Some("must-roll-back".into())),
                    permission_ceiling_layers: Some(layers),
                    ..Default::default()
                },
            )
            .await;
        assert!(result.is_err());
        let stored = runtime.ports.reader.configuration(session).await.unwrap();
        assert_eq!(stored.profile, original.profile);
        assert_eq!(
            stored.permission_ceiling_layers,
            original.permission_ceiling_layers
        );
        assert_eq!(
            runtime
                .ports
                .reader
                .metadata(session)
                .await
                .unwrap()
                .metadata_version,
            metadata.metadata_version
        );
    }
    let mut extended = original.permission_ceiling_layers.clone();
    extended.push(original.permission_ceiling.clone());
    runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                permission_ceiling_layers: Some(extended.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(session)
            .await
            .unwrap()
            .permission_ceiling_layers,
        extended
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn stale_revision_rejects_profile_and_title_as_one_atomic_update() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let session = create(&runtime).await;
    let version = runtime
        .ports
        .reader
        .metadata(session)
        .await
        .unwrap()
        .metadata_version
        .get();
    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                title: Some("Concurrent title".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let error = runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                expected_version: Some(version),
                profile: Some("developer".into()),
                title: Some("Stale title".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, KernelError::Conflict(_)));
    assert_eq!(
        runtime.ports.reader.metadata(session).await.unwrap().title,
        "Concurrent title"
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(session)
            .await
            .unwrap()
            .profile
            .as_deref(),
        Some("reviewer")
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn queued_and_running_work_blocks_switching_but_allows_idempotent_selection() {
    for running in [false, true] {
        let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
        let session = create(&runtime).await;
        let execution = send(&runtime, session).await;
        if running {
            runtime
                .ports
                .executor
                .claim_next(RequestId::new(), session, execution.id)
                .await
                .unwrap();
        }
        let error = runtime
            .ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    profile: Some("developer".into()),
                    title: Some("Must roll back".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, KernelError::Conflict(_)));
        assert_eq!(
            runtime.ports.reader.metadata(session).await.unwrap().title,
            "Profile session"
        );
        assert_eq!(
            runtime
                .ports
                .reader
                .configuration(session)
                .await
                .unwrap()
                .profile
                .as_deref(),
            Some("reviewer")
        );

        runtime
            .ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    profile: Some("reviewer".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let error = runtime
            .ports
            .sessions
            .configure(
                RequestId::new(),
                session,
                SessionConfigurationChange {
                    profile: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, KernelError::Conflict(_)));
        runtime
            .ports
            .sessions
            .cancel(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        runtime
            .ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    profile: Some("developer".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            runtime
                .ports
                .reader
                .configuration(session)
                .await
                .unwrap()
                .profile
                .as_deref(),
            Some("developer")
        );
        runtime.ports.clone().shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn forks_and_rewinds_keep_the_profile_and_permission_ceiling() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let parent = create(&runtime).await;
    let execution = send(&runtime, parent).await;
    runtime
        .ports
        .sessions
        .cancel(RequestId::new(), parent, execution.id)
        .await
        .unwrap();
    let fork = runtime
        .ports
        .sessions
        .fork(RequestId::new(), parent, execution.message_id)
        .await
        .unwrap()
        .value
        .id;
    let rewind = runtime
        .ports
        .sessions
        .rewind(RequestId::new(), parent, execution.message_id)
        .await
        .unwrap()
        .value
        .session
        .id;
    for branch in [fork, rewind] {
        let stored = runtime.ports.reader.configuration(branch).await.unwrap();
        assert_eq!(stored.profile.as_deref(), Some("reviewer"));
        assert_eq!(
            stored.selection_defaults,
            configuration().selection_defaults
        );
        assert_eq!(
            stored.permission_ceiling,
            configuration().permission_ceiling
        );
        assert_eq!(
            stored.permission_ceiling_layers,
            configuration().permission_ceiling_layers
        );
        assert_eq!(stored.prompts, configuration().prompts);
    }
    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            fork,
            MetadataChange {
                profile: Some("developer".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(parent)
            .await
            .unwrap()
            .profile
            .as_deref(),
        Some("reviewer")
    );
    runtime.ports.clone().shutdown().await.unwrap();
}
