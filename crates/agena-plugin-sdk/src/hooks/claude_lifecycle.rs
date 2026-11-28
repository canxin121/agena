use std::path::PathBuf;

use agena_domain::{ContentId, ExecutionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedMessageDisplayInput {
    pub session_id: i64,
    pub turn_id: ExecutionId,
    pub message_id: ContentId,
    pub index: u32,
    #[serde(rename = "final")]
    pub final_batch: bool,
    pub delta: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportedConfigChangeSource {
    UserSettings,
    ProjectSettings,
    LocalSettings,
    PolicySettings,
    Skills,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedConfigChangeInput {
    pub session_id: i64,
    pub source: ImportedConfigChangeSource,
    pub file_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedDirectoryAddedInput {
    pub session_id: i64,
    pub directory: PathBuf,
    pub cwd: PathBuf,
}

/// The Claude wire representation of in-flight work at a Stop hook.
#[derive(Debug, Clone, Default)]
pub struct ImportedClaudeStopWork {
    pub background_tasks: Vec<serde_json::Value>,
    pub session_crons: Vec<serde_json::Value>,
}
