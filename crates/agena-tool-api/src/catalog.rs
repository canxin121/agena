use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Metadata copied at the execution-catalog boundary, never an executable
/// registry handle or plugin definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogTool {
    pub name: String,
    pub canonical_name: String,
    pub plugin_id: String,
    pub summary: String,
    pub help: String,
    pub tags: Vec<String>,
    pub input_schema: Value,
    pub input_usage: Option<String>,
    pub input_examples: Vec<String>,
    pub output_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPlugin {
    pub id: String,
    pub version: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ToolApiCatalog {
    pub tools: Vec<CatalogTool>,
    pub plugins: Vec<CatalogPlugin>,
}
