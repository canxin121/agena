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
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub metadata: agena_domain::MemoryMetadata,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to write a memory document.
pub struct MemoryWriteRequest {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub memory_type: Option<MemoryType>,
    pub body: String,
    /// Optional for older clients; current editors send the revision they read.
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

use super::{Deserialize, MemoryType, Serialize};
