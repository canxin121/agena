use super::*;
use agena_plugin_host::sdk::{
    EventEnvelope, EventFilter, PluginErrorKind, async_trait,
    host_api::{EventSubscription, LogLevel, NoopHostClient, PermissionQuery},
};

struct AddressHost {
    replies: BTreeMap<String, PermissionQuery>,
    seen: std::sync::Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl HostClient for AddressHost {
    async fn log(&self, _: LogLevel, _: String, _: serde_json::Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> SdkResult<()> {
        unreachable!("permission checks must not publish events")
    }
    async fn subscribe_events(&self, _: EventFilter) -> SdkResult<EventSubscription> {
        unreachable!("permission checks must not subscribe to events")
    }
    async fn read_config(&self, _: Option<String>) -> SdkResult<serde_json::Value> {
        panic!("DNS authorization must use the host authority, not reconstruct policy from config")
    }
    async fn invoke_tool(&self, _: String, _: serde_json::Value) -> SdkResult<ToolInvokeOutput> {
        unreachable!("permission checks must not invoke tools")
    }
    async fn prepare_worker_sandbox(
        &self,
        _: agena_plugin_sdk::host_api::HostProcessResources,
    ) -> SdkResult<agena_plugin_sdk::host_api::HostPreparedProcessSandbox> {
        Ok(agena_plugin_sdk::host_api::HostPreparedProcessSandbox {
            scope: "route-fixture".into(),
            workspace_root: String::new(),
            cwd: String::new(),
            policy: serde_json::to_value(agena_process::sandbox::SandboxPolicy::default()).unwrap(),
            permission_snapshot: None,
            call_bound: false,
        })
    }
    async fn check_resolved_network_permission(
        &self,
        original: String,
        resolved: String,
    ) -> SdkResult<PermissionQuery> {
        self.seen.lock().unwrap().push((original, resolved.clone()));
        Ok(self
            .replies
            .get(&resolved)
            .cloned()
            .unwrap_or(PermissionQuery::Deny {
                reason: "fixture IP/CIDR ceiling".into(),
            }))
    }
}

fn address_host(replies: &[(&str, PermissionQuery)]) -> AddressHost {
    AddressHost {
        replies: replies
            .iter()
            .map(|(address, query)| (address.to_string(), query.clone()))
            .collect(),
        seen: Default::default(),
    }
}

#[tokio::test]
async fn http_dns_results_keep_only_addresses_allowed_by_the_scoped_host() {
    let url = url::Url::parse("https://example.org/data").unwrap();
    let allowed = "8.8.8.8:443".parse().unwrap();
    let denied = "1.1.1.1:443".parse().unwrap();
    let host = address_host(&[("8.8.8.8:443", PermissionQuery::Allow {})]);
    let addresses = authorized_network_addresses(&url, vec![denied, allowed], &host, false)
        .await
        .unwrap();
    assert_eq!(addresses, vec![allowed]);
    assert_eq!(
        host.seen.lock().unwrap().as_slice(),
        [
            ("https://example.org/data".into(), "1.1.1.1:443".into()),
            ("https://example.org/data".into(), "8.8.8.8:443".into()),
        ]
    );
}

#[tokio::test]
async fn unpinned_transport_rejects_mixed_allowed_and_denied_dns_answers() {
    let url = url::Url::parse("https://example.org").unwrap();
    let host = address_host(&[("8.8.8.8:443", PermissionQuery::Allow {})]);
    let error = authorized_network_addresses(
        &url,
        vec![
            "8.8.8.8:443".parse().unwrap(),
            "1.1.1.1:443".parse().unwrap(),
        ],
        &host,
        true,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    assert!(error.to_string().contains("fixture IP/CIDR ceiling"));
}

#[tokio::test]
async fn dns_checks_fail_closed_for_ask_denied_or_unsupported_hosts() {
    let url = url::Url::parse("https://example.org").unwrap();
    let addresses = vec!["8.8.8.8:443".parse().unwrap()];
    for answer in [
        PermissionQuery::Ask {
            reason: "IP approval missing".into(),
        },
        PermissionQuery::Deny {
            reason: "IP blocked".into(),
        },
    ] {
        let host = address_host(&[("8.8.8.8:443", answer)]);
        assert_eq!(
            authorized_network_addresses(&url, addresses.clone(), &host, false)
                .await
                .unwrap_err()
                .kind,
            PluginErrorKind::PolicyDenied,
        );
    }
    assert_eq!(
        authorized_network_addresses(&url, addresses, &NoopHostClient, false)
            .await
            .unwrap_err()
            .kind,
        PluginErrorKind::HostUnavailable,
    );
}

#[tokio::test]
async fn direct_http_route_retains_pinned_ip_and_host_denials() {
    let url = url::Url::parse("https://8.8.8.8").unwrap();
    let network = network_policy::WebNetworkConfig {
        proxy: Some(String::new()),
        ..Default::default()
    };
    let host = address_host(&[("8.8.8.8:443", PermissionQuery::Allow {})]);
    let route = resolve_public_http_route(&url, &network, &host)
        .await
        .unwrap();
    assert!(route.cache_key().contains("8.8.8.8:443"));
    let denied = address_host(&[]);
    assert_eq!(
        resolve_public_http_route(&url, &network, &denied)
            .await
            .unwrap_err()
            .kind,
        PluginErrorKind::PolicyDenied,
    );
}

#[tokio::test]
async fn remote_dns_proxy_is_rejected_before_host_authorization_or_connection() {
    let url = url::Url::parse("https://remote-dns-policy.example.invalid").unwrap();
    let network = network_policy::WebNetworkConfig {
        proxy: Some("http://127.0.0.1:9".into()),
        ..Default::default()
    };
    let host = address_host(&[]);
    let error = resolve_public_http_route(&url, &network, &host)
        .await
        .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    assert!(error.to_string().contains("remote DNS"));
    assert!(host.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn direct_rendered_fetch_uses_the_invocation_sandbox_mode() {
    use agena_plugin_host::sdk::host_api::{
        HostCallbackContext, run_in_isolated_host_callback_context,
    };

    let previous = rendered_fetch::runtime_available();
    run_in_isolated_host_callback_context(
        HostCallbackContext {
            sandbox_enabled: Some(false),
            ..Default::default()
        },
        async {
            rendered_fetch::require_runtime().unwrap();
        },
    )
    .await;
    assert_eq!(rendered_fetch::runtime_available(), previous);
    assert_eq!(previous, !agena_process::sandbox::enabled());
}

#[tokio::test]
async fn render_gate_prevents_launching_chrome_for_sandboxed_fetch() {
    let plugin = WebPlugin::default();
    let url = url::Url::parse("https://example.org").unwrap();
    let error = rendered_fetch::fetch(&plugin, &url, &WebFetchOptions::default())
        .await
        .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::HostUnavailable);
    assert!(plugin.state.get().is_none());
    assert!(!plugin.renderer.running().await.unwrap());
}
