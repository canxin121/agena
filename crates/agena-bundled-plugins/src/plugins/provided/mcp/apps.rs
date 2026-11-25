use super::*;
use agena_mcp_client::apps::{
    APP_MESSAGE_LIMIT, APP_MIME_TYPE, McpAppBinding, app_html, app_resource_uri, has_app_mime_type,
    is_app_uri, tool_visible_to,
};
use agena_mcp_client::protocol::ToolDescriptor;
use agena_plugin_host::sdk::host_api::current_host_callback_context;

impl McpPlugin {
    pub(super) async fn app_binding(
        &self,
        context: &ToolInvokeContext<'_>,
        server: &str,
    ) -> SdkResult<Option<McpAppBinding>> {
        let Some(app) = current_host_callback_context().and_then(|context| context.mcp_app) else {
            return Ok(None);
        };
        let binding: McpAppBinding =
            serde_json::from_str(&app.reference).map_err(|_| unavailable_app())?;
        if binding.server != server {
            return Err(unavailable_app());
        }
        let spec = self
            .manager
            .server_spec(server)
            .await
            .map_err(|_| unavailable_app())?;
        if !binding
            .verifies(context.session_id, app.source_call_id, &spec)
            .await
        {
            return Err(unavailable_app());
        }
        Ok(Some(binding))
    }

    pub(super) async fn model_tool_descriptor(
        &self,
        manager: &McpConnectionManager,
        server: &str,
        name: &str,
    ) -> SdkResult<ToolDescriptor> {
        if let Some(tool) = manager
            .tool_descriptor(server, name)
            .await
            .map_err(|error| PluginError::internal_error(&error))?
        {
            return Ok(tool);
        }
        manager
            .refresh_tools(server)
            .await
            .map_err(|error| PluginError::internal_error(&error))?
            .into_iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| PluginError::invalid_params("The MCP tool is unavailable."))
    }

    async fn verified_app_connection(
        &self,
        binding: &McpAppBinding,
    ) -> SdkResult<(Arc<McpConnectionManager>, Vec<ToolDescriptor>)> {
        let manager = self.authorize_server(&binding.server).await?;
        let tools = manager
            .refresh_tools(&binding.server)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        if !tools.iter().any(|tool| {
            tool.name == binding.tool
                && tool_visible_to(tool, "model")
                && app_resource_uri(tool) == Some(binding.uri.as_str())
        }) {
            return Err(unavailable_app());
        }
        Ok((manager, tools))
    }

    pub(super) async fn read_app_resource(
        &self,
        input: &ReadResourceInput,
        binding: &McpAppBinding,
    ) -> SdkResult<ToolInvokeOutput> {
        if is_app_uri(&input.uri) && input.uri != binding.uri {
            return Err(unavailable_app());
        }
        let (manager, _) = self.verified_app_connection(binding).await?;
        let result = manager
            .read_resource(&input.server, &input.uri)
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        if input.uri == binding.uri {
            let html = app_html(&binding.uri, result).ok_or_else(unavailable_app)?;
            return bounded_app_output(
                serde_json::json!({"contents":[{
                    "uri": binding.uri, "mimeType": APP_MIME_TYPE, "text": html,
                }]}),
                8 * APP_MESSAGE_LIMIT,
            );
        }
        let mut result = result;
        result
            .contents
            .retain(|item| !is_app_content(&item.uri, item.mime_type.as_deref()));
        bounded_app_output(result, APP_MESSAGE_LIMIT)
    }

    pub(super) async fn call_app_tool(
        &self,
        input: &CallToolInput,
        binding: &McpAppBinding,
    ) -> SdkResult<ToolInvokeOutput> {
        let (manager, tools) = self.verified_app_connection(binding).await?;
        if !tools
            .iter()
            .any(|tool| tool.name == input.name && tool_visible_to(tool, "app"))
        {
            return Err(PluginError::invalid_params(
                "The MCP tool is unavailable to this app.",
            ));
        }
        let result = manager
            .call_tool(
                &input.server,
                &input.name,
                empty_object_to_none(input.arguments.clone()),
            )
            .await
            .map_err(|error| PluginError::internal_error(&error))?;
        bounded_app_output(result, APP_MESSAGE_LIMIT)
    }
}

fn unavailable_app() -> PluginError {
    PluginError::invalid_params("The original MCP App connection or resource is unavailable.")
}

fn bounded_app_output(value: impl Serialize, limit: usize) -> SdkResult<ToolInvokeOutput> {
    let payload =
        serde_json::to_value(value).map_err(|error| PluginError::internal_error(&error))?;
    if payload.to_string().len() > limit {
        return Err(PluginError::invalid_params(
            "The MCP App response exceeds its size limit.",
        ));
    }
    Ok(ToolInvokeOutput::from_parts(
        "MCP App",
        "MCP App result",
        "",
        Some(payload),
        Default::default(),
        Vec::new(),
    ))
}

pub(super) fn is_app_content(uri: &str, mime: Option<&str>) -> bool {
    is_app_uri(uri) || mime.is_some_and(has_app_mime_type)
}

pub(super) fn is_app_block(block: &ContentBlock) -> bool {
    match block {
        ContentBlock::Resource { resource, .. } => {
            is_app_content(&resource.uri, resource.mime_type.as_deref())
        }
        ContentBlock::ResourceLink { resource } => {
            is_app_content(&resource.uri, resource.mime_type.as_deref())
        }
        _ => false,
    }
}
