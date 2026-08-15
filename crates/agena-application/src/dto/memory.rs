#[derive(Debug, Clone, Serialize, Deserialize)]
/// A memory document as exposed by the application.
pub struct MemoryResource {
    pub name: String,
    pub file_name: String,
    pub path: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<MemoryType>,
    pub body: String,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to write a memory document.
pub struct MemoryWriteRequest {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub memory_type: Option<MemoryType>,
    pub body: String,
}

use super::{Deserialize, MemoryType, Serialize};
