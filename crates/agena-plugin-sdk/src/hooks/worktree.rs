use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A session is creating or removing an isolated workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeHookInput {
    pub session_id: i64,
    pub cwd: PathBuf,
    pub operation: WorktreeOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WorktreeOperation {
    Create { name: String },
    Remove { path: PathBuf },
}

/// A provider returned a checkout path or handled removal. An absent reply
/// allows the host to use its default provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WorktreeHookOutput {
    Created { path: PathBuf },
    Removed,
}
