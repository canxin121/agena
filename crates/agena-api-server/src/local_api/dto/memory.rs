#[derive(Debug, Clone, Serialize)]
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
pub struct MemoryWriteRequest {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub memory_type: Option<MemoryType>,
    pub body: String,
}

use super::{Deserialize, MemoryType, Serialize};
