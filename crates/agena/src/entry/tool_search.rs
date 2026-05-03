use crate::message::{BuiltinToolOutput, ToolSearchToolInput};

use super::{BuiltinExecution, ToolError, ToolExecutionView, ToolExecutor};

const DEFAULT_LIMIT: usize = 8;
const MAX_LIMIT: usize = 25;

#[derive(Debug, Clone)]
pub(crate) struct SearchableTool {
    pub name: String,
    pub description: String,
    pub search_terms: Vec<String>,
    pub behavior_label: String,
    pub read_only: bool,
    pub deferred: bool,
}

impl SearchableTool {
    pub(crate) fn from_definition(definition: super::EntryDefinition) -> Self {
        let behavior_label = match definition.behavior {
            super::EntryBehavior::Mutating => "mutating",
            super::EntryBehavior::ReadOnly => "read_only",
            super::EntryBehavior::Task => "task",
        }
        .to_string();
        let deferred = definition.is_deferred();
        Self {
            name: definition.name,
            description: definition.description,
            search_terms: definition.search_terms,
            behavior_label,
            read_only: definition.read_only,
            deferred,
        }
    }
}

pub(crate) fn execute(
    executor: &ToolExecutor,
    input: &ToolSearchToolInput,
) -> Result<BuiltinExecution, ToolError> {
    let catalog = executor
        .searchable_tools()
        .into_iter()
        .map(SearchableTool::from_definition)
        .collect::<Vec<_>>();
    execute_with_tools(&catalog, input)
}

pub(crate) fn execute_with_tools(
    catalog: &[SearchableTool],
    input: &ToolSearchToolInput,
) -> Result<BuiltinExecution, ToolError> {
    if input.query.trim().is_empty() && input.load.is_empty() {
        return Err(ToolError::InvalidInput(
            "tool_search requires a non-empty query or at least one tool to load".to_string(),
        ));
    }

    let results = search_catalog(catalog, input.query.as_str(), input.limit);
    let loaded_tools = resolve_requested_loads(catalog, input.load.as_slice())?;

    let mut lines = Vec::new();
    if !input.query.trim().is_empty() {
        lines.push(format!(
            "Found {} tool(s) matching '{}'.",
            results.len(),
            input.query.trim()
        ));
        for definition in &results {
            lines.push(format!(
                "- {} [{}{}]: {}",
                definition.name,
                behavior_label(definition),
                if definition.deferred {
                    ", deferred"
                } else {
                    ""
                },
                definition.description
            ));
        }
    }

    if !loaded_tools.is_empty() {
        lines.push(format!(
            "Loaded deferred tools for later turns: {}.",
            loaded_tools.join(", ")
        ));
    } else if !results.is_empty() && results.iter().any(|tool| tool.deferred) {
        lines.push(
            "Call tool_search again with the exact tool names in `load` to expose deferred tools."
                .to_string(),
        );
    }

    let output = BuiltinToolOutput::ToolSearch {
        results: results.iter().map(|tool| tool.name.clone()).collect(),
        loaded_tools,
    };

    let output_text = lines.join("\n");
    let mut view = ToolExecutionView::simple("Tool search", output_text);
    view.metadata
        .insert("matched_tools".to_string(), results.len().to_string());
    if !input.query.trim().is_empty() {
        view.metadata
            .insert("query".to_string(), input.query.trim().to_string());
    }

    Ok(BuiltinExecution::new(output, view))
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

fn resolve_requested_loads(
    catalog: &[SearchableTool],
    requested: &[String],
) -> Result<Vec<String>, ToolError> {
    let mut loaded = Vec::new();
    for requested_name in requested {
        let Some(definition) = catalog
            .iter()
            .find(|tool| tool.name.eq_ignore_ascii_case(requested_name.trim()))
        else {
            return Err(ToolError::InvalidInput(format!(
                "tool_search cannot load unknown tool '{}'",
                requested_name.trim()
            )));
        };

        if !loaded.iter().any(|name| name == &definition.name) {
            loaded.push(definition.name.clone());
        }
    }
    Ok(loaded)
}

fn score_tool(definition: &SearchableTool, normalized_query: &str, tokens: &[String]) -> i32 {
    let normalized_name = normalize(definition.name.as_str());
    let normalized_description = normalize(definition.description.as_str());
    let normalized_terms = definition
        .search_terms
        .iter()
        .map(normalize)
        .collect::<Vec<_>>();

    let mut score = 0;

    if normalized_name == normalized_query {
        score += 100;
    } else if normalized_name.contains(normalized_query) {
        score += 45;
    }

    if normalized_terms
        .iter()
        .any(|term| term == normalized_query || term.contains(normalized_query))
    {
        score += 35;
    }

    if normalized_description.contains(normalized_query) {
        score += 20;
    }

    for token in tokens {
        if normalized_name.contains(token) {
            score += 12;
        }
        if normalized_terms.iter().any(|term| term.contains(token)) {
            score += 8;
        }
        if normalized_description.contains(token) {
            score += 5;
        }
    }

    if definition.deferred {
        score += 1;
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

fn behavior_label(definition: &SearchableTool) -> &str {
    if definition.read_only {
        "read_only"
    } else {
        definition.behavior_label.as_str()
    }
}
