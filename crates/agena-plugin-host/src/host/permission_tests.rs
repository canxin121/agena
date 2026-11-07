use super::*;
use crate::sdk::host_api::{PathPermissionQuery, PermissionQuery};
use serde_json::json;

#[derive(Default)]
struct PermissionHost {
    queries: std::sync::atomic::AtomicUsize,
    requirements: std::sync::atomic::AtomicUsize,
    mcp_authorizations: std::sync::atomic::AtomicUsize,
    mcp_starts: std::sync::atomic::AtomicUsize,
    allow_mcp: std::sync::atomic::AtomicBool,
    allowed_query_session: Option<i64>,
    query_contexts: std::sync::Mutex<Vec<HostCallbackContext>>,
}

#[async_trait::async_trait]
impl HostClient for PermissionHost {
    async fn log(&self, _: LogLevel, _: String, _: serde_json::Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> crate::sdk::Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, _: EventFilter) -> crate::sdk::Result<EventSubscription> {
        Ok(EventSubscription { id: "test".into() })
    }
    async fn read_config(&self, _: Option<String>) -> crate::sdk::Result<serde_json::Value> {
        Ok(json!({}))
    }
    async fn invoke_tool(
        &self,
        _: String,
        _: serde_json::Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        unreachable!()
    }
    async fn check_path_permission(
        &self,
        _: PathPermissionQuery,
    ) -> crate::sdk::Result<PermissionQuery> {
        self.queries
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let context = host_api::current_host_callback_context().unwrap_or_default();
        let allowed = self
            .allowed_query_session
            .is_some_and(|session| context.session_id == Some(session));
        self.query_contexts.lock().unwrap().push(context);
        if allowed {
            return Ok(PermissionQuery::Allow {});
        }
        Ok(PermissionQuery::Ask {
            reason: "review required".into(),
        })
    }
    async fn check_network_permission(&self, _: String) -> crate::sdk::Result<PermissionQuery> {
        self.check_path_permission(PathPermissionQuery::read("unused"))
            .await
    }
    async fn require_path_permission(&self, _: PathPermissionQuery) -> crate::sdk::Result<()> {
        let context = host_api::current_host_callback_context().unwrap();
        assert_eq!(
            (context.session_id, context.call_id),
            (Some(78135), Some(12))
        );
        self.requirements
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Err(PluginError::from_kind(
            PluginErrorKind::UserDeclined,
            "review declined",
        ))
    }
    async fn require_network_permission(&self, _: String) -> crate::sdk::Result<()> {
        self.require_path_permission(PathPermissionQuery::read("unused"))
            .await
    }

    async fn storage_list(
        &self,
        request: HostStorageListRequest,
    ) -> crate::sdk::Result<HostStorageListResponse> {
        assert!(request.namespace.is_none());
        assert!(request.prefix.is_none());
        Ok(HostStorageListResponse::default())
    }

    async fn require_mcp_server_permission(
        &self,
        _: crate::sdk::host_api::HostMcpServerSpec,
    ) -> crate::sdk::Result<()> {
        let context = host_api::current_host_callback_context().unwrap();
        assert_eq!(
            (context.session_id, context.call_id),
            (Some(78135), Some(12))
        );
        self.mcp_authorizations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.allow_mcp.load(std::sync::atomic::Ordering::Relaxed) {
            Ok(())
        } else {
            Err(PluginError::from_kind(
                PluginErrorKind::UserDeclined,
                "review declined",
            ))
        }
    }

    async fn mcp_add_server(
        &self,
        _: crate::sdk::host_api::HostMcpAddServerRequest,
    ) -> crate::sdk::Result<()> {
        self.mcp_starts
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Clone, Default)]
struct MessagesPermissionProbe {
    host: Arc<std::sync::OnceLock<Arc<dyn HostClient>>>,
}

#[crate::sdk::agena_plugin(
    namespace = "test",
    name = "messages-permissions",
    version = "test",
    summary = "Recall context only when the current session grants read access."
)]
impl MessagesPermissionProbe {
    #[hook(
        init,
        summary = "Bind permission queries",
        help = "Keep the scoped host supplied by the real dispatcher."
    )]
    async fn init(
        &self,
        _context: crate::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> crate::sdk::Result<crate::sdk::InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("permission probe initialized twice"))?;
        Ok(crate::sdk::InitOutcome::ack(crate::sdk::Plugin::manifest(
            self,
        )))
    }

    #[hook(chat.messages, summary = "Recall permitted context", help = "Query path permission through the real callback authority before adding context.")]
    async fn messages(
        &self,
        input: crate::sdk::ChatMessagesTransformInput,
    ) -> crate::sdk::Result<Option<crate::sdk::ChatMessagesTransformPatch>> {
        let allowed = self
            .host
            .get()
            .expect("initialized permission host")
            .check_path_permission(PathPermissionQuery::read("memory/record.md"))
            .await?
            .is_allowed();
        if !allowed {
            return Ok(None);
        }
        let mut messages = input.messages;
        messages.insert(
            0,
            crate::sdk::ChatMessage::system("permitted saved context"),
        );
        Ok(Some(crate::sdk::ChatMessagesTransformPatch {
            messages: Some(messages),
        }))
    }
}

#[tokio::test]
async fn chat_messages_permission_queries_use_input_session_and_workspace_without_tool_authority() {
    let inner = Arc::new(PermissionHost {
        allowed_query_session: Some(41),
        ..Default::default()
    });
    let key: PluginKey = "test.messages-permissions".parse().unwrap();
    let mut config = crate::config::PluginsConfig::default();
    config
        .list
        .insert(key.to_string(), ConfiguredPlugin::static_default());
    let host = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            key.clone(),
            MessagesPermissionProbe::default(),
        )],
        config,
        workspace_root: "/permission-tests".into(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: Some(inner.clone()),
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();

    for (session_id, workspace_root, expected_messages) in [
        (41, Some("/session-41"), 2),
        (42, Some("/session-42"), 1),
        (41, None, 2),
    ] {
        let transformed = host_api::run_in_isolated_host_callback_context(
            HostCallbackContext {
                session_id: Some(999),
                call_id: Some(777),
                tool_name: Some("test.unrelated_tool".into()),
                workspace_root: Some("/unrelated-session".into()),
                ..Default::default()
            },
            host.dispatch_chat_messages_transform(crate::sdk::ChatMessagesTransformInput {
                session_id,
                workspace_root: workspace_root.map(str::to_owned),
                messages: vec![crate::sdk::ChatMessage::user("recall saved context")],
            }),
        )
        .await
        .unwrap();
        assert_eq!(transformed.messages.len(), expected_messages);
        if expected_messages == 2 {
            assert_eq!(
                transformed.messages[0].text(),
                Some("permitted saved context")
            );
        }
    }

    let contexts = inner.query_contexts.lock().unwrap().clone();
    assert_eq!(contexts.len(), 3);
    assert_eq!(contexts[0].session_id, Some(41));
    assert_eq!(contexts[1].session_id, Some(42));
    assert_eq!(contexts[2].session_id, Some(41));
    assert_eq!(contexts[0].workspace_root.as_deref(), Some("/session-41"));
    assert_eq!(contexts[1].workspace_root.as_deref(), Some("/session-42"));
    // Legacy callers that omit the workspace must not inherit unrelated
    // ambient workspace authority from the thread running the hook.
    assert_eq!(contexts[2].workspace_root, None);
    for context in contexts {
        assert_eq!(
            context.plugin_id.as_deref(),
            Some("test.messages-permissions")
        );
        assert_eq!(context.call_id, None);
        assert_eq!(context.tool_name, None);
    }
    host.shutdown_report().await.into_result().unwrap();
}

fn context() -> HostCallbackContext {
    HostCallbackContext {
        session_id: Some(78135),
        call_id: Some(12),
        workspace_root: Some("/workspace".into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn scoped_queries_are_read_only_and_requirements_preserve_declines() {
    let inner = Arc::new(PermissionHost::default());
    let handle = Arc::new(HostHandle::new(inner.clone()));
    let plugin: PluginKey = "test.permissions".parse().unwrap();
    handle.begin_plugin_instance(plugin.clone());
    let scoped = handle.scoped_host_client(plugin.to_string());
    handle
        .run_in_authorized_callback_context(&plugin, context(), async {
            assert!(matches!(
                scoped
                    .check_path_permission(PathPermissionQuery::read("file"))
                    .await
                    .unwrap(),
                PermissionQuery::Ask { .. }
            ));
            assert!(matches!(
                scoped
                    .check_network_permission("https://example.org".into())
                    .await
                    .unwrap(),
                PermissionQuery::Ask { .. }
            ));
            assert_eq!(
                inner
                    .requirements
                    .load(std::sync::atomic::Ordering::Relaxed),
                0
            );
            assert_eq!(
                scoped
                    .require_path_permission(PathPermissionQuery::read("file"))
                    .await
                    .unwrap_err()
                    .kind,
                PluginErrorKind::UserDeclined
            );
            assert_eq!(
                scoped
                    .require_network_permission("https://example.org".into())
                    .await
                    .unwrap_err()
                    .kind,
                PluginErrorKind::UserDeclined
            );
        })
        .await;
    assert_eq!(inner.queries.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(
        inner
            .requirements
            .load(std::sync::atomic::Ordering::Relaxed),
        2
    );
}

#[tokio::test]
async fn permission_rpc_requires_live_exact_authority_and_forwards_the_enforcing_method() {
    let inner = Arc::new(PermissionHost::default());
    let handle = HostHandle::new(inner.clone());
    let plugin: PluginKey = "test.permissions".parse().unwrap();
    handle.begin_plugin_instance(plugin.clone());
    let (authorized, lease) = handle.issue_callback_authority(&plugin, context());
    for (method, input) in [
        (
            method::HOST_PERMISSION_REQUIRE_PATH,
            json!({"request":PathPermissionQuery::read("file")}),
        ),
        (
            method::HOST_PERMISSION_REQUIRE_NETWORK,
            json!({"target":"https://example.org"}),
        ),
    ] {
        let mut params = input.clone();
        params["context"] = json!(authorized);
        assert_eq!(
            handle
                .handle_call_for_plugin(&plugin.to_string(), method, params.clone())
                .await
                .unwrap_err()
                .kind,
            PluginErrorKind::UserDeclined
        );
        params["context"]["call_id"] = json!(13);
        assert_eq!(
            handle
                .handle_call_for_plugin(&plugin.to_string(), method, params)
                .await
                .unwrap_err()
                .kind,
            PluginErrorKind::PolicyDenied
        );
        let mut forged = input.clone();
        forged["context"] = json!(context());
        assert_eq!(
            handle
                .handle_call_for_plugin(&plugin.to_string(), method, forged)
                .await
                .unwrap_err()
                .kind,
            PluginErrorKind::PolicyDenied
        );
    }
    drop(lease);
    let expired = json!({"target":"https://example.org", "context":authorized});
    assert_eq!(
        handle
            .handle_call_for_plugin(
                &plugin.to_string(),
                method::HOST_PERMISSION_REQUIRE_NETWORK,
                expired
            )
            .await
            .unwrap_err()
            .kind,
        PluginErrorKind::PolicyDenied
    );
    assert_eq!(
        inner
            .requirements
            .load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    assert_eq!(inner.queries.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[tokio::test]
async fn native_mcp_authorization_preserves_approval_and_never_starts_a_server() {
    use crate::sdk::host_api::HostMcpServerSpec;
    use std::sync::atomic::Ordering;

    let inner = Arc::new(PermissionHost::default());
    let handle = Arc::new(HostHandle::new(inner.clone()));
    let plugin: PluginKey = "test.mcp".parse().unwrap();
    handle.begin_plugin_instance(plugin.clone());
    let scoped = handle.scoped_host_client(plugin.to_string());
    let spec = HostMcpServerSpec::Stdio {
        command: "must-not-be-executed".into(),
        args: Vec::new(),
        env: Default::default(),
        cwd: Some("/workspace".into()),
    };
    // Native lifecycle callbacks may have plugin-only context. A callback
    // claiming session authority still needs a live host-issued lease.
    let denied = host_api::run_in_host_callback_context(
        context(),
        scoped.require_mcp_server_permission(spec.clone()),
    )
    .await
    .unwrap_err();
    assert_eq!(denied.kind, PluginErrorKind::PolicyDenied);
    assert_eq!(inner.mcp_authorizations.load(Ordering::Relaxed), 0);
    handle
        .run_in_authorized_callback_context(&plugin, context(), async {
            assert_eq!(
                scoped
                    .require_mcp_server_permission(spec.clone())
                    .await
                    .unwrap_err()
                    .kind,
                PluginErrorKind::UserDeclined
            );
            inner.allow_mcp.store(true, Ordering::Relaxed);
            scoped.require_mcp_server_permission(spec).await.unwrap();
        })
        .await;
    assert_eq!(inner.mcp_authorizations.load(Ordering::Relaxed), 2);
    assert_eq!(inner.mcp_starts.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn required_request_rpc_envelopes_preserve_wire_validation() {
    let inner = Arc::new(PermissionHost::default());
    let handle = HostHandle::new(inner.clone());
    let request = json!(PathPermissionQuery::read("fixture.txt"));
    for params in [
        json!({"request": request}),
        json!({"request": request, "context": null}),
        json!({"request": request, "extension": {"enabled": true}}),
    ] {
        let result = handle
            .handle_call(method::HOST_PERMISSION_CHECK_PATH, params)
            .await
            .unwrap();
        assert!(matches!(
            serde_json::from_value::<PermissionQuery>(result).unwrap(),
            PermissionQuery::Ask { .. }
        ));
    }
    assert_eq!(inner.queries.load(std::sync::atomic::Ordering::Relaxed), 3);
    for params in [
        json!({}),
        json!({"context": null}),
        json!({"request": null}),
        json!({"request": "invalid"}),
        json!({"request": request, "context": 7}),
    ] {
        let error = handle
            .handle_call(method::HOST_PERMISSION_CHECK_PATH, params)
            .await
            .unwrap_err();
        assert_eq!(error.kind, PluginErrorKind::InvalidParams);
    }
    assert_eq!(inner.queries.load(std::sync::atomic::Ordering::Relaxed), 3);
}

#[tokio::test]
async fn rpc_defaulted_and_strict_envelopes_keep_distinct_contracts() {
    let handle = HostHandle::new(Arc::new(PermissionHost::default()));
    assert!(
        handle
            .handle_call(method::HOST_STORAGE_LIST, json!({}))
            .await
            .is_ok()
    );
    assert_eq!(
        handle
            .handle_call(method::HOST_STORAGE_LIST, json!({"request": null}))
            .await
            .unwrap_err()
            .kind,
        PluginErrorKind::InvalidParams
    );
    assert_eq!(
        handle
            .handle_call(
                method::HOST_CONFIG_RELOAD_STATUS,
                json!({"request": {"task_id": "fixture"}, "context": null})
            )
            .await
            .unwrap_err()
            .kind,
        PluginErrorKind::HostUnavailable
    );
    assert_eq!(
        handle
            .handle_call(
                method::HOST_CONFIG_RELOAD_STATUS,
                json!({"request": {"task_id": "fixture"}, "extension": true})
            )
            .await
            .unwrap_err()
            .kind,
        PluginErrorKind::InvalidParams
    );
}
