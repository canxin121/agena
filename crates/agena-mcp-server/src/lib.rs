//! # agena-mcp-server
//!
//! Model Context Protocol (MCP) server implementation for Agena.
//!
//! Exposes Agena capabilities (tools, resources, prompts) as an MCP server
//! over stdio or other [`rmcp`]-backed transports. A [`McpServerBackend`]
//! adapter provides the Agena-side implementation, so protocol mapping stays
//! decoupled from the runtime.
//!
//! ## Entry points
//!
//! - [`serve_stdio`] — serve a backend over stdio with the full MCP surface
//!   (tools, resources, prompts).
//! - [`serve_tools_stdio`] — serve only tools over stdio.
//! - [`text_result`] / [`text_error`] — build tool call results from plain
//!   text.
//!
//! ## Errors
//!
//! Protocol and backend failures are reported through [`McpServerError`].

use std::borrow::Cow;
use std::sync::Arc;

use agena_mcp_client::protocol::{
    CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult,
    PromptDescriptor, PromptMessage, ReadResourceParams, ReadResourceResult, ResourceContents,
    ResourceDescriptor, ToolDescriptor,
};
use async_trait::async_trait;
use rmcp::ServerHandler;
use rmcp::model::{
    ContentBlock as RmcpContentBlock, ErrorCode, ErrorData, GetPromptRequestParams,
    GetPromptResult as RmcpGetPromptResult, Implementation, ListPromptsResult, ListResourcesResult,
    ListToolsResult, Prompt as RmcpPrompt, PromptArgument as RmcpPromptArgument,
    PromptMessage as RmcpPromptMessage, ReadResourceRequestParams,
    ReadResourceResult as RmcpReadResourceResult, Resource as RmcpResource,
    ResourceContents as RmcpResourceContents, Role, ServerCapabilities,
    ServerInfo as RmcpServerInfo, Tool as RmcpTool,
};
use rmcp::service::{RequestContext, RoleServer};

#[derive(Debug, thiserror::Error)]
/// Error from the MCP server.
pub enum McpServerError {
    #[error("invalid params: {0}")]
    InvalidParams(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
/// Server information advertised by the MCP server.
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub instructions: Option<String>,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            name: "agena".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            instructions: Some(
                "Agena exposes its local tools, resources, and prompts over MCP.".to_owned(),
            ),
        }
    }
}

#[async_trait]
/// Backend implementing MCP tool execution.
pub trait McpServerBackend: Send + Sync + 'static {
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpServerError>;
    async fn call_tool(&self, params: CallToolParams) -> Result<CallToolResult, McpServerError>;

    async fn list_resources(&self) -> Result<Vec<ResourceDescriptor>, McpServerError> {
        Ok(Vec::new())
    }

    async fn read_resource(
        &self,
        params: ReadResourceParams,
    ) -> Result<ReadResourceResult, McpServerError> {
        Err(McpServerError::NotFound(params.uri))
    }

    async fn list_prompts(&self) -> Result<Vec<PromptDescriptor>, McpServerError> {
        Ok(Vec::new())
    }

    async fn get_prompt(&self, params: GetPromptParams) -> Result<GetPromptResult, McpServerError> {
        Err(McpServerError::NotFound(params.name))
    }
}

struct BackendHandler<B> {
    backend: Arc<B>,
    info: ServerInfo,
    resources_enabled: bool,
    prompts_enabled: bool,
}

impl<B> BackendHandler<B> {
    fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            info: ServerInfo::default(),
            resources_enabled: true,
            prompts_enabled: true,
        }
    }

    fn new_tools_only(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            info: ServerInfo {
                instructions: Some(
                    "Agena exposes its local runtime tools over MCP. Resources and prompts are intentionally unavailable on this endpoint."
                        .to_owned(),
                ),
                ..ServerInfo::default()
            },
            resources_enabled: false,
            prompts_enabled: false,
        }
    }
}

impl<B> ServerHandler for BackendHandler<B>
where
    B: McpServerBackend,
{
    fn get_info(&self) -> RmcpServerInfo {
        let capabilities = if self.resources_enabled && self.prompts_enabled {
            ServerCapabilities::builder()
                .enable_prompts()
                .enable_resources()
                .enable_tools()
                .build()
        } else {
            ServerCapabilities::builder().enable_tools().build()
        };
        let info = RmcpServerInfo::new(capabilities).with_server_info(Implementation::new(
            self.info.name.clone(),
            self.info.version.clone(),
        ));
        match &self.info.instructions {
            Some(instructions) => info.with_instructions(instructions.clone()),
            None => info,
        }
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let backend = Arc::clone(&self.backend);
        async move {
            let tools = backend.list_tools().await.map_err(to_rmcp_error)?;
            let tools = tools
                .into_iter()
                .map(convert_tool_descriptor)
                .collect::<Result<Vec<_>, _>>()
                .map_err(to_rmcp_error)?;
            Ok(ListToolsResult {
                meta: None,
                next_cursor: None,
                tools,
            })
        }
    }

    fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::CallToolResult, ErrorData>> + Send + '_
    {
        let backend = Arc::clone(&self.backend);
        async move {
            let params = CallToolParams {
                name: request.name.to_string(),
                arguments: request.arguments.map(serde_json::Value::Object),
            };
            let result = backend.call_tool(params).await.map_err(to_rmcp_error)?;
            convert_call_tool_result(result).map_err(to_rmcp_error)
        }
    }

    fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, ErrorData>> + Send + '_ {
        let backend = Arc::clone(&self.backend);
        let resources_enabled = self.resources_enabled;
        async move {
            if !resources_enabled {
                return Err(to_rmcp_error(McpServerError::NotFound(
                    "resources are not exposed by this MCP server".to_owned(),
                )));
            }
            let resources = backend.list_resources().await.map_err(to_rmcp_error)?;
            let resources = resources
                .into_iter()
                .map(convert_resource_descriptor)
                .collect();
            Ok(ListResourcesResult {
                meta: None,
                next_cursor: None,
                resources,
            })
        }
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<RmcpReadResourceResult, ErrorData>> + Send + '_
    {
        let backend = Arc::clone(&self.backend);
        let resources_enabled = self.resources_enabled;
        async move {
            if !resources_enabled {
                return Err(to_rmcp_error(McpServerError::NotFound(
                    "resources are not exposed by this MCP server".to_owned(),
                )));
            }
            let result = backend
                .read_resource(ReadResourceParams { uri: request.uri })
                .await
                .map_err(to_rmcp_error)?;
            Ok(RmcpReadResourceResult::new(
                result
                    .contents
                    .into_iter()
                    .map(convert_resource_contents)
                    .collect(),
            ))
        }
    }

    fn list_prompts(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListPromptsResult, ErrorData>> + Send + '_ {
        let backend = Arc::clone(&self.backend);
        let prompts_enabled = self.prompts_enabled;
        async move {
            if !prompts_enabled {
                return Err(to_rmcp_error(McpServerError::NotFound(
                    "prompts are not exposed by this MCP server".to_owned(),
                )));
            }
            let prompts = backend.list_prompts().await.map_err(to_rmcp_error)?;
            let prompts = prompts.into_iter().map(convert_prompt_descriptor).collect();
            Ok(ListPromptsResult {
                meta: None,
                next_cursor: None,
                prompts,
            })
        }
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<RmcpGetPromptResult, ErrorData>> + Send + '_ {
        let backend = Arc::clone(&self.backend);
        let prompts_enabled = self.prompts_enabled;
        async move {
            if !prompts_enabled {
                return Err(to_rmcp_error(McpServerError::NotFound(
                    "prompts are not exposed by this MCP server".to_owned(),
                )));
            }
            let params = GetPromptParams {
                name: request.name,
                arguments: request.arguments.map(json_object_to_string_map),
            };
            let result = backend.get_prompt(params).await.map_err(to_rmcp_error)?;
            convert_get_prompt_result(result).map_err(to_rmcp_error)
        }
    }
}

pub async fn serve_stdio<B>(backend: B) -> Result<(), McpServerError>
where
    B: McpServerBackend,
{
    let running = rmcp::serve_server(BackendHandler::new(backend), rmcp::transport::stdio())
        .await
        .map_err(|err| McpServerError::Backend(err.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|err| McpServerError::Backend(err.to_string()))?;
    Ok(())
}

/// Serve only the MCP Tool API over stdio.
///
/// This is the public Agena integration surface: it deliberately advertises
/// no MCP resources or prompts, so external clients receive only the exact
/// runtime tools returned by [`McpServerBackend::list_tools`].
pub async fn serve_tools_stdio<B>(backend: B) -> Result<(), McpServerError>
where
    B: McpServerBackend,
{
    let running = rmcp::serve_server(
        BackendHandler::new_tools_only(backend),
        rmcp::transport::stdio(),
    )
    .await
    .map_err(|err| McpServerError::Backend(err.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|err| McpServerError::Backend(err.to_string()))?;
    Ok(())
}

pub fn text_result(text: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text {
            text: text.into(),
            annotations: None,
            meta: None,
        }],
        is_error: false,
        structured_content: None,
        meta: None,
    }
}

pub fn text_error(text: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text {
            text: text.into(),
            annotations: None,
            meta: None,
        }],
        is_error: true,
        structured_content: None,
        meta: None,
    }
}

fn to_rmcp_error(error: McpServerError) -> ErrorData {
    match error {
        McpServerError::InvalidParams(message) => {
            ErrorData::new(ErrorCode::INVALID_PARAMS, message, None)
        }
        McpServerError::NotFound(message) => {
            ErrorData::new(ErrorCode::METHOD_NOT_FOUND, message, None)
        }
        other => ErrorData::new(ErrorCode::INTERNAL_ERROR, other.to_string(), None),
    }
}

fn convert_tool_descriptor(tool: ToolDescriptor) -> Result<RmcpTool, McpServerError> {
    let input_schema = json_object_from_optional_value(tool.input_schema)?;
    let mut output = RmcpTool::new_with_raw(
        Cow::Owned(tool.name),
        tool.description.map(Cow::Owned),
        Arc::new(input_schema),
    );
    output.title = tool.title;
    output.output_schema = tool
        .output_schema
        .map(json_object_from_value)
        .transpose()?
        .map(Arc::new);
    output.annotations = deserialize_optional(tool.annotations)?;
    output.execution = deserialize_optional(tool.execution)?;
    output.icons = deserialize_values(tool.icons)?;
    output.meta = deserialize_optional(tool.meta)?;
    Ok(output)
}

fn convert_call_tool_result(
    result: CallToolResult,
) -> Result<rmcp::model::CallToolResult, McpServerError> {
    let mut content = Vec::new();
    for block in result.content {
        if let Some(block) = convert_content_block(block)? {
            content.push(block);
        }
    }
    let mut output = if result.is_error {
        rmcp::model::CallToolResult::error(content)
    } else {
        rmcp::model::CallToolResult::success(content)
    };
    output.is_error = result.is_error.then_some(true);
    output.structured_content = result.structured_content;
    output.meta = deserialize_optional(result.meta)?;
    Ok(output)
}

fn convert_resource_descriptor(resource: ResourceDescriptor) -> rmcp::model::Resource {
    let mut output = RmcpResource::new(
        resource.uri.clone(),
        resource.name.unwrap_or_else(|| resource.uri.clone()),
    );
    output.description = resource.description;
    output.title = resource.title;
    output.mime_type = resource.mime_type;
    output.size = resource.size;
    output.icons = deserialize_values(resource.icons).unwrap_or_default();
    output.annotations = deserialize_optional(resource.annotations).unwrap_or_default();
    output.meta = deserialize_optional(resource.meta).unwrap_or_default();
    output
}

fn convert_resource_contents(resource: ResourceContents) -> RmcpResourceContents {
    match (resource.text, resource.blob) {
        (Some(text), _) => RmcpResourceContents::TextResourceContents {
            uri: resource.uri,
            mime_type: resource.mime_type,
            text,
            meta: deserialize_optional(resource.meta).unwrap_or_default(),
        },
        (None, blob) => RmcpResourceContents::BlobResourceContents {
            uri: resource.uri,
            mime_type: resource.mime_type,
            blob: blob.unwrap_or_default(),
            meta: deserialize_optional(resource.meta).unwrap_or_default(),
        },
    }
}

fn convert_prompt_descriptor(prompt: PromptDescriptor) -> RmcpPrompt {
    let arguments = if prompt.arguments.is_empty() {
        None
    } else {
        Some(
            prompt
                .arguments
                .into_iter()
                .map(|argument| {
                    let mut output = RmcpPromptArgument::new(argument.name);
                    output.description = argument.description;
                    output.required = argument.required.then_some(true);
                    output
                })
                .collect(),
        )
    };
    RmcpPrompt::from_raw(prompt.name, prompt.description, arguments)
}

fn convert_get_prompt_result(
    result: GetPromptResult,
) -> Result<RmcpGetPromptResult, McpServerError> {
    let mut messages = Vec::new();
    for message in result.messages {
        messages.push(convert_prompt_message(message)?);
    }
    let mut prompt = RmcpGetPromptResult::new(messages);
    prompt.description = result.description;
    Ok(prompt)
}

fn convert_prompt_message(message: PromptMessage) -> Result<RmcpPromptMessage, McpServerError> {
    let role = match message.role.as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        other => {
            return Err(McpServerError::InvalidParams(format!(
                "unsupported prompt role '{other}'"
            )));
        }
    };
    let content = convert_content_block(message.content)?.ok_or_else(|| {
        McpServerError::InvalidParams("unsupported prompt content block".to_string())
    })?;
    Ok(RmcpPromptMessage::new(role, content))
}

fn convert_content_block(block: ContentBlock) -> Result<Option<RmcpContentBlock>, McpServerError> {
    let output = match block {
        ContentBlock::Text {
            text,
            annotations,
            meta,
        } => {
            let mut output = rmcp::model::TextContent::new(text);
            if let Some(annotations) = deserialize_optional(annotations)? {
                output = output.with_annotations(annotations);
            }
            if let Some(meta) = deserialize_optional(meta)? {
                output = output.with_meta(meta);
            }
            RmcpContentBlock::Text(output)
        }
        ContentBlock::Image {
            data,
            mime_type,
            annotations,
            meta,
        } => {
            let mut output = rmcp::model::ImageContent::new(data, mime_type);
            if let Some(annotations) = deserialize_optional(annotations)? {
                output = output.with_annotations(annotations);
            }
            if let Some(meta) = deserialize_optional(meta)? {
                output = output.with_meta(meta);
            }
            RmcpContentBlock::Image(output)
        }
        ContentBlock::Audio {
            data,
            mime_type,
            annotations,
            meta,
        } => {
            let mut output = rmcp::model::AudioContent::new(data, mime_type);
            if let Some(annotations) = deserialize_optional(annotations)? {
                output = output.with_annotations(annotations);
            }
            if let Some(meta) = deserialize_optional(meta)? {
                output = output.with_meta(meta);
            }
            RmcpContentBlock::Audio(output)
        }
        ContentBlock::Resource {
            resource,
            annotations,
            meta,
        } => {
            let mut output =
                rmcp::model::EmbeddedResource::new(convert_resource_contents(resource));
            if let Some(annotations) = deserialize_optional(annotations)? {
                output = output.with_annotations(annotations);
            }
            if let Some(meta) = deserialize_optional(meta)? {
                output = output.with_meta(meta);
            }
            RmcpContentBlock::Resource(output)
        }
        ContentBlock::ResourceLink { resource } => {
            RmcpContentBlock::ResourceLink(convert_resource_descriptor(resource))
        }
        ContentBlock::Unknown { raw } => match serde_json::from_value(raw) {
            Ok(block) => block,
            Err(_) => return Ok(None),
        },
    };
    Ok(Some(output))
}

fn json_object_from_value(
    value: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, McpServerError> {
    match value {
        serde_json::Value::Object(map) => Ok(map),
        other => Err(McpServerError::InvalidParams(format!(
            "expected a JSON object, got {other}"
        ))),
    }
}

fn deserialize_optional<T: serde::de::DeserializeOwned>(
    value: Option<serde_json::Value>,
) -> Result<Option<T>, McpServerError> {
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(McpServerError::Json)
}

fn deserialize_values<T: serde::de::DeserializeOwned>(
    values: Vec<serde_json::Value>,
) -> Result<Option<Vec<T>>, McpServerError> {
    if values.is_empty() {
        return Ok(None);
    }
    values
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
        .map_err(McpServerError::Json)
}

fn json_object_from_optional_value(
    value: Option<serde_json::Value>,
) -> Result<serde_json::Map<String, serde_json::Value>, McpServerError> {
    match value {
        None => Ok(serde_json::Map::new()),
        Some(serde_json::Value::Object(map)) => Ok(map),
        Some(other) => Err(McpServerError::InvalidParams(format!(
            "tool input_schema must be a JSON object, got {other}"
        ))),
    }
}

fn json_object_to_string_map(
    value: serde_json::Map<String, serde_json::Value>,
) -> std::collections::BTreeMap<String, String> {
    value
        .into_iter()
        .map(|(key, value)| {
            let value = match value {
                serde_json::Value::String(text) => text,
                other => other.to_string(),
            };
            (key, value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{BackendHandler, CallToolParams, CallToolResult, McpServerBackend, McpServerError};
    use async_trait::async_trait;
    use rmcp::ServerHandler;

    #[derive(Debug)]
    struct ToolOnlyFixture;

    #[async_trait]
    impl McpServerBackend for ToolOnlyFixture {
        async fn list_tools(
            &self,
        ) -> Result<Vec<agena_mcp_client::protocol::ToolDescriptor>, McpServerError> {
            Ok(Vec::new())
        }

        async fn call_tool(
            &self,
            _params: CallToolParams,
        ) -> Result<CallToolResult, McpServerError> {
            Ok(super::text_result("ok"))
        }
    }

    #[test]
    fn tool_only_handler_advertises_no_resources_or_prompts() {
        let handler = BackendHandler::new_tools_only(ToolOnlyFixture);
        let info = ServerHandler::get_info(&handler);
        let value = serde_json::to_value(info).expect("serialize server info");
        let capabilities = value
            .get("capabilities")
            .and_then(serde_json::Value::as_object)
            .expect("server capabilities");
        assert!(capabilities.contains_key("tools"));
        assert!(!capabilities.contains_key("resources"));
        assert!(!capabilities.contains_key("prompts"));
    }
}
