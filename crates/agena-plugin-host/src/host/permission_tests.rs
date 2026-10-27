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
