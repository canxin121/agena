//! Connection manager — owns one [`McpClient`] per configured server,
//! handles startup, registers a shared sampling handler, and exposes a
//! flat tool catalog `Vec<(server_name, ToolDescriptor)>` for the caller.
//!
//! The manager is intentionally synchronous in its registration calls
//! (caller drives ordering) and async in its tool invocation calls.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
use url::Url;

use crate::client::{McpClient, ServerRequestHandler};
use crate::error::{McpError, McpResult};
use crate::protocol::{
    CallToolResult, GetPromptResult, ListPromptsResult, ListResourcesResult, ReadResourceResult,
    ToolDescriptor,
};
use crate::transport::{HttpTransport, HttpTransportMode, StdioTransport, WsTransport};

#[derive(Debug, Clone)]
pub enum ServerSpec {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        cwd: Option<PathBuf>,
    },
    Http {
        url: Url,
        mode: HttpTransportMode,
        headers: HashMap<String, String>,
        auth: Option<HttpAuth>,
    },
    Ws {
        url: Url,
        headers: HashMap<String, String>,
        auth: Option<HttpAuth>,
    },
}

/// Authentication strategy for remote MCP servers. Materialized into an
/// `Authorization` header at connect time; explicit `headers` always win
/// when both are set.
#[derive(Debug, Clone)]
pub enum HttpAuth {
    /// Static `Authorization: Bearer <token>` header. Most cloud-hosted
    /// MCP servers use this.
    Bearer(String),
    /// Resolve the bearer token at connect time by reading the named env
    /// var. Empty / missing values are treated as "no auth" with a warn.
    BearerFromEnv(String),
    /// Read the bearer token from the per-server entry in
    /// [`crate::TokenStore`] (configured separately).
    BearerFromStore,
    /// Free-form: every (header, value) pair is set verbatim.
    Custom(HashMap<String, String>),
}

pub struct ConnectedServer {
    pub name: String,
    pub client: Arc<McpClient>,
    pub tools: Vec<ToolDescriptor>,
    pub network_target: Option<String>,
}

#[derive(Default)]
pub struct McpConnectionManager {
    inner: Arc<RwLock<Inner>>,
    sampling_handler: arc_swap::ArcSwapOption<ServerRequestHandler>,
    client_name: String,
    client_version: String,
    token_store: Option<Arc<dyn TokenStore>>,
}

#[derive(Default)]
struct Inner {
    servers: BTreeMap<String, Arc<ConnectedServer>>,
}

impl McpConnectionManager {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self {
            inner: Default::default(),
            sampling_handler: arc_swap::ArcSwapOption::from(None),
            client_name: client_name.into(),
            client_version: client_version.into(),
            token_store: None,
        }
    }

    /// Install a token store so `HttpAuth::BearerFromStore` can resolve.
    pub fn set_token_store(&mut self, store: Arc<dyn TokenStore>) {
        self.token_store = Some(store);
    }

    /// Install the handler that will be called when *any* MCP server
    /// invokes a server→client request such as `sampling/createMessage`.
    pub fn set_sampling_handler(&self, handler: ServerRequestHandler) {
        self.sampling_handler.store(Some(Arc::new(handler)));
    }

    /// Spawn a new server, perform `initialize`, and pre-load its tool list.
    pub async fn add_server(&self, name: &str, spec: ServerSpec) -> McpResult<()> {
        let (client, network_target) = match spec {
            ServerSpec::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let t = StdioTransport::spawn(&command, &args, &env, cwd.as_ref()).await?;
                (McpClient::new(Arc::new(t)), None)
            }
            ServerSpec::Http {
                url,
                mode,
                mut headers,
                auth,
            } => {
                let network_target = Some(url.to_string());
                if let Some(auth) = auth {
                    apply_http_auth(name, auth, &mut headers, self.token_store.as_deref());
                }
                let t = HttpTransport::connect(url, mode, headers).await?;
                (McpClient::new(Arc::new(t)), network_target)
            }
            ServerSpec::Ws {
                url,
                mut headers,
                auth,
            } => {
                let network_target = Some(url.to_string());
                if let Some(auth) = auth {
                    apply_http_auth(name, auth, &mut headers, self.token_store.as_deref());
                }
                let t = WsTransport::connect(url, headers).await?;
                (McpClient::new(Arc::new(t)), network_target)
            }
        };
        // Wire shared sampling handler if installed.
        if let Some(h) = self.sampling_handler.load_full() {
            client.set_server_request_handler((*h).clone());
        }
        // Auto-refresh the tools cache when the server sends
        // notifications/tools/list_changed. We hold a weak ref so the
        // notification handler does not keep the manager alive past its
        // owners.
        let weak_inner = Arc::downgrade(&self.inner);
        let server_name_owned = name.to_string();
        let client_arc_for_handler: Arc<McpClient> = Arc::new(client);
        let client_weak = Arc::downgrade(&client_arc_for_handler);
        client_arc_for_handler.set_notification_handler(Arc::new(move |method, _params| {
            if method != crate::protocol::method::TOOLS_LIST_CHANGED {
                return;
            }
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };
            let Some(client) = client_weak.upgrade() else {
                return;
            };
            let server_name = server_name_owned.clone();
            tokio::spawn(async move {
                let tools = match client.list_tools().await {
                    Ok(r) => r.tools,
                    Err(err) => {
                        tracing::warn!(
                            target: "agena_mcp_client::manager",
                            server = %server_name,
                            "auto-refresh after tools/list_changed failed: {err}"
                        );
                        return;
                    }
                };
                let mut g = inner.write().await;
                if let Some(existing) = g.servers.get_mut(&server_name) {
                    let new_server = Arc::new(ConnectedServer {
                        name: existing.name.clone(),
                        client: existing.client.clone(),
                        tools,
                        network_target: existing.network_target.clone(),
                    });
                    *existing = new_server;
                    tracing::debug!(
                        target: "agena_mcp_client::manager",
                        server = %server_name,
                        "tool catalog refreshed via list_changed"
                    );
                }
            });
        }));
        let client = client_arc_for_handler;
        client
            .initialize(&self.client_name, &self.client_version)
            .await?;
        let tools = match client.list_tools().await {
            Ok(r) => r.tools,
            Err(e) => {
                tracing::warn!(
                    target: "agena_mcp_client::manager",
                    "list_tools failed for '{name}': {e}"
                );
                Vec::new()
            }
        };
        let connected = Arc::new(ConnectedServer {
            name: name.to_string(),
            client,
            tools,
            network_target,
        });
        let mut g = self.inner.write().await;
        g.servers.insert(name.to_string(), connected);
        Ok(())
    }

    pub async fn remove_server(&self, name: &str) -> McpResult<()> {
        let mut g = self.inner.write().await;
        if let Some(s) = g.servers.remove(name) {
            let _ = s.client.shutdown().await;
        }
        Ok(())
    }

    pub async fn server_names(&self) -> Vec<String> {
        let g = self.inner.read().await;
        g.servers.keys().cloned().collect()
    }

    /// Flat catalog of `(server_name, tool_descriptor)` entries.
    pub async fn all_tools(&self) -> Vec<(String, ToolDescriptor)> {
        let g = self.inner.read().await;
        g.servers
            .values()
            .flat_map(|s| s.tools.iter().map(|t| (s.name.clone(), t.clone())))
            .collect()
    }

    pub async fn server_network_targets(&self) -> BTreeMap<String, String> {
        let g = self.inner.read().await;
        g.servers
            .iter()
            .filter_map(|(name, server)| {
                server
                    .network_target
                    .clone()
                    .map(|target| (name.clone(), target))
            })
            .collect()
    }

    pub async fn refresh_tools(&self, name: &str) -> McpResult<Vec<ToolDescriptor>> {
        let server = self.get(name).await?;
        let tools = server.client.list_tools().await?.tools;
        let mut g = self.inner.write().await;
        if let Some(existing) = g.servers.get_mut(name) {
            let new_server = Arc::new(ConnectedServer {
                name: existing.name.clone(),
                client: existing.client.clone(),
                tools: tools.clone(),
                network_target: existing.network_target.clone(),
            });
            *existing = new_server;
        }
        Ok(tools)
    }

    async fn get(&self, name: &str) -> McpResult<Arc<ConnectedServer>> {
        let g = self.inner.read().await;
        g.servers
            .get(name)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_string()))
    }

    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<Value>,
    ) -> McpResult<CallToolResult> {
        self.get(server)
            .await?
            .client
            .call_tool(tool, arguments)
            .await
    }

    pub async fn list_resources(&self, server: &str) -> McpResult<ListResourcesResult> {
        self.get(server).await?.client.list_resources().await
    }

    pub async fn read_resource(&self, server: &str, uri: &str) -> McpResult<ReadResourceResult> {
        self.get(server).await?.client.read_resource(uri).await
    }

    pub async fn list_prompts(&self, server: &str) -> McpResult<ListPromptsResult> {
        self.get(server).await?.client.list_prompts().await
    }

    pub async fn get_prompt(
        &self,
        server: &str,
        name: &str,
        arguments: Option<std::collections::BTreeMap<String, String>>,
    ) -> McpResult<GetPromptResult> {
        self.get(server)
            .await?
            .client
            .get_prompt(name, arguments)
            .await
    }

    pub async fn shutdown_all(&self) {
        let mut g = self.inner.write().await;
        for (_, s) in std::mem::take(&mut g.servers) {
            let _ = s.client.shutdown().await;
        }
    }
}

/// Pluggable per-server credential lookup. Implementations resolve a
/// bearer token by server name; missing entries return `None`.
pub trait TokenStore: Send + Sync {
    fn bearer(&self, server: &str) -> Option<String>;
}

const AUTH_HEADER: &str = "Authorization";

fn apply_http_auth(
    server: &str,
    auth: HttpAuth,
    headers: &mut HashMap<String, String>,
    store: Option<&dyn TokenStore>,
) {
    if has_auth_header(headers) {
        tracing::debug!(
            target: "agena_mcp_client::auth",
            server,
            "explicit Authorization header set; skipping HttpAuth"
        );
        return;
    }
    match auth {
        HttpAuth::Bearer(token) => set_bearer(headers, &token, server),
        HttpAuth::BearerFromEnv(var) => match std::env::var(&var).ok().filter(|t| !t.is_empty()) {
            Some(token) => set_bearer(headers, &token, server),
            None => tracing::warn!(
                target: "agena_mcp_client::auth",
                server,
                env = %var,
                "HttpAuth::BearerFromEnv referenced an unset env var"
            ),
        },
        HttpAuth::BearerFromStore => {
            let token = store
                .and_then(|s| s.bearer(server))
                .filter(|t| !t.is_empty());
            match token {
                Some(token) => set_bearer(headers, &token, server),
                None => tracing::warn!(
                    target: "agena_mcp_client::auth",
                    server,
                    "HttpAuth::BearerFromStore had no token (store missing or empty)"
                ),
            }
        }
        HttpAuth::Custom(map) => {
            for (k, v) in map {
                headers.entry(k).or_insert(v);
            }
        }
    }
}

fn has_auth_header(headers: &HashMap<String, String>) -> bool {
    headers.keys().any(|k| k.eq_ignore_ascii_case(AUTH_HEADER))
}

fn set_bearer(headers: &mut HashMap<String, String>, token: &str, server: &str) {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        tracing::warn!(
            target: "agena_mcp_client::auth",
            server,
            "bearer token resolved to an empty string; not setting Authorization"
        );
        return;
    }
    headers.insert(AUTH_HEADER.to_string(), format!("Bearer {trimmed}"));
}
