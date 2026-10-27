use agena_domain::{AgentStatus, ComposerDocument, ComposerNode, SessionRelationKind};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};

async fn fixture(prompt: Option<String>) -> (SqliteKernel, i64, SessionId) {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime.workspaces.ensure_id("/prompt-tests").await.unwrap();
    let session = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Prompt test".into(),
            SessionConfiguration {
                additional_system_prompt: prompt,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    (runtime, workspace, session)
}

#[test]
fn old_configuration_and_serialized_updates_preserve_inheritance_states() {
    let old: SessionConfiguration = serde_json::from_str("{}").unwrap();
    assert_eq!(old.additional_system_prompt, None);
    for expected in [
        None,
        Some(None),
        Some(Some(String::new())),
        Some(Some("custom".into())),
    ] {
        let metadata = MetadataChange {
            additional_system_prompt: expected.clone(),
            ..Default::default()
        };
        let json = serde_json::to_value(&metadata).unwrap();
        assert_eq!(
            json.get("additional_system_prompt").is_some(),
            expected.is_some()
        );
        let restored: MetadataChange = serde_json::from_value(json).unwrap();
        assert_eq!(restored.additional_system_prompt, expected);
        let change = SessionConfigurationChange {
            additional_system_prompt: expected.clone(),
            ..Default::default()
        };
        let restored: SessionConfigurationChange =
            serde_json::from_value(serde_json::to_value(change).unwrap()).unwrap();
        assert_eq!(restored.additional_system_prompt, expected);
    }
}

#[tokio::test]
async fn metadata_updates_keep_empty_and_restore_inheritance() {
    let (runtime, _, session) = fixture(Some("initial".into())).await;
    let original = runtime.ports.reader.metadata(session).await.unwrap();
    for expected in [Some("  exact text\n".to_owned()), Some(String::new()), None] {
        let receipt = runtime
            .ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    additional_system_prompt: Some(expected.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(receipt.value.metadata_version > original.metadata_version);
        assert_eq!(
            runtime
                .ports
                .reader
                .configuration(session)
                .await
                .unwrap()
                .additional_system_prompt,
            expected
        );
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
        assert_eq!(
            runtime
                .ports
                .reader
                .configuration(session)
                .await
                .unwrap()
                .additional_system_prompt,
            expected
        );
    }
    runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                additional_system_prompt: Some(Some(String::new())),
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
            .additional_system_prompt
            .as_deref(),
        Some("")
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn oversized_prompt_rolls_back_the_entire_metadata_update() {
    let (runtime, _, session) = fixture(Some("initial".into())).await;
    let original = runtime.ports.reader.metadata(session).await.unwrap();
    let result = runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                title: Some("Must not be committed".into()),
                additional_system_prompt: Some(Some("x".repeat(65536))),
                ..Default::default()
            },
        )
        .await;
    assert!(result.is_err());
    assert_eq!(
        runtime.ports.reader.metadata(session).await.unwrap(),
        original
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(session)
            .await
            .unwrap()
            .additional_system_prompt
            .as_deref(),
        Some("initial")
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn branches_copy_raw_overrides_and_new_children_start_with_inheritance() {
    for expected in [
        None,
        Some(String::new()),
        Some("parent instructions".to_owned()),
    ] {
        let (runtime, workspace, parent) = fixture(expected.clone()).await;
        let execution = runtime
            .ports
            .sessions
            .send(
                RequestId::new(),
                parent,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "A completed turn".into(),
                }]),
            )
            .await
            .unwrap()
            .value;
        runtime
            .ports
            .sessions
            .cancel(RequestId::new(), parent, execution.id)
            .await
            .unwrap();
        let history = runtime.ports.reader.history(parent, None, 8).await.unwrap();
        let message = history
            .entries
            .iter()
            .find(|entry| matches!(entry.payload, EntryPayload::User { .. }))
            .unwrap()
            .id;
        let fork = runtime
            .ports
            .sessions
            .fork(RequestId::new(), parent, message)
            .await
            .unwrap()
            .value
            .id;
        let rewind = runtime
            .ports
            .sessions
            .rewind(RequestId::new(), parent, message)
            .await
            .unwrap()
            .value
            .session
            .id;
        for branch in [fork, rewind] {
            assert_eq!(
                runtime
                    .ports
                    .reader
                    .configuration(branch)
                    .await
                    .unwrap()
                    .additional_system_prompt,
                expected
            );
        }
        for relation_kind in [SessionRelationKind::Child, SessionRelationKind::Subagent] {
            let child = runtime
                .ports
                .sessions
                .create_configured(
                    RequestId::new(),
                    workspace,
                    "Child".into(),
                    SessionConfiguration {
                        parent_session_id: Some(parent),
                        relation_kind,
                        agent: relation_kind
                            .is_subagent()
                            .then(|| SessionAgentConfiguration {
                                name: "child".into(),
                                status: AgentStatus::Running,
                                started_at_ms: Some(1),
                                finished_at_ms: None,
                                failure: None,
                                limits: AgentExecutionLimits::default(),
                            }),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .value
                .id;
            assert_eq!(
                runtime
                    .ports
                    .reader
                    .configuration(child)
                    .await
                    .unwrap()
                    .additional_system_prompt,
                None
            );
        }
        runtime.ports.clone().shutdown().await.unwrap();
    }
}
