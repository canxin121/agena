use super::*;

use agena_macros::ToolInputShape;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("title"), non_empty("title"))]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionRenameToolInput {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SessionToolResponse {
    pub(crate) session: HostSession,
}
