use agena_macros::ToolInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("title"), non_empty("title"))]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionRenameToolInput {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SessionToolResponse {
    pub(crate) session: HostSession,
}
use super::{Deserialize, HostSession, JsonSchema, Serialize};
