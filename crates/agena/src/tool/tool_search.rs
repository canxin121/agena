use crate::message::ToolSearchToolInput;
use crate::plugin::registry::RegisteredTool;
use crate::plugin::sdk::ToolTag;
use crate::search::tool_catalog::{ToolCatalogDocument, search_tool_catalog};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

const DEFAULT_LIMIT: usize = 8;
const MAX_LIMIT: usize = 25;

#[derive(Debug, Clone)]
pub(crate) struct SearchableTool {
    pub name: String,
    pub description: String,
    pub tags: Vec<ToolTag>,
}

impl SearchableTool {
    pub(crate) fn from_registered_tool(registered_tool: RegisteredTool) -> Self {
        let description = registered_tool
            .summary_text()
            .unwrap_or_default()
            .to_string();
        let tags = registered_tool.effective_tags();
        Self {
            name: crate::tool::catalog_target_name(registered_tool.canonical_name().as_str()),
            description,
            tags,
        }
    }
}

pub(crate) fn execute(
    executor: &ToolExecutor,
    input: &ToolSearchToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let catalog = executor
        .searchable_tools()
        .into_iter()
        .map(SearchableTool::from_registered_tool)
        .collect::<Vec<_>>();
    execute_with_tools(&catalog, input)
}

pub(crate) fn execute_with_tools(
    catalog: &[SearchableTool],
    input: &ToolSearchToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    if input.query.trim().is_empty() {
        return Err(ToolError::InvalidInput(
            "tool_search requires a non-empty query".to_string(),
        ));
    }

    let limit = input
        .limit
        .unwrap_or(DEFAULT_LIMIT as u32)
        .clamp(1, MAX_LIMIT as u32) as usize;
    let documents = catalog
        .iter()
        .map(|tool| {
            ToolCatalogDocument::new(
                tool.name.clone(),
                tool.description.clone(),
                tool.tags.iter().map(ToString::to_string).collect(),
                None,
            )
        })
        .collect::<Vec<_>>();
    let results = search_tool_catalog(&documents, input.query.as_str(), limit)
        .map_err(|err| ToolError::Plugin(format!("tool_search failed: {err}")))?;

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
                crate::tool::gateway_help_tool_name()
            ));
        }
    }

    let output = ToolPayloadOutput::ToolSearch {
        results: results.iter().map(|tool| tool.name.clone()).collect(),
    };

    let output_text = lines.join("\n");
    let mut view = ToolExecutionView::simple("Tool search", output_text);
    view.metadata
        .insert("matched_tools".to_string(), results.len().to_string());
    if !input.query.trim().is_empty() {
        view.metadata
            .insert("query".to_string(), input.query.trim().to_string());
    }

    Ok(ToolPayloadExecution::new(output, view))
}

fn tags_summary(definition: &ToolCatalogDocument) -> String {
    if definition.tags.is_empty() {
        return "untagged".to_string();
    }
    definition.tags.join(", ")
}
