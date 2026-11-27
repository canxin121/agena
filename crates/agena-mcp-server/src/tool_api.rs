use agena_domain::{StructuredObject, ToolApiCall, ToolApiFunction};
pub use agena_tool_api::ToolApiAction;
use agena_tool_api::{ToolApiCatalog, ToolApiRegistry};
use serde_json::{Value, json};

use super::{CallToolParams, McpServerError, ToolDescriptor};

pub fn tool_api_descriptors() -> Vec<ToolDescriptor> {
    ToolApiRegistry
        .definitions()
        .into_iter()
        .zip(ToolApiFunction::ALL)
        .map(|(definition, function)| {
            let description = match function {
                ToolApiFunction::PluginsList => "List Agena plugins and their available tools by owner. Pass a plugin ID or a list of IDs to filter. Use offset and limit to page through the results.",
                ToolApiFunction::List => "List Agena execution tools and their exact names. Pass a plugin ID or a list of IDs to filter. Use offset and limit to page through the results.",
                ToolApiFunction::Help => "Read the live schema and usage of one or more Agena tools. Pass the exact tool name or an array of names in tool.",
                ToolApiFunction::Call => "Execute one Agena tool. Pass its exact name in tool and its arguments in input as an object or a JSON string containing an object. Use tools_help to get the tool's contract.",
            };
            let read_only = function != ToolApiFunction::Call;
            ToolDescriptor {
                name: definition.name,
                title: None,
                aliases: Vec::new(),
                description: Some(description.into()),
                before_help: None,
                after_help: None,
                input_schema: Some(definition.input_schema),
                output_schema: None,
                annotations: Some(json!({
                    "readOnlyHint": read_only,
                    "destructiveHint": !read_only,
                    "idempotentHint": read_only,
                    "openWorldHint": !read_only,
                })),
                execution: None,
                icons: Vec::new(),
                meta: None,
            }
        })
        .collect()
}

pub fn dispatch_tool_api(
    catalog: &ToolApiCatalog,
    params: CallToolParams,
) -> Result<ToolApiAction, McpServerError> {
    let function = ToolApiFunction::from_function_name(&params.name).ok_or_else(|| {
        McpServerError::NotFound(format!("Unknown Agena MCP function '{}'.", params.name))
    })?;
    let arguments = StructuredObject::try_from(
        params
            .arguments
            .unwrap_or_else(|| Value::Object(Default::default())),
    )
    .map_err(McpServerError::InvalidParams)?;
    catalog
        .dispatch(&ToolApiCall {
            function,
            arguments,
        })
        .map_err(|error| McpServerError::InvalidParams(error.to_string()))
}
