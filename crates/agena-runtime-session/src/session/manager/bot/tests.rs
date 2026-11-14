use super::*;

pub(super) async fn fixture() -> (SessionManager, SessionId) {
    fixture_with_plugins(vec![], Default::default()).await
}

pub(super) async fn fixture_with_plugins(
    static_plugins: Vec<agena_plugin_host::StaticPluginRegistration>,
    config: agena_plugin_host::PluginsConfig,
) -> (SessionManager, SessionId) {
    let kernel = Arc::new(
        agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap(),
    );
    let root = std::env::temp_dir();
    let workspace = kernel
        .workspaces
        .ensure_id(&root.to_string_lossy())
        .await
        .unwrap();
    let session = kernel
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Bot".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let plugins = agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
        static_plugins,
        config,
        workspace_root: root.clone(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        root.clone(),
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        plugins.clone(),
        None,
        None,
    );
    let preferences = Arc::new(agena_storage_sqlite::SqliteCompanionRepository::new(
        kernel.database.clone(),
    ));
    let manager = SessionManager::new(
        kernel,
        Arc::new(ProviderRegistry::default()),
        ContextGovernor::new(Default::default()),
        SessionProcessor::new(plugins),
        executor,
        Default::default(),
    );
    manager.install_memory_preferences(preferences);
    (manager, session)
}

#[tokio::test]
async fn canonical_identity_survives_repeated_ensure_and_manager_recreation_without_a_model() {
    let (manager, titled_bot) = fixture().await;
    let mut calls = Vec::new();
    for _ in 0..12 {
        let manager = manager.clone();
        calls.push(tokio::spawn(async move {
            manager.ensure_bot_session().await.unwrap()
        }));
    }
    let mut ids = Vec::new();
    for call in calls {
        ids.push(call.await.unwrap());
    }
    let id = ids[0];
    assert_ne!(id, titled_bot.get());
    assert!(ids.iter().all(|value| *value == id));
    assert!(
        manager
            .kernel()
            .reader
            .metadata(session_id(id).unwrap())
            .await
            .unwrap()
            .selection
            .is_none()
    );
    manager
        .rename_session(id, "User operator title".into())
        .await
        .unwrap();
    let state = manager.state.load_full();
    let restarted = SessionManager::new(
        manager.kernel_runtime.clone(),
        state.providers.clone(),
        state.governor.clone(),
        state.processor.clone(),
        state.executor.clone(),
        state.config.clone(),
    );
    assert_eq!(restarted.ensure_bot_session().await.unwrap(), id);
    assert_eq!(
        restarted
            .kernel()
            .reader
            .metadata(session_id(id).unwrap())
            .await
            .unwrap()
            .title,
        "User operator title"
    );
    assert_eq!(
        restarted.bot_context(titled_bot.get()).await.unwrap(),
        json!({"enabled":false})
    );
    assert_eq!(restarted.bot_context(id).await.unwrap()["enabled"], true);
}

#[tokio::test]
async fn global_bot_root_is_an_operator_but_not_a_project_target() {
    let (manager, ordinary) = fixture().await;
    let bot = manager.ensure_bot_session().await.unwrap();
    let metadata = manager
        .kernel()
        .reader
        .metadata(session_id(bot).unwrap())
        .await
        .unwrap();
    assert_eq!(metadata.workspace_id, None);
    assert!(manager.session_owner(bot).await.unwrap().is_some());
    manager.require_operator_bot(bot).await.unwrap();
    assert!(manager.require_project_session(bot).await.is_err());
    manager
        .require_project_session(ordinary.get())
        .await
        .unwrap();
    assert!(manager.bot_target(bot, Some(bot)).await.is_err());
    assert_eq!(
        manager.bot_target(bot, Some(ordinary.get())).await.unwrap(),
        ordinary.get()
    );
}

#[tokio::test]
async fn operator_and_target_guards_reject_subagents_side_conversations_and_channels() {
    let (manager, ordinary) = fixture().await;
    let bot = manager.ensure_bot_session().await.unwrap();
    let workspace = manager
        .kernel()
        .reader
        .metadata(ordinary)
        .await
        .unwrap()
        .workspace_id
        .expect("ordinary project workspace");
    let channel = agena_domain::ChannelAuthority {
        binding_id: "bot-guard-channel".into(),
        epoch: 1,
        actor: "member".into(),
        role: agena_domain::ChannelRole::Member,
        identity: agena_domain::ChannelIdentity {
            tenant: "test".into(),
            account: "test".into(),
        },
        resource: "private".into(),
        kind: agena_domain::ConversationKind::Direct,
        thread: None,
        session_mode: agena_domain::ChannelSessionMode::PerConversation,
        grant: Default::default(),
        expires_at_ms: i64::MAX,
        memory_workspace_root: std::env::temp_dir(),
        private_paths: Default::default(),
        revoked: false,
    };
    let configs = [
        SessionConfiguration {
            parent_session_id: Some(ordinary),
            relation_kind: agena_domain::SessionRelationKind::Subagent,
            agent: Some(SessionAgentConfiguration {
                name: "guard_probe".into(),
                status: agena_domain::AgentStatus::Created,
                started_at_ms: None,
                finished_at_ms: None,
                failure: None,
                limits: Default::default(),
            }),
            ..Default::default()
        },
        SessionConfiguration {
            conversation: Some(agena_domain::SessionConversation {
                mode: agena_domain::ConversationMode::Side,
                parent_session_id: ordinary.get(),
            }),
            ..Default::default()
        },
        SessionConfiguration {
            channel_authority: Some(channel),
            ..Default::default()
        },
    ];
    assert!(manager.require_operator_bot(ordinary.get()).await.is_err());
    assert!(manager.bot_target(bot, Some(bot)).await.is_err());
    let bot_fork = manager
        .kernel()
        .sessions
        .create_global(
            RequestId::new(),
            "Bot-history fork".into(),
            SessionConfiguration {
                parent_session_id: Some(session_id(bot).unwrap()),
                relation_kind: agena_domain::SessionRelationKind::Fork,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    assert!(manager.bot_target(bot, Some(bot_fork.get())).await.is_err());
    let authorizing = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session_id(bot).unwrap(),
            ComposerDocument(vec![ComposerNode::Text {
                text: "authorize exact native target".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let authorizing_entry = manager
        .kernel()
        .reader
        .entry(session_id(bot).unwrap(), authorizing.message_id)
        .await
        .unwrap();
    for config in configs {
        let id = manager
            .kernel()
            .sessions
            .create_configured(RequestId::new(), workspace, "excluded".into(), config)
            .await
            .unwrap()
            .value
            .id
            .get();
        assert!(manager.require_operator_bot(id).await.is_err());
        assert!(manager.bot_target(bot, Some(id)).await.is_err());
        assert_eq!(manager.bot_context(id).await.unwrap()["enabled"], false);
        let inherited = manager
            .kernel()
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "ordinary-looking inherited boundary".into(),
                SessionConfiguration {
                    parent_session_id: Some(session_id(id).unwrap()),
                    relation_kind: agena_domain::SessionRelationKind::Fork,
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        assert!(
            manager
                .bot_target(bot, Some(inherited.get()))
                .await
                .is_err(),
            "ordinary relation escaped excluded ancestor"
        );
        manager
            .kernel()
            .sessions
            .delete(RequestId::new(), session_id(id).unwrap())
            .await
            .unwrap();
        assert!(
            manager
                .bot_target(bot, Some(inherited.get()))
                .await
                .is_err(),
            "deleting excluded ancestor erased inherited scope"
        );
        let snapshot = manager
            .kernel()
            .reader
            .control_snapshot(inherited, 1)
            .await
            .unwrap();
        let guard = agena_storage::kernel::DeleteGuard {
            metadata_version: snapshot.metadata.metadata_version.get(),
            control_version: snapshot.control_version,
            entry_count: snapshot.summary.entry_count,
            source_session: session_id(bot).unwrap(),
            source_message: authorizing.message_id,
            source_revision: authorizing_entry.revision,
        };
        assert!(
            matches!(
                manager
                    .kernel()
                    .sessions
                    .delete_checked(RequestId::new(), inherited, guard)
                    .await,
                Err(KernelError::Ownership)
            ),
            "native writer lost tombstoned private ancestry"
        );
    }
    let request: BotRequest = serde_json::from_value(json!({"action":"projects"})).unwrap();
    assert!(
        manager.bot_operation(bot, 12345, request).await.is_err(),
        "an ID alone created a callback capability"
    );
    let private = agena_storage_layout::StorageLayout::current()
        .root()
        .join("channel-scopes")
        .join("private-user");
    assert!(SessionManager::require_project_path(&private).is_err());
}

#[tokio::test]
async fn bot_delivery_is_system_origin_fifo_and_idempotent_and_manual_input_remains_user_origin() {
    let (manager, target) = fixture().await;
    let payload = json!({"kind":"bot_instruction","source_session_id":99,"source_message_id":123,"text":"bounded project request"});
    let command = RequestId::new();
    let first = manager
        .kernel()
        .executor
        .deliver_control_notice(
            command,
            target,
            "bot-test".into(),
            payload.clone(),
            "Operator Bot instruction".into(),
        )
        .await
        .unwrap()
        .value;
    let retry = manager
        .kernel()
        .executor
        .deliver_control_notice(
            command,
            target,
            "bot-test".into(),
            payload,
            "Operator Bot instruction".into(),
        )
        .await
        .unwrap()
        .value;
    assert_eq!(first.id, retry.id);
    assert_eq!(first.purpose, ExecutionPurpose::Continuation);
    assert!(!matches!(
        manager
            .kernel()
            .reader
            .entry(target, first.message_id)
            .await
            .unwrap()
            .payload,
        EntryPayload::User { .. }
    ));
    let human = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            target,
            ComposerDocument(vec![ComposerNode::Text {
                text: "human directly continues".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    assert_ne!(first.id, human.id);
    assert_eq!(human.purpose, ExecutionPurpose::Message);
    assert!(matches!(
        manager
            .kernel()
            .reader
            .entry(target, human.message_id)
            .await
            .unwrap()
            .payload,
        EntryPayload::User { .. }
    ));
    assert!(matches!(
        manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), target, human.id)
            .await,
        Err(KernelError::QueueOutOfOrder)
    ));
    manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), target, first.id)
        .await
        .unwrap()
        .value
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), target, human.id)
        .await
        .unwrap();
}

#[tokio::test]
async fn subagent_result_can_resume_a_bot_started_project_without_a_fake_user_message() {
    let (manager, target) = fixture().await;
    let first = manager
        .kernel()
        .executor
        .deliver_control_notice(
            RequestId::new(),
            target,
            "bot-only-project".into(),
            json!({"kind":"bot_instruction","source_session_id":99,"source_message_id":123,"text":"use a Subagent"}),
            "Operator Bot instruction".into(),
        )
        .await
        .unwrap()
        .value;
    manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), target, first.id)
        .await
        .unwrap()
        .value
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let delivery = manager
        .kernel()
        .executor
        .deliver(
            RequestId::new(),
            target,
            "child-result".into(),
            json!({"kind":"agent_result","message_id":first.message_id.get(),"text":"Subagent evidence"}),
        )
        .await
        .unwrap()
        .value;
    let resumed = manager
        .kernel()
        .executor
        .resume_delivery(
            RequestId::new(),
            target,
            delivery.id,
            first.message_id,
            "Subagent evidence".into(),
        )
        .await
        .unwrap()
        .value;
    assert_ne!(first.id, resumed.id);
    assert_eq!(resumed.purpose, ExecutionPurpose::Continuation);
    assert_ne!(first.message_id, resumed.message_id);
    let history = manager
        .kernel()
        .reader
        .history(target, None, 50)
        .await
        .unwrap();
    assert!(
        history
            .entries
            .iter()
            .all(|entry| !matches!(entry.payload, EntryPayload::User { .. }))
    );
    assert!(
        manager
            .latest_origin_message(target)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn opening_bot_does_not_wait_for_management_and_deleted_bot_is_replaced_once() {
    let (manager, _) = fixture().await;
    let id = manager.ensure_bot_session().await.unwrap();
    let pending_permission = manager.bot_operations.lock().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), manager.ensure_bot_session())
            .await
            .unwrap()
            .unwrap(),
        id
    );
    drop(pending_permission);
    manager
        .kernel()
        .sessions
        .delete(RequestId::new(), session_id(id).unwrap())
        .await
        .unwrap();
    let (a, b) = tokio::join!(manager.ensure_bot_session(), manager.ensure_bot_session());
    let replacement = a.unwrap();
    assert_eq!(b.unwrap(), replacement);
    assert_ne!(replacement, id);
    assert!(manager.require_operator_bot(id).await.is_err());
    assert!(manager.require_operator_bot(replacement).await.is_ok());
}

#[tokio::test]
async fn ordinary_children_and_forks_remain_routable_with_tool_filters() {
    let (manager, parent) = fixture().await;
    let bot = manager.ensure_bot_session().await.unwrap();
    let workspace = manager
        .kernel()
        .reader
        .metadata(parent)
        .await
        .unwrap()
        .workspace_id
        .expect("ordinary project workspace");
    manager
        .kernel()
        .sessions
        .configure(
            RequestId::new(),
            parent,
            SessionConfigurationChange {
                capability_denied_tool_names: Some(["shell.run".into()].into_iter().collect()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for relation_kind in [
        agena_domain::SessionRelationKind::Child,
        agena_domain::SessionRelationKind::Fork,
    ] {
        let child = manager
            .kernel()
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "ordinary project continuation".into(),
                SessionConfiguration {
                    parent_session_id: Some(parent),
                    relation_kind,
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        assert_eq!(
            manager.bot_target(bot, Some(child.get())).await.unwrap(),
            child.get()
        );
    }
}

#[tokio::test]
async fn bot_fixture_has_no_retired_task_schema() {
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    let (manager, _) = fixture().await;
    manager.ensure_bot_session().await.unwrap();
    let row = manager
        .kernel_runtime
        .database
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT count(*) AS retired FROM sqlite_master WHERE name GLOB 'agena_superagent_*'"
                .to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "retired").unwrap(), 0);
}

#[test]
fn management_validation_rejects_misplaced_effects_and_keeps_simple_calls() {
    for value in [
        json!({"action":"projects"}),
        json!({"action":"sessions","workspace_id":1}),
        json!({"action":"sessions","query":"investigation","states":["ready"]}),
        json!({"action":"read","session_id":2}),
        json!({"action":"send","session_id":2,"instruction":"continue","user_quote":"continue","request_key":"send"}),
        json!({"action":"models","session_id":2,"cursor":"returned","limit":5}),
        json!({"action":"project","workspace_id":1,"cursor":"returned","limit":5}),
    ] {
        let request: BotRequest = serde_json::from_value(value).unwrap();
        validate_request(&request).unwrap();
    }
    for value in [
        json!({"action":"read","session_id":2,"instruction":"delete"}),
        json!({"action":"create","workspace_id":1,"title":"new","session_id":2}),
        json!({"action":"organize","session_id":2,"expected_version":1}),
        json!({"action":"delete","session_id":2,"expected_version":1,"expected_control_version":2}),
        json!({"action":"fork","session_id":2,"pinned":true}),
        json!({"action":"queue_pause","session_id":2,"expected_control_version":1}),
        json!({"action":"configure","session_id":2,"expected_version":1,"reset_model":true,"model":{"provider_id":"p","model_id":"m"}}),
    ] {
        let request: BotRequest = serde_json::from_value(value).unwrap();
        assert!(
            validate_request(&request).is_err(),
            "accepted confusing request: {request:?}"
        );
    }
}

#[tokio::test]
async fn checked_fork_keeps_native_inheritance_and_replays_after_source_deleted() {
    use agena_storage::kernel::ForkGuard;
    let (manager, source) = fixture().await;
    let bot = manager.ensure_bot_session().await.unwrap();
    manager
        .kernel()
        .sessions
        .configure(
            RequestId::new(),
            source,
            SessionConfigurationChange {
                capability_denied_tool_names: Some(["shell.run".into()].into_iter().collect()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    manager
        .kernel()
        .sessions
        .update_selection(
            RequestId::new(),
            source,
            SelectionChange::Model(Some(agena_domain::ModelRef::new("fixture", "test"))),
        )
        .await
        .unwrap();
    let execution = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            source,
            ComposerDocument(vec![ComposerNode::Text {
                text: "source transcript".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let metadata = manager.kernel().reader.metadata(source).await.unwrap();
    let command = RequestId::new();
    let guard = ForkGuard {
        metadata_version: metadata.metadata_version.get(),
        entry_count: manager
            .kernel()
            .reader
            .summary(source)
            .await
            .unwrap()
            .entry_count,
        title: Some("Human chosen fork".into()),
    };
    let fork = manager
        .kernel()
        .sessions
        .fork_checked(command, source, execution.message_id, guard.clone())
        .await
        .unwrap()
        .value;
    assert_eq!(fork.workspace_id, metadata.workspace_id);
    assert_eq!(fork.title, "Human chosen fork");
    assert_eq!(fork.selection, metadata.selection);
    let config = manager
        .kernel()
        .reader
        .configuration(fork.id)
        .await
        .unwrap();
    assert_eq!(
        config.relation_kind,
        agena_domain::SessionRelationKind::Fork
    );
    assert_eq!(config.parent_session_id, Some(source));
    assert!(config.capability_denied_tool_names.contains("shell.run"));
    assert_eq!(config.title_state.source, SessionTitleSource::Manual);
    assert_eq!(
        manager
            .kernel()
            .reader
            .history(fork.id, None, 10)
            .await
            .unwrap()
            .entries[0]
            .id,
        execution.message_id
    );
    manager
        .kernel()
        .sessions
        .delete(RequestId::new(), source)
        .await
        .unwrap();
    let replay = manager
        .kernel()
        .sessions
        .fork_checked(command, source, execution.message_id, guard)
        .await
        .unwrap()
        .value;
    assert_eq!(replay.id, fork.id);
    assert_eq!(
        manager.bot_target(bot, Some(fork.id.get())).await.unwrap(),
        fork.id.get()
    );
    let continued = manager
        .submit_bot_instruction(
            fork.id.get(),
            bot,
            123,
            "continue-after-source-delete",
            "continue this surviving native fork",
        )
        .await
        .unwrap();
    assert_eq!(continued["accepted"], true);
    assert_eq!(
        manager
            .kernel()
            .reader
            .summary(fork.id)
            .await
            .unwrap()
            .queued_executions,
        1
    );
    let snapshot = manager
        .kernel()
        .reader
        .control_snapshot(fork.id, 1)
        .await
        .unwrap();
    let nested = manager
        .kernel()
        .sessions
        .fork_checked(
            RequestId::new(),
            fork.id,
            execution.message_id,
            ForkGuard {
                metadata_version: snapshot.metadata.metadata_version.get(),
                entry_count: snapshot.summary.entry_count,
                title: None,
            },
        )
        .await
        .unwrap()
        .value;
    assert_eq!(
        manager
            .bot_target(bot, Some(nested.id.get()))
            .await
            .unwrap(),
        nested.id.get()
    );
}

#[tokio::test]
async fn checked_delete_refuses_new_target_work_and_a_new_authorizing_human_turn() {
    use agena_storage::kernel::DeleteGuard;
    let (manager, target) = fixture().await;
    let source = session_id(manager.ensure_bot_session().await.unwrap()).unwrap();
    let human = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            source,
            ComposerDocument(vec![ComposerNode::Text {
                text: "delete the exact idle conversation".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let entry = manager
        .kernel()
        .reader
        .entry(source, human.message_id)
        .await
        .unwrap();
    let snapshot = manager
        .kernel()
        .reader
        .control_snapshot(target, 1)
        .await
        .unwrap();
    let guard = DeleteGuard {
        metadata_version: snapshot.metadata.metadata_version.get(),
        control_version: snapshot.control_version,
        entry_count: snapshot.summary.entry_count,
        source_session: source,
        source_message: human.message_id,
        source_revision: entry.revision,
    };
    manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            source,
            ComposerDocument(vec![ComposerNode::Text {
                text: "wait, keep it".into(),
            }]),
        )
        .await
        .unwrap();
    assert!(matches!(
        manager
            .kernel()
            .sessions
            .delete_checked(RequestId::new(), target, guard)
            .await,
        Err(KernelError::Conflict(_))
    ));
    assert!(manager.kernel().reader.metadata(target).await.is_ok());
    let latest = manager
        .kernel()
        .reader
        .history(source, None, 1)
        .await
        .unwrap();
    let latest = &latest.entries[0];
    let guard = DeleteGuard {
        metadata_version: snapshot.metadata.metadata_version.get(),
        control_version: snapshot.control_version,
        entry_count: snapshot.summary.entry_count,
        source_session: source,
        source_message: latest.id,
        source_revision: latest.revision,
    };
    manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            target,
            ComposerDocument(vec![ComposerNode::Text {
                text: "new project work".into(),
            }]),
        )
        .await
        .unwrap();
    assert!(matches!(
        manager
            .kernel()
            .sessions
            .delete_checked(RequestId::new(), target, guard)
            .await,
        Err(KernelError::Conflict(_))
    ));
    assert!(manager.kernel().reader.metadata(target).await.is_ok());
}

#[tokio::test]
async fn checked_delete_receipt_survives_soft_deletion_and_source_changes() {
    use agena_storage::kernel::DeleteGuard;
    let (manager, target) = fixture().await;
    let source = session_id(manager.ensure_bot_session().await.unwrap()).unwrap();
    let human = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            source,
            ComposerDocument(vec![ComposerNode::Text {
                text: "delete this exact conversation".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let entry = manager
        .kernel()
        .reader
        .entry(source, human.message_id)
        .await
        .unwrap();
    let snapshot = manager
        .kernel()
        .reader
        .control_snapshot(target, 1)
        .await
        .unwrap();
    let guard = DeleteGuard {
        metadata_version: snapshot.metadata.metadata_version.get(),
        control_version: snapshot.control_version,
        entry_count: snapshot.summary.entry_count,
        source_session: source,
        source_message: human.message_id,
        source_revision: entry.revision,
    };
    let command = RequestId::new();
    let deleted = manager
        .kernel()
        .sessions
        .delete_checked(command, target, guard.clone())
        .await
        .unwrap()
        .value;
    assert!(manager.kernel().reader.metadata(target).await.is_err());
    manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            source,
            ComposerDocument(vec![ComposerNode::Text {
                text: "another human turn".into(),
            }]),
        )
        .await
        .unwrap();
    let replay = manager
        .kernel()
        .sessions
        .delete_checked(command, target, guard)
        .await
        .unwrap()
        .value;
    assert_eq!(replay.id, deleted.id);
    assert_eq!(replay.metadata_version, deleted.metadata_version);
}

#[tokio::test]
async fn accepted_send_without_model_is_honest_and_retries_never_requeue() {
    let (manager, target) = fixture().await;
    let first = manager
        .submit_bot_instruction(
            target.get(),
            99,
            123,
            "missing-model-retry",
            "bounded request",
        )
        .await
        .unwrap();
    assert_eq!(first["accepted"], true);
    assert_eq!(first["phase"], "queued");
    assert_eq!(first["dispatch"], "blocked");
    assert!(first["reason"].as_str().unwrap().contains("do not resend"));
    let retry = manager
        .submit_bot_instruction(
            target.get(),
            99,
            123,
            "missing-model-retry",
            "bounded request",
        )
        .await
        .unwrap();
    assert_eq!(retry["execution_id"], first["execution_id"]);
    assert_eq!(
        manager
            .kernel()
            .reader
            .summary(target)
            .await
            .unwrap()
            .queued_executions,
        1
    );
    manager
        .kernel()
        .sessions
        .delete(RequestId::new(), target)
        .await
        .unwrap();
    let deleted_retry = manager
        .submit_bot_instruction(
            target.get(),
            99,
            123,
            "missing-model-retry",
            "bounded request",
        )
        .await
        .unwrap();
    assert_eq!(deleted_retry["execution_id"], first["execution_id"]);
    assert_eq!(deleted_retry["target_deleted"], true);
}

#[tokio::test]
async fn checked_delete_refuses_active_subagent_even_when_parent_is_idle() {
    use agena_storage::kernel::DeleteGuard;
    let (manager, target) = fixture().await;
    let source = session_id(manager.ensure_bot_session().await.unwrap()).unwrap();
    let human = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            source,
            ComposerDocument(vec![ComposerNode::Text {
                text: "delete idle parent only".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let entry = manager
        .kernel()
        .reader
        .entry(source, human.message_id)
        .await
        .unwrap();
    let workspace = manager
        .kernel()
        .reader
        .metadata(target)
        .await
        .unwrap()
        .workspace_id
        .expect("ordinary project workspace");
    manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "active native Subagent".into(),
            SessionConfiguration {
                parent_session_id: Some(target),
                relation_kind: agena_domain::SessionRelationKind::Subagent,
                agent: Some(SessionAgentConfiguration {
                    name: "active_probe".into(),
                    status: agena_domain::AgentStatus::Created,
                    started_at_ms: None,
                    finished_at_ms: None,
                    failure: None,
                    limits: Default::default(),
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let snapshot = manager
        .kernel()
        .reader
        .control_snapshot(target, 1)
        .await
        .unwrap();
    assert_eq!(snapshot.summary.queued_executions, 0);
    assert_eq!(snapshot.summary.running_executions, 0);
    let guard = DeleteGuard {
        metadata_version: snapshot.metadata.metadata_version.get(),
        control_version: snapshot.control_version,
        entry_count: snapshot.summary.entry_count,
        source_session: source,
        source_message: human.message_id,
        source_revision: entry.revision,
    };
    let error = manager
        .kernel()
        .sessions
        .delete_checked(RequestId::new(), target, guard)
        .await
        .unwrap_err();
    assert!(
        matches!(error,KernelError::Conflict(ref reason) if reason.contains("Subagents")),
        "unexpected error {error}"
    );
    assert!(manager.kernel().reader.metadata(target).await.is_ok());
}
