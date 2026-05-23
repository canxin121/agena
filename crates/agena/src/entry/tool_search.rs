use crate::message::ToolSearchToolInput;
use crate::plugin::registry::PluginEntry as RegistryPluginEntry;
use crate::plugin::sdk::ToolTag;

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
    pub(crate) fn from_entry(entry: RegistryPluginEntry) -> Self {
        let description = entry
            .summary_text()
            .unwrap_or_else(|| entry.description_text())
            .to_string();
        let tags = entry.effective_tags();
        Self {
            name: entry.exposed_name,
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
        .map(SearchableTool::from_entry)
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

    let results = search_catalog(catalog, input.query.as_str(), input.limit);

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
            lines.push(
                "Call `tools` with command `help` and an exact tool name for detailed usage."
                    .to_string(),
            );
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

fn search_catalog(
    catalog: &[SearchableTool],
    query: &str,
    limit: Option<u32>,
) -> Vec<SearchableTool> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Vec::new();
    }

    let normalized_query = normalize(trimmed_query);
    let tokens = normalized_tokens(trimmed_query);
    let limit = limit
        .unwrap_or(DEFAULT_LIMIT as u32)
        .clamp(1, MAX_LIMIT as u32) as usize;

    let mut ranked = catalog
        .iter()
        .filter_map(|definition| {
            let score = score_tool(definition, normalized_query.as_str(), tokens.as_slice());
            (score > 0).then_some((score, definition))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_score, left_tool), (right_score, right_tool)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_tool.name.cmp(&right_tool.name))
    });

    ranked
        .into_iter()
        .take(limit)
        .map(|(_, definition)| definition.clone())
        .collect()
}

fn score_tool(definition: &SearchableTool, normalized_query: &str, tokens: &[String]) -> i32 {
    let normalized_name = normalize(definition.name.as_str());
    let normalized_description = normalize(definition.description.as_str());
    let normalized_tags = definition
        .tags
        .iter()
        .map(ToString::to_string)
        .map(normalize)
        .collect::<Vec<_>>();

    let mut score = 0;

    if normalized_name == normalized_query {
        score += 100;
    } else if normalized_name.contains(normalized_query) {
        score += 45;
    }

    if normalized_tags
        .iter()
        .any(|tag| tag == normalized_query || tag.contains(normalized_query))
    {
        score += 24;
    }

    if normalized_description.contains(normalized_query) {
        score += 20;
    }

    for token in tokens {
        if normalized_name.contains(token) {
            score += 12;
        }
        if normalized_tags.iter().any(|tag| tag.contains(token)) {
            score += 6;
        }
        if normalized_description.contains(token) {
            score += 5;
        }
    }

    score
}

fn normalize(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-'], " ")
}

fn normalized_tokens(value: &str) -> Vec<String> {
    normalize(value)
        .split_whitespace()
        .filter(|token| token.len() >= 2)
        .map(str::to_string)
        .collect()
}

fn tags_summary(definition: &SearchableTool) -> String {
    if definition.tags.is_empty() {
        return "untagged".to_string();
    }
    definition
        .tags
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
