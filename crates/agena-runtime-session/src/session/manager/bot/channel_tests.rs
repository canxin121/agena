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
    let queued = manager
        .kernel()
        .sessions
        .send_with_blobs_as(
            RequestId::new(),
            source,
            ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
            &[],
            Some(agena_domain::MessageAuthor::channel_member(
                "member",
                agena_domain::ChannelRole::Member,
                Some("External participant".into()),
            )),
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
        assert_eq!(
            manager.bot_context(source.get()).await.unwrap()["enabled"],
            true
        );
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

        let (attempt, round, tool, message) = live_channel_tool(&manager, source, quote).await;
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
