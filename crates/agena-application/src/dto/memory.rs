#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// A current-format memory record exposed by the application.
pub struct MemoryResource {
    pub name: String,
    pub file_name: String,
    pub path: String,
    pub description: String,
    pub body: String,
    pub sha256: String,
    pub metadata: agena_domain::MemoryMetadata,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// Create a record without a revision, or update one at its observed revision.
pub struct MemoryWriteRequest {
    #[serde(default)]
    pub description: String,
    pub kind: agena_domain::MemoryKind,
    pub body: String,
    pub expected_sha256: Option<String>,
}

use super::{Deserialize, Serialize};
