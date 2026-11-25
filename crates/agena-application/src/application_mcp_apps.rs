use agena_api::mcp_apps::{McpAppDocument, McpAppRequest};
use agena_domain::{ContentFormat, RawOutput, StructuredObject, ToolInvocation};
use agena_mcp_client::apps::{
    APP_MESSAGE_LIMIT, APP_METADATA_KEY, APP_RESULT_METADATA_KEY, McpAppBinding, app_html,
};
use agena_plugin_host::sdk::host_api::{
    McpAppHostBinding, McpAppHostRequest, scope_mcp_app_request,
};
use agena_storage::kernel::{EntryPayload, ToolPhase};
use serde_json::{Value, json};

use crate::{Application, ApplicationError};

const MCP_PLUGIN: &str = "agena.mcp";
const TOOL_CALL: &str = "tools.call";
const RESOURCE_READ: &str = "resources.read";

struct BoundApp {
    call_id: i64,
    binding: McpAppBinding,
    reference: String,
    result: Value,
    invocation: ToolInvocation,
}

impl Application {
    pub async fn mcp_app_document(
        &self,
        session_id: i64,
        part_id: i64,
    ) -> Result<McpAppDocument, ApplicationError> {
        let app = self.bound_mcp_app(session_id, part_id).await?;
        let raw = self.mcp_app_result(session_id, &app.result).await?;
        let result = self
            .execute_mcp_app_request(
                session_id,
                &app,
                RESOURCE_READ,
                json!({
                    "server": app.binding.server, "uri": app.binding.uri,
                }),
            )
            .await?;
        let resource = serde_json::from_value(result).map_err(|_| unavailable_app())?;
        let html = app_html(&app.binding.uri, resource).ok_or_else(unavailable_app)?;
        let input: Value = app.invocation.input.into();
        let arguments = input
            .get("arguments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if Value::Object(arguments.clone()).to_string().len() > APP_MESSAGE_LIMIT {
            return Err(ApplicationError::bad_request(
                "The MCP App's original input exceeds its size limit.",
            ));
        }
        Ok(McpAppDocument {
            revision: app.binding.revision().to_owned(),
            name: app.binding.tool,
            html,
            arguments,
            tool_result: raw,
        })
    }

    pub async fn mcp_app_request(
        &self,
        session_id: i64,
        part_id: i64,
        revision: &str,
        request: McpAppRequest,
    ) -> Result<Value, ApplicationError> {
        let app = self.bound_mcp_app(session_id, part_id).await?;
        if revision != app.binding.revision() {
            return Err(ApplicationError::conflict(
                "The MCP App changed. Reopen it to continue.",
            ));
        }
        let server = &app.binding.server;
        let (tool_name, input) = match request {
            McpAppRequest::CallTool { name, arguments } if valid_guest_field(&name, 256) => (
                TOOL_CALL,
                json!({"server": server, "name": name, "arguments": arguments}),
            ),
            McpAppRequest::ReadResource { uri } if valid_guest_field(&uri, 2048) => {
                (RESOURCE_READ, json!({"server": server, "uri": uri}))
            }
            _ => {
                return Err(ApplicationError::bad_request(
                    "The MCP App request is invalid.",
                ));
            }
        };
        self.execute_mcp_app_request(session_id, &app, tool_name, input)
            .await
    }

    async fn bound_mcp_app(
        &self,
        session_id: i64,
        part_id: i64,
    ) -> Result<BoundApp, ApplicationError> {
        let session = crate::kernel_session_id(session_id)?;
        if part_id <= 0 {
            return Err(unavailable_app());
        }
        let reader = &self.session_kernel()?.reader;
        // Forked histories can still display the original tool result. Only its
        // owning session can launch the app or use a live connection.
        let history = reader
            .history_by_ids(session, &[part_id])
            .await
            .map_err(crate::kernel_error)?;
        let entry = history
            .entries
            .iter()
            .find(|entry| entry.id.get() == part_id && entry.origin_session_id == session)
            .ok_or_else(unavailable_app)?;
        let EntryPayload::Tool { tool_id } = entry.payload else {
            return Err(unavailable_app());
        };
        let view = reader
            .tool(session, tool_id)
            .await
            .map_err(crate::kernel_error)?;
        if view.entry_id != entry.id || view.phase != ToolPhase::Succeeded {
            return Err(unavailable_app());
        }
        let result = view.result.as_ref().ok_or_else(unavailable_app)?;
        let reference = result
            .get("metadata")
            .and_then(|meta| meta.get(APP_METADATA_KEY))
            .and_then(Value::as_str)
            .filter(|reference| reference.len() <= 8192)
            .ok_or_else(unavailable_app)?
            .to_owned();
        let binding =
            serde_json::from_str::<McpAppBinding>(&reference).map_err(|_| unavailable_app())?;
        let invocation = result
            .get("invocation")
            .cloned()
            .and_then(|value| serde_json::from_value::<ToolInvocation>(value).ok())
            .ok_or_else(unavailable_app)?;
        let input = &invocation.input;
        if invocation.plugin_name.as_deref() != Some(MCP_PLUGIN)
            || agena_tool::normalized_tool_identity(&invocation.name) != "mcp.tools.call"
            || input
                .get("server")
                .and_then(agena_domain::StructuredValue::as_text)
                .map(str::trim)
                != Some(binding.server.as_str())
            || input
                .get("name")
                .and_then(agena_domain::StructuredValue::as_text)
                .map(str::trim)
                != Some(binding.tool.as_str())
        {
            return Err(unavailable_app());
        }
        Ok(BoundApp {
            call_id: tool_id.get(),
            binding,
            reference,
            result: result.clone(),
            invocation,
        })
    }

    async fn execute_mcp_app_request(
        &self,
        session_id: i64,
        app: &BoundApp,
        tool_name: &'static str,
        input: Value,
    ) -> Result<Value, ApplicationError> {
        let invocation = ToolInvocation::plugin_named(
            format!("{MCP_PLUGIN}.{tool_name}"),
            MCP_PLUGIN,
            StructuredObject::try_from(input.clone()).expect("MCP App tool input is an object"),
        );
        let request = McpAppHostRequest {
            session_id,
            tool_name,
            input,
            binding: McpAppHostBinding {
                source_call_id: app.call_id,
                reference: app.reference.clone(),
            },
        };
        let executor = self.session_execution_services()?.tool_execution;
        let result = scope_mcp_app_request(request, executor.execute_authorized_session_tool(session_id, invocation))
            .await.map_err(|error| ApplicationError::conflict_with_diagnostic(
                "The MCP App request was unavailable under the current connection or session permissions.", error,
            ))?;
        result.payload.ok_or_else(unavailable_app)
    }

    async fn mcp_app_result(
        &self,
        session_id: i64,
        result: &Value,
    ) -> Result<Value, ApplicationError> {
        let raw = serde_json::from_value::<RawOutput>(
            result
                .get("raw_output")
                .cloned()
                .ok_or_else(unavailable_app)?,
        )
        .map_err(|_| unavailable_app())?;
        let payload = if let Some(payload) = raw.payload {
            payload
        } else {
            let field = raw
                .fields
                .iter()
                .find(|field| field.pointer.is_empty() && field.format == ContentFormat::Json)
                .ok_or_else(unavailable_app)?;
            let page = self
                .read_content_text(agena_api::content::ReadContentTextParams {
                    session_id,
                    resource_id: field.resource.resource_id,
                    position: None,
                    max_bytes: APP_MESSAGE_LIMIT,
                })
                .await?;
            if page.has_more || page.gap {
                return Err(ApplicationError::bad_request(
                    "The MCP App's original result exceeds its size limit.",
                ));
            }
            serde_json::from_str(
                &page
                    .slices
                    .into_iter()
                    .map(|slice| slice.text)
                    .collect::<String>(),
            )
            .map_err(|_| unavailable_app())?
        };
        let mut protocol_result =
            json!({"content": payload.get("content").cloned().unwrap_or_else(|| json!([]))});
        if let Some(structured) = payload
            .get("structured_content")
            .filter(|value| !value.is_null())
        {
            protocol_result["structuredContent"] = structured.clone();
        }
        if let Some(meta) = result
            .get("metadata")
            .and_then(|meta| meta.get(APP_RESULT_METADATA_KEY))
            .and_then(Value::as_str)
            .and_then(|meta| serde_json::from_str::<Value>(meta).ok())
        {
            protocol_result["_meta"] = meta;
        }
        if protocol_result.to_string().len() > APP_MESSAGE_LIMIT {
            return Err(ApplicationError::bad_request(
                "The MCP App's original result exceeds its size limit.",
            ));
        }
        Ok(protocol_result)
    }
}

fn unavailable_app() -> ApplicationError {
    ApplicationError::not_found("The MCP App is unavailable in this conversation.")
}

fn valid_guest_field(field: &str, limit: usize) -> bool {
    !field.is_empty()
        && field.len() <= limit
        && field.trim() == field
        && !field.chars().any(char::is_control)
}
