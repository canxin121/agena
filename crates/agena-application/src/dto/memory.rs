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

pub use agena_api::business::MemoryWriteRequest;

use super::{Deserialize, Serialize};
