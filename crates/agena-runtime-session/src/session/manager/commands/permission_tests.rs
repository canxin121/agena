use super::queue_tests::fixture_with_config;
use super::*;
use agena_domain::{PathAccessModes, PathPermissionConfig, PathPermissionMode, PermissionConfig};

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
async fn invalid_session_permission_does_not_change_saved_configuration_or_version() {
    let (manager, session) = fixture_with_config(Default::default()).await;
    let saved = paths(&[("<workspace>/output", PathPermissionMode::Readwrite)]);
    manager
        .set_session_permission(session.get(), saved.clone())
        .await
        .unwrap();
    let original = manager
        .kernel()
        .reader
        .configuration(session)
        .await
        .unwrap();
    let version = manager
        .kernel()
        .reader
        .metadata(session)
        .await
        .unwrap()
        .metadata_version;

    let invalid_network: PermissionConfig = serde_json::from_value(serde_json::json!({
        "network": { "egress": { "allow": [{ "to": [{ "cidr": "*.example.com/32" }] }] } }
    }))
    .unwrap();
    let invalid_tool: PermissionConfig = serde_json::from_value(serde_json::json!({
        "tools": { "rules": { "agena.fs.read": { "*.txt": "allow" } } }
    }))
    .unwrap();
    let mut approval_mode = paths(&[]);
    approval_mode.path.as_mut().unwrap().workspace = Some(PathAccessModes {
        read: Some(PermissionMode::Allow),
        write: Some(PermissionMode::Ask),
    });
    for (permission, field) in [
        (
            paths(&[("<workspace>/**/*.env", PathPermissionMode::Deny)]),
            "path",
        ),
        (
            paths(&[
                ("<workspace>/secrets", PathPermissionMode::Deny),
                ("<workspace>/secrets/key", PathPermissionMode::Readonly),
            ]),
            "path",
        ),
        (approval_mode, "path"),
        (invalid_network, "network"),
        (invalid_tool, "tool"),
    ] {
        let error = SessionExecutionCommandService::set_session_permission(
            &manager,
            session.get(),
            permission,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains(field), "{error}");
        assert_eq!(
            manager
                .kernel()
                .reader
                .configuration(session)
                .await
                .unwrap(),
            original
        );
        assert_eq!(
            manager
                .kernel()
                .reader
                .metadata(session)
                .await
                .unwrap()
                .metadata_version,
            version
        );
    }
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn public_session_permission_conflicts_keep_the_field_and_both_paths() {
    let (manager, session) = fixture_with_config(Default::default()).await;
    let error = SessionExecutionCommandService::set_session_permission(
        &manager,
        session.get(),
        paths(&[
            ("<workspace>/secrets", PathPermissionMode::Deny),
            ("<workspace>/secrets/key", PathPermissionMode::Readonly),
        ]),
    )
    .await
    .unwrap_err();
    let message = error.to_string();
    for detail in [
        "session.permission",
        "permission.path.rules",
        "<workspace>/secrets/key",
        r#"conflicts with "<workspace>/secrets""#,
        "an explicitly denied directory",
    ] {
        assert!(message.contains(detail), "missing {detail:?}: {message}");
    }
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn session_permission_checks_the_merged_policy_before_saving() {
    let config = RuntimeSessionManagerConfig {
        permission: paths(&[("<workspace>/private", PathPermissionMode::Deny)]),
        ..Default::default()
    };
    let (manager, session) = fixture_with_config(config).await;
    let original = manager
        .kernel()
        .reader
        .configuration(session)
        .await
        .unwrap();
    let error = manager
        .set_session_permission(
            session.get(),
            paths(&[(
                "<workspace>/private/report.txt",
                PathPermissionMode::Readwrite,
            )]),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("effective session.permission"),
        "{error}"
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .configuration(session)
            .await
            .unwrap(),
        original
    );
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn session_permission_aliases_resolve_in_the_owning_workspace() {
    let (manager, server_session) = fixture_with_config(Default::default()).await;
    let owner = manager
        .kernel_runtime
        .workspaces
        .ensure_id("/permission-tests/owner")
        .await
        .unwrap();
    let session = manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            owner,
            "Owning workspace".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let original = manager
        .kernel()
        .reader
        .configuration(session)
        .await
        .unwrap();
    let error = manager
        .set_session_permission(
            session.get(),
            paths(&[
                ("/permission-tests/owner/secrets", PathPermissionMode::Deny),
                ("<workspace>/secrets/key", PathPermissionMode::Readonly),
            ]),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("secrets"), "{error}");
    assert_eq!(
        manager
            .kernel()
            .reader
            .configuration(session)
            .await
            .unwrap(),
        original
    );

    // The absolute deny is outside the server session's workspace and does not
    // overlap its resource grant. This proves the target session selects scope.
    manager
        .set_session_permission(
            server_session.get(),
            paths(&[
                ("/permission-tests/owner/secrets", PathPermissionMode::Deny),
                ("<workspace>/secrets/key", PathPermissionMode::Readonly),
            ]),
        )
        .await
        .unwrap();
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn global_session_rejects_workspace_and_state_aliases_before_saving() {
    let config = RuntimeSessionManagerConfig {
        permission: PermissionConfig::global_default(),
        ..Default::default()
    };
    let (manager, _) = fixture_with_config(config).await;
    let session = manager
        .kernel()
        .sessions
        .create_global(RequestId::new(), "Global".into(), Default::default())
        .await
        .unwrap()
        .value
        .id;
    let original = manager
        .kernel()
        .reader
        .configuration(session)
        .await
        .unwrap();
    for (pattern, alias) in [
        ("input", "<workspace>"),
        ("<workspace>/input", "<workspace>"),
        ("<state>", "<state>"),
        ("<state>/output", "<state>"),
    ] {
        let error = manager
            .set_session_permission(
                session.get(),
                paths(&[(pattern, PathPermissionMode::Readwrite)]),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("global sessions have no {alias}")),
            "{error}"
        );
    }
    assert_eq!(
        manager
            .kernel()
            .reader
            .configuration(session)
            .await
            .unwrap(),
        original
    );
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn global_session_validates_host_paths_and_preserves_inherited_state_templates() {
    let base = PermissionConfig::global_default();
    let (manager, _) = fixture_with_config(RuntimeSessionManagerConfig {
        permission: base.clone(),
        ..Default::default()
    })
    .await;
    let session = manager
        .kernel()
        .sessions
        .create_global(
            RequestId::new(),
            "Global host paths".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let name = format!("agena-global-validation-{}", std::process::id());
    let target = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(&name);
    let target_text = target.to_string_lossy();
    let allowed = paths(&[(&target_text, PathPermissionMode::Readonly)]);
    manager
        .set_session_permission(session.get(), allowed.clone())
        .await
        .unwrap();
    let original = manager
        .kernel()
        .reader
        .configuration(session)
        .await
        .unwrap();
    let version = manager
        .kernel()
        .reader
        .metadata(session)
        .await
        .unwrap()
        .metadata_version;
    let alias = format!("<tmp>/{name}");
    let child = target.join("output.txt").to_string_lossy().into_owned();
    for (permission, reason) in [
        (
            paths(&[("<tmp>", PathPermissionMode::Readwrite)]),
            "shared OS temporary root",
        ),
        (
            paths(&[
                (&alias, PathPermissionMode::Readonly),
                (&child, PathPermissionMode::Readwrite),
            ]),
            "conflicts",
        ),
    ] {
        let error = manager
            .set_session_permission(session.get(), permission)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
        assert_eq!(
            manager
                .kernel()
                .reader
                .configuration(session)
                .await
                .unwrap(),
            original
        );
        assert_eq!(
            manager
                .kernel()
                .reader
                .metadata(session)
                .await
                .unwrap()
                .metadata_version,
            version
        );
    }
    let invalid_ceiling = SessionConfiguration {
        permission_ceiling: paths(&[("<tmp>", PathPermissionMode::Readonly)]),
        ..Default::default()
    };
    let error = manager
        .validate_session_permission_configuration(None, &invalid_ceiling)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("session.permission_ceiling"),
        "{error}"
    );
    assert!(
        error.to_string().contains("shared OS temporary root"),
        "{error}"
    );

    // Managed and delegated global sessions inherit both the effective policy
    // and its ceiling, including the factory's inactive <state> template.
    let inherited = base.merged_with(&allowed);
    let delegated = manager
        .kernel()
        .sessions
        .create_global(
            RequestId::new(),
            "Inherited global paths".into(),
            SessionConfiguration {
                permission: inherited.clone(),
                permission_ceiling: inherited.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let updated = inherited.merged_with(&paths(&[(&child, PathPermissionMode::Readonly)]));
    manager
        .set_session_permission(delegated.get(), updated)
        .await
        .unwrap();
    let saved = manager
        .kernel()
        .reader
        .configuration(delegated)
        .await
        .unwrap();
    assert_eq!(
        saved.permission.path.as_ref().unwrap().rules["<state>"],
        inherited.path.as_ref().unwrap().rules["<state>"],
    );
    assert!(
        saved
            .permission
            .path
            .as_ref()
            .unwrap()
            .rules
            .contains_key(&child)
    );
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn reordered_session_rules_are_persisted_and_advance_the_metadata_version() {
    let (manager, session) = fixture_with_config(Default::default()).await;
    let cases = [
        (
            r#"{"tools":{"rules":{"agena.shell.exec":{"git push *":"deny","git *":"ask"}}}}"#,
            r#"{"tools":{"rules":{"agena.shell.exec":{"git *":"ask","git push *":"deny"}}}}"#,
        ),
        #[cfg(not(target_os = "macos"))]
        (
            r#"{"network":{"egress":{"default":"deny","allow":[{"to":[{"cidr":"192.0.2.0/24"}]},{"to":[{"cidr":"198.51.100.0/24"}]}]}}}"#,
            r#"{"network":{"egress":{"default":"deny","allow":[{"to":[{"cidr":"198.51.100.0/24"}]},{"to":[{"cidr":"192.0.2.0/24"}]}]}}}"#,
        ),
    ];
    for (first, reordered) in cases {
        manager
            .set_session_permission(session.get(), serde_json::from_str(first).unwrap())
            .await
            .unwrap();
        let version = manager
            .kernel()
            .reader
            .metadata(session)
            .await
            .unwrap()
            .metadata_version;
        let desired: PermissionConfig = serde_json::from_str(reordered).unwrap();
        manager
            .set_session_permission(session.get(), desired.clone())
            .await
            .unwrap();
        let saved = manager
            .kernel()
            .reader
            .configuration(session)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_string(&saved.permission).unwrap(),
            serde_json::to_string(&desired).unwrap(),
        );
        assert!(
            manager
                .kernel()
                .reader
                .metadata(session)
                .await
                .unwrap()
                .metadata_version
                .get()
                > version.get(),
        );
    }
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn inherited_permission_ceiling_cannot_be_changed_by_reordering_rules() {
    let (manager, session) = fixture_with_config(Default::default()).await;
    let first = serde_json::from_str(
        r#"{"tools":{"rules":{"agena.shell.exec":{"git *":"ask","git push *":"deny"}}}}"#,
    )
    .unwrap();
    manager
        .kernel()
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                permission_ceiling: Some(first),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let original = manager
        .kernel()
        .reader
        .configuration(session)
        .await
        .unwrap();
    let version = manager
        .kernel()
        .reader
        .metadata(session)
        .await
        .unwrap()
        .metadata_version;
    let reordered = serde_json::from_str(
        r#"{"tools":{"rules":{"agena.shell.exec":{"git push *":"deny","git *":"ask"}}}}"#,
    )
    .unwrap();
    let error = manager
        .kernel()
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                permission_ceiling: Some(reordered),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("permission ceiling is immutable"),
        "{error}"
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .configuration(session)
            .await
            .unwrap(),
        original
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .metadata(session)
            .await
            .unwrap()
            .metadata_version,
        version
    );
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn session_creation_rejects_invalid_effective_rules_before_inserting() {
    let config = RuntimeSessionManagerConfig {
        permission: serde_json::from_value(serde_json::json!({
            "network": { "egress": { "allow": [{ "to": [{ "cidr": "https://example.com/private" }] }] } }
        }))
        .unwrap(),
        ..Default::default()
    };
    let (manager, existing) = fixture_with_config(config).await;
    let workspace = manager
        .kernel()
        .reader
        .metadata(existing)
        .await
        .unwrap()
        .workspace_id
        .unwrap();
    manager.workspace.set(workspace).unwrap();
    let before = manager
        .kernel()
        .reader
        .query_summaries(SummaryQuery::default())
        .await
        .unwrap();
    let error = manager
        .create_session(SessionCreateRequest {
            title: "Rejected configuration".into(),
            auto_title: false,
            parent_session_id: None,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("network"), "{error}");
    let after = manager
        .kernel()
        .reader
        .query_summaries(SummaryQuery::default())
        .await
        .unwrap();
    assert_eq!(after.sessions.len(), before.sessions.len());
    assert_eq!(after.sessions[0].id, before.sessions[0].id);
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn session_archive_rejects_invalid_network_rules_before_publication() {
    let (manager, existing) = fixture_with_config(Default::default()).await;
    let workspace = manager
        .kernel()
        .reader
        .metadata(existing)
        .await
        .unwrap()
        .workspace_id
        .unwrap();
    manager.workspace.set(workspace).unwrap();
    // Model an archive from an older host whose storage allowed this format.
    // Runtime policy must be applied to its exact parsed configuration.
    let source = manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Older archive".into(),
            SessionConfiguration {
                permission: serde_json::from_value(serde_json::json!({
                    "network": { "egress": { "allow": [{ "to": [{ "cidr": "*.example.com/32" }] }] } }
                }))
                .unwrap(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let path = std::env::temp_dir().join(format!(
        "agena-permission-import-{}.jsonl",
        uuid::Uuid::new_v4()
    ));
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    manager
        .kernel()
        .reader
        .export_jsonl(source, &mut file)
        .await
        .unwrap();
    file.sync_all().await.unwrap();
    drop(file);
    let before = manager
        .kernel()
        .reader
        .query_summaries(SummaryQuery::default())
        .await
        .unwrap()
        .sessions
        .len();
    let error =
        SessionExecutionCommandService::import_session_file(&manager, RequestId::new(), &path)
            .await
            .unwrap_err();
    tokio::fs::remove_file(path).await.unwrap();
    assert!(error.to_string().contains("network"), "{error}");
    let after = manager
        .kernel()
        .reader
        .query_summaries(SummaryQuery::default())
        .await
        .unwrap();
    assert_eq!(after.sessions.len(), before);
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}
