//! Read-only projection of successful file edits recorded by one session.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionFileChangesResource {
    pub files: Vec<SessionFileChangeResource>,
    pub total_files: usize,
    pub offset: usize,
    pub has_more: bool,
    /// Shell writes are declarations, not evidence of changed files.
    pub recording_incomplete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFileChangeResource {
    /// Literal path recorded by the tool, not a current filesystem/Git lookup.
    pub path: String,
    /// Fingerprint of this path's persisted operations, independent of other
    /// files. Clients use it to keep an already displayed diff unchanged.
    #[serde(default)]
    pub revision: String,
    pub operation_count: usize,
    /// Multiple edits without a complete baseline are an operation history,
    /// never a synthesized session net diff.
    pub operation_history: bool,
    pub operations: Vec<SessionFileEditResource>,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct SessionFileEditResource {
    pub part_id: i64,
    pub tool: String,
    pub kind: String,
    pub from_path: Option<String>,
    pub before_sha256: Option<String>,
    pub after_sha256: Option<String>,
    pub diff: Option<String>,
    pub diff_truncated: bool,
    /// Patch results contain the diff for the whole operation (possibly
    /// several files), not necessarily just this row's path.
    pub diff_scope: String,
    pub diff_unavailable_reason: Option<String>,
}
