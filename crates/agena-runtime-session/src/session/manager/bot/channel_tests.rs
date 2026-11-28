use super::*;
use agena_plugin_host::sdk;
use agena_runtime_contracts::ToolSessionContext;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Default)]
struct ChannelVerifier {
    revoked: AtomicBool,
}

#[async_trait::async_trait]
impl agena_runtime_contracts::ChannelAccessVerifier for ChannelVerifier {
    async fn authorize(&self, authority: &agena_domain::ChannelAuthority) -> Result<(), String> {
        if self.revoked.load(Ordering::SeqCst)
            || !authority.active_at(chrono::Utc::now().timestamp_millis())
        {
            return Err("channel binding was revoked".into());
        }
        authority.grant.validate()
    }
}

/// Exercise the executor's Bot-only catalog and final delegated dispatch.
/// Actual management effects are exercised through bot_operation below.
struct BotCatalogProbe;

#[sdk::async_trait]
impl sdk::Plugin for BotCatalogProbe {
    fn manifest(&self) -> sdk::PluginManifest {
        let mut manifest = sdk::PluginManifest::new("agena", "bot", "1.0.0");
        manifest.tools = vec![sdk::ToolDefinition {
            name: "sessions".into(),
            contract: sdk::ToolContract {
                input_schema: json!({"type":"object"}),
                output_schema: json!({"type":"null"}),
            },
            docs: Default::default(),
            runtime: Default::default(),
            tags: vec![],
            applies_to: agena_domain::ApplicableSessions::bot(),
        }];
        manifest
    }

    async fn tool_invoke(&self, input: sdk::ToolInvokeInput) -> sdk::Result<sdk::ToolInvokeOutput> {
        assert_eq!(input.tool_name, "sessions");
        Ok(sdk::ToolInvokeOutput::text("Bot-owned channel dispatched"))
    }
}

async fn channel(
    manager: &SessionManager,
    bot: i64,
    kind: agena_domain::ConversationKind,
) -> SessionId {
    let root = std::env::temp_dir().join(format!("agena-bot-channel-{}", uuid::Uuid::new_v4()));
    let workspace = manager
        .kernel_runtime
        .workspaces
        .ensure_id(&root.to_string_lossy())
        .await
        .unwrap();
    let mut owner = manager.session_owner(bot).await.unwrap().unwrap();
    owner.surface = kind.into();
    owner.scope_key = Some(format!("channel-{}", kind.as_str()));
    let authority = agena_domain::ChannelAuthority {
        platform: String::new(),
        group_ids: Vec::new(),
        profile_ids: Vec::new(),
        profile_fingerprint: String::new(),
        permission_overlay: Default::default(),
        permission_complete: false,
        source_workspace_root: None,
        source_admin: false,
        binding_id: "bot-management-channel".into(),
        epoch: 1,
        actor: "member".into(),
        role: agena_domain::ChannelRole::Member,
        identity: agena_domain::ChannelIdentity {
            tenant: "test".into(),
            account: "bot".into(),
        },
        resource: kind.as_str().into(),
        kind,
        thread: None,
        session_mode: agena_domain::ChannelSessionMode::PerConversation,
        grant: agena_domain::ChannelGrant {
            tools: ["agena.bot.sessions".into()].into(),
            ..Default::default()
        },
        expires_at_ms: i64::MAX,
        memory_workspace_root: root.clone(),
        private_paths: Default::default(),
        revoked: false,
    };
    manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "External Bot".into(),
            SessionConfiguration {
                owner: Some(owner),
                channel_authority: Some(authority),
                effective_workspace_root: Some(root),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id
}

async fn live_channel_tool(
    manager: &SessionManager,
    source: SessionId,
    text: &str,
) -> (AttemptHandle, RoundHandle, ToolHandle, EntryId) {
    let mut author = agena_domain::MessageAuthor::channel_member(
        "member",
        agena_domain::ChannelRole::Member,
        Some("External participant".into()),
    );
    author.channel_authority = manager
        .kernel()
        .reader
        .configuration(source)
        .await
        .unwrap()
        .channel_authority
        .clone();
    author.group_ids = author.channel_authority.as_ref().unwrap().group_ids.clone();
    let queued = manager
        .kernel()
        .sessions
        .send_with_blobs_as(
            RequestId::new(),
            source,
            ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
            &[],
            Some(author),
            None,
        )
        .await
        .unwrap()
        .value;
    let attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), source, queued.id)
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
                call_id: "channel-bot-sessions".into(),
                name: "agena.bot.sessions".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let control = Arc::new(RunControl {
        session: source,
        execution: queued.id,
        generation: attempt.generation(),
        model: ModelRef::new("test", "test"),
        state: manager.state.load_full(),
        cordis_child_jobs: Default::default(),
        cancel: CancellationToken::new(),
        phase: StdMutex::new(agena_domain::ExecutionPhase::ExecutingTools),
        done: Notify::new(),
        budget: None,
        failure: Default::default(),
        settled: Default::default(),
    });
    manager.tools.lock().unwrap().insert(
        (source.get(), tool.id().get()),
        LiveTool {
            tool: tool.clone(),
            control,
            permit: Default::default(),
            permissions: Default::default(),
            cordis_reads: Default::default(),
        },
    );
    (attempt, round, tool, queued.message_id)
}

#[tokio::test]
async fn bot_owned_channels_dispatch_and_manage_project_sessions_from_member_input() {
    let (manager, project) = super::tests::fixture_with_plugins(
        vec![agena_plugin_host::StaticPluginRegistration::new(
            "agena.bot".parse().unwrap(),
            BotCatalogProbe,
        )],
        agena_plugin_host::PluginsConfig {
            list: [("agena.bot".into(), Default::default())].into(),
            ..Default::default()
        },
    )
    .await;
    let verifier = Arc::new(ChannelVerifier::default());
    manager.install_channel_verifier(verifier.clone());
    let bot = manager.ensure_bot_session().await.unwrap();
    let workspace = manager
        .kernel()
        .reader
        .metadata(project)
        .await
        .unwrap()
        .workspace_id
        .unwrap();
    let quote = "Create and rename a project session, then send the bounded task.";

    for kind in [
        agena_domain::ConversationKind::Direct,
        agena_domain::ConversationKind::Group,
        agena_domain::ConversationKind::Broadcast,
    ] {
        verifier.revoked.store(false, Ordering::SeqCst);
        let source = channel(&manager, bot, kind).await;
        let session = manager.get_session(source.get()).await.unwrap();
        assert!(!session.execution.is_global());
        assert_eq!(
            session.execution.session_kind(),
            agena_domain::SessionKind::Bot
        );
        let context = manager.execution_context_value(source).await.unwrap();
        assert_eq!(context.session_kind(), agena_domain::SessionKind::Bot);
        assert!(context.effective_workspace_root().is_some());
        let idle = manager
            .tool_executor()
            .for_session_context_async(&context)
            .await;
        assert!(idle.detailed_tools_async().await.is_empty());
        assert_eq!(
            manager.bot_context(source.get()).await.unwrap()["enabled"],
            true
        );
        let (attempt, round, tool, message) = live_channel_tool(&manager, source, quote).await;
        let context = manager.execution_context_value(source).await.unwrap();
        let mut ungranted = context.clone();
        ungranted
            .channel_authority
            .as_mut()
            .unwrap()
            .grant
            .tools
            .clear();
        let unavailable = manager
            .tool_executor()
            .for_session_context_async(&ungranted)
            .await;
        assert!(unavailable.detailed_tools_async().await.is_empty());
        // A locally configured source administrator has the source Bot's
        // catalog even when the ordinary channel grant is empty.
        let mut administrator = ungranted.clone();
        let authority = administrator.channel_authority.as_mut().unwrap();
        authority.source_admin = true;
        authority.role = agena_domain::ChannelRole::Administrator;
        authority.session_mode = agena_domain::ChannelSessionMode::PerActor;
        let full = manager
            .tool_executor()
            .for_session_context_async(&administrator)
            .await;
        assert!(
            full.detailed_tools_async()
                .await
                .iter()
                .any(|tool| tool.canonical_name() == "agena.bot.sessions")
        );
        let invocation = agena_domain::ToolInvocation::new("bot.sessions", Default::default());
        full.prepare_invocation(&invocation, source.get(), 1)
            .await
            .unwrap();
        // Source restrictions are still inherited, and cached executors
        // re-check revocation before invoking any tool.
        let mut restricted = administrator.clone();
        restricted
            .capability_denied_tool_names
            .insert("bot.sessions".into());
        let restricted = manager
            .tool_executor()
            .for_session_context_async(&restricted)
            .await;
        assert!(restricted.detailed_tools_async().await.is_empty());
        verifier.revoked.store(true, Ordering::SeqCst);
        assert!(
            full.prepare_invocation(&invocation, source.get(), 1)
                .await
                .is_err()
        );
        verifier.revoked.store(false, Ordering::SeqCst);
        let scoped = manager
            .tool_executor()
            .for_session_context_async(&context)
            .await;
        assert!(
            scoped
                .detailed_tools_async()
                .await
                .iter()
                .any(|tool| { tool.canonical_name() == "agena.bot.sessions" })
        );
        manager
            .execute_session_scoped_tool(
                source.get(),
                agena_domain::ToolInvocation::new("bot.sessions", Default::default()),
            )
            .await
            .unwrap();

        let call = tool.id().get();
        let operation = |value| serde_json::from_value::<BotRequest>(value).unwrap();
        let create = operation(json!({
            "action":"create","workspace_id":workspace,"title":"Channel project",
            "user_quote":quote,"request_key":"create"
        }));
        let created = manager
            .bot_operation(source.get(), call, create.clone())
            .await
            .unwrap();
        assert_eq!(
            created,
            manager
                .bot_operation(source.get(), call, create)
                .await
                .unwrap()
        );
        let target = created["session_id"].as_i64().unwrap();
        assert_ne!(
            manager.get_session(target).await.unwrap().workspace_id,
            session.workspace_id
        );
        let renamed = manager
            .bot_operation(
                source.get(),
                call,
                operation(json!({
                    "action":"rename","session_id":target,"title":"Managed from external chat",
                    "expected_version":created["version"],"user_quote":quote,"request_key":"rename"
                })),
            )
            .await
            .unwrap();
        assert_eq!(renamed["title"], "Managed from external chat");
        assert_eq!(renamed["origin"]["user_message_id"], message.get());
        manager
            .kernel()
            .sessions
            .configure(
                RequestId::new(),
                session_id(target).unwrap(),
                SessionConfigurationChange {
                    message_queue_paused: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let send = operation(json!({
            "action":"send","session_id":target,"instruction":"Inspect the project",
            "user_quote":quote,"request_key":"send"
        }));
        let sent = manager
            .bot_operation(source.get(), call, send.clone())
            .await
            .unwrap();
        assert_eq!(sent["accepted"], true);
        assert_eq!(sent["dispatch"], "paused");
        assert_eq!(
            sent,
            manager
                .bot_operation(source.get(), call, send)
                .await
                .unwrap()
        );
        assert_eq!(
            manager
                .kernel()
                .reader
                .summary(session_id(target).unwrap())
                .await
                .unwrap()
                .queued_executions,
            1
        );
        assert!(
            manager
                .bot_operation(
                    source.get(),
                    call,
                    operation(json!({
                        "action":"rename","session_id":target,"title":"Unquoted change",
                        "expected_version":renamed["version"],"user_quote":"not in current input",
                        "request_key":"bad-quote"
                    }))
                )
                .await
                .is_err()
        );
        verifier.revoked.store(true, Ordering::SeqCst);
        assert!(
            manager
                .bot_operation(
                    source.get(),
                    call,
                    operation(json!({
                        "action":"status","session_id":target
                    }))
                )
                .await
                .is_err()
        );

        manager.tools.lock().unwrap().remove(&(source.get(), call));
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({}),
            },
        )
        .await
        .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn bot_owned_derived_conversations_do_not_require_a_local_global_root() {
    let (manager, ordinary) = super::tests::fixture().await;
    let bot = manager.ensure_bot_session().await.unwrap();
    manager.install_channel_verifier(Arc::new(ChannelVerifier::default()));
    let parent = channel(&manager, bot, agena_domain::ConversationKind::Group).await;
    let parent_config = manager.kernel().reader.configuration(parent).await.unwrap();
    let workspace = manager
        .kernel()
        .reader
        .metadata(parent)
        .await
        .unwrap()
        .workspace_id
        .unwrap();
    for relation_kind in [
        agena_domain::SessionRelationKind::Child,
        agena_domain::SessionRelationKind::Fork,
    ] {
        let source = manager
            .kernel()
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "Bot-derived source".into(),
                SessionConfiguration {
                    owner: parent_config.owner.clone().map(|mut owner| {
                        owner.scope_key = None;
                        owner
                    }),
                    channel_authority: parent_config.channel_authority.clone(),
                    parent_session_id: Some(parent),
                    relation_kind,
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        manager.require_operator_bot(source.get()).await.unwrap();
        assert_eq!(
            manager.bot_context(source.get()).await.unwrap()["enabled"],
            true
        );
        assert_eq!(
            manager
                .bot_target(source.get(), Some(ordinary.get()))
                .await
                .unwrap(),
            ordinary.get()
        );
    }
}

#[tokio::test]
async fn shared_channel_binds_tools_groups_and_memory_to_claimed_sender() {
    let (manager, _) = super::tests::fixture().await;
    let verifier = Arc::new(ChannelVerifier::default());
    manager.install_channel_verifier(verifier.clone());
    let bot = manager.ensure_bot_session().await.unwrap();
    let template = channel(&manager, bot, agena_domain::ConversationKind::Group).await;
    let metadata = manager.kernel().reader.metadata(template).await.unwrap();
    let mut configuration = (*manager
        .kernel()
        .reader
        .configuration(template)
        .await
        .unwrap())
    .clone();
    if let Some(owner) = &mut configuration.owner {
        owner.scope_key = Some("shared-execution-identity-regression".into());
    }
    let baseline = configuration.channel_authority.as_mut().unwrap();
    baseline.actor = "@shared".into();
    baseline.platform = "telegram".into();
    baseline.grant.private_memory = true;
    let baseline = baseline.clone();
    let session = manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            metadata.workspace_id.unwrap(),
            "Shared room".into(),
            configuration,
        )
        .await
        .unwrap()
        .value
        .id;
    let mut alice = baseline.clone();
    alice.actor = "alice".into();
    alice.group_ids = vec!["maintainers".into()];
    alice.memory_workspace_root = baseline.memory_workspace_root.join("alice");
    let mut bob = baseline.clone();
    bob.actor = "bob".into();
    bob.group_ids = vec!["guests".into()];
    bob.grant = Default::default();
    bob.memory_workspace_root = baseline.memory_workspace_root.join("bob");
    let mut executions = Vec::new();
    for authority in [&alice, &bob] {
        let mut author =
            agena_domain::MessageAuthor::channel_member(&authority.actor, authority.role, None);
        author.group_ids = authority.group_ids.clone();
        author.channel_authority = Some(authority.clone());
        executions.push(
            manager
                .kernel()
                .sessions
                .send_with_blobs_as(
                    RequestId::new(),
                    session,
                    ComposerDocument(vec![ComposerNode::Text {
                        text: "Inspect this room".into(),
                    }]),
                    &[],
                    Some(author),
                    None,
                )
                .await
                .unwrap()
                .value,
        );
    }
    let state = manager.state.load_full();
    let first = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, executions[0].id)
        .await
        .unwrap()
        .value;
    // Bob is queued, but all implicit session callbacks still belong to Alice.
    let context = manager
        .execution_context_with_state(session, &state)
        .await
        .unwrap();
    assert_eq!(context.channel_authority.as_ref(), Some(&alice));
    assert_eq!(
        context.effective_workspace_root.as_ref(),
        Some(&alice.memory_workspace_root)
    );
    let current = manager.get_session(session.get()).await.unwrap();
    assert_eq!(current.execution.channel_authority.as_ref(), Some(&alice));
    assert!(alice.allows_private_memory());
    assert!(alice.allows_tool("agena.bot.sessions", chrono::Utc::now().timestamp_millis()));
    let child = manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            metadata.workspace_id.unwrap(),
            "Delegated room task".into(),
            SessionConfiguration {
                parent_session_id: Some(session),
                relation_kind: agena_domain::SessionRelationKind::Subagent,
                channel_authority: Some(alice.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let delegated = manager
        .kernel()
        .sessions
        .send_with_blobs_as(
            RequestId::new(),
            child,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Delegated task".into(),
            }]),
            &[],
            None,
            None,
        )
        .await
        .unwrap()
        .value;
    let child_context = manager
        .execution_context_for_execution(child, &state, Some(delegated.id))
        .await
        .unwrap();
    assert_eq!(child_context.channel_authority.as_ref(), Some(&alice));
    assert_eq!(
        child_context.effective_workspace_root.as_ref(),
        Some(&alice.memory_workspace_root)
    );
    verifier.revoked.store(true, Ordering::SeqCst);
    assert!(
        manager
            .execution_context_for_execution(child, &state, Some(delegated.id))
            .await
            .is_err()
    );
    assert!(
        manager
            .execution_context_for_execution(session, &state, Some(executions[0].id))
            .await
            .is_err()
    );
    verifier.revoked.store(false, Ordering::SeqCst);
    manager
        .kernel()
        .sessions
        .cancel_owned(RequestId::new(), child, delegated.id)
        .await
        .unwrap();
    let round = first
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    round.finish(RequestId::new(), None, true).await.unwrap();
    first
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let idle = manager.get_session(session.get()).await.unwrap();
    let idle_authority = idle.execution.channel_authority.as_ref().unwrap();
    assert_eq!(idle_authority.actor, "@shared");
    assert!(!idle_authority.allows_private_memory());
    assert!(
        !idle_authority.allows_tool("agena.bot.sessions", chrono::Utc::now().timestamp_millis())
    );
    let second = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, executions[1].id)
        .await
        .unwrap()
        .value;
    let context = manager
        .execution_context_with_state(session, &state)
        .await
        .unwrap();
    assert_eq!(context.channel_authority.as_ref(), Some(&bob));
    assert_eq!(
        context.effective_workspace_root.as_ref(),
        Some(&bob.memory_workspace_root)
    );
    assert!(!bob.allows_private_memory());
    assert!(!bob.allows_tool("agena.bot.sessions", chrono::Utc::now().timestamp_millis()));
    // The first sender is durable even while a different user owns the room.
    assert_eq!(
        manager
            .channel_execution_authority(session.get(), executions[0].id)
            .await
            .unwrap(),
        Some(alice)
    );
    let round = second
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    round.finish(RequestId::new(), None, true).await.unwrap();
    second
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let mut spoof = agena_domain::MessageAuthor::channel_member("mallory", bob.role, None);
    spoof.group_ids = bob.group_ids.clone();
    spoof.channel_authority = Some(bob);
    let input = manager
        .kernel()
        .sessions
        .send_with_blobs_as(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Use Bob's identity".into(),
            }]),
            &[],
            Some(spoof),
            None,
        )
        .await
        .unwrap()
        .value;
    assert!(
        manager
            .channel_execution_authority(session.get(), input.id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn legacy_shared_session_retains_history_without_first_speaker_authority() {
    let (manager, _) = super::tests::fixture().await;
    manager.install_channel_verifier(Arc::new(ChannelVerifier::default()));
    let bot = manager.ensure_bot_session().await.unwrap();
    let session = channel(&manager, bot, agena_domain::ConversationKind::Group).await;
    let stored = manager
        .kernel()
        .reader
        .configuration(session)
        .await
        .unwrap();
    let first = stored.channel_authority.as_ref().unwrap();
    assert_eq!(first.actor, "member");
    let mut next = first.clone();
    next.actor = "new-speaker".into();
    next.group_ids = vec!["readers".into()];
    next.grant = Default::default();
    next.memory_workspace_root = first.memory_workspace_root.join("new-speaker");
    let old_input = manager
        .kernel()
        .sessions
        .send_with_blobs_as(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Historical unscoped input".into(),
            }]),
            &[],
            Some(agena_domain::MessageAuthor::channel_member(
                "member", first.role, None,
            )),
            None,
        )
        .await
        .unwrap()
        .value;
    assert!(
        manager
            .channel_execution_authority(session.get(), old_input.id)
            .await
            .is_err()
    );
    let mut author = agena_domain::MessageAuthor::channel_member(&next.actor, next.role, None);
    author.group_ids = next.group_ids.clone();
    author.channel_authority = Some(next.clone());
    let input = manager
        .kernel()
        .sessions
        .send_with_blobs_as(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "New verified input".into(),
            }]),
            &[],
            Some(author),
            None,
        )
        .await
        .unwrap()
        .value;
    let context = manager
        .execution_context_for_execution(session, &manager.state.load_full(), Some(input.id))
        .await
        .unwrap();
    assert_eq!(context.scope_session_id, Some(session.get()));
    assert_eq!(context.channel_authority.as_ref(), Some(&next));
    assert!(
        !context
            .channel_authority
            .as_ref()
            .unwrap()
            .allows_tool("agena.bot.sessions", chrono::Utc::now().timestamp_millis())
    );
    assert_eq!(
        context.effective_workspace_root.as_ref(),
        Some(&next.memory_workspace_root)
    );
    assert!(
        manager
            .kernel()
            .reader
            .entry(session, old_input.message_id)
            .await
            .is_ok()
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .configuration(session)
            .await
            .unwrap()
            .channel_authority
            .as_ref()
            .unwrap()
            .actor,
        "member"
    );
    assert_eq!(
        manager
            .get_session(session.get())
            .await
            .unwrap()
            .execution
            .channel_authority
            .unwrap()
            .actor,
        "@shared"
    );
}

#[tokio::test]
async fn shared_channel_child_continuation_keeps_alice_after_bob_speaks() {
    let (manager, _) = super::tests::fixture().await;
    manager.install_channel_verifier(Arc::new(ChannelVerifier::default()));
    let bot = manager.ensure_bot_session().await.unwrap();
    let session = channel(&manager, bot, agena_domain::ConversationKind::Group).await;
    let metadata = manager.kernel().reader.metadata(session).await.unwrap();
    let baseline = manager
        .kernel()
        .reader
        .configuration(session)
        .await
        .unwrap()
        .channel_authority
        .clone()
        .unwrap();
    let mut alice = baseline.clone();
    alice.actor = "alice".into();
    alice.group_ids = vec!["maintainers".into()];
    alice.memory_workspace_root = baseline.memory_workspace_root.join("alice");
    let mut bob = baseline;
    bob.actor = "bob".into();
    bob.group_ids = vec!["guests".into()];
    bob.grant = Default::default();
    bob.memory_workspace_root = alice.memory_workspace_root.with_file_name("bob");
    let mut inputs = Vec::new();
    for authority in [&alice, &bob] {
        let mut author =
            agena_domain::MessageAuthor::channel_member(&authority.actor, authority.role, None);
        author.group_ids = authority.group_ids.clone();
        author.channel_authority = Some(authority.clone());
        let input = manager
            .kernel()
            .sessions
            .send_with_blobs_as(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: format!("Message from {}", authority.actor),
                }]),
                &[],
                Some(author),
                None,
            )
            .await
            .unwrap()
            .value;
        let attempt = manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), session, input.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        inputs.push(input);
    }
    let mut agent = SessionAgentConfiguration {
        origin_message_id: Some(inputs[0].message_id),
        workflow: None,
        cordis: None,
        name: "alice_worker".into(),
        status: agena_domain::AgentStatus::Running,
        started_at_ms: Some(1000),
        finished_at_ms: None,
        failure: None,
        limits: Default::default(),
    };
    let child = manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            metadata.workspace_id.unwrap(),
            "Alice's asynchronous task".into(),
            SessionConfiguration {
                parent_session_id: Some(session),
                relation_kind: agena_domain::SessionRelationKind::Subagent,
                agent: Some(agent.clone()),
                channel_authority: Some(alice.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    // A control anchor makes the eventual wake a new notice-backed execution.
    // Without a durable origin a manual continuation has no external caller.
    let manual = manager
        .kernel()
        .sessions
        .start_control_execution(
            RequestId::new(),
            session,
            ControlExecutionPurpose::Continuation,
        )
        .await
        .unwrap()
        .value;
    assert!(
        manager
            .channel_execution_authority(session.get(), manual.id)
            .await
            .is_err()
    );
    manager
        .kernel()
        .sessions
        .cancel_owned(RequestId::new(), session, manual.id)
        .await
        .unwrap();
    agent.status = agena_domain::AgentStatus::Completed;
    agent.finished_at_ms = Some(2000);
    manager.kernel().sessions.finish_agent(
        RequestId::new(), child, agent, "alice-worker-result".into(),
        json!({"kind":"agent_result","agent_id":child.get(),"message_id":inputs[0].message_id.get(),"name":"alice_worker","text":"Alice's result"}),
    ).await.unwrap();
    let deliveries = manager
        .kernel()
        .reader
        .pending_deliveries(session, None, 10)
        .await
        .unwrap();
    let delivery = deliveries
        .iter()
        .find(|delivery| delivery.business_key == "alice-worker-result")
        .unwrap();
    let resumed = manager
        .kernel()
        .executor
        .resume_delivery(
            RequestId::new(),
            session,
            delivery.id,
            inputs[0].message_id,
            "Alice's result".into(),
        )
        .await
        .unwrap()
        .value;
    assert_eq!(resumed.purpose, ExecutionPurpose::Continuation);
    assert_ne!(resumed.message_id, inputs[0].message_id);
    let state = manager.state.load_full();
    let resumed_context = manager
        .execution_context_for_execution(session, &state, Some(resumed.id))
        .await
        .unwrap();
    assert_eq!(resumed_context.channel_authority.as_ref(), Some(&alice));
    assert_eq!(
        resumed_context.effective_workspace_root.as_ref(),
        Some(&alice.memory_workspace_root)
    );
    assert_ne!(resumed_context.channel_authority.as_ref(), Some(&bob));
    // Reading again from the durable kernel needs no in-memory RunControl.
    assert_eq!(
        manager
            .channel_execution_authority(session.get(), resumed.id)
            .await
            .unwrap(),
        Some(alice)
    );
    manager
        .kernel()
        .sessions
        .cancel_owned(RequestId::new(), session, resumed.id)
        .await
        .unwrap();
    let forged = manager.kernel().executor.deliver(
        RequestId::new(), session, "forged-worker-origin".into(),
        json!({"kind":"agent_result","agent_id":child.get(),"message_id":inputs[1].message_id.get(),"text":"Wrong origin"}),
    ).await.unwrap().value;
    let forged_run = manager
        .kernel()
        .executor
        .resume_delivery(
            RequestId::new(),
            session,
            forged.id,
            inputs[1].message_id,
            "Wrong origin".into(),
        )
        .await
        .unwrap()
        .value;
    assert!(
        manager
            .channel_execution_authority(session.get(), forged_run.id)
            .await
            .is_err()
    );
}
