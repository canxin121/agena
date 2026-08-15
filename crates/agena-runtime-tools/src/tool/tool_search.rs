use crate::part::ToolSearchToolInput;
use agena_plugin_host::registry::RegisteredTool;
use agena_plugin_host::sdk::ToolTag;
use agena_tool::tool_search::{ToolSearchDocument, search_tools};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

const DEFAULT_LIMIT: usize = 8;

#[derive(Debug, Clone)]
pub(crate) struct SearchableTool {
    pub name: String,
    pub description: String,
    pub tags: Vec<ToolTag>,
}

impl SearchableTool {
    pub(crate) fn from_registered_tool(registered_tool: RegisteredTool, tool_name: String) -> Self {
        let description = registered_tool
            .summary_text()
            .unwrap_or_default()
            .to_string();
        let tags = registered_tool.effective_tags();
        Self {
            name: tool_name,
            description,
            tags,
        }
    }
}

pub(crate) fn execute(
    executor: &ToolExecutor,
    input: &ToolSearchToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let tools = executor.available_execution_tools();
    let names = crate::tool::execution_tool_names(&tools);
    let available_tools = tools
        .into_iter()
        .zip(names)
        .map(|(tool, name)| SearchableTool::from_registered_tool(tool.into_registered(), name))
        .collect::<Vec<_>>();
    execute_with_tools(&available_tools, input)
}

pub(crate) fn execute_with_tools(
    available_tools: &[SearchableTool],
    input: &ToolSearchToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    if input.query.trim().is_empty() {
        return Err(ToolError::invalid_input(
            "tool_search requires a non-empty query".to_string(),
        ));
    }

    let limit = input.limit.unwrap_or(DEFAULT_LIMIT as u32).max(1) as usize;
    let documents = available_tools
        .iter()
        .map(|tool| {
            ToolSearchDocument::new(
                tool.name.clone(),
                tool.description.clone(),
                tool.tags.iter().map(ToString::to_string).collect(),
                None,
            )
        })
        .collect::<Vec<_>>();
    let results = search_tools(&documents, input.query.as_str(), limit);

    let mut lines = Vec::new();
    if !input.query.trim().is_empty() {
        lines.push(format!(
            "Found {} tool(s) matching '{}'.",
            results.len(),
            input.query.trim()
        ));
        for definition in &results {
            lines.push(format!(
                "- {} [{}]: {}",
                definition.name,
                tags_summary(definition),
                definition.description
            ));
        }
        if !results.is_empty() {
            lines.push(format!(
                "Use `{}` with an exact tool name for detailed usage.",
                crate::tool::tools_help_function_name()
            ));
        }
    }

    let output = ToolPayloadOutput::ToolSearch {
        results: results.iter().map(|tool| tool.name.clone()).collect(),
    };

    let output_text = lines.join("\n");
    let title = if input.query.trim().is_empty() {
        "Search tools".to_string()
    } else {
        format!("Search tools · {}", input.query.trim())
    };
    let mut view = ToolExecutionView::simple(
        title,
        format!("{} tools matched", results.len()),
        output_text,
    );
    view.metadata
        .insert("matched_tools".to_string(), results.len().to_string());
    if !input.query.trim().is_empty() {
        view.metadata
            .insert("query".to_string(), input.query.trim().to_string());
    }

    Ok(ToolPayloadExecution::new(output, view))
}

fn tags_summary(definition: &ToolSearchDocument) -> String {
    if definition.tags.is_empty() {
        return "untagged".to_string();
    }
    definition.tags.join(", ")
}
