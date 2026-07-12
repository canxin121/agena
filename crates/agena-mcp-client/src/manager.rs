use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use http::{HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::model::{
    ClientCapabilities, ClientInfo, Content, GetPromptRequestParams, Implementation,
    PromptMessageContent, PromptMessageRole, ReadResourceRequestParams,
    ResourceContents as RmcpResourceContents, Tool,
};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock};
use tracing::warn;
use url::Url;

use crate::error::{McpError, McpResult};
use crate::protocol::{
    CallToolResult, ContentBlock, GetPromptResult, ListPromptsResult, ListResourcesResult,
    ReadResourceResult, ResourceContents, ResourceDescriptor, ToolDescriptor,
};

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
        headers: HashMap<String, String>,
        auth: Option<HttpAuth>,
    },
}

#[derive(Debug, Clone)]
pub enum HttpAuth {
    Bearer(String),
    BearerFromEnv(String),
    BearerFromStore,
    Custom(HashMap<String, String>),
}

type RunningClient = RunningService<RoleClient, ClientInfo>;

pub struct ConnectedServer {
    name: String,
    peer: Peer<RoleClient>,
    running: Mutex<Option<RunningClient>>,
    tools: RwLock<Vec<ToolDescriptor>>,
    network_target: Option<String>,
}

impl ConnectedServer {
    fn new(
        name: String,
        peer: Peer<RoleClient>,
        running: RunningClient,
        tools: Vec<ToolDescriptor>,
        network_target: Option<String>,
    ) -> Self {
        Self {
            name,
            peer,
            running: Mutex::new(Some(running)),
            tools: RwLock::new(tools),
            network_target,
        }
    }
}

#[derive(Default)]
pub struct McpConnectionManager {
    inner: Arc<RwLock<Inner>>,
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
            client_name: client_name.into(),
            client_version: client_version.into(),
            token_store: None,
        }
    }

    pub fn set_token_store(&mut self, store: Arc<dyn TokenStore>) {
        self.token_store = Some(store);
    }

    pub async fn add_server(&self, name: &str, spec: ServerSpec) -> McpResult<()> {
        if let Some(existing) = {
            let mut inner = self.inner.write().await;
            inner.servers.remove(name)
        } {
            shutdown_server(existing).await;
        }

        let client_info = self.client_info();
        let (running, network_target) = match spec {
            ServerSpec::Stdio {
                command,
                args,
                env,
                cwd,
            } => (
                connect_stdio(client_info, command, args, env, cwd).await?,
                None,
            ),
            ServerSpec::Http { url, headers, auth } => {
                let target = Some(url.to_string());
                let running = connect_http(
                    client_info,
                    name,
                    url,
                    headers,
                    auth,
                    self.token_store.as_deref(),
                )
                .await?;
                (running, target)
            }
        };

        let peer = running.peer().clone();
        let tools = convert_tools(peer.list_all_tools().await?);
        let connected = Arc::new(ConnectedServer::new(
            name.to_string(),
            peer,
            running,
            tools,
            network_target,
        ));

        let mut inner = self.inner.write().await;
        inner.servers.insert(name.to_string(), connected);
        Ok(())
    }

    pub async fn remove_server(&self, name: &str) -> McpResult<()> {
        let removed = {
            let mut inner = self.inner.write().await;
            inner.servers.remove(name)
        };
        if let Some(server) = removed {
            shutdown_server(server).await;
        }
        Ok(())
    }

    pub async fn server_names(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner.servers.keys().cloned().collect()
    }

    pub async fn all_tools(&self) -> Vec<(String, ToolDescriptor)> {
        let servers = {
            let inner = self.inner.read().await;
            inner.servers.values().cloned().collect::<Vec<_>>()
        };
        let mut out = Vec::new();
        for server in servers {
            let tools = server.tools.read().await.clone();
            out.extend(tools.into_iter().map(|tool| (server.name.clone(), tool)));
        }
        out
    }

    pub async fn server_network_targets(&self) -> BTreeMap<String, String> {
        let inner = self.inner.read().await;
        inner
            .servers
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
        let tools = convert_tools(server.peer.list_all_tools().await?);
        *server.tools.write().await = tools.clone();
        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<Value>,
    ) -> McpResult<CallToolResult> {
        let server = self.get(server).await?;
        let mut params = rmcp::model::CallToolRequestParams::new(tool.to_string());
        if let Some(arguments) = value_to_json_object(arguments)? {
            params.arguments = Some(arguments);
        }
        let result = server.peer.call_tool(params).await?;
        Ok(convert_call_tool_result(result))
    }

    pub async fn list_resources(&self, server: &str) -> McpResult<ListResourcesResult> {
        let server = self.get(server).await?;
        let result = server.peer.list_resources(None).await?;
        Ok(ListResourcesResult {
            resources: result
                .resources
                .into_iter()
                .map(convert_resource_descriptor)
                .collect(),
            next_cursor: result.next_cursor,
        })
    }

    pub async fn read_resource(&self, server: &str, uri: &str) -> McpResult<ReadResourceResult> {
        let server = self.get(server).await?;
        let result = server
            .peer
            .read_resource(ReadResourceRequestParams::new(uri.to_string()))
            .await?;
        Ok(ReadResourceResult {
            contents: result
                .contents
                .into_iter()
                .map(convert_resource_contents)
                .collect(),
        })
    }

    pub async fn list_prompts(&self, server: &str) -> McpResult<ListPromptsResult> {
        let server = self.get(server).await?;
        let result = server.peer.list_prompts(None).await?;
        Ok(ListPromptsResult {
            prompts: result
                .prompts
                .into_iter()
                .map(convert_prompt_descriptor)
                .collect(),
            next_cursor: result.next_cursor,
        })
    }

    pub async fn get_prompt(
        &self,
        server: &str,
        name: &str,
        arguments: Option<BTreeMap<String, String>>,
    ) -> McpResult<GetPromptResult> {
        let server = self.get(server).await?;
        let mut params = GetPromptRequestParams::new(name.to_string());
        if let Some(arguments) = arguments {
            params.arguments = Some(
                arguments
                    .into_iter()
                    .map(|(key, value)| (key, Value::String(value)))
                    .collect(),
            );
        }
        let result = server.peer.get_prompt(params).await?;
        Ok(GetPromptResult {
            description: result.description,
            messages: result
                .messages
                .into_iter()
                .map(convert_prompt_message)
                .collect(),
        })
    }

    pub async fn shutdown_all(&self) {
        let servers = {
            let mut inner = self.inner.write().await;
            std::mem::take(&mut inner.servers)
                .into_values()
                .collect::<Vec<_>>()
        };
        for server in servers {
            shutdown_server(server).await;
        }
    }

    async fn get(&self, name: &str) -> McpResult<Arc<ConnectedServer>> {
        let inner = self.inner.read().await;
        inner
            .servers
            .get(name)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_string()))
    }

    fn client_info(&self) -> ClientInfo {
        ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new(self.client_name.clone(), self.client_version.clone()),
        )
    }
}

async fn connect_stdio(
    client_info: ClientInfo,
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: Option<PathBuf>,
) -> McpResult<RunningClient> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd.envs(env);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let transport = TokioChildProcess::new(cmd)?;
    Ok(client_info.serve(transport).await?)
}

async fn connect_http(
    client_info: ClientInfo,
    server_name: &str,
    url: Url,
    mut headers: HashMap<String, String>,
    auth: Option<HttpAuth>,
    token_store: Option<&dyn TokenStore>,
) -> McpResult<RunningClient> {
    let has_authorization = headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case("authorization"));
    if let Some(auth) = auth {
        apply_http_auth(server_name, auth, &mut headers, token_store);
    }

    let mut custom_headers = HashMap::new();
    let mut bearer = None;
    for (key, value) in headers {
        if key.eq_ignore_ascii_case("authorization")
            && !has_authorization
            && let Some(stripped) = value.strip_prefix("Bearer ")
        {
            bearer = Some(stripped.to_string());
            continue;
        }
        custom_headers.insert(parse_header_name(&key)?, parse_header_value(&value)?);
    }

    let config = StreamableHttpClientTransportConfig::with_uri(url.to_string())
        .custom_headers(custom_headers)
        .reinit_on_expired_session(true);
    let config = match bearer {
        Some(token) => config.auth_header(token),
        None => config,
    };
    let transport = StreamableHttpClientTransport::from_config(config);
    Ok(client_info.serve(transport).await?)
}

async fn shutdown_server(server: Arc<ConnectedServer>) {
    if let Some(running) = server.running.lock().await.take() {
        let _ = running.cancel().await;
    }
}

fn apply_http_auth(
    server: &str,
    auth: HttpAuth,
    headers: &mut HashMap<String, String>,
    token_store: Option<&dyn TokenStore>,
) {
    match auth {
        HttpAuth::Bearer(token) => {
            headers
                .entry("Authorization".to_string())
                .or_insert_with(|| format!("Bearer {token}"));
        }
        HttpAuth::BearerFromEnv(env_name) => match std::env::var(&env_name) {
            Ok(token) if !token.trim().is_empty() => {
                headers
                    .entry("Authorization".to_string())
                    .or_insert_with(|| format!("Bearer {token}"));
            }
            _ => warn!(
                target: "agena_mcp_client::manager",
                server,
                env = %env_name,
                "missing bearer token env for MCP server"
            ),
        },
        HttpAuth::BearerFromStore => {
            let Some(token_store) = token_store else {
                warn!(
                    target: "agena_mcp_client::manager",
                    server,
                    "no token store configured for MCP bearer lookup"
                );
                return;
            };
            match token_store.bearer(server) {
                Some(token) => {
                    headers
                        .entry("Authorization".to_string())
                        .or_insert_with(|| format!("Bearer {token}"));
                }
                None => warn!(
                    target: "agena_mcp_client::manager",
                    server,
                    "no bearer token found in token store for MCP server"
                ),
            }
        }
        HttpAuth::Custom(custom_headers) => {
            for (key, value) in custom_headers {
                headers.entry(key).or_insert(value);
            }
        }
    }
}

fn parse_header_name(name: &str) -> McpResult<HeaderName> {
    HeaderName::try_from(name)
        .map_err(|err| McpError::Http(format!("invalid HTTP header name '{name}': {err}")))
}

fn parse_header_value(value: &str) -> McpResult<HeaderValue> {
    HeaderValue::from_str(value)
        .map_err(|err| McpError::Http(format!("invalid HTTP header value: {err}")))
}

fn value_to_json_object(value: Option<Value>) -> McpResult<Option<rmcp::model::JsonObject>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) if map.is_empty() => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(other) => Err(McpError::Malformed(format!(
            "tool arguments must be a JSON object, got {other}"
        ))),
    }
}

fn convert_tools(tools: Vec<Tool>) -> Vec<ToolDescriptor> {
    tools.into_iter().map(convert_tool_descriptor).collect()
}

fn convert_tool_descriptor(tool: Tool) -> ToolDescriptor {
    ToolDescriptor {
        name: tool.name.to_string(),
        aliases: Vec::new(),
        description: tool.description.map(|value| value.into_owned()),
        before_help: None,
        after_help: None,
        input_schema: Some(Value::Object(tool.input_schema.as_ref().clone())),
    }
}

fn convert_call_tool_result(result: rmcp::model::CallToolResult) -> CallToolResult {
    CallToolResult {
        content: result
            .content
            .into_iter()
            .map(convert_content_block)
            .collect(),
        is_error: result.is_error.unwrap_or(false),
    }
}

fn convert_content_block(content: Content) -> ContentBlock {
    match content.raw {
        rmcp::model::RawContent::Text(text) => ContentBlock::Text { text: text.text },
        rmcp::model::RawContent::Image(image) => ContentBlock::Image {
            data: image.data,
            mime_type: image.mime_type,
        },
        rmcp::model::RawContent::Resource(resource) => ContentBlock::Resource {
            resource: convert_resource_contents(resource.resource),
        },
        rmcp::model::RawContent::ResourceLink(resource) => ContentBlock::Resource {
            resource: ResourceContents {
                uri: resource.uri,
                mime_type: resource.mime_type,
                text: None,
                blob: None,
            },
        },
        rmcp::model::RawContent::Audio(_) => ContentBlock::Other,
    }
}

fn convert_resource_descriptor(resource: rmcp::model::Resource) -> ResourceDescriptor {
    ResourceDescriptor {
        uri: resource.uri.to_string(),
        name: Some(resource.name.clone()),
        description: resource.description.clone(),
        mime_type: resource.mime_type.clone(),
    }
}

fn convert_resource_contents(resource: RmcpResourceContents) -> ResourceContents {
    match resource {
        RmcpResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            ..
        } => ResourceContents {
            uri,
            mime_type,
            text: Some(text),
            blob: None,
        },
        RmcpResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            ..
        } => ResourceContents {
            uri,
            mime_type,
            text: None,
            blob: Some(blob),
        },
    }
}

fn convert_prompt_descriptor(prompt: rmcp::model::Prompt) -> crate::protocol::PromptDescriptor {
    crate::protocol::PromptDescriptor {
        name: prompt.name,
        description: prompt.description,
        arguments: prompt
            .arguments
            .unwrap_or_default()
            .into_iter()
            .map(|argument| crate::protocol::PromptArgument {
                name: argument.name,
                description: argument.description,
                required: argument.required.unwrap_or(false),
            })
            .collect(),
    }
}

fn convert_prompt_message(message: rmcp::model::PromptMessage) -> crate::protocol::PromptMessage {
    crate::protocol::PromptMessage {
        role: match message.role {
            PromptMessageRole::User => "user".to_string(),
            PromptMessageRole::Assistant => "assistant".to_string(),
        },
        content: match message.content {
            PromptMessageContent::Text { text } => ContentBlock::Text { text },
            PromptMessageContent::Image { image } => ContentBlock::Image {
                data: image.data.clone(),
                mime_type: image.mime_type.clone(),
            },
            PromptMessageContent::Resource { resource } => ContentBlock::Resource {
                resource: convert_resource_contents(resource.resource.clone()),
            },
            PromptMessageContent::ResourceLink { link } => ContentBlock::Resource {
                resource: ResourceContents {
                    uri: link.uri.to_string(),
                    mime_type: link.mime_type.clone(),
                    text: None,
                    blob: None,
                },
            },
        },
    }
}

pub trait TokenStore: Send + Sync {
    fn bearer(&self, server: &str) -> Option<String>;
}
