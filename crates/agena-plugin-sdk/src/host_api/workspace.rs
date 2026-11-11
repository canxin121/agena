//! Workspace and file access for plugins.
//!
//! The web client can list projects, browse their files and read them. A chat
//! plugin needs the same reach to answer "what is in this project". The host
//! keeps the path and network permission fence around every call, so these
//! requests are subject to the same policy a tool call would be.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostWorkspaceQueryRequest {
    pub query: HostWorkspaceQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum HostWorkspaceQuery {
    /// Every registered project, with an optional session count.
    List {
        #[serde(default)]
        include_session_count: bool,
    },
    /// One registered project.
    Get { workspace_id: i64 },
    /// One directory level of a project tree. `path` is relative to the
    /// project root; an absent path lists the root.
    Files {
        workspace_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// Read one file inside a project. Bounded by the host's read budget.
    Read {
        workspace_id: i64,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_bytes: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostWorkspaceQueryResponse {
    pub data: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_carries_its_path_and_budget() {
        let query = HostWorkspaceQuery::Read {
            workspace_id: 3,
            path: "src/main.rs".into(),
            max_bytes: Some(4096),
        };
        let encoded = serde_json::to_value(&query).unwrap();
        assert_eq!(encoded["kind"], "read");
        assert_eq!(encoded["path"], "src/main.rs");
    }

    #[test]
    fn files_without_path_lists_the_root() {
        let encoded = serde_json::to_value(HostWorkspaceQuery::Files {
            workspace_id: 1,
            path: None,
            limit: None,
        })
        .unwrap();
        assert_eq!(encoded["kind"], "files");
        assert!(encoded.get("path").is_none());
    }
}
