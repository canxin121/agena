//! Model protocol dispatch. Discovery reads a catalog projection and bypasses
//! execution dispatch, invocation hooks, effect checks, and plugin rendering.

use super::*;
use crate::session::kernel::KernelToolApiCall;
use agena_tool_api::{
    CatalogPlugin, CatalogTool, ToolApiAction, ToolApiCatalog, ToolApiError, ToolApiResponse,
};
use serde_json::json;

pub(super) async fn catalog(executor: &ToolExecutor) -> ToolApiCatalog {
    let tools = executor.available_execution_tools_async().await;
    let names = agena_runtime_tools::tool::execution_tool_names(&tools);
    let mut entries = tools
        .iter()
        .zip(names)
        .map(|(tool, name)| {
            let input_schema = tool.input_schema();
            CatalogTool {
                name,
                canonical_name: tool.canonical_name(),
                plugin_id: tool.plugin_full_name(),
                summary: tool.summary_text().unwrap_or_default().to_owned(),
                help: [
                    tool.before_help_text(),
                    tool.help_text(),
                    tool.after_help_text(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n\n"),
                tags: tool
                    .effective_tags()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                input_usage: agena_runtime_tools::tool::definition::schema_usage_text(
                    &input_schema,
                ),
                input_examples: agena_runtime_tools::tool::definition::schema_example_texts(
                    &input_schema,
                ),
                input_schema,
                output_schema: tool.output_schema(),
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let mut plugins = executor
        .plugin_manager()
        .plugins()
        .iter()
        .map(|plugin| {
            let id = plugin.key().to_string();
            CatalogPlugin {
                tool_count: entries.iter().filter(|tool| tool.plugin_id == id).count(),
                id,
                version: plugin.manifest.version.clone(),
                summary: plugin.manifest.summary.clone().unwrap_or_default(),
                tags: plugin
                    .manifest
                    .tags
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    plugins.sort_by(|left, right| left.id.cmp(&right.id));
    ToolApiCatalog {
        tools: entries,
        plugins,
    }
}

impl SessionManager {
    pub(super) async fn dispatch_tool_api(
        &self,
        protocol: KernelToolApiCall,
        executor: ToolExecutor,
        control: Arc<RunControl>,
        state: Arc<ExecutionState>,
        model: ModelRef,
    ) -> Result<(), AppError> {
        if control.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let action = match protocol.error {
            Some(error) => Err(error),
            None => {
                let catalog = if protocol.call.function == agena_domain::ToolApiFunction::Call {
                    ToolApiCatalog::default()
                } else {
                    catalog(&executor).await
                };
                catalog.dispatch(&protocol.call)
            }
        };
        let response = match action {
            Ok(ToolApiAction::Execute(request)) => {
                // This is the only crossing into plugin execution. From here
                // on, the executor sees solely the target and its own input.
                return self
                    .execute_tool(
                        protocol.tool,
                        agena_domain::ToolInvocation::new(request.tool, request.input),
                        executor,
                        control,
                        state,
                        model,
                    )
                    .await;
            }
            Ok(ToolApiAction::Respond(response)) => Ok(response),
            Err(error) => Err(error),
        };
        complete_protocol_response(protocol.tool, response).await
    }
}

async fn complete_protocol_response(
    tool: ToolHandle,
    response: Result<ToolApiResponse, ToolApiError>,
) -> Result<(), AppError> {
    let (phase, value) = match response {
        Ok(mut response) => {
            if response.output_text.len() > 64 * 1024 {
                let command = request_key("tool-api-response", &tool.id().get().to_string());
                let source = persist(|| tool.begin_text(command))
                    .await
                    .map_err(kernel_error)?
                    .value;
                super::results::append_text_chunks(&source, &response.output_text)
                    .await
                    .map_err(kernel_error)?;
                response.output_text.clear();
            }
            (
                ToolPhase::Succeeded,
                json!({"title": response.title, "summary": response.summary, "tool_api_response": response}),
            )
        }
        Err(error) => {
            let failure = error.failure();
            (
                ToolPhase::Failed,
                json!({"failure": failure, "error": agena_failure::UserProblem::from(&failure)}),
            )
        }
    };
    let result = ToolResult { phase, value };
    let command = RequestId::new();
    persist(|| tool.complete(command, result.clone()))
        .await
        .map_err(kernel_error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
