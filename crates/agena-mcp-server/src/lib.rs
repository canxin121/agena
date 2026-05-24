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
    AnnotateAble, Content, ErrorCode, ErrorData, GetPromptRequestParams,
    GetPromptResult as RmcpGetPromptResult, Implementation, ListPromptsResult, ListResourcesResult,
    ListToolsResult, Prompt as RmcpPrompt, PromptArgument as RmcpPromptArgument,
    PromptMessage as RmcpPromptMessage, PromptMessageContent, PromptMessageRole, RawResource,
    ReadResourceRequestParams, ReadResourceResult as RmcpReadResourceResult,
    ResourceContents as RmcpResourceContents, ServerCapabilities, ServerInfo as RmcpServerInfo,
    Tool as RmcpTool,
};
use rmcp::service::{RequestContext, RoleServer};

#[derive(Debug, thiserror::Error)]
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
}

impl<B> BackendHandler<B> {
    fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            info: ServerInfo::default(),
        }
    }
}

impl<B> ServerHandler for BackendHandler<B>
where
    B: McpServerBackend,
{
    fn get_info(&self) -> RmcpServerInfo {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .enable_prompts()
            .build();
        let info = RmcpServerInfo::new(capabilities).with_server_info(Implementation::new(
            self.info.name.clone(),
            self.info.version.clone(),
        ));
        if let Some(instructions) = self.info.instructions.clone() {
            info.with_instructions(instructions)
        } else {
            info
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
            let mut result = ListToolsResult::default();
            result.tools = tools
                .into_iter()
                .map(convert_tool_descriptor)
                .collect::<Result<Vec<_>, _>>()
                .map_err(to_rmcp_error)?;
            Ok(result)
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
        async move {
            let resources = backend.list_resources().await.map_err(to_rmcp_error)?;
            let mut result = ListResourcesResult::default();
            result.resources = resources
                .into_iter()
                .map(convert_resource_descriptor)
                .collect();
            Ok(result)
        }
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<RmcpReadResourceResult, ErrorData>> + Send + '_
    {
        let backend = Arc::clone(&self.backend);
        async move {
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
        async move {
            let prompts = backend.list_prompts().await.map_err(to_rmcp_error)?;
            let mut result = ListPromptsResult::default();
            result.prompts = prompts.into_iter().map(convert_prompt_descriptor).collect();
            Ok(result)
        }
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<RmcpGetPromptResult, ErrorData>> + Send + '_ {
        let backend = Arc::clone(&self.backend);
        async move {
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

pub fn text_result(text: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text { text: text.into() }],
        is_error: Some(false),
    }
}

pub fn text_error(text: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text { text: text.into() }],
        is_error: Some(true),
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
    Ok(RmcpTool::new_with_raw(
        Cow::Owned(tool.name),
        tool.description.map(Cow::Owned),
        Arc::new(input_schema),
    ))
}

fn convert_call_tool_result(
    result: CallToolResult,
) -> Result<rmcp::model::CallToolResult, McpServerError> {
    let mut content = Vec::new();
    for block in result.content {
        match block {
            ContentBlock::Text { text } => content.push(Content::text(text)),
            ContentBlock::Image { data, mime_type } => {
                content.push(Content::image(data, mime_type));
            }
            ContentBlock::Resource { resource } => {
                content.push(Content::resource(convert_resource_contents(resource)));
            }
            ContentBlock::Other => {}
        }
    }
    let mut output = if matches!(result.is_error, Some(true)) {
        rmcp::model::CallToolResult::error(content)
    } else {
        rmcp::model::CallToolResult::success(content)
    };
    output.is_error = result.is_error;
    Ok(output)
}

fn convert_resource_descriptor(resource: ResourceDescriptor) -> rmcp::model::Resource {
    let mut raw = RawResource::new(
        resource.uri.clone(),
        resource.name.unwrap_or_else(|| resource.uri.clone()),
    );
    if let Some(description) = resource.description {
        raw = raw.with_description(description);
    }
    if let Some(mime_type) = resource.mime_type {
        raw = raw.with_mime_type(mime_type);
    }
    raw.no_annotation()
}

fn convert_resource_contents(resource: ResourceContents) -> RmcpResourceContents {
    match (resource.text, resource.blob) {
        (Some(text), _) => RmcpResourceContents::TextResourceContents {
            uri: resource.uri,
            mime_type: resource.mime_type,
            text,
            meta: None,
        },
        (None, blob) => RmcpResourceContents::BlobResourceContents {
            uri: resource.uri,
            mime_type: resource.mime_type,
            blob: blob.unwrap_or_default(),
            meta: None,
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
                    if let Some(description) = argument.description {
                        output = output.with_description(description);
                    }
                    if let Some(required) = argument.required {
                        output = output.with_required(required);
                    }
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
        "user" => PromptMessageRole::User,
        "assistant" => PromptMessageRole::Assistant,
        other => {
            return Err(McpServerError::InvalidParams(format!(
                "unsupported prompt role '{other}'"
            )));
        }
    };
    let content = match message.content {
        ContentBlock::Text { text } => PromptMessageContent::Text { text },
        ContentBlock::Image { data, mime_type } => PromptMessageContent::Image {
            image: rmcp::model::RawImageContent {
                data,
                mime_type,
                meta: None,
            }
            .no_annotation(),
        },
        ContentBlock::Resource { resource } => PromptMessageContent::Resource {
            resource: rmcp::model::RawEmbeddedResource::new(convert_resource_contents(resource))
                .no_annotation(),
        },
        ContentBlock::Other => {
            return Err(McpServerError::InvalidParams(
                "unsupported prompt content block".to_string(),
            ));
        }
    };
    Ok(RmcpPromptMessage::new(role, content))
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
