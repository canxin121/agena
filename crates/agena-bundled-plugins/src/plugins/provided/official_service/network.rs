//! Provider tools bind every actual connection to the IPs permitted by the
//! current invocation. The model provider's independent transport is separate.
use super::*;
use agena_plugin_host::sdk::{PluginErrorKind, host_api::PermissionQuery};
use std::net::SocketAddr;

pub(crate) struct AuthorizedProviderHttpClient {
    client: reqwest::Client,
    route: agena_web::WebHttpRoute,
    connect_timeout: Duration,
    timeout: Duration,
}

impl std::ops::Deref for AuthorizedProviderHttpClient {
    type Target = reqwest::Client;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl AuthorizedProviderHttpClient {
    pub(crate) async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, agena_web::CrawlError> {
        self.execute(request.build()?).await
    }

    pub(crate) async fn execute(
        &self,
        request: reqwest::Request,
    ) -> Result<reqwest::Response, agena_web::CrawlError> {
        self.route
            .execute(
                &self.client,
                request,
                self.connect_timeout,
                Some(self.timeout),
                None,
            )
            .await
    }
}

pub(crate) async fn authorized_provider_addresses(
    host: &Arc<dyn HostClient>,
    url: &str,
    timeout: Duration,
) -> SdkResult<Vec<SocketAddr>> {
    let permission_url = provider_permission_url(url)?;
    host.require_network_permission(permission_url.to_string())
        .await?;
    let hostname = permission_url
        .host_str()
        .ok_or_else(|| PluginError::invalid_params("provider URL has no host"))?;
    let port = permission_url
        .port_or_known_default()
        .ok_or_else(|| PluginError::invalid_params("provider URL has no port"))?;
    let addresses = tokio::time::timeout(
        timeout,
        tokio::net::lookup_host((hostname.trim_matches(['[', ']']), port)),
    )
    .await
    .map_err(|_| PluginError::internal("provider endpoint DNS lookup timed out"))?
    .map_err(|_| PluginError::internal("provider endpoint DNS lookup failed"))?
    .collect::<std::collections::BTreeSet<_>>();
    let mut permitted = Vec::new();
    for address in addresses {
        match host
            .check_resolved_network_permission(permission_url.to_string(), address.to_string())
            .await?
        {
            PermissionQuery::Allow {} => permitted.push(address),
            PermissionQuery::Deny { .. } | PermissionQuery::Ask { .. } => {}
        }
    }
    if permitted.is_empty() {
        return Err(PluginError::from_kind(
            PluginErrorKind::PolicyDenied,
            "no resolved provider endpoint IP is permitted by the active network policy",
        ));
    }
    Ok(permitted)
}

pub(crate) async fn authorized_provider_http_client(
    host: &Arc<dyn HostClient>,
    url: &str,
    connection: &agena_plugin_sdk::host_api::BuiltinProviderConnection,
) -> SdkResult<AuthorizedProviderHttpClient> {
    let permission_url = provider_permission_url(url)?;
    let route = agena_web::WebHttpRoute::for_url(&permission_url, None)
        .map_err(|_| PluginError::invalid_params("invalid provider HTTP route"))?;
    authorized_provider_http_client_for_route(host, url, connection, route).await
}

async fn authorized_provider_http_client_for_route(
    host: &Arc<dyn HostClient>,
    url: &str,
    connection: &agena_plugin_sdk::host_api::BuiltinProviderConnection,
    route: agena_web::WebHttpRoute,
) -> SdkResult<AuthorizedProviderHttpClient> {
    let permission_url = provider_permission_url(url)?;
    if route.remote_dns() {
        return Err(PluginError::from_kind(
            PluginErrorKind::PolicyDenied,
            "the provider tool proxy uses remote DNS, so approved endpoint IPs cannot be pinned; configure a direct route or a SOCKS proxy with local DNS",
        ));
    }
    let addresses = authorized_provider_addresses(
        host,
        url,
        Duration::from_secs(connection.connect_timeout_secs.max(1).min(30)),
    )
    .await?;
    let route = crate::sandbox_operations::http_route(
        host.as_ref(),
        &permission_url,
        route.with_addresses(addresses),
    )
    .await?;
    let client = route
        .apply(provider_http_builder(connection), &permission_url)
        .build()
        .map_err(|error| PluginError::internal_error(&error))?;
    Ok(AuthorizedProviderHttpClient {
        client,
        route,
        connect_timeout: Duration::from_secs(connection.connect_timeout_secs.max(1)),
        timeout: Duration::from_secs(connection.timeout_secs.max(1)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_host::sdk::{
        EventEnvelope, EventFilter,
        host_api::{EventSubscription, LogLevel},
    };

    struct AddressHost {
        allowed: Option<SocketAddr>,
        denied: PermissionQuery,
        seen: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl HostClient for AddressHost {
        async fn log(&self, _: LogLevel, _: String, _: serde_json::Value) {}
        async fn publish_event(&self, _: EventEnvelope) -> SdkResult<()> {
            unreachable!()
        }
        async fn subscribe_events(&self, _: EventFilter) -> SdkResult<EventSubscription> {
            unreachable!()
        }
        async fn read_config(&self, _: Option<String>) -> SdkResult<serde_json::Value> {
            unreachable!()
        }
        async fn invoke_tool(
            &self,
            _: String,
            _: serde_json::Value,
        ) -> SdkResult<ToolInvokeOutput> {
            unreachable!()
        }
        async fn check_network_permission(&self, _: String) -> SdkResult<PermissionQuery> {
            Ok(PermissionQuery::Allow {})
        }
        async fn check_resolved_network_permission(
            &self,
            original: String,
            resolved: String,
        ) -> SdkResult<PermissionQuery> {
            self.seen.lock().unwrap().push((original, resolved.clone()));
            Ok(
                if self
                    .allowed
                    .is_some_and(|address| address.to_string() == resolved)
                {
                    PermissionQuery::Allow {}
                } else {
                    self.denied.clone()
                },
            )
        }
    }

    fn connection(url: &str) -> agena_plugin_sdk::host_api::BuiltinProviderConnection {
        agena_plugin_sdk::host_api::BuiltinProviderConnection {
            provider_id: "fixture".into(),
            base_url: url.into(),
            messages_url: None,
            realtime_ws_url: None,
            headers: BTreeMap::new(),
            model: None,
            timeout_secs: 3,
            connect_timeout_secs: 2,
            receipt_key: "fixture".into(),
        }
    }

    #[tokio::test]
    async fn provider_ip_denials_and_ask_stop_before_connecting_and_hide_query_secrets() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "http://{}/endpoint?key=private-query#fragment",
            listener.local_addr().unwrap()
        );
        for decision in [
            PermissionQuery::Deny {
                reason: "denied".into(),
            },
            PermissionQuery::Ask {
                reason: "ask".into(),
            },
        ] {
            let host = Arc::new(AddressHost {
                allowed: None,
                denied: decision,
                seen: Default::default(),
            });
            let authority: Arc<dyn HostClient> = host.clone();
            let error = authorized_provider_addresses(&authority, &url, Duration::from_secs(1))
                .await
                .unwrap_err();
            assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
            assert_eq!(host.seen.lock().unwrap().len(), 1);
            for (original, _) in host.seen.lock().unwrap().iter() {
                assert!(!original.contains("private-query"));
                assert!(!original.contains('?'));
                assert!(!original.contains('#'));
            }
            assert!(!error.to_string().contains("private-query"));
        }
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[tokio::test]
    async fn provider_http_client_pins_approved_ips_with_sandbox_disabled() {
        agena_process::sandbox::scope_enabled(false, async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = listener.local_addr().unwrap();
            let url = format!("http://localhost:{}/endpoint", endpoint.port());
            let host = Arc::new(AddressHost {
                allowed: Some(endpoint),
                denied: PermissionQuery::Deny {
                    reason: "other address denied".into(),
                },
                seen: Default::default(),
            });
            let authority: Arc<dyn HostClient> = host.clone();
            let client = authorized_provider_http_client_for_route(
                &authority,
                &url,
                &connection(&url),
                agena_web::WebHttpRoute::default(),
            )
            .await
            .unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                    assert!(request.len() < 16_384);
                }
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await
                    .unwrap();
                String::from_utf8(request).unwrap()
            });
            assert_eq!(
                client
                    .send(client.get(&url))
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "ok"
            );
            assert!(
                server
                    .await
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains(&format!("host: localhost:{}\r\n", endpoint.port()))
            );
            assert!(
                host.seen
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(_, resolved)| resolved == &endpoint.to_string())
            );
        })
        .await;
    }
}
