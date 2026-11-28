use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Register a directory in the session that owns the selected workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterSessionWorkingDirectory {
    pub path: PathBuf,
}

/// Result captured by the same transaction that updates the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWorkingDirectoryRegistration {
    pub added: bool,
    pub directory: PathBuf,
    pub directories: Vec<PathBuf>,
}
