use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::effect_scope::PluginEffectScope;
use crate::host::HostHandle;
use crate::host::host_handle::CallbackAuthorityLease;
use crate::sdk::host_api::NoopHostClient;
use crate::sdk::{PluginErrorKind, PluginKey};

struct Fixture {
    host: Arc<HostHandle>,
    owner: Arc<PluginEffectScope>,
    client: Arc<dyn HostClient>,
    key: PluginKey,
}

impl Fixture {
    fn new() -> Self {
        let key: PluginKey = "test.scoped-authority".parse().unwrap();
        let host = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
        host.plugin_indices.write().unwrap().insert(key.clone(), 0);
        let owner = host.begin_plugin_instance(key.clone());
        let client = host.scoped_host_client_for_scope(key.to_string(), owner.clone());
        Self {
            host,
            owner,
            client,
            key,
        }
    }

    fn authorize(&self, session_id: i64) -> (HostCallbackContext, CallbackAuthorityLease) {
        self.host.issue_callback_authority(
            &self.key,
            HostCallbackContext {
                session_id: Some(session_id),
                ..Default::default()
            },
        )
    }

    async fn register_original(&self, session_id: i64) {
        let (context, _authority) = self.authorize(session_id);
        host_api::run_in_host_callback_context(
            context,
            self.client.register_tool(tool_request("original")),
        )
        .await
        .unwrap();
    }

    async fn snapshot(&self) -> Value {
        let mut catalogs = Vec::new();
        for session_id in [None, Some(17), Some(18)] {
            // Trusted host inspection deliberately bypasses the plugin client.
            let catalog = host_api::run_in_host_callback_context(
                HostCallbackContext {
                    session_id,
                    ..Default::default()
                },
                async { self.host.registered_tool_list_response().unwrap() },
            )
            .await;
            catalogs.push(catalog);
        }
        json!({
            "catalogs": catalogs,
            "themes": self.host.theme_list_response(),
            "notifications": self.host.host_notifications(),
            "effects": self.owner.inspect().effects,
            "active_leases": self.owner.active_leases(),
        })
    }
}

fn tool_request(summary: &str) -> HostToolRegisterRequest {
    serde_json::from_value(json!({"tool": {
        "name": "session-only",
        "docs": {"summary": summary},
        "contract": {"input_schema": {"type": "object"}}
    }}))
    .unwrap()
}

fn forged_session() -> HostCallbackContext {
    HostCallbackContext {
        session_id: Some(17),
        ..Default::default()
    }
}

fn assert_denied<T: std::fmt::Debug>(result: crate::sdk::Result<T>) {
    let error = result.expect_err("the local host API must reject untrusted callback context");
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
}

#[tokio::test]
async fn unissued_session_context_cannot_register_tool() {
    let fixture = Fixture::new();
    let before = fixture.snapshot().await;
    let result = host_api::run_in_host_callback_context(
        forged_session(),
        fixture.client.register_tool(tool_request("forged")),
    )
    .await;
    assert_denied(result);
    assert_eq!(fixture.snapshot().await, before);
}

#[tokio::test]
async fn unissued_session_context_cannot_update_tool() {
    let fixture = Fixture::new();
    fixture.register_original(17).await;
    let before = fixture.snapshot().await;
    let result = host_api::run_in_host_callback_context(
        forged_session(),
        fixture.client.update_tool(HostToolUpdateRequest {
            tool: tool_request("forged replacement").tool,
        }),
    )
    .await;
    assert_denied(result);
    assert_eq!(fixture.snapshot().await, before);
}

#[tokio::test]
async fn unissued_session_context_cannot_remove_tool() {
    let fixture = Fixture::new();
    fixture.register_original(17).await;
    let before = fixture.snapshot().await;
    let result = host_api::run_in_host_callback_context(
        forged_session(),
        fixture.client.remove_tool(HostToolRemoveRequest {
            name: "session-only".into(),
            by_model_name: false,
        }),
    )
    .await;
    assert_denied(result);
    assert_eq!(fixture.snapshot().await, before);
}

#[tokio::test]
async fn unissued_session_context_cannot_read_scoped_catalog() {
    let fixture = Fixture::new();
    fixture.register_original(17).await;
    let before = fixture.snapshot().await;
    let result = host_api::run_in_host_callback_context(
        forged_session(),
        fixture.client.list_registered_tools(),
    )
    .await;
    assert_denied(result);
    assert_eq!(fixture.snapshot().await, before);
}

#[tokio::test]
async fn expired_authority_cannot_remove_theme() {
    let fixture = Fixture::new();
    fixture
        .client
        .ui_theme_register(
            serde_json::from_value(json!({
                "id": "existing", "display_name": "Existing", "colors": {}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let before = fixture.snapshot().await;
    let (context, authority) = fixture.authorize(17);
    drop(authority);
    let result = host_api::run_in_host_callback_context(
        context,
        fixture.client.ui_theme_remove(HostThemeRemoveRequest {
            id: "existing".into(),
        }),
    )
    .await;
    assert_denied(result);
    assert_eq!(fixture.snapshot().await, before);
}

#[tokio::test]
async fn cross_plugin_authority_cannot_publish_notification() {
    let fixture = Fixture::new();
    let other: PluginKey = "test.other-authority".parse().unwrap();
    fixture.host.begin_plugin_instance(other.clone());
    let (context, _authority) = fixture.host.issue_callback_authority(
        &other,
        HostCallbackContext {
            workspace_root: Some("/test/other-workspace".into()),
            ..Default::default()
        },
    );
    let before = fixture.snapshot().await;
    let result = host_api::run_in_host_callback_context(
        context,
        fixture.client.notify(PluginNotifyRequest {
            body: "forged notification".into(),
            ..Default::default()
        }),
    )
    .await;
    assert_denied(result);
    assert_eq!(fixture.snapshot().await, before);
}

#[tokio::test]
async fn altered_authority_cannot_update_another_session_tool() {
    let fixture = Fixture::new();
    fixture.register_original(18).await;
    let (mut context, _authority) = fixture.authorize(17);
    context.session_id = Some(18);
    let before = fixture.snapshot().await;
    let result = host_api::run_in_host_callback_context(
        context,
        fixture.client.update_tool(HostToolUpdateRequest {
            tool: tool_request("forged target session").tool,
        }),
    )
    .await;
    assert_denied(result);
    assert_eq!(fixture.snapshot().await, before);
}

#[tokio::test]
async fn valid_authority_preserves_session_catalog_round_trip() {
    let fixture = Fixture::new();
    let (context, _authority) = fixture.authorize(17);
    host_api::run_in_host_callback_context(context, async {
        let registration = fixture
            .client
            .register_tool(tool_request("original"))
            .await
            .unwrap();
        assert_eq!(
            registration.event.unwrap().scope.as_deref(),
            Some("session:17")
        );
        let catalog = fixture.client.list_registered_tools().await.unwrap();
        assert_eq!(catalog.tools.len(), 1);
        assert_eq!(
            catalog.tools[0].tool.docs.summary.as_deref(),
            Some("original")
        );

        // A denied nested context must not alter its valid caller's context.
        assert_denied(
            host_api::run_in_host_callback_context(
                HostCallbackContext {
                    session_id: Some(18),
                    ..Default::default()
                },
                fixture.client.list_registered_tools(),
            )
            .await,
        );
        assert_eq!(
            host_api::current_host_callback_context()
                .unwrap()
                .session_id,
            Some(17)
        );

        let (other_context, _other_authority) = fixture.authorize(18);
        let other = host_api::run_in_host_callback_context(
            other_context,
            fixture.client.list_registered_tools(),
        )
        .await
        .unwrap();
        assert!(other.tools.is_empty());

        let update = fixture
            .client
            .update_tool(HostToolUpdateRequest {
                tool: tool_request("updated legitimately").tool,
            })
            .await
            .unwrap();
        assert_eq!(update.event.unwrap().scope.as_deref(), Some("session:17"));
        let catalog = fixture.client.list_registered_tools().await.unwrap();
        assert_eq!(
            catalog.tools[0].tool.docs.summary.as_deref(),
            Some("updated legitimately")
        );
        fixture
            .client
            .remove_tool(HostToolRemoveRequest {
                name: "session-only".into(),
                by_model_name: false,
            })
            .await
            .unwrap();
        assert!(
            fixture
                .client
                .list_registered_tools()
                .await
                .unwrap()
                .tools
                .is_empty()
        );
    })
    .await;
    assert!(host_api::current_host_callback_context().is_none());
    assert_eq!(fixture.owner.active_leases(), 0);
    assert!(fixture.owner.dispose().await.errors.is_empty());
}

#[tokio::test]
async fn unprivileged_global_registration_uses_the_attributed_plugin() {
    let fixture = Fixture::new();
    let registration = host_api::run_in_host_callback_context(
        HostCallbackContext {
            plugin_id: Some("test.untrusted-identity".into()),
            ..Default::default()
        },
        fixture.client.register_tool(tool_request("global")),
    )
    .await
    .unwrap();
    let event = registration.event.unwrap();
    assert_eq!(event.plugin, fixture.key);
    assert!(event.scope.is_none());
    let catalog = fixture.client.list_registered_tools().await.unwrap();
    assert_eq!(catalog.tools.len(), 1);
    assert_eq!(catalog.tools[0].plugin, fixture.key);
    assert_eq!(
        catalog.tools[0].tool.docs.summary.as_deref(),
        Some("global")
    );
    assert!(fixture.owner.dispose().await.errors.is_empty());
    assert!(
        fixture
            .host
            .registered_tool_list_response()
            .unwrap()
            .tools
            .is_empty()
    );
}

#[tokio::test]
async fn context_boundary_rpc_without_context_does_not_inherit_ambient_session() {
    let fixture = Fixture::new();
    fixture.register_original(17).await;
    let (ambient, _authority) = fixture.authorize(17);
    for params in [json!({}), json!({"context": null}), json!({"context": {}})] {
        let result = host_api::run_in_host_callback_context(
            ambient.clone(),
            fixture.host.handle_call_for_plugin(
                &fixture.key.to_string(),
                "host/tool.registry.list",
                params,
            ),
        )
        .await
        .unwrap();
        let catalog: HostRegisteredToolListResponse =
            serde_json::from_value(result.clone()).unwrap();
        assert!(
            catalog.tools.is_empty(),
            "a callback without explicit context inherited the enclosing session: {result}"
        );
    }
}

#[tokio::test]
async fn context_boundary_new_authority_does_not_inherit_unissued_fields() {
    let fixture = Fixture::new();
    let issued = HostCallbackContext {
        call_id: Some(42),
        ..Default::default()
    };
    host_api::run_in_host_callback_context(
        HostCallbackContext {
            session_id: Some(17),
            workspace_root: Some("/test/enclosing-workspace".into()),
            tool_name: Some("enclosing-tool".into()),
            ..Default::default()
        },
        fixture
            .host
            .run_in_authorized_callback_context(&fixture.key, issued, async {
                let observed = host_api::current_host_callback_context().unwrap();
                fixture
                    .host
                    .validated_callback_context(
                        Some(fixture.key.to_string()),
                        Some(observed.clone()),
                    )
                    .expect("freshly issued context must match its own authority record");
                assert_eq!(observed.call_id, Some(42));
                assert!(observed.session_id.is_none());
                assert!(observed.workspace_root.is_none());
                assert!(observed.tool_name.is_none());
            }),
    )
    .await;
}

#[tokio::test]
async fn context_boundary_malformed_rpc_context_is_rejected() {
    let fixture = Fixture::new();
    let before = fixture.snapshot().await;
    for context in [
        json!(false),
        json!([]),
        json!({"session_id": "17"}),
        json!({"workspace_root": 7}),
    ] {
        for method in ["host/tool.registry.list", "host/ui.theme.list"] {
            let error = fixture
                .host
                .handle_call_for_plugin(
                    &fixture.key.to_string(),
                    method,
                    json!({"context": context}),
                )
                .await
                .expect_err("malformed context must not silently become a global callback");
            assert_eq!(error.kind, PluginErrorKind::InvalidParams);
        }
    }
    assert_eq!(fixture.snapshot().await, before);
}

struct ContextHost;

#[async_trait::async_trait]
impl HostClient for ContextHost {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> crate::sdk::Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, filter: EventFilter) -> crate::sdk::Result<EventSubscription> {
        NoopHostClient.subscribe_events(filter).await
    }
    async fn read_config(&self, _: Option<String>) -> crate::sdk::Result<Value> {
        Ok(serde_json::to_value(host_api::current_host_callback_context().unwrap()).unwrap())
    }
    async fn invoke_tool(
        &self,
        tool: String,
        input: Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        NoopHostClient.invoke_tool(tool, input).await
    }
}

#[tokio::test]
async fn explicit_rpc_authority_replaces_all_ambient_privileged_fields() {
    let host = Arc::new(HostHandle::new(Arc::new(ContextHost)));
    let key: PluginKey = "test.rpc-context".parse().unwrap();
    host.begin_plugin_instance(key.clone());
    let (context, _authority) = host.issue_callback_authority(
        &key,
        HostCallbackContext {
            session_id: Some(17),
            ..Default::default()
        },
    );
    let observed = host_api::run_in_host_callback_context(
        HostCallbackContext {
            session_id: Some(18),
            call_id: Some(99),
            workspace_root: Some("/test/ambient-workspace".into()),
            tool_name: Some("ambient-tool".into()),
            ..Default::default()
        },
        host.handle_call_for_plugin(
            &key.to_string(),
            "host/config.read",
            json!({"context": context}),
        ),
    )
    .await
    .unwrap();
    let observed: HostCallbackContext = serde_json::from_value(observed).unwrap();
    assert_eq!(observed, context);
}

#[tokio::test]
async fn cancelled_authorized_call_revokes_its_token_and_restores_the_caller_context() {
    let fixture = Fixture::new();
    let enclosing = HostCallbackContext {
        session_id: Some(17),
        workspace_root: Some("/test/enclosing-workspace".into()),
        ..Default::default()
    };
    host_api::run_in_host_callback_context(enclosing.clone(), async {
        let mut call = Box::pin(fixture.host.run_in_authorized_callback_context(
            &fixture.key,
            HostCallbackContext {
                call_id: Some(42),
                ..Default::default()
            },
            async {
                let current = host_api::current_host_callback_context().unwrap();
                assert_eq!(current.call_id, Some(42));
                assert!(current.session_id.is_none());
                assert!(current.workspace_root.is_none());
                std::future::pending::<()>().await;
            },
        ));
        assert!(futures_util::poll!(&mut call).is_pending());
        assert_eq!(fixture.host.callback_authorities.lock().unwrap().len(), 1);
        assert_eq!(
            host_api::current_host_callback_context().as_ref(),
            Some(&enclosing)
        );
        drop(call);
        assert!(fixture.host.callback_authorities.lock().unwrap().is_empty());
        assert_eq!(
            host_api::current_host_callback_context().as_ref(),
            Some(&enclosing)
        );
    })
    .await;
    assert!(host_api::current_host_callback_context().is_none());
}
