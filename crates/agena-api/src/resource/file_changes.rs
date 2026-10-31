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
    pub revision: String,
    /// Lines this path's recorded operations add and remove, merged into one
    /// pair per file. Recorded operation diffs are aggregated; this is never a
    /// synthesized net diff of the session.
    #[serde(default)]
    pub added: usize,
    #[serde(default)]
    pub removed: usize,
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

#[cfg(test)]
mod tests {
    use super::SessionFileChangeResource;

    #[test]
    fn file_change_requires_its_path_revision() {
        let current = serde_json::json!({
            "path":"src/你好.rs", "revision":"current-path-facts",
            "added":3, "removed":1, "operation_count":1,
            "operation_history":false, "operations":[],
        });
        let decoded: SessionFileChangeResource = serde_json::from_value(current.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), current);
        let mut incomplete = current;
        incomplete.as_object_mut().unwrap().remove("revision");
        assert!(serde_json::from_value::<SessionFileChangeResource>(incomplete).is_err());
    }
}
