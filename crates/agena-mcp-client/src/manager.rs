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
use crate::transport::{HttpTransport, HttpTransportMode, StdioTransport};

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
    },
}

pub struct ConnectedServer {
    pub name: String,
    pub client: Arc<McpClient>,
    pub tools: Vec<ToolDescriptor>,
}

#[derive(Default)]
pub struct McpConnectionManager {
    inner: RwLock<Inner>,
    sampling_handler: arc_swap::ArcSwapOption<ServerRequestHandler>,
    client_name: String,
    client_version: String,
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
        }
    }

    /// Install the handler that will be called when *any* MCP server
    /// invokes a server→client request such as `sampling/createMessage`.
    pub fn set_sampling_handler(&self, handler: ServerRequestHandler) {
        self.sampling_handler.store(Some(Arc::new(handler)));
    }

    /// Spawn a new server, perform `initialize`, and pre-load its tool list.
    pub async fn add_server(&self, name: &str, spec: ServerSpec) -> McpResult<()> {
        let client = match spec {
            ServerSpec::Stdio { command, args, env, cwd } => {
                let t = StdioTransport::spawn(&command, &args, &env, cwd.as_ref()).await?;
                McpClient::new(Arc::new(t))
            }
            ServerSpec::Http { url, mode, headers } => {
                let t = HttpTransport::connect(url, mode, headers).await?;
                McpClient::new(Arc::new(t))
            }
        };
        // Wire shared sampling handler if installed.
        if let Some(h) = self.sampling_handler.load_full() {
            client.set_server_request_handler((*h).clone());
        }
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
            client: Arc::new(client),
            tools,
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

    pub async fn refresh_tools(&self, name: &str) -> McpResult<Vec<ToolDescriptor>> {
        let server = self.get(name).await?;
        let tools = server.client.list_tools().await?.tools;
        let mut g = self.inner.write().await;
        if let Some(existing) = g.servers.get_mut(name) {
            let new_server = Arc::new(ConnectedServer {
                name: existing.name.clone(),
                client: existing.client.clone(),
                tools: tools.clone(),
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
        self.get(server).await?.client.call_tool(tool, arguments).await
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
        self.get(server).await?.client.get_prompt(name, arguments).await
    }

    pub async fn shutdown_all(&self) {
        let mut g = self.inner.write().await;
        for (_, s) in std::mem::take(&mut g.servers) {
            let _ = s.client.shutdown().await;
        }
    }
}
