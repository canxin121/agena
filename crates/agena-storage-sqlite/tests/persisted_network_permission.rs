use agena_domain::{
    ComposerDocument, ComposerNode, PermissionAction, PermissionMode, PermissionReply,
    PermissionReplyKind, PermissionScope,
};
#[cfg(not(target_os = "macos"))]
use agena_storage::PermissionRuleBatchScope;
use agena_storage::kernel::*;
use agena_storage::{
    PermissionRuleListQuery, PermissionRuleRepositoryError, PersistedPermissionRule,
};
use agena_storage_sqlite::{SeaPermissionRuleTransactionWriter, open_kernel_runtime};
use sea_orm::TransactionTrait;

fn rule(action: PermissionAction, mode: PermissionMode) -> PersistedPermissionRule {
    PersistedPermissionRule {
        id: None,
        created_at_ms: None,
        updated_at_ms: None,
        action_key: serde_json::to_string(&action).unwrap(),
        mode,
        scope: PermissionScope::Global,
        session_id: None,
        workspace_id: None,
        source: "native-network-test".into(),
        reason: None,
        operator: None,
        revoked_at_ms: None,
        revoked_reason: None,
        revoked_by: None,
    }
}

fn network(target: &str, host: &str, port: Option<u16>) -> PermissionAction {
    PermissionAction::NetworkAccess {
        target: target.into(),
        host: host.into(),
        port,
    }
}

fn tool_rule() -> PersistedPermissionRule {
    rule(
        PermissionAction::Tool {
            tool_name: "agena.fs.read".into(),
            qualifier: None,
        },
        PermissionMode::Allow,
    )
}

fn list_query() -> PermissionRuleListQuery {
    PermissionRuleListQuery {
        search: None,
        workspace_id: None,
        before_updated_at_ms: None,
        before_id: None,
        limit: 100,
    }
}

#[tokio::test]
async fn invalid_network_upserts_replacements_and_transaction_writes_leave_storage_unchanged() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let original = runtime.permissions.upsert(&tool_rule()).await.unwrap().0;
    for invalid in [
        rule(
            network("example.com:443", "example.com", Some(443)),
            PermissionMode::Deny,
        ),
        rule(
            network("192.0.2.7:443", "192.0.2.7", Some(443)),
            PermissionMode::Ask,
        ),
        rule(
            network("192.0.2.7:443", "192.0.2.7", Some(443)),
            PermissionMode::Auto,
        ),
        rule(
            network("https://192.0.2.7/private", "192.0.2.7", Some(443)),
            PermissionMode::Deny,
        ),
        rule(
            network("192.0.2.7:443", "192.0.2.8", Some(443)),
            PermissionMode::Deny,
        ),
    ] {
        assert!(matches!(
            runtime.permissions.upsert(&invalid).await,
            Err(PermissionRuleRepositoryError::InvalidInput(_))
        ));
        assert!(matches!(
            runtime.permissions.replace(original.id, &invalid).await,
            Err(PermissionRuleRepositoryError::InvalidInput(_))
        ));
        let txn = runtime.database.begin().await.unwrap();
        assert!(matches!(
            SeaPermissionRuleTransactionWriter::upsert_in_transaction(&txn, &invalid).await,
            Err(PermissionRuleRepositoryError::InvalidInput(_))
        ));
        txn.rollback().await.unwrap();
        let stored = runtime.permissions.get(original.id).await.unwrap().unwrap();
        assert_eq!(stored.action_key, original.action_key);
        assert_eq!(stored.mode, original.mode);
        assert_eq!(stored.updated_at_ms, original.updated_at_ms);
        assert_eq!(
            runtime.permissions.list(list_query()).await.unwrap().len(),
            1
        );
    }
    runtime.ports.clone().shutdown().await.unwrap();
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn mixed_mode_batches_validate_every_network_rule_before_updating_any_row() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let tool = runtime.permissions.upsert(&tool_rule()).await.unwrap().0;
    let network = runtime
        .permissions
        .upsert(&rule(
            network("192.0.2.7:443", "192.0.2.7", Some(443)),
            PermissionMode::Deny,
        ))
        .await
        .unwrap()
        .0;
    let scope = PermissionRuleBatchScope {
        scope: PermissionScope::Global,
        workspace_id: None,
        session_id: None,
    };
    for mode in [PermissionMode::Ask, PermissionMode::Auto] {
        let error = runtime
            .permissions
            .set_mode_many(&[tool.id, network.id], mode, scope)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            PermissionRuleRepositoryError::InvalidInput(_)
        ));
        for original in [&tool, &network] {
            let stored = runtime.permissions.get(original.id).await.unwrap().unwrap();
            assert_eq!(stored.mode, original.mode);
            assert_eq!(stored.updated_at_ms, original.updated_at_ms);
        }
    }
    let updated = runtime
        .permissions
        .set_mode_many(&[tool.id, network.id], PermissionMode::Allow, scope)
        .await
        .unwrap();
    assert_eq!(updated.len(), 2);
    assert!(updated.iter().all(|row| row.mode == "allow"));
    runtime.ports.clone().shutdown().await.unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn macos_cannot_persist_a_specific_ip_network_decision() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    for mode in [PermissionMode::Allow, PermissionMode::Deny] {
        let error = runtime
            .permissions
            .upsert(&rule(
                network("192.0.2.7:443", "192.0.2.7", Some(443)),
                mode,
            ))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Seatbelt"), "{error}");
    }
    assert!(
        runtime
            .permissions
            .list(list_query())
            .await
            .unwrap()
            .is_empty()
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn unsupported_remembered_network_reply_rolls_back_the_interaction_too() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/native-network-rule-tests")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Native decisions".into())
        .await
        .unwrap()
        .value
        .id;
    let execution = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Check a permission decision".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "network-decision".into(),
                name: "network".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let interaction = tool
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: serde_json::to_string(&network(
                    "example.com:443",
                    "example.com",
                    Some(443),
                ))
                .unwrap(),
                description: "Remember an unsupported hostname decision".into(),
            },
        )
        .await
        .unwrap()
        .value;
    let before = runtime
        .ports
        .reader
        .interaction(session, interaction)
        .await
        .unwrap();
    let error = runtime
        .ports
        .sessions
        .reply(
            RequestId::new(),
            session,
            interaction,
            InteractionDecision::Permission {
                reply: PermissionReply {
                    request_id: interaction.get().to_string(),
                    kind: PermissionReplyKind::DenyAlways,
                    reason: None,
                    scope: Some(PermissionScope::Global),
                },
                allowed: false,
            },
            Some(RememberPermission {
                mode: PermissionMode::Deny,
                scope: PermissionScope::Global,
            }),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("literal IP"), "{error}");
    let after = runtime
        .ports
        .reader
        .interaction(session, interaction)
        .await
        .unwrap();
    assert_eq!(after.decision, before.decision);
    assert_eq!(after.remembered, before.remembered);
    assert!(
        runtime
            .permissions
            .list(list_query())
            .await
            .unwrap()
            .is_empty()
    );
    runtime
        .ports
        .sessions
        .cancel(RequestId::new(), session, execution.id)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}
